//! WebSocket handler — Yjs CRDT document sync.
//!
//! Route: `GET /ws/doc/:universe_slug/:doc_id`
//!
//! Auth: JWT from `?token=` query param **or** `session` cookie.
//! Returns 401 for anonymous (no valid JWT) connections.
//!
//! # Yjs sync protocol (v1)
//!
//! All frames are binary. First varuint = message category:
//! - `0` = sync
//!   - sub-type `0` = sync-step-1 (client sends state vector → server replies with step-2)
//!   - sub-type `1` = sync-step-2 (server → client, full update)
//!   - sub-type `2` = update      (client → server → broadcast to all others)
//! - `1` = awareness              (forwarded to all others, no server-side state)
//!
//! Payload bytes in a sync frame are **length-prefixed** (lib0 `writeVarUint8Array`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio::time::{Duration, Instant, interval};
use tracing::{debug, warn};
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, ReadTxn, Text, Transact, Update};

use crate::auth::{decode_user_id, extract_session_cookie, jwt_secret};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Timing / rate constants
// ---------------------------------------------------------------------------

const PING_INTERVAL: Duration = Duration::from_secs(30);
const SILENCE_TIMEOUT: Duration = Duration::from_secs(60);
const PERSIST_DELAY: Duration = Duration::from_secs(5);
/// Token-bucket refill rate: max messages per second per client.
const RATE_LIMIT_PER_SEC: f64 = 100.0;

// ---------------------------------------------------------------------------
// Yjs protocol constants
// ---------------------------------------------------------------------------

const MSG_SYNC: u8 = 0;
const MSG_AWARENESS: u8 = 1;
const SYNC_STEP1: u8 = 0;
const SYNC_STEP2: u8 = 1;
const SYNC_UPDATE: u8 = 2;

// ---------------------------------------------------------------------------
// Binary encoding helpers (lib0-compatible varuint)
// ---------------------------------------------------------------------------

/// Read a varuint from `buf` at `*pos`, advancing `*pos`.
pub(crate) fn read_varuint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        if *pos >= buf.len() {
            return None;
        }
        let byte = buf[*pos];
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// Write a varuint to `buf`.
pub(crate) fn write_varuint(buf: &mut Vec<u8>, mut n: u64) {
    loop {
        if n < 0x80 {
            buf.push(n as u8);
            break;
        }
        buf.push((n as u8) | 0x80);
        n >>= 7;
    }
}

/// Read a length-prefixed byte slice.
pub(crate) fn read_varbytes<'a>(buf: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let len = read_varuint(buf, pos)? as usize;
    if *pos + len > buf.len() {
        return None;
    }
    let bytes = &buf[*pos..*pos + len];
    *pos += len;
    Some(bytes)
}

/// Write a length-prefixed byte slice.
pub(crate) fn write_varbytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    write_varuint(buf, bytes.len() as u64);
    buf.extend_from_slice(bytes);
}

/// Build `[MSG_SYNC, subtype, <len-prefixed payload>]`.
fn encode_sync_frame(subtype: u8, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(2 + 4 + payload.len());
    write_varuint(&mut buf, MSG_SYNC as u64);
    write_varuint(&mut buf, subtype as u64);
    write_varbytes(&mut buf, payload);
    buf
}

// ---------------------------------------------------------------------------
// Per-document room
// ---------------------------------------------------------------------------

pub struct DocRoom {
    /// Y.Doc holding shared CRDT state.
    pub doc: Arc<Mutex<Doc>>,
    /// Broadcast to all connected clients (updates, awareness).
    pub tx: broadcast::Sender<Vec<u8>>,
    /// Number of active connections.
    pub client_count: Arc<AtomicUsize>,
    /// Notified on every update — drives idle-persist debounce.
    pub dirty: Arc<tokio::sync::Notify>,
}

impl DocRoom {
    fn new(doc: Doc) -> Self {
        let (tx, _) = broadcast::channel(256);
        DocRoom {
            doc: Arc::new(Mutex::new(doc)),
            tx,
            client_count: Arc::new(AtomicUsize::new(0)),
            dirty: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

/// `"slug:path"` → `Arc<DocRoom>`.
pub type DocRoomManager = Arc<RwLock<HashMap<String, Arc<DocRoom>>>>;

pub fn new_room_manager() -> DocRoomManager {
    Arc::new(RwLock::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Token-bucket rate limiter (per connection)
// ---------------------------------------------------------------------------

struct RateLimiter {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            tokens: RATE_LIMIT_PER_SEC,
            last_refill: Instant::now(),
        }
    }

    /// Returns `true` if allowed; `false` if rate-limited.
    fn check(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;
        self.tokens = (self.tokens + elapsed * RATE_LIMIT_PER_SEC).min(RATE_LIMIT_PER_SEC);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct WsParams {
    pub token: Option<String>,
}

// ---------------------------------------------------------------------------
// HTTP handler — auth + upgrade
// ---------------------------------------------------------------------------

/// `GET /ws/doc/:slug/:doc_id`
///
/// Requires JWT in `?token=` query param or `session` cookie.
/// Returns 401 for unauthenticated requests (no WebSocket upgrade).
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Path((slug, doc_id)): Path<(String, String)>,
    Query(params): Query<WsParams>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    let token = params.token.or_else(|| extract_session_cookie(&headers));

    let user_id = match token.and_then(|t| decode_user_id(&t, &jwt_secret()).ok()) {
        Some(id) => id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };

    ws.on_upgrade(move |socket| handle_socket(socket, user_id, slug, doc_id, state))
}

// ---------------------------------------------------------------------------
// WebSocket session
// ---------------------------------------------------------------------------

async fn handle_socket(
    socket: WebSocket,
    user_id: String,
    slug: String,
    doc_id: String,
    state: AppState,
) {
    let room_key = format!("{slug}:{doc_id}");
    let room = get_or_create_room(&state, &room_key, &slug, &doc_id).await;
    room.client_count.fetch_add(1, Ordering::Relaxed);

    let broadcast_tx = room.tx.clone();
    let mut broadcast_rx = broadcast_tx.subscribe();
    let client_count = room.client_count.clone();
    let dirty = room.dirty.clone();
    let doc_arc = room.doc.clone();

    debug!("ws connect: user={user_id} room={room_key}");

    // Split socket → concurrent send + receive.
    let (mut sink, mut stream) = socket.split();

    // Outgoing message queue (avoids double-borrow of sink).
    let (send_tx, mut send_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    // Sender task: drain send_rx → sink.
    let send_task = tokio::spawn(async move {
        while let Some(msg) = send_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Send initial state to this client (sync-step-2 with full doc).
    {
        let doc = doc_arc.lock().await;
        let txn = doc.transact();
        let update = txn.encode_diff_v1(&yrs::StateVector::default());
        let frame = encode_sync_frame(SYNC_STEP2, &update);
        let _ = send_tx.send(Message::Binary(frame.into()));
    }

    // Main loop.
    let mut rate_limiter = RateLimiter::new();
    let mut ping_ticker = interval(PING_INTERVAL);
    let mut last_heard_from = Instant::now();

    loop {
        tokio::select! {
            biased;

            // Incoming from this client.
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        last_heard_from = Instant::now();
                        if !rate_limiter.check() {
                            warn!("ws rate-limit exceeded: user={user_id} room={room_key}");
                            break;
                        }
                        process_message(&data, &doc_arc, &broadcast_tx, &send_tx, &dirty).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_heard_from = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        debug!("ws recv error: {e}");
                        break;
                    }
                    _ => {}
                }
            }

            // Broadcast from other clients.
            result = broadcast_rx.recv() => {
                match result {
                    Ok(frame) => {
                        let _ = send_tx.send(Message::Binary(frame.into()));
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("ws lagged {n} messages for user={user_id}");
                    }
                    Err(_) => break,
                }
            }

            // Heartbeat.
            _ = ping_ticker.tick() => {
                if last_heard_from.elapsed() > SILENCE_TIMEOUT {
                    debug!("ws silence timeout: user={user_id}");
                    break;
                }
                let _ = send_tx.send(Message::Ping(vec![].into()));
            }
        }
    }

    // ---- Cleanup ----
    send_task.abort();
    let prev_count = client_count.fetch_sub(1, Ordering::Relaxed);
    debug!(
        "ws disconnect: user={user_id} room={room_key} remaining={}",
        prev_count.saturating_sub(1)
    );

    if prev_count == 1 {
        // Last client — save immediately and remove room.
        save_room(&doc_arc, &slug, &doc_id, &state).await;
        state.realtime.doc_rooms.write().await.remove(&room_key);
        debug!("ws room removed: {room_key}");
    } else {
        // Others still connected — mark dirty for idle-persist.
        dirty.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Message processing
// ---------------------------------------------------------------------------

async fn process_message(
    data: &[u8],
    doc_arc: &Arc<Mutex<Doc>>,
    broadcast_tx: &broadcast::Sender<Vec<u8>>,
    send_tx: &tokio::sync::mpsc::UnboundedSender<Message>,
    dirty: &Arc<tokio::sync::Notify>,
) {
    let mut pos = 0;

    let Some(msg_type) = read_varuint(data, &mut pos) else {
        return;
    };

    match msg_type as u8 {
        MSG_SYNC => {
            let Some(sync_type) = read_varuint(data, &mut pos) else {
                return;
            };
            let Some(payload) = read_varbytes(data, &mut pos) else {
                return;
            };

            match sync_type as u8 {
                SYNC_STEP1 => {
                    // Copy bytes so no yrs type is held across the .await below.
                    let sv_bytes = payload.to_vec();
                    let doc = doc_arc.lock().await;
                    let sv = match yrs::StateVector::decode_v1(&sv_bytes) {
                        Ok(sv) => sv,
                        Err(e) => {
                            warn!("ws bad state-vector: {e}");
                            return;
                        }
                    };
                    let txn = doc.transact();
                    let update = txn.encode_diff_v1(&sv);
                    drop(txn);
                    drop(doc);
                    let frame = encode_sync_frame(SYNC_STEP2, &update);
                    let _ = send_tx.send(Message::Binary(frame.into()));
                }

                SYNC_STEP2 | SYNC_UPDATE => {
                    // Copy bytes first — `Update` is !Send, must not cross .await.
                    let payload_owned = payload.to_vec();
                    {
                        let doc = doc_arc.lock().await;
                        let update = match Update::decode_v1(&payload_owned) {
                            Ok(u) => u,
                            Err(e) => {
                                warn!("ws bad update: {e}");
                                return;
                            }
                        };
                        let mut txn = doc.transact_mut();
                        if let Err(e) = txn.apply_update(update) {
                            warn!("ws apply_update failed: {e}");
                            return;
                        }
                        // update, txn, doc guard all dropped here
                    }
                    dirty.notify_one();
                    let frame = encode_sync_frame(SYNC_UPDATE, &payload_owned);
                    let _ = broadcast_tx.send(frame);
                }

                _ => {}
            }
        }

        MSG_AWARENESS => {
            // Forward verbatim — no server-side awareness state.
            let mut out = Vec::with_capacity(1 + data.len() - pos);
            write_varuint(&mut out, MSG_AWARENESS as u64);
            out.extend_from_slice(&data[pos..]);
            let _ = broadcast_tx.send(out);
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Room lifecycle
// ---------------------------------------------------------------------------

/// Return or create the `DocRoom` for `room_key`.
async fn get_or_create_room(
    state: &AppState,
    room_key: &str,
    slug: &str,
    doc_id: &str,
) -> Arc<DocRoom> {
    // Fast path.
    {
        let rooms = state.realtime.doc_rooms.read().await;
        if let Some(room) = rooms.get(room_key) {
            return Arc::clone(room);
        }
    }

    // Load existing content (empty string if entry not found).
    let content = state
        .core
        .storage
        .lock()
        .get_entry_body(slug, doc_id)
        .unwrap_or_default();

    let doc = Doc::new();
    if !content.is_empty() {
        let text = doc.get_or_insert_text("content");
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, &content);
    }

    let room = Arc::new(DocRoom::new(doc));

    // Spawn idle-persist background task.
    spawn_persist_task(
        Arc::clone(&room),
        slug.to_string(),
        doc_id.to_string(),
        room_key.to_string(),
        state.clone(),
    );

    // Write-lock and double-check (avoid racing two creates).
    let mut rooms = state.realtime.doc_rooms.write().await;
    if let Some(existing) = rooms.get(room_key) {
        return Arc::clone(existing);
    }
    rooms.insert(room_key.to_string(), Arc::clone(&room));
    room
}

fn spawn_persist_task(
    room: Arc<DocRoom>,
    slug: String,
    doc_id: String,
    room_key: String,
    state: AppState,
) {
    tokio::spawn(async move {
        loop {
            room.dirty.notified().await;

            // Debounce: wait 5 s, restarting the timer on each new notification.
            loop {
                tokio::select! {
                    () = tokio::time::sleep(PERSIST_DELAY) => break,
                    () = room.dirty.notified() => {}
                }
            }

            save_room(&room.doc, &slug, &doc_id, &state).await;

            if room.client_count.load(Ordering::Relaxed) == 0 {
                state.realtime.doc_rooms.write().await.remove(&room_key);
                break;
            }
        }
    });
}

async fn save_room(doc_arc: &Arc<Mutex<Doc>>, slug: &str, doc_id: &str, state: &AppState) {
    let content = {
        let doc = doc_arc.lock().await;
        let text = doc.get_or_insert_text("content");
        let txn = doc.transact();
        text.get_string(&txn)
    };
    let storage = state.core.storage.lock();
    if let Err(e) = storage.update_entry_body(slug, doc_id, &content) {
        warn!("ws persist failed for {slug}/{doc_id}: {e}");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- varuint ---

    #[test]
    fn test_varuint_roundtrip() {
        for &n in &[0u64, 1, 127, 128, 255, 16383, 16384, u32::MAX as u64] {
            let mut buf = Vec::new();
            write_varuint(&mut buf, n);
            let mut pos = 0;
            let decoded = read_varuint(&buf, &mut pos).unwrap();
            assert_eq!(decoded, n, "roundtrip failed for {n}");
            assert_eq!(pos, buf.len(), "did not consume all bytes for {n}");
        }
    }

    // --- varbytes ---

    #[test]
    fn test_varbytes_roundtrip() {
        let payload = b"hello crdt world";
        let mut buf = Vec::new();
        write_varbytes(&mut buf, payload);
        let mut pos = 0;
        let decoded = read_varbytes(&buf, &mut pos).unwrap();
        assert_eq!(decoded, payload);
        assert_eq!(pos, buf.len());
    }

    // --- sync frame structure ---

    #[test]
    fn test_encode_sync_frame_structure() {
        let payload = &[0xAA, 0xBB, 0xCC];
        let frame = encode_sync_frame(SYNC_STEP2, payload);

        let mut pos = 0;
        assert_eq!(read_varuint(&frame, &mut pos).unwrap(), MSG_SYNC as u64);
        assert_eq!(read_varuint(&frame, &mut pos).unwrap(), SYNC_STEP2 as u64);
        let decoded = read_varbytes(&frame, &mut pos).unwrap();
        assert_eq!(decoded, payload);
        assert_eq!(pos, frame.len());
    }

    // --- rate limiter ---

    #[test]
    fn test_rate_limiter_burst_then_block() {
        let mut rl = RateLimiter::new();
        for _ in 0..100 {
            assert!(rl.check(), "should allow within budget");
        }
        assert!(!rl.check(), "should deny when budget exhausted");
    }

    // --- DocRoom init ---

    #[test]
    fn test_doc_room_initialises_content() {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("content");
        {
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, 0, "# Hello");
        }
        let room = DocRoom::new(doc);
        assert_eq!(room.client_count.load(Ordering::Relaxed), 0);

        let doc = room.doc.blocking_lock();
        let text = doc.get_or_insert_text("content");
        let txn = doc.transact();
        assert_eq!(text.get_string(&txn), "# Hello");
    }

    // --- anonymous → 401 ---
    //
    // WebSocketUpgrade requires a real Hyper connection, so we bind to 127.0.0.1:0.

    #[tokio::test]
    async fn test_anonymous_ws_returns_401() {
        use std::sync::Mutex as StdMutex;

        use crate::auth::AuthStore;
        use crate::config::WebConfig;
        use crate::experiment::ExperimentStore;
        use crate::server::{
            AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
        };
        use crate::storage::Storage;
        use tokio::net::TcpListener;
        use tokio_tungstenite::connect_async;

        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Storage::new(tmp.path());
        let experiment = ExperimentStore::new(tmp.path());
        let auth_store = AuthStore::new(tmp.path()).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db = tmp.path().join("game.db");
        let game_storage = Arc::new(game_core::storage::Storage::open(&game_db).unwrap());

        let config = WebConfig {
            port: 0,
            data_dir: tmp.path().to_string_lossy().into(),
            static_dir: "co-web/static".into(),
            default_variant: "a".into(),
            experiments: false,
            plugins_dir: "plugins".into(),
            game_db_path: None,
            universo_dir: tmp.path().join("universes").to_string_lossy().into(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
            staging: false,
            staging_latency_ms: 50,
            staging_error_rate: 0.05,
        };
        let state: crate::server::AppState = AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx: {
                    let (tx, _) = crate::embedding_worker::channel();
                    tx
                },
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: Default::default(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: StdMutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });

        let app = build_router(state, None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Connect without a token — server must reject with non-101.
        let url = format!("ws://127.0.0.1:{port}/ws/doc/test-universe/README.md");
        let result = connect_async(&url).await;
        assert!(
            result.is_err(),
            "anonymous WS connection should be rejected (401)"
        );
        // tokio-tungstenite surfaces 401 as a tungstenite error.
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("401") || err.contains("HTTP error"),
            "expected 401 in error, got: {err}"
        );
    }

    // --- two logged-in users sync ---

    #[tokio::test]
    async fn test_two_users_sync() {
        use std::sync::Mutex as StdMutex;

        use crate::auth::{AuthStore, sign_jwt};
        use crate::config::WebConfig;
        use crate::experiment::ExperimentStore;
        use crate::server::{
            AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
        };
        use crate::storage::Storage;
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio::net::TcpListener;
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as WsMsg;

        // Safety: tests run in a single process. All co-web tests MUST use
        // "test-jwt-secret" to avoid parallel-test JWT clobbering (see CO-295).
        unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };

        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Storage::new(tmp.path());
        let experiment = ExperimentStore::new(tmp.path());
        let auth_store = AuthStore::new(tmp.path()).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db = tmp.path().join("game.db");
        let game_storage = Arc::new(game_core::storage::Storage::open(&game_db).unwrap());

        let config = WebConfig {
            port: 0,
            data_dir: tmp.path().to_string_lossy().into(),
            static_dir: "co-web/static".into(),
            default_variant: "a".into(),
            experiments: false,
            plugins_dir: "plugins".into(),
            game_db_path: None,
            universo_dir: tmp.path().join("universes").to_string_lossy().into(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
            staging: false,
            staging_latency_ms: 50,
            staging_error_rate: 0.05,
        };
        let state: crate::server::AppState = AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx: {
                    let (tx, _) = crate::embedding_worker::channel();
                    tx
                },
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: Default::default(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: StdMutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });

        let app = build_router(state.clone(), None);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Must match the JWT_SECRET set at line ~724 (CO-295: all co-web tests use "test-jwt-secret").
        let secret = "test-jwt-secret";
        let (tok1, _) = sign_jwt("user1", "a@test.com", "free", secret).unwrap();
        let (tok2, _) = sign_jwt("user2", "b@test.com", "free", secret).unwrap();

        let base = format!("ws://127.0.0.1:{port}/ws/doc/test-slug/test.md");

        // Connect client 1 — receives initial sync-step-2.
        let (mut ws1, _) = connect_async(format!("{base}?token={tok1}")).await.unwrap();
        let init1 = ws1.next().await.unwrap().unwrap();
        assert!(
            matches!(init1, WsMsg::Binary(_)),
            "expected initial sync-step-2"
        );

        // Connect client 2.
        let (mut ws2, _) = connect_async(format!("{base}?token={tok2}")).await.unwrap();
        let _init2 = ws2.next().await.unwrap().unwrap();

        // Client 1 sends a sync-update.
        let update_bytes = {
            let doc = Doc::new();
            let text = doc.get_or_insert_text("content");
            {
                let mut txn = doc.transact_mut();
                text.insert(&mut txn, 0, "hello from user1");
            }
            let txn = doc.transact();
            txn.encode_diff_v1(&yrs::StateVector::default())
        };
        let frame = encode_sync_frame(SYNC_UPDATE, &update_bytes);
        ws1.send(WsMsg::Binary(frame.into())).await.unwrap();

        // Client 2 must receive the update within 2 s.
        // Skip over any WebSocket control frames (Ping/Pong) — only accept Binary.
        let data = loop {
            let msg2 = tokio::time::timeout(Duration::from_secs(2), ws2.next())
                .await
                .expect("timeout waiting for sync-update")
                .unwrap()
                .unwrap();
            match msg2 {
                WsMsg::Binary(b) => break b,
                WsMsg::Ping(_) | WsMsg::Pong(_) => continue,
                other => panic!("expected Binary, got {other:?}"),
            }
        };
        let mut pos = 0;
        assert_eq!(read_varuint(&data, &mut pos).unwrap(), MSG_SYNC as u64);
        let sync_type = read_varuint(&data, &mut pos).unwrap();
        assert!(
            sync_type == SYNC_UPDATE as u64 || sync_type == SYNC_STEP2 as u64,
            "unexpected sync_type {sync_type}"
        );

        // Anonymous user: edit locally → no WS → login activates CRDT.
        // (Enforced by the 401 gate in ws_handler; covered by test_anonymous_ws_returns_401.)

        ws1.close(None).await.ok();
        ws2.close(None).await.ok();
    }
}
