//! CO-353 — Sala WebSocket: realtime presence + collaborative canvas.
//!
//! Route: `GET /ws/sala/{universe_key}/{workspace_slug}`
//!
//! Auth: JWT from `Authorization: Bearer`, the `session` cookie, or a `?token=`
//! query param. Unauthenticated clients are admitted as **read-only visitors**
//! (`anon:<uuid>`) — writes are rejected server-side and answered with a
//! `revert` so the client can roll its optimistic change back.
//!
//! Lifecycle (see [`crate::content::workspace::lobby`] for the shared state):
//! 1. connect → authenticate (or assign anon) → join the room
//! 2. server sends `{type:"snapshot", state, users}` to the newcomer
//! 3. server broadcasts `{type:"user_join"}` to the rest of the room
//! 4. cursor / node / edge / suggest / publish ops fan out (LWW on the server)
//! 5. layout flushes to `workspace_states` every 2 s while dirty, and once more
//!    when the last connection leaves

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant, interval};

use super::lobby::{
    CURSOR_MIN_INTERVAL_MS, ClientMsg, PresenceUser, Room, RoomMsg, SHARED_USER, SalaEvent,
    color_for, cursor_allowed, workspace_id,
};
use crate::server::AppState;

/// Flush the dirty layout to `workspace_states` at most this often.
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
/// Keep-alive ping cadence.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Drop the socket if the client has been silent longer than this.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Deserialize, Default)]
pub struct SalaWsQuery {
    /// Optional JWT for non-cookie clients (e.g. the test harness, native apps).
    pub token: Option<String>,
}

// ---------------------------------------------------------------------------
// Upgrade handler
// ---------------------------------------------------------------------------

pub async fn sala_ws_handler(
    State(state): State<AppState>,
    Path((universe_key, workspace_slug)): Path<(String, String)>,
    Query(q): Query<SalaWsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // Resolve identity: real user from JWT, else a read-only visitor.
    let (user_id, anon) = resolve_identity(&q, &headers);
    let name = display_name(&state, &user_id, anon);
    let presence = PresenceUser {
        user_id: user_id.clone(),
        name,
        color: color_for(&user_id, anon),
        anon,
    };

    let id = workspace_id(&universe_key, &workspace_slug);
    let conn_id = state.core.sala_lobby.next_conn_id();

    // Create the room (loading any persisted layout) if it does not yet exist.
    let initial = load_initial_layout(&state, &universe_key, &workspace_slug);
    let room = state.core.sala_lobby.get_or_create(&id, initial);

    let state2 = state.clone();
    ws.on_upgrade(move |socket| async move {
        handle_ws(
            socket,
            state2,
            room,
            id,
            universe_key,
            workspace_slug,
            conn_id,
            presence,
        )
        .await;
    })
}

/// Resolve `(user_id, anon)` from token sources. Falls back to an anonymous
/// read-only visitor with a stable per-connection id.
fn resolve_identity(q: &SalaWsQuery, headers: &HeaderMap) -> (String, bool) {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| q.token.clone())
        .or_else(|| crate::auth::extract_session_cookie(headers));

    if let Some(token) = token
        && let Ok(uid) = crate::auth::decode_user_id(&token, &crate::auth::jwt_secret())
    {
        return (uid, false);
    }
    // Anonymous visitor — short, stable-within-connection id.
    let short = uuid::Uuid::new_v4().as_simple().to_string()[..4].to_string();
    (format!("anon:{short}"), true)
}

fn display_name(state: &AppState, user_id: &str, anon: bool) -> String {
    if anon {
        let short = user_id.strip_prefix("anon:").unwrap_or(user_id);
        return format!("Visitante {short}");
    }
    let storage = state.core.storage.lock();
    storage
        .get_user_display_info(user_id)
        .map(|(name, _)| name)
        .unwrap_or_else(|| user_id.to_string())
}

// ---------------------------------------------------------------------------
// Persistence (reuses CO-352's workspace_states via internal storage call)
// ---------------------------------------------------------------------------

/// Load the layout a new room should start from: the shared realtime layout if
/// it exists, else the most-recent public per-user layout (CO-352), else empty.
pub fn load_initial_layout(
    state: &AppState,
    universe_key: &str,
    workspace_slug: &str,
) -> serde_json::Value {
    let storage = state.core.storage.lock();
    let layout_json = storage
        .workspace_state_for_user(universe_key, workspace_slug, SHARED_USER)
        .ok()
        .flatten()
        .or_else(|| {
            storage
                .public_workspace_state(universe_key, workspace_slug)
                .ok()
                .flatten()
        })
        .map(|ws| ws.layout_json);
    match layout_json.and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok()) {
        Some(v) if v.is_object() => v,
        _ => empty_layout(),
    }
}

fn empty_layout() -> serde_json::Value {
    serde_json::json!({"nodes": [], "edges": [], "view": {"tx": 0, "ty": 0, "scale": 1}})
}

/// Flush the room's authoritative layout to `workspace_states` under the shared
/// user. Clears the room `dirty` flag. No-op when the room is not dirty.
pub fn flush_room(state: &AppState, room: &Room, universe_key: &str, workspace_slug: &str) {
    let layout_json = {
        let mut inner = room.inner.lock();
        if !inner.dirty {
            return;
        }
        inner.dirty = false;
        inner.layout.to_string()
    };
    let now = Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let storage = state.core.storage.lock();
    let _ = storage.upsert_workspace_state(
        &id,
        universe_key,
        workspace_slug,
        SHARED_USER,
        &layout_json,
        true, // shared layout is publicly readable
        &now,
    );
}

// ---------------------------------------------------------------------------
// Per-connection handler
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn handle_ws(
    socket: WebSocket,
    state: AppState,
    room: Arc<Room>,
    room_id: String,
    universe_key: String,
    workspace_slug: String,
    conn_id: u64,
    presence: PresenceUser,
) {
    let user_id = presence.user_id.clone();
    let anon = presence.anon;

    // 1. Register the connection; broadcast user_join if first for this user.
    let (snapshot, is_first_for_user) = {
        let mut inner = room.inner.lock();
        let first = inner.conn_count_for(&user_id) == 0;
        inner.connections.insert(conn_id, presence.clone());
        let snap = SalaEvent::Snapshot {
            state: inner.layout.clone(),
            users: inner.roster(),
        };
        (snap, first)
    };
    if is_first_for_user {
        // Tag with our conn id so the joiner doesn't get an echo of its own
        // join (it is already in the snapshot roster); the rest of the room does.
        let _ = room.tx.send(RoomMsg::from_conn(
            conn_id,
            &SalaEvent::UserJoin { user: presence },
        ));
    }

    // 2. Subscribe BEFORE sending the snapshot so no event is missed.
    let mut rx = room.tx.subscribe();
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Direct server→this-connection channel (snapshots-on-demand, reverts).
    let (direct_tx, mut direct_rx) = mpsc::unbounded_channel::<String>();

    // 3. Send the initial snapshot.
    let snap_json = serde_json::to_string(&snapshot).unwrap_or_default();
    if ws_tx.send(Message::Text(snap_json.into())).await.is_err() {
        cleanup(
            &state,
            &room,
            &room_id,
            &universe_key,
            &workspace_slug,
            conn_id,
            &user_id,
        )
        .await;
        return;
    }

    // 4. Writer task — fans out broadcast + direct messages, pings, periodic flush.
    let last_activity = Arc::new(std::sync::Mutex::new(Instant::now()));
    let last_w = Arc::clone(&last_activity);
    let state_w = state.clone();
    let room_w = Arc::clone(&room);
    let uk_w = universe_key.clone();
    let ws_w = workspace_slug.clone();
    let mut writer = tokio::spawn(async move {
        let mut ping = interval(PING_INTERVAL);
        ping.tick().await;
        let mut flush = interval(FLUSH_INTERVAL);
        flush.tick().await;
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Ok(m) => {
                        // Skip echoing a client's own op back to it.
                        if m.origin == conn_id { continue; }
                        if ws_tx.send(Message::Text(m.json.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                direct = direct_rx.recv() => match direct {
                    Some(text) => {
                        if ws_tx.send(Message::Text(text.into())).await.is_err() { break; }
                    }
                    None => break,
                },
                _ = flush.tick() => {
                    flush_room(&state_w, &room_w, &uk_w, &ws_w);
                }
                _ = ping.tick() => {
                    if last_w.lock().unwrap().elapsed() > SILENCE_TIMEOUT { break; }
                    if ws_tx.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
                }
            }
        }
    });

    // 5. Reader task — validates + relays client messages, enforces throttle/auth.
    let room_r = Arc::clone(&room);
    let tx_r = room.tx.clone();
    let last_r = Arc::clone(&last_activity);
    let direct_r = direct_tx.clone();
    let user_r = user_id.clone();
    let mut last_cursor: Option<Instant> = None;
    let started = Instant::now();
    let mut reader = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Text(text) => {
                    *last_r.lock().unwrap() = Instant::now();
                    let Ok(cm) = serde_json::from_str::<ClientMsg>(&text) else {
                        continue; // ignore malformed
                    };
                    match cm {
                        ClientMsg::Ping => {}
                        ClientMsg::Snapshot => {
                            let snap = {
                                let inner = room_r.inner.lock();
                                SalaEvent::Snapshot {
                                    state: inner.layout.clone(),
                                    users: inner.roster(),
                                }
                            };
                            let _ = direct_r.send(serde_json::to_string(&snap).unwrap_or_default());
                        }
                        ClientMsg::Cursor { x, y } => {
                            // Throttle outbound cursor frames to ~20 Hz per user.
                            let now = Instant::now();
                            let now_ms = now.duration_since(started).as_millis() as u64;
                            let last_ms =
                                last_cursor.map(|t| t.duration_since(started).as_millis() as u64);
                            if cursor_allowed(last_ms, now_ms, CURSOR_MIN_INTERVAL_MS) {
                                last_cursor = Some(now);
                                let _ = tx_r.send(RoomMsg::from_conn(
                                    conn_id,
                                    &SalaEvent::Cursor {
                                        user_id: user_r.clone(),
                                        x,
                                        y,
                                    },
                                ));
                            }
                        }
                        other => {
                            // Mutating ops: reject for anonymous visitors.
                            if other.is_write() && anon {
                                if let Some(op_id) = other.op_id() {
                                    let revert = SalaEvent::Revert {
                                        op_id: op_id.to_string(),
                                    };
                                    let _ = direct_r
                                        .send(serde_json::to_string(&revert).unwrap_or_default());
                                }
                                continue;
                            }
                            let is_write = other.is_write();
                            if let Some(event) = other.into_broadcast(&user_r) {
                                if is_write {
                                    let mut inner = room_r.inner.lock();
                                    if inner.apply(&event) {
                                        inner.dirty = true;
                                    }
                                }
                                let _ = tx_r.send(RoomMsg::from_conn(conn_id, &event));
                            }
                        }
                    }
                }
                Message::Pong(_) => *last_r.lock().unwrap() = Instant::now(),
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // 6. Run until either half ends, then tear down.
    tokio::select! {
        _ = &mut writer => reader.abort(),
        _ = &mut reader => writer.abort(),
    }
    cleanup(
        &state,
        &room,
        &room_id,
        &universe_key,
        &workspace_slug,
        conn_id,
        &user_id,
    )
    .await;
}

/// Remove the connection, broadcast `user_leave` on last connection for the
/// user, flush + drop the room when it becomes empty.
#[allow(clippy::too_many_arguments)]
async fn cleanup(
    state: &AppState,
    room: &Arc<Room>,
    room_id: &str,
    universe_key: &str,
    workspace_slug: &str,
    conn_id: u64,
    user_id: &str,
) {
    let (was_last_for_user, room_empty) = {
        let mut inner = room.inner.lock();
        inner.connections.remove(&conn_id);
        let last_for_user = inner.conn_count_for(user_id) == 0;
        (last_for_user, inner.connections.is_empty())
    };
    if was_last_for_user {
        let _ = room.tx.send(RoomMsg::from_conn(
            conn_id,
            &SalaEvent::UserLeave {
                user_id: user_id.to_string(),
            },
        ));
    }
    if room_empty {
        // Force a final flush even if a periodic flush already cleared `dirty`
        // mid-session — guarantees the latest layout reaches storage.
        room.inner.lock().dirty = true;
        flush_room(state, room, universe_key, workspace_slug);
        state.core.sala_lobby.remove(room_id);
    }
}
