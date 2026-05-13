//! CO-194 — Chat WebSocket: live messages + presence (Phase 4 slice 2).
//!
//! Route: `GET /api/v1/universes/:slug/chat/rooms/:room_slug/ws`
//!
//! Auth: JWT from `Authorization: Bearer <token>` header or `session` cookie.
//! Auth failures return HTTP 401/403/410 BEFORE the WS upgrade, matching the
//! REST gate pattern so they are testable with `tower::ServiceExt::oneshot`.
//!
//! ## Wire format (JSON Lines)
//!
//! Server → client:
//! - `ready`            — initial state (presence roster, your role)
//! - `message.created`  — fanned out by REST POST handler
//! - `message.edited`   — CO-196 (not yet implemented)
//! - `message.deleted`  — CO-196
//! - `presence.join`    — first connection for a user
//! - `presence.leave`   — last connection for a user closed
//! - `typing.start`     — rate-limited to 1/min per user
//! - `typing.stop`
//! - `error`            — server-initiated error
//!
//! Client → server:
//! - `ping`             — keep-alive (updates last-activity timer)
//! - `typing.start`
//! - `typing.stop`
//!
//! Messages are NOT sent over WS; use `POST /messages` (REST). The REST
//! handler fans out `message.created` via the in-process broadcast channel.

use std::sync::Arc;

use axum::extract::Path;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::time::{Duration, interval};

use crate::auth::{decode_user_id, extract_session_cookie, jwt_secret};
use crate::server::AppState;
use crate::storage::chat::{ChatAuthor, ChatMessageWithAuthor};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Capacity of the per-room broadcast ring. Messages beyond this are dropped
/// and lagging subscribers receive a `RecvError::Lagged` that triggers a close.
pub const BROADCAST_CAPACITY: usize = 256;

const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Drop the connection if the client hasn't sent any frame (or Pong) for this
/// long. Checked each time the PING_INTERVAL timer fires.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(40);

// ---------------------------------------------------------------------------
// Server → client events
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum ChatEvent {
    #[serde(rename = "ready")]
    Ready {
        room_id: String,
        your_role: String,
        presence: Vec<ChatAuthor>,
    },
    #[serde(rename = "message.created")]
    MessageCreated { message: ChatMessageWithAuthor },
    #[serde(rename = "message.edited")]
    MessageEdited { message: ChatMessageWithAuthor },
    #[serde(rename = "message.deleted")]
    MessageDeleted {
        message_id: String,
        deleted_at: String,
    },
    #[serde(rename = "presence.join")]
    PresenceJoin { user: ChatAuthor },
    #[serde(rename = "presence.leave")]
    PresenceLeave { user_id: String },
    #[serde(rename = "typing.start")]
    TypingStart { user_id: String },
    #[serde(rename = "typing.stop")]
    TypingStop { user_id: String },
    #[serde(rename = "error")]
    Error { code: String, message: String },
}

// ---------------------------------------------------------------------------
// Client → server messages
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ClientMsg {
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "typing.start")]
    TypingStart,
    #[serde(rename = "typing.stop")]
    TypingStop,
}

// ---------------------------------------------------------------------------
// Role helpers (mirrors chat_routes)
// ---------------------------------------------------------------------------

fn resolve_role(
    storage: &crate::storage::Storage,
    universe_key: &str,
    user_id: &str,
) -> Option<String> {
    if let Some(role) = storage.universe_member_role(universe_key, user_id) {
        return Some(role);
    }
    if storage.is_subscribed(user_id, universe_key) {
        return Some("subscriber".to_string());
    }
    None
}

fn can_read(role: &str) -> bool {
    matches!(role, "owner" | "admin" | "member" | "viewer" | "subscriber")
}

// ---------------------------------------------------------------------------
// Presence helpers
// ---------------------------------------------------------------------------

/// Increment the per-user refcount for `(room_id, user_id)`.
/// Returns `true` when this is the FIRST connection for that user in the room.
fn join_presence(state: &AppState, room_id: &str, user_id: &str) -> bool {
    let mut map = state.chat_presence.lock().unwrap();
    let room = map.entry(room_id.to_string()).or_default();
    let count = room.entry(user_id.to_string()).or_insert(0);
    *count += 1;
    *count == 1
}

/// Decrement the per-user refcount. Returns `true` when this was the LAST
/// connection for that user in the room.
fn leave_presence(state: &AppState, room_id: &str, user_id: &str) -> bool {
    let mut map = state.chat_presence.lock().unwrap();
    if let Some(room) = map.get_mut(room_id)
        && let Some(count) = room.get_mut(user_id)
    {
        *count = count.saturating_sub(1);
        if *count == 0 {
            room.remove(user_id);
            return true;
        }
    }
    false
}

/// Snapshot the current presence roster as `ChatAuthor` entries, resolving
/// display names from the users table.
fn current_presence(
    state: &AppState,
    room_id: &str,
    storage: &crate::storage::Storage,
) -> Vec<ChatAuthor> {
    let user_ids: Vec<String> = {
        let map = state.chat_presence.lock().unwrap();
        map.get(room_id)
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default()
    };
    user_ids
        .into_iter()
        .map(|uid| {
            let (display_name, usuario) = storage
                .get_user_display_info(&uid)
                .unwrap_or_else(|| (uid.clone(), None));
            ChatAuthor {
                user_id: uid,
                display_name,
                usuario,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Upgrade handler
// ---------------------------------------------------------------------------

pub async fn chat_ws_handler(
    State(state): State<AppState>,
    Path((slug, room_slug)): Path<(String, String)>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // 1. Auth — extract JWT from Bearer header or session cookie.
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(&headers));

    let user_id = match token {
        None => return StatusCode::UNAUTHORIZED.into_response(),
        Some(t) => match decode_user_id(&t, &jwt_secret()) {
            Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
            Ok(id) => id,
        },
    };

    // 2. Connect rate-limit: 5 connects per user per minute.
    {
        let mut limiter = match state.rate_limiter.lock() {
            Ok(l) => l,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let key = format!("chat:ws:connect:{}", user_id);
        if limiter.check(&key, 5).is_err() {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
    }

    // 3. Membership + room check (all reads under one storage lock).
    let (room_id, your_role, author) = {
        let storage = state.storage.lock();

        if storage.get_universe(&slug).is_none() {
            return StatusCode::NOT_FOUND.into_response();
        }

        let role = match resolve_role(&storage, &slug, &user_id) {
            None => return StatusCode::FORBIDDEN.into_response(),
            Some(r) => r,
        };
        if !can_read(&role) {
            return StatusCode::FORBIDDEN.into_response();
        }

        let room = match storage.get_chat_room_by_slug(&slug, &room_slug) {
            None => return StatusCode::NOT_FOUND.into_response(),
            Some(r) => r,
        };
        if room.archived_at.is_some() {
            return StatusCode::GONE.into_response();
        }

        let (display_name, usuario) = storage
            .get_user_display_info(&user_id)
            .unwrap_or_else(|| (user_id.clone(), None));
        let author = ChatAuthor {
            user_id: user_id.clone(),
            display_name,
            usuario,
        };

        (room.id, role, author)
    };

    // 4. Get or create broadcast channel for the room.
    let tx = {
        let mut map = match state.chat_rooms_broadcast.lock() {
            Ok(m) => m,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        map.entry(room_id.clone())
            .or_insert_with(|| broadcast::channel::<ChatEvent>(BROADCAST_CAPACITY).0)
            .clone()
    };

    // 5. Upgrade the HTTP connection to WebSocket.
    let state2 = Arc::clone(&state);
    ws.on_upgrade(move |socket| async move {
        handle_ws(socket, state2, room_id, user_id, your_role, author, tx).await;
    })
}

// ---------------------------------------------------------------------------
// Per-connection handler
// ---------------------------------------------------------------------------

async fn handle_ws(
    socket: WebSocket,
    state: AppState,
    room_id: String,
    user_id: String,
    your_role: String,
    author: ChatAuthor,
    tx: broadcast::Sender<ChatEvent>,
) {
    // 1. Register presence; broadcast join if first connection for this user.
    let is_first = join_presence(&state, &room_id, &user_id);
    if is_first {
        let _ = tx.send(ChatEvent::PresenceJoin {
            user: author.clone(),
        });
    }

    // 2. Build current presence list for the `ready` event.
    let presence_list = {
        let storage = state.storage.lock();
        current_presence(&state, &room_id, &storage)
    };

    // 3. Subscribe to the broadcast channel BEFORE splitting the socket so we
    // don't miss events that arrive between the ready send and the select loop.
    let mut rx = tx.subscribe();

    // 4. Split socket into sender + receiver halves.
    let (mut ws_tx, mut ws_rx) = socket.split();

    // 5. Send the `ready` event.
    let ready = ChatEvent::Ready {
        room_id: room_id.clone(),
        your_role,
        presence: presence_list,
    };
    let ready_json = serde_json::to_string(&ready).unwrap_or_default();
    if ws_tx.send(Message::Text(ready_json.into())).await.is_err() {
        let is_last = leave_presence(&state, &room_id, &user_id);
        if is_last {
            let _ = tx.send(ChatEvent::PresenceLeave {
                user_id: user_id.clone(),
            });
        }
        return;
    }

    // 6. Shared last-activity instant (tokio time, so it's affected by
    //    tokio::time::pause/advance in tests).
    let last_activity = Arc::new(std::sync::Mutex::new(tokio::time::Instant::now()));

    // 7. Writer task — forwards broadcast events and sends keep-alive pings.
    let last_w = Arc::clone(&last_activity);
    let mut writer_handle = tokio::spawn(async move {
        let mut ping_timer = interval(PING_INTERVAL);
        ping_timer.tick().await; // consume the immediate first tick

        loop {
            tokio::select! {
                evt = rx.recv() => {
                    match evt {
                        Ok(event) => {
                            let json = serde_json::to_string(&event).unwrap_or_default();
                            if ws_tx.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let err_json = serde_json::to_string(&ChatEvent::Error {
                                code: "internal".into(),
                                message: "Broadcast lag — please reconnect".into(),
                            })
                            .unwrap_or_default();
                            let _ = ws_tx.send(Message::Text(err_json.into())).await;
                            break;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }

                _ = ping_timer.tick() => {
                    let elapsed = last_w.lock().unwrap().elapsed();
                    if elapsed > SILENCE_TIMEOUT {
                        break; // client has been silent too long
                    }
                    if ws_tx.send(Message::Ping(vec![].into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // 8. Reader task — deserialises client messages and handles typing events.
    let state_r = Arc::clone(&state);
    let room_id_r = room_id.clone();
    let user_id_r = user_id.clone();
    let tx_r = tx.clone();
    let last_r = Arc::clone(&last_activity);
    let mut reader_handle = tokio::spawn(async move {
        while let Some(msg_result) = ws_rx.next().await {
            match msg_result {
                Ok(Message::Text(text)) => {
                    *last_r.lock().unwrap() = tokio::time::Instant::now();
                    if let Ok(client_msg) = serde_json::from_str::<ClientMsg>(&text) {
                        match client_msg {
                            ClientMsg::Ping => {} // activity already recorded
                            ClientMsg::TypingStart => {
                                let key = format!("chat:typing:{}:{}", user_id_r, room_id_r);
                                let allowed = state_r
                                    .rate_limiter
                                    .lock()
                                    .map(|mut l| l.check(&key, 1).is_ok())
                                    .unwrap_or(false);
                                if allowed {
                                    let _ = tx_r.send(ChatEvent::TypingStart {
                                        user_id: user_id_r.clone(),
                                    });
                                }
                            }
                            ClientMsg::TypingStop => {
                                let _ = tx_r.send(ChatEvent::TypingStop {
                                    user_id: user_id_r.clone(),
                                });
                            }
                        }
                    }
                }
                Ok(Message::Pong(_)) => {
                    *last_r.lock().unwrap() = tokio::time::Instant::now();
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    // 9. Wait for whichever task finishes first, then abort the other.
    tokio::select! {
        _ = &mut writer_handle => reader_handle.abort(),
        _ = &mut reader_handle => writer_handle.abort(),
    }

    // 10. Clean up presence and broadcast leave event if last connection.
    let is_last = leave_presence(&state, &room_id, &user_id);
    if is_last {
        let _ = tx.send(ChatEvent::PresenceLeave {
            user_id: user_id.clone(),
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use futures_util::{SinkExt as _, StreamExt as _};
    use rusqlite::params;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::sync::broadcast;
    use tokio::time::Duration;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as WsMsg;
    use tokio_tungstenite::tungstenite::http::Request as TRequest;
    use tower::ServiceExt;

    use crate::auth::sign_jwt;
    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::server::{AppState, AppStateInner, build_router};
    use crate::storage::Storage;

    use super::ChatEvent;

    type WsStream = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    fn isolate_env() {
        unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    }

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
            port: 0,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: dir.join("universes").to_string_lossy().to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        }
    }

    fn make_state(dir: &std::path::Path) -> AppState {
        let config = test_config(dir);
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path).expect("open test game storage"),
        );
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        Arc::new(AppStateInner {
            storage: parking_lot::Mutex::new(storage),
            experiment: Mutex::new(experiment),
            config,
            auth_store: Mutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            cache: crate::cache::CacheLayer::new(),
            rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
            embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
            chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
            chat_presence: Mutex::new(std::collections::HashMap::new()),
            geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
        })
    }

    fn insert_user(dir: &std::path::Path, email: &str) -> String {
        let storage = Storage::new(dir.to_str().unwrap());
        let id = format!("usr_test_{}", nanoid::nanoid!(8));
        let usuario = email.split('@').next().unwrap_or("user").to_lowercase();
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
                 VALUES (?1, ?2, ?3, 'player', ?4, ?5)",
                params![id, email, email, now, usuario],
            )
            .expect("insert test user");
        id
    }

    fn insert_universe(dir: &std::path::Path, key: &str, owner_id: &str) {
        let storage = Storage::new(dir.to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, visibility) \
                 VALUES (?1, ?2, '', ?3, ?4, 'private')",
                params![key, key, owner_id, now],
            )
            .expect("insert test universe");
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universe_members \
                 (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, owner_id, now],
            )
            .expect("insert owner member");
        storage
            .ensure_default_room(key)
            .expect("ensure_default_room");
    }

    fn add_member(dir: &std::path::Path, universe_key: &str, user_id: &str, role: &str) {
        let storage = Storage::new(dir.to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universe_members \
                 (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![universe_key, user_id, role, now],
            )
            .expect("insert member");
    }

    fn archive_room(dir: &std::path::Path, universe_key: &str, room_slug: &str) {
        let storage = Storage::new(dir.to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "UPDATE chat_rooms SET archived_at = ?1 \
                 WHERE universe_key = ?2 AND slug = ?3",
                params![now, universe_key, room_slug],
            )
            .expect("archive room");
    }

    fn make_jwt(user_id: &str) -> String {
        isolate_env();
        let (token, _) =
            sign_jwt(user_id, "test@example.com", "player", "test-jwt-secret").unwrap();
        token
    }

    /// Spin up a full axum server bound to 127.0.0.1:0 and return the port.
    async fn spawn_server(state: AppState) -> u16 {
        let app = build_router(state, None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        port
    }

    // --- 1. Unauthenticated → HTTP 401 before upgrade ---------------------------

    #[tokio::test]
    async fn test_ws_unauthenticated_4401() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws1@example.com");
        insert_universe(dir.path(), "ws1", &owner_id);
        let state = make_state(dir.path());
        let port = spawn_server(state).await;

        let status =
            ws_try_connect(port, "/api/v1/universes/ws1/chat/rooms/general/ws", None).await;
        assert_eq!(status, Some(401), "expected 401, got {status:?}");
    }

    // --- 2. Non-member → HTTP 403 before upgrade ---------------------------------

    #[tokio::test]
    async fn test_ws_non_member_4403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws2@example.com");
        let outsider_id = insert_user(dir.path(), "outsider_ws2@example.com");
        insert_universe(dir.path(), "ws2", &owner_id);
        let token = make_jwt(&outsider_id);
        let state = make_state(dir.path());
        let port = spawn_server(state).await;

        let status = ws_try_connect(
            port,
            "/api/v1/universes/ws2/chat/rooms/general/ws",
            Some(&token),
        )
        .await;
        assert_eq!(status, Some(403), "expected 403, got {status:?}");
    }

    // --- 3. Archived room → HTTP 410 before upgrade ------------------------------

    #[tokio::test]
    async fn test_ws_archived_room_4410() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws3@example.com");
        insert_universe(dir.path(), "ws3", &owner_id);
        archive_room(dir.path(), "ws3", "general");
        let token = make_jwt(&owner_id);
        let state = make_state(dir.path());
        let port = spawn_server(state).await;

        let status = ws_try_connect(
            port,
            "/api/v1/universes/ws3/chat/rooms/general/ws",
            Some(&token),
        )
        .await;
        assert_eq!(status, Some(410), "expected 410, got {status:?}");
    }

    // --- 4. Connect rate-limit: 6th connect/min → HTTP 429 ----------------------

    #[tokio::test]
    async fn test_ws_connect_rate_limit_6th_in_minute_fails() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws4@example.com");
        insert_universe(dir.path(), "ws4", &owner_id);
        let token = make_jwt(&owner_id);
        let state = make_state(dir.path());
        let port = spawn_server(state).await;

        let path = "/api/v1/universes/ws4/chat/rooms/general/ws";

        // First 5 connections should succeed (bucket capacity = 5).
        for _ in 0..5 {
            let url = format!("ws://127.0.0.1:{port}{path}");
            let mut ws = ws_connect(&url, &token).await;
            ws.close(None).await.ok();
        }

        // 6th connection: bucket exhausted → HTTP 429.
        let status = ws_try_connect(port, path, Some(&token)).await;
        assert_eq!(
            status,
            Some(429),
            "6th connect should be rate-limited (429), got {status:?}"
        );
    }

    /// Connect via WS with Bearer auth header; panics on failure.
    async fn ws_connect(url: &str, token: &str) -> WsStream {
        // Parse host from the ws:// URL so the Host header is correct.
        let host = url
            .trim_start_matches("ws://")
            .split('/')
            .next()
            .unwrap_or("127.0.0.1");
        let req = TRequest::builder()
            .uri(url)
            .header("Host", host)
            .header("Authorization", format!("Bearer {token}"))
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .body(())
            .unwrap();
        let (ws, _): (WsStream, _) = connect_async(req).await.unwrap();
        ws
    }

    /// Try a WS connect and return the HTTP status when the server rejects it.
    /// Returns `None` if the connection succeeded (shouldn't happen in auth-gate tests).
    async fn ws_try_connect(port: u16, path: &str, token: Option<&str>) -> Option<u16> {
        let url = format!("ws://127.0.0.1:{port}{path}");
        let host = format!("127.0.0.1:{port}");
        let mut builder = TRequest::builder()
            .uri(&url)
            .header("Host", &host)
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket");
        if let Some(t) = token {
            builder = builder.header("Authorization", format!("Bearer {t}"));
        }
        let req = builder.body(()).unwrap();
        match connect_async(req).await {
            Ok((mut ws, _)) => {
                ws.close(None).await.ok();
                None // connected — not what we wanted
            }
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Some(resp.status().as_u16()),
            Err(_) => None,
        }
    }

    // --- 5. Ready event includes presence ----------------------------------------

    #[tokio::test]
    async fn test_ws_ready_event_includes_presence() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws5@example.com");
        insert_universe(dir.path(), "ws5", &owner_id);
        let state = make_state(dir.path());
        let port = spawn_server(state).await;

        let token = make_jwt(&owner_id);
        let url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws5/chat/rooms/general/ws");
        let mut ws = ws_connect(&url, &token).await;

        // First frame must be the `ready` event
        let frame = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("timeout waiting for ready event")
            .unwrap()
            .unwrap();

        let text = match frame {
            WsMsg::Text(t) => t.to_string(),
            other => panic!("expected Text frame, got {other:?}"),
        };
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            json["type"], "ready",
            "first frame must be ready, got: {text}"
        );
        assert!(json["room_id"].is_string());
        assert!(json["presence"].is_array());

        ws.close(None).await.ok();
    }

    // --- 6. Broadcast to all subscribers -----------------------------------------

    #[tokio::test]
    async fn test_ws_broadcast_to_all_subscribers() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws6@example.com");
        let member_id = insert_user(dir.path(), "member_ws6@example.com");
        insert_universe(dir.path(), "ws6", &owner_id);
        add_member(dir.path(), "ws6", &member_id, "member");

        let state = make_state(dir.path());

        // Pre-register a broadcast channel for the general room so we can
        // subscribe before the REST POST.
        let room_id = {
            let storage = state.storage.lock();
            storage
                .get_chat_room_by_slug("ws6", "general")
                .expect("general room")
                .id
        };

        let (tx, mut rx1) = broadcast::channel::<ChatEvent>(64);
        let mut rx2 = tx.subscribe();
        {
            let mut map = state.chat_rooms_broadcast.lock().unwrap();
            map.insert(room_id.clone(), tx);
        }

        let app = build_router(Arc::clone(&state), None);
        let token = make_jwt(&owner_id);

        // POST a message via REST
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/ws6/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"broadcast test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Both subscribers must receive message.created
        let evt1 = tokio::time::timeout(Duration::from_millis(200), async {
            rx1.recv().await.unwrap()
        })
        .await
        .expect("rx1 timeout");

        let evt2 = tokio::time::timeout(Duration::from_millis(200), async {
            rx2.recv().await.unwrap()
        })
        .await
        .expect("rx2 timeout");

        assert!(
            matches!(&evt1, ChatEvent::MessageCreated { .. }),
            "rx1: expected MessageCreated, got {evt1:?}"
        );
        assert!(
            matches!(&evt2, ChatEvent::MessageCreated { .. }),
            "rx2: expected MessageCreated, got {evt2:?}"
        );
    }

    // --- 7. No broadcast to other rooms ------------------------------------------

    #[tokio::test]
    async fn test_ws_no_broadcast_to_other_rooms() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws7@example.com");
        insert_universe(dir.path(), "ws7", &owner_id);

        let state = make_state(dir.path());

        // Get room A (general) and create room B
        let (room_a_id, room_b_id) = {
            let storage = state.storage.lock();
            let room_a = storage
                .get_chat_room_by_slug("ws7", "general")
                .expect("general room");
            let room_b = storage
                .create_chat_room("ws7", "Other", None, &owner_id)
                .expect("create other room");
            (room_a.id, room_b.id)
        };

        // Subscribe only to room B
        let (tx_b, mut rx_b) = broadcast::channel::<ChatEvent>(64);
        {
            let mut map = state.chat_rooms_broadcast.lock().unwrap();
            map.insert(room_b_id.clone(), tx_b);
        }

        let app = build_router(Arc::clone(&state), None);
        let token = make_jwt(&owner_id);

        // POST to room A
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/ws7/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"room a message"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Room B subscriber must NOT receive anything
        let result = tokio::time::timeout(Duration::from_millis(100), rx_b.recv()).await;
        assert!(
            result.is_err(),
            "room B should not receive events from room A"
        );

        let _ = room_a_id;
    }

    // --- 8. Presence join/leave events -------------------------------------------

    #[tokio::test]
    async fn test_ws_presence_join_leave_events() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws8@example.com");
        let member_id = insert_user(dir.path(), "member_ws8@example.com");
        insert_universe(dir.path(), "ws8", &owner_id);
        add_member(dir.path(), "ws8", &member_id, "member");

        let state = make_state(dir.path());
        let port = spawn_server(Arc::clone(&state)).await;

        let tok_owner = make_jwt(&owner_id);
        let tok_member = make_jwt(&member_id);

        let ws_url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws8/chat/rooms/general/ws");

        // Client 1 (owner) connects
        let mut ws_owner = ws_connect(&ws_url, &tok_owner).await;

        // Receive ready frame for client 1
        let _ready1 = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        // Client 2 (member) connects; client 1 should receive presence.join
        let mut ws_member = ws_connect(&ws_url, &tok_member).await;

        // Drain frames from client 1 until we get presence.join
        let mut got_join = false;
        for _ in 0..10 {
            let frame = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
                .await
                .expect("timeout waiting for presence.join")
                .unwrap()
                .unwrap();
            if let WsMsg::Text(text) = frame {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                if json["type"] == "presence.join" {
                    got_join = true;
                    break;
                }
            }
        }
        assert!(
            got_join,
            "client 1 must receive presence.join when client 2 connects"
        );

        // Consume client 2's ready frame
        let _ready2 = tokio::time::timeout(Duration::from_secs(2), ws_member.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        // Client 2 disconnects; client 1 should receive presence.leave
        ws_member.close(None).await.ok();

        let mut got_leave = false;
        for _ in 0..10 {
            let frame = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
                .await
                .expect("timeout waiting for presence.leave")
                .unwrap()
                .unwrap();
            if let WsMsg::Text(text) = frame {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                if json["type"] == "presence.leave" {
                    got_leave = true;
                    break;
                }
            }
        }
        assert!(
            got_leave,
            "client 1 must receive presence.leave when client 2 disconnects"
        );

        ws_owner.close(None).await.ok();
    }

    // --- 9. Same user, multiple connections → one presence.join -----------------

    #[tokio::test]
    async fn test_ws_same_user_multiple_connections_dedup() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws9@example.com");
        let member_id = insert_user(dir.path(), "member_ws9@example.com");
        insert_universe(dir.path(), "ws9", &owner_id);
        add_member(dir.path(), "ws9", &member_id, "member");

        let state = make_state(dir.path());
        let port = spawn_server(Arc::clone(&state)).await;

        let tok_owner = make_jwt(&owner_id);
        let tok_member = make_jwt(&member_id);

        let ws_url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws9/chat/rooms/general/ws");

        // Owner (observer) connects first
        let mut ws_owner = ws_connect(&ws_url, &tok_owner).await;
        let _ready_owner = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        // Member connects TWICE (simulating two tabs)
        let mut ws_m1 = ws_connect(&ws_url, &tok_member).await;
        let _ready_m1 = tokio::time::timeout(Duration::from_secs(2), ws_m1.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        let mut ws_m2 = ws_connect(&ws_url, &tok_member).await;
        let _ready_m2 = tokio::time::timeout(Duration::from_secs(2), ws_m2.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        // Collect frames from owner for a short window
        let mut join_count = 0usize;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, ws_owner.next()).await {
                Ok(Some(Ok(WsMsg::Text(text)))) => {
                    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    if json["type"] == "presence.join" {
                        join_count += 1;
                    }
                }
                _ => {}
            }
        }

        // Only ONE presence.join should have been emitted for the member
        assert_eq!(
            join_count, 1,
            "expected exactly 1 presence.join for member (refcount dedup), got {join_count}"
        );

        ws_m1.close(None).await.ok();
        ws_m2.close(None).await.ok();
        ws_owner.close(None).await.ok();
    }

    // --- 10. Typing rate-limit: second typing.start within 2s is dropped ---------

    #[tokio::test]
    async fn test_ws_typing_rate_limit_2s() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws10@example.com");
        let member_id = insert_user(dir.path(), "member_ws10@example.com");
        insert_universe(dir.path(), "ws10", &owner_id);
        add_member(dir.path(), "ws10", &member_id, "member");

        let state = make_state(dir.path());
        let port = spawn_server(Arc::clone(&state)).await;

        let tok_owner = make_jwt(&owner_id);
        let tok_member = make_jwt(&member_id);
        let ws_url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws10/chat/rooms/general/ws");

        // Owner connects as the observer.
        let mut ws_owner = ws_connect(&ws_url, &tok_owner).await;
        let _ready_owner = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        // Member connects as the typist.
        let mut ws_member = ws_connect(&ws_url, &tok_member).await;
        let _ready_member = tokio::time::timeout(Duration::from_secs(2), ws_member.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        // Owner receives the presence.join for member (drain it).
        tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
            .await
            .ok();

        // Member sends two rapid typing.start messages.
        ws_member
            .send(WsMsg::Text(r#"{"type":"typing.start"}"#.into()))
            .await
            .unwrap();
        ws_member
            .send(WsMsg::Text(r#"{"type":"typing.start"}"#.into()))
            .await
            .unwrap();

        // Drain the owner's WS stream for a short window; count typing.start events.
        let mut typing_count = 0usize;
        for _ in 0..10 {
            match tokio::time::timeout(Duration::from_millis(200), ws_owner.next()).await {
                Ok(Some(Ok(WsMsg::Text(text)))) => {
                    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    if json["type"] == "typing.start" {
                        typing_count += 1;
                    }
                }
                _ => break,
            }
        }

        // Only the first typing.start should have been broadcast; the second
        // is silently dropped by the rate limiter (capacity = 1 per key).
        assert_eq!(
            typing_count, 1,
            "second rapid typing.start should be rate-limited (got {typing_count} events)"
        );

        ws_member.close(None).await.ok();
        ws_owner.close(None).await.ok();
    }

    // --- 11. Keep-alive: silent client is dropped --------------------------------

    #[tokio::test]
    async fn test_ws_keepalive_drops_silent_client() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_ws11@example.com");
        insert_universe(dir.path(), "ws11", &owner_id);

        let state = make_state(dir.path());
        let port = spawn_server(state).await;

        let token = make_jwt(&owner_id);
        let ws_url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws11/chat/rooms/general/ws");

        let mut ws = ws_connect(&ws_url, &token).await;

        // Consume ready frame
        let _ready = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout waiting for ready frame")
            .unwrap()
            .unwrap();

        // Advance tokio simulated time past the silence threshold (30s ping interval + 40s timeout = 70s)
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(35)).await; // writer sends ping at t=30
        tokio::time::advance(Duration::from_secs(35)).await; // t=70 > 40s silence threshold

        // Client should eventually see the connection close
        let mut saw_close = false;
        for _ in 0..20 {
            match tokio::time::timeout(Duration::from_millis(50), ws.next()).await {
                Ok(Some(Ok(WsMsg::Close(_)))) => {
                    saw_close = true;
                    break;
                }
                Ok(None) => {
                    saw_close = true;
                    break;
                }
                Ok(Some(Err(_))) => {
                    saw_close = true;
                    break;
                }
                Ok(Some(Ok(WsMsg::Ping(_) | WsMsg::Pong(_) | WsMsg::Text(_)))) => {
                    continue;
                }
                Err(_timeout) => {
                    // no data yet — time might not have advanced enough
                    break;
                }
                _ => {}
            }
        }

        // Note: this test is inherently racy with real TCP + simulated time.
        // We assert connection is closed, but allow some slack.
        assert!(
            saw_close,
            "server should close the connection after silence timeout"
        );
    }
}
