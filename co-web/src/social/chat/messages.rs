//! Message handlers and related request/response types.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
use crate::error::AppError;
use crate::server::AppState;

use super::permissions::{can_post, can_read, lock_storage, resolve_role};
use super::ws::ChatEvent;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub before: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct PostMessageRequest {
    pub body: String,
    pub reply_to_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListMessagesResponse {
    pub messages: Vec<crate::storage::chat::ChatMessageWithAuthor>,
    pub has_more: bool,
}

#[derive(Debug, Serialize)]
pub struct PostMessageResponse {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct EditMessageRequest {
    pub body: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteMessageResponse {
    pub deleted_at: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/:slug/chat/rooms/:room_slug/messages
pub async fn list_messages_handler(
    State(state): State<AppState>,
    Path((slug, room_slug)): Path<(String, String)>,
    user_id: UserId,
    Query(query): Query<ListMessagesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let storage = lock_storage(&state);

    // CO-198: "dm" sentinel slug — resolve via DM room table
    let room = if slug == "dm" {
        let room = storage
            .get_dm_room_by_slug(&room_slug)
            .ok_or_else(|| AppError::NotFound(format!("DM room '{room_slug}' not found")))?;
        if !storage.is_dm_member(&room.id, &user_id.0) {
            return Err(AppError::Forbidden("Not a member of this DM".into()));
        }
        room
    } else {
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

        let role = resolve_role(&storage, &slug, &user_id.0).ok_or_else(|| {
            AppError::Forbidden("Chat is only available to universe members".into())
        })?;

        if !can_read(&role) {
            return Err(AppError::Forbidden("Insufficient role to read chat".into()));
        }

        storage
            .get_chat_room_by_slug(&slug, &room_slug)
            .ok_or_else(|| AppError::NotFound(format!("Room '{room_slug}' not found")))?
    };

    // Fetch limit + 1 to detect has_more
    let mut messages = storage.list_chat_messages(&room.id, query.before.as_deref(), limit + 1);
    let has_more = messages.len() > limit;
    messages.truncate(limit);

    Ok(axum::Json(ListMessagesResponse { messages, has_more }))
}

/// POST /api/v1/universes/:slug/chat/rooms/:room_slug/messages
pub async fn post_message_handler(
    State(state): State<AppState>,
    Path((slug, room_slug)): Path<(String, String)>,
    user_id: UserId,
    axum::Json(body): axum::Json<PostMessageRequest>,
) -> Result<impl IntoResponse, AppError> {
    let trimmed = body.body.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Message body cannot be empty".into()));
    }
    if trimmed.len() > 4000 {
        return Err(AppError::BadRequest(
            "Message body must be 4000 characters or fewer".into(),
        ));
    }

    // Rate limit: 20 messages per user per minute
    {
        let mut limiter = state
            .integrations
            .rate_limiter
            .lock()
            .map_err(|_| AppError::Internal("Rate limiter lock failed".into()))?;
        let key = format!("chat:post:{}", user_id.0);
        match limiter.check(&key, 20) {
            Ok(()) => {}
            Err(retry_after) => {
                return Err(AppError::RateLimited {
                    retry_after_secs: retry_after,
                });
            }
        }
    }

    // Insert the message and collect the data needed for the WS broadcast.
    let (msg_id, broadcast_payload) = {
        let storage = lock_storage(&state);

        // CO-198: "dm" sentinel slug — use DM auth instead of universe membership.
        let room = if slug == "dm" {
            let room = storage
                .get_dm_room_by_slug(&room_slug)
                .ok_or_else(|| AppError::NotFound(format!("DM room '{room_slug}' not found")))?;
            if !storage.is_dm_member(&room.id, &user_id.0) {
                return Err(AppError::Forbidden("Not a member of this DM".into()));
            }
            // Determine the other member for block check
            let other_member_id: Option<String> = storage
                .conn()
                .query_row(
                    "SELECT user_id FROM chat_room_members \
                     WHERE room_id = ?1 AND user_id != ?2 LIMIT 1",
                    rusqlite::params![room.id, user_id.0],
                    |row| row.get(0),
                )
                .ok();
            if let Some(ref other_id) = other_member_id
                && storage.is_blocked_either_way(&user_id.0, other_id)
            {
                return Err(AppError::Forbidden(
                    "dm.error.blocked: cannot message this user".into(),
                ));
            }
            room
        } else {
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

            let role = resolve_role(&storage, &slug, &user_id.0).ok_or_else(|| {
                AppError::Forbidden("Chat is only available to universe members".into())
            })?;

            if !can_post(&role) {
                return Err(AppError::Forbidden(
                    "Viewers and subscribers cannot post messages".into(),
                ));
            }

            storage
                .get_chat_room_by_slug(&slug, &room_slug)
                .ok_or_else(|| AppError::NotFound(format!("Room '{room_slug}' not found")))?
        };

        let msg_id = storage
            .post_chat_message(&room.id, &user_id.0, &trimmed, body.reply_to_id.as_deref())
            .map_err(|e| AppError::Internal(e.to_string()))?;

        // CO-199: create notifications for room members
        {
            let actor_display = storage
                .get_user_display_info(&user_id.0)
                .map(|(dn, _)| dn)
                .unwrap_or_else(|| user_id.0.clone());

            let other_members: Vec<String> = {
                let mut stmt = storage
                    .conn()
                    .prepare(
                        "SELECT user_id FROM chat_room_members \
                         WHERE room_id = ?1 AND user_id != ?2",
                    )
                    .expect("prepare room members for notif");
                stmt.query_map(rusqlite::params![room.id, user_id.0], |r| r.get(0))
                    .expect("query room members for notif")
                    .filter_map(|r| r.ok())
                    .collect()
            };

            let is_dm = room.kind == "dm";
            let universe_key_opt: Option<&str> = if is_dm { None } else { Some(&slug) };

            for member_id in &other_members {
                let prefs = storage.get_preferences(member_id);
                if is_dm {
                    if prefs.in_app_chat_dm {
                        let _ = storage.create_notification(
                            member_id,
                            "chat.dm",
                            None,
                            Some(&room.id),
                            &user_id.0,
                            &msg_id,
                            "notif.chat.dm",
                            serde_json::json!({"name": actor_display}),
                        );
                    }
                } else if prefs.in_app_chat_message {
                    let _ = storage.create_notification(
                        member_id,
                        "chat.message",
                        universe_key_opt,
                        Some(&room.id),
                        &user_id.0,
                        &msg_id,
                        "notif.chat.message",
                        serde_json::json!({"universe": slug}),
                    );
                }
            }

            // CO-199: @mention notifications
            let mentions = crate::storage::notifications::parse_mentions(&trimmed);
            for mention in &mentions {
                if let Some(mentioned_user) = storage.get_user_by_usuario(mention) {
                    let mentioned_id = mentioned_user.id.clone();
                    if mentioned_id != user_id.0 && other_members.contains(&mentioned_id) {
                        let prefs = storage.get_preferences(&mentioned_id);
                        if prefs.in_app_mention {
                            let _ = storage.create_notification(
                                &mentioned_id,
                                "chat.mention",
                                universe_key_opt,
                                Some(&room.id),
                                &user_id.0,
                                &msg_id,
                                "notif.chat.mention",
                                serde_json::json!({"name": actor_display, "room": room.name}),
                            );
                        }
                    }
                }
            }
        }

        let full_msg = storage.get_chat_message_by_id(&msg_id);
        let room_id = room.id.clone();
        (msg_id, full_msg.map(|m| (m, room_id)))
    }; // storage lock released

    // Fan out to WS subscribers (best-effort: no error if no subscribers).
    if let Some((msg, room_id)) = broadcast_payload
        && let Ok(map) = state.realtime.chat_rooms_broadcast.lock()
        && let Some(tx) = map.get(&room_id)
    {
        let _ = tx.send(ChatEvent::MessageCreated { message: msg });
    }

    Ok((
        StatusCode::CREATED,
        axum::Json(PostMessageResponse { id: msg_id }),
    ))
}

/// PATCH /api/v1/universes/:slug/chat/rooms/:room_slug/messages/:msg_id
pub async fn edit_message_handler(
    State(state): State<AppState>,
    Path((slug, room_slug, msg_id)): Path<(String, String, String)>,
    user_id: UserId,
    axum::Json(body): axum::Json<EditMessageRequest>,
) -> Result<impl IntoResponse, AppError> {
    let (updated_msg, room_id) = {
        let storage = lock_storage(&state);

        // CO-198: DM path
        let room = if slug == "dm" {
            let r = storage
                .get_dm_room_by_slug(&room_slug)
                .ok_or_else(|| AppError::NotFound(format!("DM room '{room_slug}' not found")))?;
            if !storage.is_dm_member(&r.id, &user_id.0) {
                return Err(AppError::Forbidden("Not a member of this DM".into()));
            }
            r
        } else {
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

            let role = resolve_role(&storage, &slug, &user_id.0).ok_or_else(|| {
                AppError::Forbidden("Chat is only available to universe members".into())
            })?;

            if !can_read(&role) {
                return Err(AppError::Forbidden(
                    "Insufficient role to access chat".into(),
                ));
            }

            storage
                .get_chat_room_by_slug(&slug, &room_slug)
                .ok_or_else(|| AppError::NotFound(format!("Room '{room_slug}' not found")))?
        };
        let room_id = room.id.clone();

        let msg = storage
            .edit_chat_message(
                &msg_id,
                &user_id.0,
                &body.body,
                chrono::Duration::minutes(15),
            )
            .map_err(|e| match e {
                crate::storage::chat::EditError::NotFound => {
                    AppError::NotFound("Message not found".into())
                }
                crate::storage::chat::EditError::NotAuthor => {
                    AppError::Forbidden("You can only edit your own messages".into())
                }
                crate::storage::chat::EditError::AlreadyDeleted => {
                    AppError::Gone("Message has been deleted".into())
                }
                crate::storage::chat::EditError::EditWindowExpired => {
                    AppError::Forbidden("edit_window_expired".into())
                }
                crate::storage::chat::EditError::BodyEmpty => {
                    AppError::BadRequest("Message body cannot be empty".into())
                }
                crate::storage::chat::EditError::BodyTooLong => {
                    AppError::BadRequest("Message body must be 4000 characters or fewer".into())
                }
                crate::storage::chat::EditError::Db(e) => AppError::Internal(e.to_string()),
            })?;

        (msg, room_id)
    };

    if let Ok(map) = state.realtime.chat_rooms_broadcast.lock()
        && let Some(tx) = map.get(&room_id)
    {
        let _ = tx.send(ChatEvent::MessageEdited {
            message: updated_msg.clone(),
        });
    }

    Ok(axum::Json(updated_msg))
}

/// DELETE /api/v1/universes/:slug/chat/rooms/:room_slug/messages/:msg_id
pub async fn delete_message_handler(
    State(state): State<AppState>,
    Path((slug, room_slug, msg_id)): Path<(String, String, String)>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let (deleted_at, room_id) = {
        let storage = lock_storage(&state);

        // CO-198: DM path — caller can only delete own messages (no moderation role)
        let (room, caller_can_moderate) = if slug == "dm" {
            let r = storage
                .get_dm_room_by_slug(&room_slug)
                .ok_or_else(|| AppError::NotFound(format!("DM room '{room_slug}' not found")))?;
            if !storage.is_dm_member(&r.id, &user_id.0) {
                return Err(AppError::Forbidden("Not a member of this DM".into()));
            }
            (r, false)
        } else {
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

            let role = resolve_role(&storage, &slug, &user_id.0).ok_or_else(|| {
                AppError::Forbidden("Chat is only available to universe members".into())
            })?;

            if !can_read(&role) {
                return Err(AppError::Forbidden(
                    "Insufficient role to access chat".into(),
                ));
            }

            let can_mod = matches!(role.as_str(), "owner" | "admin");
            let r = storage
                .get_chat_room_by_slug(&slug, &room_slug)
                .ok_or_else(|| AppError::NotFound(format!("Room '{room_slug}' not found")))?;
            (r, can_mod)
        };
        let room_id = room.id.clone();
        let deleted_at = storage
            .delete_chat_message(&msg_id, &user_id.0, caller_can_moderate)
            .map_err(|e| match e {
                crate::storage::chat::DeleteError::NotFound => {
                    AppError::NotFound("Message not found".into())
                }
                crate::storage::chat::DeleteError::NotAuthorAndNotMod => {
                    AppError::Forbidden("You can only delete your own messages".into())
                }
                crate::storage::chat::DeleteError::AlreadyDeleted => {
                    AppError::Gone("Message has already been deleted".into())
                }
                crate::storage::chat::DeleteError::Db(e) => AppError::Internal(e.to_string()),
            })?;

        (deleted_at, room_id)
    };

    let deleted_at_str = deleted_at.to_rfc3339();

    if let Ok(map) = state.realtime.chat_rooms_broadcast.lock()
        && let Some(tx) = map.get(&room_id)
    {
        let _ = tx.send(ChatEvent::MessageDeleted {
            message_id: msg_id.clone(),
            deleted_at: deleted_at_str.clone(),
        });
    }

    Ok(axum::Json(DeleteMessageResponse {
        deleted_at: deleted_at_str,
    }))
}
