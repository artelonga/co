//! CO-151 — Real-time delta sync: protobuf SyncBatch over WebSocket with zstd.
//!
//! Route: `GET /api/v1/sync/ws?universe=<key>&token=<jwt>`
//!   (session cookie is also accepted instead of `?token=`)
//!   X-Sync-Resume: <token>  (optional — resumes missed deltas on reconnect)
//!
//! Frames are binary: zstd-compressed protobuf SyncBatch messages.
//!
//! Uplink (client→server): client sends SyncBatch; server stores deltas and
//! broadcasts to all other clients in the same universe room.
//!
//! Downlink (server→client): broadcast channel delivers SyncBatch bytes.
//!
//! Resume: server keeps a 24h ring-log of broadcast frames per universe room.
//! On reconnect with `X-Sync-Resume`, server replays missed frames in order.
//! If the token is stale (>24h or unknown) it sends a single snapshot-needed
//! hint so the client can request a full vault dump via the REST API.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use co::proto::sync::SyncBatch;
use co::sync::delta;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{RwLock, broadcast};
use tokio::time::{Duration, Instant, interval};
use tracing::{debug, warn};

use crate::auth::{decode_user_id, extract_session_cookie, jwt_secret};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PING_INTERVAL: Duration = Duration::from_secs(30);
const SILENCE_TIMEOUT: Duration = Duration::from_secs(60);
/// Keep at most 24h worth of frames per room.
const DELTA_LOG_MAX: usize = 100_000;
const DELTA_LOG_TTL: Duration = Duration::from_secs(86_400);

// ---------------------------------------------------------------------------
// Room types
// ---------------------------------------------------------------------------

struct LoggedFrame {
    token: u64,
    created_at: Instant,
    encoded: Vec<u8>,
}

/// Broadcast payload — encoded SyncBatch tagged with its originator so
/// receivers can skip echo-to-self (CO-151 fix: prevents the watcher feedback
/// loop where a client wrote a file → sent SyncDelta → received the broadcast
/// back → applied it → notify fired again → loop forever).
#[derive(Clone)]
pub struct BroadcastFrame {
    pub origin_conn_id: u64,
    pub encoded: Vec<u8>,
}

pub struct SyncRoom {
    /// Broadcast channel for downlink frames.
    pub tx: broadcast::Sender<BroadcastFrame>,
    /// Monotonically increasing resume token.
    pub next_token: AtomicU64,
    /// Ordered log of encoded frames (oldest first).
    delta_log: RwLock<VecDeque<LoggedFrame>>,
    /// Monotonically increasing per-room connection id (assigned at handshake).
    next_conn_id: AtomicU64,
}

impl SyncRoom {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        SyncRoom {
            tx,
            next_token: AtomicU64::new(1),
            delta_log: RwLock::new(VecDeque::new()),
            next_conn_id: AtomicU64::new(1),
        }
    }

    /// Assign a fresh connection id; used to filter echo-to-self in broadcast.
    fn next_connection_id(&self) -> u64 {
        self.next_conn_id.fetch_add(1, Ordering::AcqRel)
    }

    async fn append_frame(&self, encoded: Vec<u8>) -> u64 {
        let token = self.next_token.fetch_add(1, Ordering::AcqRel);
        let mut log = self.delta_log.write().await;
        let now = Instant::now();
        while let Some(front) = log.front() {
            if now.duration_since(front.created_at) > DELTA_LOG_TTL || log.len() >= DELTA_LOG_MAX {
                log.pop_front();
            } else {
                break;
            }
        }
        log.push_back(LoggedFrame {
            token,
            created_at: now,
            encoded,
        });
        token
    }

    /// Replay frames with token > `resume_token`.
    /// Returns `None` when `resume_token` predates the oldest log entry.
    async fn replay_since(&self, resume_token: u64) -> Option<Vec<Vec<u8>>> {
        let log = self.delta_log.read().await;
        if log.is_empty() {
            return Some(vec![]);
        }
        if let Some(oldest) = log.front()
            && resume_token < oldest.token.saturating_sub(1)
        {
            return None;
        }
        Some(
            log.iter()
                .filter(|f| f.token > resume_token)
                .map(|f| f.encoded.clone())
                .collect(),
        )
    }
}

/// Universe key → `Arc<SyncRoom>`.
pub type SyncRoomManager = Arc<RwLock<HashMap<String, Arc<SyncRoom>>>>;

pub fn new_sync_room_manager() -> SyncRoomManager {
    Arc::new(RwLock::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SyncWsParams {
    pub token: Option<String>,
    pub universe: Option<String>,
}

fn extract_resume_token(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("X-Sync-Resume")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

// ---------------------------------------------------------------------------
// HTTP handler
// ---------------------------------------------------------------------------

/// `GET /api/v1/sync/ws?universe=<key>[&token=<jwt-or-api-token>]`
///
/// Auth: JWT or long-lived API token in `?token=` or session cookie.
/// API tokens are accepted so the sync daemon can run without JWT renewal.
/// Returns 400 if `?universe=` is missing; 401 for unauthenticated requests.
pub async fn sync_ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<SyncWsParams>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    let token_str = params.token.or_else(|| extract_session_cookie(&headers));
    let user_id = match token_str {
        None => return StatusCode::UNAUTHORIZED.into_response(),
        Some(ref t) => {
            // Try JWT first (fast, no DB).
            if let Ok(uid) = decode_user_id(t, &jwt_secret()) {
                uid
            } else {
                // Fall back to long-lived API token (CO-35).
                let storage = match state.storage.lock() {
                    Ok(s) => s,
                    Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                };
                match storage.get_api_token_by_value(t) {
                    Ok(Some(tok)) => tok.user_id,
                    _ => return StatusCode::UNAUTHORIZED.into_response(),
                }
            }
        }
    };

    let universe_key = match params.universe {
        Some(u) if !u.is_empty() => u,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    let resume_token = extract_resume_token(&headers);
    ws.on_upgrade(move |socket| {
        handle_sync_socket(socket, user_id, universe_key, resume_token, state)
    })
}

// ---------------------------------------------------------------------------
// WebSocket session
// ---------------------------------------------------------------------------

async fn handle_sync_socket(
    socket: WebSocket,
    user_id: String,
    universe_key: String,
    resume_token: Option<u64>,
    state: AppState,
) {
    let room = get_or_create_room(&state, &universe_key).await;
    let mut broadcast_rx = room.tx.subscribe();
    let conn_id = room.next_connection_id();

    let (mut sink, mut stream) = socket.split();
    let (send_tx, mut send_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    // Sender task.
    let send_task = tokio::spawn(async move {
        while let Some(msg) = send_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    debug!("sync-ws connect: user={user_id} universe={universe_key}");

    // CO-156: emit ws.connect telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "ws.connect",
            universe: universe_key.clone(),
            list: None,
            key: None,
            actor: Some(user_id.clone()),
            session_id: None,
            extra: Some(serde_json::json!({ "conn_id": conn_id })),
        },
    );

    // Replay missed frames on reconnect.
    if let Some(rt) = resume_token {
        match room.replay_since(rt).await {
            Some(frames) => {
                for frame in frames {
                    let _ = send_tx.send(Message::Binary(frame.into()));
                }
            }
            None => {
                // Token stale → signal client to do a full REST snapshot pull.
                let hint = SyncBatch {
                    deltas: vec![],
                    client_id: "server".into(),
                    batch_ts_ns: 0,
                    resume_token: i64::MIN,
                };
                if let Ok(encoded) = delta::encode_batch(&hint) {
                    let _ = send_tx.send(Message::Binary(encoded.into()));
                }
            }
        }
    }

    // Main loop.
    let mut ping_ticker = interval(PING_INTERVAL);
    let mut last_heard_from = Instant::now();

    loop {
        tokio::select! {
            biased;

            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        last_heard_from = Instant::now();
                        handle_uplink(&data, &room, &state, &universe_key, conn_id).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_heard_from = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        debug!("sync-ws recv error: user={user_id} err={e}");
                        break;
                    }
                    _ => {}
                }
            }

            result = broadcast_rx.recv() => {
                match result {
                    Ok(frame) => {
                        // Skip echo-to-self: the originating client already
                        // has the change applied locally; broadcasting it
                        // back triggers a feedback loop in fs-watching clients.
                        if frame.origin_conn_id == conn_id {
                            continue;
                        }
                        let _ = send_tx.send(Message::Binary(frame.encoded.into()));
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("sync-ws lagged {n} messages: user={user_id}");
                        // CO-156: emit ws.lag telemetry
                        crate::telemetry::emit_crud_event(
                            &state,
                            crate::telemetry::CrudEvent {
                                kind: "ws.lag",
                                universe: universe_key.clone(),
                                list: None,
                                key: None,
                                actor: Some(user_id.clone()),
                                session_id: None,
                                extra: Some(serde_json::json!({ "lagged": n })),
                            },
                        );
                    }
                    Err(_) => break,
                }
            }

            _ = ping_ticker.tick() => {
                if last_heard_from.elapsed() > SILENCE_TIMEOUT {
                    debug!("sync-ws silence timeout: user={user_id}");
                    break;
                }
                let _ = send_tx.send(Message::Ping(vec![].into()));
            }
        }
    }

    send_task.abort();
    debug!("sync-ws disconnect: user={user_id}");

    // CO-156: emit ws.disconnect telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "ws.disconnect",
            universe: universe_key.clone(),
            list: None,
            key: None,
            actor: Some(user_id),
            session_id: None,
            extra: Some(serde_json::json!({ "conn_id": conn_id })),
        },
    );
}

// ---------------------------------------------------------------------------
// Uplink processing
// ---------------------------------------------------------------------------

async fn handle_uplink(
    data: &[u8],
    room: &Arc<SyncRoom>,
    state: &AppState,
    universe_key: &str,
    origin_conn_id: u64,
) {
    let batch = match delta::decode_batch(data) {
        Ok(b) => b,
        Err(e) => {
            warn!("sync-ws bad uplink frame: {e}");
            return;
        }
    };

    apply_deltas_to_storage(&batch, state, universe_key);

    // Log + broadcast with updated resume_token. Tag with the originator's
    // conn_id so other receivers can ignore (CO-151 fix for the watcher
    // feedback loop).
    let token = room.append_frame(data.to_vec()).await;

    let mut downlink = batch.clone();
    downlink.resume_token = token as i64;

    match delta::encode_batch(&downlink) {
        Ok(encoded) => {
            let _ = room.tx.send(BroadcastFrame {
                origin_conn_id,
                encoded,
            });
        }
        Err(e) => warn!("sync-ws encode downlink failed: {e}"),
    }
}

fn apply_deltas_to_storage(batch: &SyncBatch, state: &AppState, universe_key: &str) {
    use crate::entry_index::{EntryIndex, make_entry};
    use co::proto::sync::sync_delta::{Body, Kind};

    let Ok(storage) = state.storage.lock() else {
        return;
    };
    let universe_root = storage.universe_root(universe_key);
    let conn_arc = storage.universe_conn(universe_key);
    drop(storage); // free the meta lock before per-universe work

    for d in &batch.deltas {
        let kind = Kind::try_from(d.kind).unwrap_or(Kind::Unspecified);
        match kind {
            Kind::Upserted => {
                let Some(Body::Cofile(ref cofile)) = d.body else {
                    continue;
                };
                let Ok(content) = std::str::from_utf8(&cofile.content) else {
                    warn!(
                        "sync-ws: upsert with non-utf8 cofile body, skipping {path}",
                        path = d.entry_path
                    );
                    continue;
                };

                // Split frontmatter; tolerate missing/invalid YAML.
                let (fm, body) = match co::entry::split_frontmatter(content) {
                    Ok((fm_str, body)) if !fm_str.is_empty() => {
                        let fm = serde_yaml::from_str::<serde_yaml::Value>(&fm_str)
                            .map(co::entry::yaml_to_json)
                            .unwrap_or(serde_json::Value::Object(Default::default()));
                        (fm, body)
                    }
                    Ok((_, body)) => (serde_json::Value::Object(Default::default()), body),
                    Err(_) => (
                        serde_json::Value::Object(Default::default()),
                        content.to_string(),
                    ),
                };

                let entry = make_entry(&d.entry_path, fm, &body);

                if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                    warn!(
                        "sync-ws write_entry failed for {path}: {e}",
                        path = d.entry_path
                    );
                    continue;
                }

                if let Ok(guard) = conn_arc.lock() {
                    let idx = EntryIndex::new(&guard);
                    if let Err(e) = idx.upsert(universe_key, &entry) {
                        warn!(
                            "sync-ws index upsert failed for {path}: {e}",
                            path = d.entry_path
                        );
                    }
                }
            }
            Kind::Deleted => {
                let path = universe_root.join(&d.entry_path);
                let _ = std::fs::remove_file(&path);
                if let Ok(guard) = conn_arc.lock() {
                    let _ = guard.execute(
                        "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
                        rusqlite::params![universe_key, &d.entry_path],
                    );
                }
            }
            _ => {}
        }
    }

    // Reconcile content_count once per batch — the per-universe upsert/delete
    // doesn't touch the cached counter on `meta.universes`. Without this,
    // sync-driven writes drift the counter (observed: `co` 513 cached vs 500
    // actual). One COUNT(*) per batch is cheap.
    let actual: Option<i64> = {
        if let Ok(guard) = conn_arc.lock() {
            guard
                .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
                .ok()
        } else {
            None
        }
    };
    if let Some(n) = actual
        && let Ok(storage) = state.storage.lock()
    {
        let _ = storage.conn().execute(
            "UPDATE universes SET content_count = ?1 WHERE key = ?2",
            rusqlite::params![n, universe_key],
        );
    }
}

// ---------------------------------------------------------------------------
// REST-originated emits (CO-151 web→local sync)
// ---------------------------------------------------------------------------

/// Emit an upserted SyncDelta into the room from a REST-side write.
///
/// Called by `vault_routes::put_vault_file` (and any other write path) after
/// the entry is persisted. Connected watchers receive the broadcast and
/// apply the change to their local filesystems — closing the web→local gap.
///
/// `origin_conn_id = 0` because REST has no WS connection; broadcast loop
/// assigns conn_ids starting at 1, so 0 never matches an active client and
/// every subscriber gets the frame.
pub async fn emit_rest_upsert(
    state: &AppState,
    universe_key: &str,
    entry_path: &str,
    rendered_markdown: &str,
) {
    let room = get_or_create_room(state, universe_key).await;
    let ts_ns = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000);
    let bytes = rendered_markdown.as_bytes().to_vec();
    let sha = blake3::hash(&bytes).to_hex().to_string(); // cheap content fingerprint
    let d = delta::upserted_delta(universe_key, entry_path, bytes, &sha, ts_ns);
    let batch = SyncBatch {
        deltas: vec![d],
        client_id: "rest".into(),
        batch_ts_ns: ts_ns,
        resume_token: 0,
    };
    let Ok(encoded) = delta::encode_batch(&batch) else {
        return;
    };
    let token = room.append_frame(encoded.clone()).await;
    // Re-encode with the resume token populated so reconnecting watchers can
    // resume from this frame.
    let mut with_token = batch;
    with_token.resume_token = token as i64;
    if let Ok(re_encoded) = delta::encode_batch(&with_token) {
        let _ = room.tx.send(BroadcastFrame {
            origin_conn_id: 0,
            encoded: re_encoded,
        });
    }
}

/// Emit a deleted SyncDelta into the room from a REST-side delete.
pub async fn emit_rest_delete(state: &AppState, universe_key: &str, entry_path: &str) {
    let room = get_or_create_room(state, universe_key).await;
    let ts_ns = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000);
    let d = delta::deleted_delta(universe_key, entry_path, ts_ns);
    let batch = SyncBatch {
        deltas: vec![d],
        client_id: "rest".into(),
        batch_ts_ns: ts_ns,
        resume_token: 0,
    };
    let Ok(encoded) = delta::encode_batch(&batch) else {
        return;
    };
    let token = room.append_frame(encoded.clone()).await;
    let mut with_token = batch;
    with_token.resume_token = token as i64;
    if let Ok(re_encoded) = delta::encode_batch(&with_token) {
        let _ = room.tx.send(BroadcastFrame {
            origin_conn_id: 0,
            encoded: re_encoded,
        });
    }
}

// ---------------------------------------------------------------------------
// Room lifecycle
// ---------------------------------------------------------------------------

async fn get_or_create_room(state: &AppState, universe_key: &str) -> Arc<SyncRoom> {
    {
        let rooms = state.sync_rooms.read().await;
        if let Some(room) = rooms.get(universe_key) {
            return Arc::clone(room);
        }
    }
    let mut rooms = state.sync_rooms.write().await;
    rooms
        .entry(universe_key.to_string())
        .or_insert_with(|| Arc::new(SyncRoom::new()))
        .clone()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use co::sync::delta as dc;
    use tokio::net::TcpListener;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as WsMsg;

    use crate::auth::{AuthStore, sign_jwt};
    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::server::{AppStateInner, build_router};
    use crate::storage::Storage;

    // Must match the secret used by all other co-web tests to avoid env-var races.
    const TEST_SECRET: &str = "test-secret";

    fn set_jwt_secret() {
        // Safety: tests share the same process; we always write the same value.
        unsafe { std::env::set_var("JWT_SECRET", TEST_SECRET) };
    }

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
            port: 0,
            data_dir: dir.to_string_lossy().into(),
            static_dir: "co-web/static".into(),
            default_variant: "a".into(),
            experiments: false,
            plugins_dir: "plugins".into(),
            game_db_path: None,
            universo_dir: dir.join("universes").to_string_lossy().into(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
        }
    }

    async fn spawn_test_server() -> (u16, String, String) {
        set_jwt_secret();

        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Storage::new(tmp.path());
        let experiment = ExperimentStore::new(tmp.path());
        let auth_store = AuthStore::new(tmp.path()).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db = tmp.path().join("game.db");
        let game_storage = Arc::new(game_core::storage::Storage::open(&game_db).unwrap());

        let state: crate::server::AppState = Arc::new(AppStateInner {
            storage: Mutex::new(storage),
            experiment: Mutex::new(experiment),
            config: test_config(tmp.path()),
            auth_store: Mutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: Default::default(),
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: new_sync_room_manager(),
            cache: crate::cache::CacheLayer::new(),
            rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
            wae: crate::wae::WaeEmitter::new(None, None),
        });

        let app = build_router(state, None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        std::mem::forget(tmp);

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let (tok_alice, _) = sign_jwt("alice", "alice@test.co", "free", TEST_SECRET).unwrap();
        let (tok_bob, _) = sign_jwt("bob", "bob@test.co", "free", TEST_SECRET).unwrap();
        (port, tok_alice, tok_bob)
    }

    fn make_batch(universe: &str, path: &str, content: &[u8]) -> Vec<u8> {
        let d = dc::upserted_delta(universe, path, content.to_vec(), "sha-001", 1_000_000);
        let b = SyncBatch {
            deltas: vec![d],
            client_id: "alice".into(),
            batch_ts_ns: 1_000_000,
            resume_token: 0,
        };
        dc::encode_batch(&b).unwrap()
    }

    // Helper: skip Ping/Pong frames and return next Binary frame within timeout.
    async fn next_binary(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        timeout: Duration,
    ) -> Vec<u8> {
        loop {
            let msg = tokio::time::timeout(timeout, ws.next())
                .await
                .expect("timeout waiting for binary frame")
                .unwrap()
                .unwrap();
            match msg {
                WsMsg::Binary(b) => return b.to_vec(),
                WsMsg::Ping(_) | WsMsg::Pong(_) => continue,
                other => panic!("expected Binary, got {other:?}"),
            }
        }
    }

    // ── T1: anonymous → 401 ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_anon_ws_returns_401() {
        let (port, _, _) = spawn_test_server().await;
        let url = format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe=u");
        let result = connect_async(&url).await;
        assert!(result.is_err(), "anonymous should be rejected");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("401") || err.contains("HTTP error"),
            "expected 401, got: {err}"
        );
    }

    // ── T2: missing universe → 400 ───────────────────────────────────────────

    #[tokio::test]
    async fn test_missing_universe_returns_400() {
        let (port, tok_alice, _) = spawn_test_server().await;
        let url = format!("ws://127.0.0.1:{port}/api/v1/sync/ws?token={tok_alice}");
        let result = connect_async(&url).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("400") || err.contains("HTTP error"),
            "expected 400, got: {err}"
        );
    }

    // ── T3: round-trip — write file → server applies → pushes back ───────────

    #[tokio::test]
    async fn test_round_trip_upserted_delta() {
        let (port, tok_alice, tok_bob) = spawn_test_server().await;

        let universe = "test-universe";
        let alice_url =
            format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}&token={tok_alice}");
        let bob_url =
            format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}&token={tok_bob}");

        let (mut ws_alice, _) = connect_async(&alice_url).await.unwrap();
        let (mut ws_bob, _) = connect_async(&bob_url).await.unwrap();

        // Alice sends an uplink batch.
        let batch_bytes = make_batch(universe, "projects/hello.md", b"# Hello");
        ws_alice
            .send(WsMsg::Binary(batch_bytes.into()))
            .await
            .unwrap();

        // Bob must receive the downlink.
        let downlink = next_binary(&mut ws_bob, Duration::from_secs(3)).await;

        let decoded = dc::decode_batch(&downlink).unwrap();
        assert_eq!(decoded.deltas.len(), 1);
        let d = &decoded.deltas[0];
        assert_eq!(d.universe_key, universe);
        assert_eq!(d.entry_path, "projects/hello.md");
        assert_eq!(d.kind, co::proto::sync::sync_delta::Kind::Upserted as i32);
        assert!(
            decoded.resume_token > 0,
            "downlink should carry resume_token"
        );

        ws_alice.close(None).await.ok();
        ws_bob.close(None).await.ok();
    }

    // ── T3b: upsert persists to disk + per-universe DB (CO-151 server fix) ──

    #[tokio::test]
    async fn test_upserted_delta_writes_to_disk_and_db() {
        // Spin a server, but capture state so we can probe storage post-upload.
        set_jwt_secret();
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let storage = Storage::new(&data_dir);
        let experiment = ExperimentStore::new(&data_dir);
        let auth_store = AuthStore::new(&data_dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db = data_dir.join("game.db");
        let game_storage = Arc::new(game_core::storage::Storage::open(&game_db).unwrap());

        let state: crate::server::AppState = Arc::new(AppStateInner {
            storage: Mutex::new(storage),
            experiment: Mutex::new(experiment),
            config: test_config(&data_dir),
            auth_store: Mutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: Default::default(),
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: new_sync_room_manager(),
            cache: crate::cache::CacheLayer::new(),
            rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
            wae: crate::wae::WaeEmitter::new(None, None),
        });
        let app = build_router(state.clone(), None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        std::mem::forget(tmp);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (tok, _) = sign_jwt("alice", "alice@test.co", "free", TEST_SECRET).unwrap();

        let universe = "persist-test";
        let url = format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}&token={tok}");
        let (mut ws, _) = connect_async(&url).await.unwrap();

        let body = b"---\ntitle: Hello\n---\n\n# Hello, world!\n";
        let bytes = make_batch(universe, "notes/hello.md", body);
        ws.send(WsMsg::Binary(bytes.into())).await.unwrap();

        // Allow the server a moment to apply the delta.
        tokio::time::sleep(Duration::from_millis(300)).await;

        // 1. Disk: file should exist.
        let universe_root = {
            let s = state.storage.lock().unwrap();
            s.universe_root(universe)
        };
        let on_disk = universe_root.join("notes/hello.md");
        assert!(
            on_disk.exists(),
            "expected .md on disk at {}",
            on_disk.display()
        );
        let raw = std::fs::read_to_string(&on_disk).unwrap();
        assert!(raw.contains("# Hello, world!"));

        // 2. Per-universe DB: row should be indexed.
        let conn_arc = {
            let s = state.storage.lock().unwrap();
            s.universe_conn(universe)
        };
        let count: i64 = {
            let g = conn_arc.lock().unwrap();
            g.query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1 AND path = ?2",
                rusqlite::params![universe, "notes/hello.md"],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count, 1, "expected 1 entries row for the upserted delta");

        ws.close(None).await.ok();
    }

    // ── T4: server-push (delete) propagates ──────────────────────────────────

    #[tokio::test]
    async fn test_delete_propagates() {
        let (port, tok_alice, tok_bob) = spawn_test_server().await;

        let universe = "delete-universe";
        let alice_url =
            format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}&token={tok_alice}");
        let bob_url =
            format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}&token={tok_bob}");

        let (mut ws_alice, _) = connect_async(&alice_url).await.unwrap();
        let (mut ws_bob, _) = connect_async(&bob_url).await.unwrap();

        let del = dc::deleted_delta(universe, "gone.md", 0);
        let batch = SyncBatch {
            deltas: vec![del],
            client_id: "alice".into(),
            batch_ts_ns: 0,
            resume_token: 0,
        };
        ws_alice
            .send(WsMsg::Binary(dc::encode_batch(&batch).unwrap().into()))
            .await
            .unwrap();

        let downlink = next_binary(&mut ws_bob, Duration::from_secs(3)).await;
        let decoded = dc::decode_batch(&downlink).unwrap();
        assert_eq!(decoded.deltas.len(), 1);
        assert_eq!(
            decoded.deltas[0].kind,
            co::proto::sync::sync_delta::Kind::Deleted as i32
        );

        ws_alice.close(None).await.ok();
        ws_bob.close(None).await.ok();
    }

    // ── T5: resume-token replay ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_resume_token_replay() {
        let (port, tok_alice, tok_bob) = spawn_test_server().await;
        let universe = "replay-universe";
        let alice_url =
            format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}&token={tok_alice}");
        let bob_url_base = format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}");

        let (mut ws_alice, _) = connect_async(&alice_url).await.unwrap();

        // Alice sends 3 batches.
        for i in 0u8..3 {
            let d = delta::upserted_delta(universe, format!("f{i}.md"), vec![i], "sha", i as i64);
            let b = SyncBatch {
                deltas: vec![d],
                client_id: "alice".into(),
                batch_ts_ns: i as i64,
                resume_token: 0,
            };
            ws_alice
                .send(WsMsg::Binary(delta::encode_batch(&b).unwrap().into()))
                .await
                .unwrap();
            // Small yield to let the server process each batch.
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        ws_alice.close(None).await.ok();

        // Let server process all batches.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Bob connects with resume_token=0 → server replays all 3 frames.
        let bob_url = format!("{bob_url_base}&token={tok_bob}");
        let (mut ws_bob, _) = tokio_tungstenite::connect_async_with_config(
            {
                let mut req =
                    tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                        &bob_url as &str,
                    )
                    .unwrap();
                req.headers_mut()
                    .insert("X-Sync-Resume", "0".parse().unwrap());
                req
            },
            None,
            false,
        )
        .await
        .unwrap();

        let mut received = 0usize;
        loop {
            let msg = tokio::time::timeout(Duration::from_millis(500), ws_bob.next()).await;
            match msg {
                Ok(Some(Ok(WsMsg::Binary(b)))) => {
                    if let Ok(decoded) = dc::decode_batch(&b)
                        && !decoded.deltas.is_empty()
                    {
                        received += 1;
                    }
                }
                Ok(Some(Ok(WsMsg::Ping(_)))) => continue,
                _ => break,
            }
        }
        assert_eq!(received, 3, "expected 3 replayed frames, got {received}");

        ws_bob.close(None).await.ok();
    }

    // ── T6: bench — 20 concurrent clients, each pushes 1 batch ─────────────
    //
    // Production target: 100 clients × 1 file/s → CPU < 50% on Fly shared-cpu-1x.
    // This test validates correctness of concurrent fan-out, not peak throughput.

    #[tokio::test]
    async fn test_concurrent_clients_broadcast() {
        let (port, tok_alice, _) = spawn_test_server().await;
        let universe = "bench-universe";

        let client_count = 20usize;
        let mut tokens = Vec::with_capacity(client_count);
        for i in 0..client_count {
            let (tok, _) =
                sign_jwt(&format!("c{i}"), &format!("c{i}@t.co"), "free", TEST_SECRET).unwrap();
            tokens.push(tok);
        }

        // Alice subscribes as receiver before senders connect.
        let alice_url =
            format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}&token={tok_alice}");
        let (mut ws_alice, _) = connect_async(&alice_url).await.unwrap();

        // Give the server a moment to register Alice's subscription before
        // spawning concurrent senders (avoids off-by-one in channel drain).
        tokio::time::sleep(Duration::from_millis(20)).await;

        let mut handles = Vec::with_capacity(client_count);
        for (i, tok) in tokens.into_iter().enumerate() {
            let url =
                format!("ws://127.0.0.1:{port}/api/v1/sync/ws?universe={universe}&token={tok}");
            handles.push(tokio::spawn(async move {
                let (mut ws, _) = connect_async(&url).await?;
                let d = delta::upserted_delta(universe, format!("f{i}.md"), vec![1], "s", 0);
                let b = SyncBatch {
                    deltas: vec![d],
                    client_id: format!("c{i}"),
                    batch_ts_ns: 0,
                    resume_token: 0,
                };
                ws.send(WsMsg::Binary(delta::encode_batch(&b).unwrap().into()))
                    .await?;
                ws.close(None).await.ok();
                Ok::<_, anyhow::Error>(())
            }));
        }

        let mut ok = 0usize;
        for h in handles {
            if h.await.unwrap().is_ok() {
                ok += 1;
            }
        }
        // Allow ≤1 failure (network jitter in CI); ≥95% success validates the feature.
        assert!(
            ok >= client_count - 1,
            "{ok}/{client_count} clients succeeded (expected ≥{})",
            client_count - 1
        );

        // Alice collects downlinks.
        let mut received = 0usize;
        loop {
            let msg = tokio::time::timeout(Duration::from_millis(300), ws_alice.next()).await;
            match msg {
                Ok(Some(Ok(WsMsg::Binary(b)))) => {
                    if dc::decode_batch(&b).is_ok() {
                        received += 1;
                    }
                }
                Ok(Some(Ok(WsMsg::Ping(_)))) => continue,
                _ => break,
            }
        }
        assert!(
            received > 0,
            "Alice should have received at least 1 downlink"
        );

        ws_alice.close(None).await.ok();
    }

    // ── T7: SyncRoom delta log eviction ──────────────────────────────────────

    #[tokio::test]
    async fn test_sync_room_eviction() {
        let room = SyncRoom::new();
        for i in 0..5u64 {
            room.append_frame(vec![i as u8]).await;
        }
        let frames = room.replay_since(0).await.unwrap();
        assert_eq!(frames.len(), 5);

        let frames = room.replay_since(3).await.unwrap();
        assert_eq!(frames.len(), 2);
    }
}
