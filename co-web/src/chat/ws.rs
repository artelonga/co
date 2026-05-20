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

use super::permissions::{can_read, resolve_role};

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
