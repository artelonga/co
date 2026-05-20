//! CO-193 — Per-universe chat: schema + REST endpoints (Phase 4 first slice).
//!
//! ## Endpoints (all require auth)
//!
//! - GET  /api/v1/universes/:slug/chat/rooms              — list rooms (member+)
//! - POST /api/v1/universes/:slug/chat/rooms              — create room (owner/admin)
//! - GET  /api/v1/universes/:slug/chat/rooms/:room/messages — paginate history (member+)
//! - POST /api/v1/universes/:slug/chat/rooms/:room/messages — post message (member, not viewer/subscriber)

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch},
};
use serde::{Deserialize, Serialize};

use crate::auth::{UserId, extractors::AuthedUser};
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn chat_router() -> Router<AppState> {
    Router::new()
        .route(
            "/{slug}/chat/rooms",
            get(list_rooms_handler).post(create_room_handler),
        )
        .route(
            "/{slug}/chat/rooms/{room_slug}/members",
            get(list_room_members_handler),
        )
        .route(
            "/{slug}/chat/rooms/{room_slug}/messages",
            get(list_messages_handler).post(post_message_handler),
        )
        .route(
            "/{slug}/chat/rooms/{room_slug}/messages/{msg_id}",
            patch(edit_message_handler).delete(delete_message_handler),
        )
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateRoomRequest {
    pub name: String,
    pub description: Option<String>,
}

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
pub struct ListRoomsResponse {
    pub rooms: Vec<crate::storage::chat::ChatRoom>,
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
// Helper
// ---------------------------------------------------------------------------

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.storage.lock()
}

/// Resolve the effective role of a caller for a universe.
/// Returns `Some(role)` if the caller is a member, `None` if anonymous / non-member.
/// Subscribers (in the `subscriptions` table but not `universe_members`) get role "subscriber".
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

/// True when the role allows reading (list rooms, read messages).
fn can_read(role: &str) -> bool {
    matches!(role, "owner" | "admin" | "member" | "viewer" | "subscriber")
}

/// True when the role allows posting messages (member or above, not viewer/subscriber).
fn can_post(role: &str) -> bool {
    matches!(role, "owner" | "admin" | "member")
}

/// True when the role allows creating or archiving rooms.
fn can_manage_rooms(role: &str) -> bool {
    matches!(role, "owner" | "admin")
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/:slug/chat/rooms
pub async fn list_rooms_handler(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user: AuthedUser,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let include_archived = params
        .get("include_archived")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let storage = lock_storage(&state);

    storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let role = resolve_role(&storage, &slug, &user.user_id)
        .ok_or_else(|| AppError::Forbidden("Chat is only available to universe members".into()))?;

    if !can_read(&role) {
        return Err(AppError::Forbidden("Insufficient role to read chat".into()));
    }

    let rooms = storage.list_chat_rooms(&slug, include_archived);
    Ok(axum::Json(ListRoomsResponse { rooms }))
}

/// GET /api/v1/universes/:slug/chat/rooms/:room_slug/members (CO-209)
pub async fn list_room_members_handler(
    State(state): State<AppState>,
    Path((slug, room_slug)): Path<(String, String)>,
    user: AuthedUser,
) -> Result<axum::Json<Vec<crate::storage::chat::ChatRoomMemberInfo>>, AppError> {
    let storage = lock_storage(&state);

    let room = if slug == "dm" {
        storage
            .get_dm_room_by_slug(&room_slug)
            .ok_or_else(|| AppError::NotFound(format!("DM room '{room_slug}' not found")))?
    } else {
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        let role = resolve_role(&storage, &slug, &user.user_id)
            .ok_or_else(|| AppError::Forbidden("Not a member of this universe".into()))?;
        if !can_read(&role) {
            return Err(AppError::Forbidden("Insufficient role to read chat".into()));
        }
        storage
            .get_chat_room_by_slug(&slug, &room_slug)
            .ok_or_else(|| AppError::NotFound(format!("Room '{room_slug}' not found")))?
    };

    // For DMs, verify caller is a participant.
    if slug == "dm" && !storage.is_dm_member(&room.id, &user.user_id) {
        return Err(AppError::Forbidden("Not a member of this DM".into()));
    }

    let members = storage.list_chat_room_members(&room.id);
    Ok(axum::Json(members))
}

/// POST /api/v1/universes/:slug/chat/rooms
pub async fn create_room_handler(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user: AuthedUser,
    axum::Json(body): axum::Json<CreateRoomRequest>,
) -> Result<impl IntoResponse, AppError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("Room name cannot be empty".into()));
    }
    if name.len() > 100 {
        return Err(AppError::BadRequest(
            "Room name must be 100 characters or fewer".into(),
        ));
    }

    let storage = lock_storage(&state);

    storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let role = resolve_role(&storage, &slug, &user.user_id)
        .ok_or_else(|| AppError::Forbidden("Chat is only available to universe members".into()))?;

    if !can_manage_rooms(&role) {
        return Err(AppError::Forbidden(
            "Only universe owners and admins can create rooms".into(),
        ));
    }

    let room = storage
        .create_chat_room(&slug, &name, body.description.as_deref(), &user.user_id)
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                AppError::Conflict("A room with this name already exists in this universe".into())
            } else {
                AppError::Internal(e.to_string())
            }
        })?;

    Ok((StatusCode::CREATED, axum::Json(room)))
}

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
        && let Ok(map) = state.chat_rooms_broadcast.lock()
        && let Some(tx) = map.get(&room_id)
    {
        let _ = tx.send(crate::chat_ws::ChatEvent::MessageCreated { message: msg });
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

    if let Ok(map) = state.chat_rooms_broadcast.lock()
        && let Some(tx) = map.get(&room_id)
    {
        let _ = tx.send(crate::chat_ws::ChatEvent::MessageEdited {
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

    if let Ok(map) = state.chat_rooms_broadcast.lock()
        && let Some(tx) = map.get(&room_id)
    {
        let _ = tx.send(crate::chat_ws::ChatEvent::MessageDeleted {
            message_id: msg_id.clone(),
            deleted_at: deleted_at_str.clone(),
        });
    }

    Ok(axum::Json(DeleteMessageResponse {
        deleted_at: deleted_at_str,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use rusqlite::params;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::storage::Storage;

    fn isolate_env() {
        unsafe {
            std::env::set_var("JWT_SECRET", "test-jwt-secret");
        }
    }

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
            port: 3000,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "quilomboaraucaria".to_string(),
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

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
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
        let state: crate::server::AppState = Arc::new(crate::server::AppStateInner {
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
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
            event_bus: crate::events::Bus::new(),
            worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
        });
        crate::server::build_router(state, None)
    }

    fn insert_user(dir: &std::path::Path, email: &str) -> String {
        let storage = Storage::new(dir.to_str().unwrap());
        let id = format!("usr_test_{}", &nanoid::nanoid!(8));
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
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, owner_id, now],
            )
            .expect("insert owner member");
        // seed the default general room
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
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![universe_key, user_id, role, now],
            )
            .expect("insert member");
    }

    fn add_subscriber(dir: &std::path::Path, universe_key: &str, user_id: &str) {
        let storage = Storage::new(dir.to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO subscriptions (user_id, universe_key, subscribed_at) \
                 VALUES (?1, ?2, ?3)",
                params![user_id, universe_key, now],
            )
            .expect("insert subscriber");
    }

    fn make_jwt(user_id: &str) -> String {
        unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
        let (token, _) =
            crate::auth::sign_jwt(user_id, "test@example.com", "player", "test-jwt-secret")
                .unwrap();
        token
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    // --- 1. GET /rooms — 401 for unauthenticated ---

    #[tokio::test]
    async fn test_list_rooms_unauthenticated() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner@example.com");
        insert_universe(dir.path(), "uni", &owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/uni/chat/rooms")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // --- 2. GET /rooms — 403 for non-member ---

    #[tokio::test]
    async fn test_list_rooms_non_member_forbidden() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner2@example.com");
        let outsider_id = insert_user(dir.path(), "outsider@example.com");
        insert_universe(dir.path(), "uni2", &owner_id);
        let token = make_jwt(&outsider_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/uni2/chat/rooms")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 3. GET /rooms — 200 with general room for member ---

    #[tokio::test]
    async fn test_list_rooms_member_sees_general() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner3@example.com");
        insert_universe(dir.path(), "uni3", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/uni3/chat/rooms")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let rooms = json["rooms"].as_array().expect("rooms array");
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0]["slug"], "general");
        assert_eq!(rooms[0]["is_default"], true);
    }

    // --- 4. POST /rooms — owner can create a room ---

    #[tokio::test]
    async fn test_create_room_owner_ok() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner4@example.com");
        insert_universe(dir.path(), "uni4", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni4/chat/rooms")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"Random","description":"Off-topic"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["slug"], "random");
    }

    // --- 5. POST /rooms — regular member cannot create ---

    #[tokio::test]
    async fn test_create_room_member_forbidden() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner5@example.com");
        let member_id = insert_user(dir.path(), "member5@example.com");
        insert_universe(dir.path(), "uni5", &owner_id);
        add_member(dir.path(), "uni5", &member_id, "member");
        let token = make_jwt(&member_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni5/chat/rooms")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"nope"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 6. POST /rooms — 409 on slug collision ---

    #[tokio::test]
    async fn test_create_room_slug_collision_409() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner6@example.com");
        insert_universe(dir.path(), "uni6", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        // First creation
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni6/chat/rooms")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"Clash"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Same name → slug collision
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni6/chat/rooms")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"Clash"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    // --- 7. POST /messages — member can post ---

    #[tokio::test]
    async fn test_post_message_member_ok() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner7@example.com");
        let member_id = insert_user(dir.path(), "member7@example.com");
        insert_universe(dir.path(), "uni7", &owner_id);
        add_member(dir.path(), "uni7", &member_id, "member");
        let token = make_jwt(&member_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni7/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"Olá pessoal!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "body: {:?}",
            body_json(resp.into_body()).await
        );
    }

    // --- 8. POST /messages — viewer cannot post ---

    #[tokio::test]
    async fn test_post_message_viewer_forbidden() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner8@example.com");
        let viewer_id = insert_user(dir.path(), "viewer8@example.com");
        insert_universe(dir.path(), "uni8", &owner_id);
        add_member(dir.path(), "uni8", &viewer_id, "viewer");
        let token = make_jwt(&viewer_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni8/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"can I post?"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 9. POST /messages — subscriber cannot post ---

    #[tokio::test]
    async fn test_post_message_subscriber_forbidden() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner9@example.com");
        let sub_id = insert_user(dir.path(), "sub9@example.com");
        insert_universe(dir.path(), "uni9", &owner_id);
        add_subscriber(dir.path(), "uni9", &sub_id);
        let token = make_jwt(&sub_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni9/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"subscriber post?"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 10. POST /messages — 400 on empty body ---

    #[tokio::test]
    async fn test_post_message_empty_body_400() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner10@example.com");
        insert_universe(dir.path(), "uni10", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni10/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- 11. POST /messages — 400 on body too long ---

    #[tokio::test]
    async fn test_post_message_too_long_400() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner11@example.com");
        insert_universe(dir.path(), "uni11", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let long_body = "a".repeat(4001);
        let payload = serde_json::json!({"body": long_body}).to_string();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni11/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- 12. GET /messages — pagination with has_more ---

    #[tokio::test]
    async fn test_list_messages_pagination() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner12@example.com");
        insert_universe(dir.path(), "uni12", &owner_id);

        // Insert 5 messages directly into storage
        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni12", "general")
                .expect("general room");
            for i in 0..5 {
                let body = format!("message {i}");
                storage
                    .post_chat_message(&room.id, &owner_id, &body, None)
                    .unwrap();
            }
        }

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        // Request only 3 → has_more = true
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/uni12/chat/rooms/general/messages?limit=3")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["messages"].as_array().unwrap().len(), 3);
        assert_eq!(json["has_more"], true);

        // Request all 10 → has_more = false
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/uni12/chat/rooms/general/messages?limit=10")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["messages"].as_array().unwrap().len(), 5);
        assert_eq!(json["has_more"], false);
    }

    // --- 13. Soft-deleted message returns tombstone ---

    #[tokio::test]
    async fn test_soft_deleted_message_tombstone() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner13@example.com");
        insert_universe(dir.path(), "uni13", &owner_id);

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni13", "general")
                .expect("general room");
            let mid = storage
                .post_chat_message(&room.id, &owner_id, "original text", None)
                .unwrap();
            // Soft-delete it
            let now = chrono::Utc::now().to_rfc3339();
            storage
                .conn()
                .execute(
                    "UPDATE chat_messages SET deleted_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, mid],
                )
                .unwrap();
            mid
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/uni13/chat/rooms/general/messages")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let msgs = json["messages"].as_array().unwrap();
        let msg = msgs.iter().find(|m| m["id"] == msg_id).expect("msg found");
        assert_eq!(msg["body"], "[mensagem removida]");
        assert!(msg["deleted_at"].is_string());
    }

    // --- 14. POST /rooms — admin can create (not just owner) ---

    #[tokio::test]
    async fn test_create_room_admin_ok() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner14@example.com");
        let admin_id = insert_user(dir.path(), "admin14@example.com");
        insert_universe(dir.path(), "uni14", &owner_id);
        add_member(dir.path(), "uni14", &admin_id, "admin");
        let token = make_jwt(&admin_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni14/chat/rooms")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"Admin Room"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // --- 15. backfill_default_rooms is idempotent ---

    #[tokio::test]
    async fn test_backfill_default_rooms_idempotent() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let owner_id = "usr_test_owner";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at) \
                 VALUES (?1, 'o@x.com', 'o', 'player', ?2)",
                rusqlite::params![owner_id, now],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT INTO universes (key, name, description, owner_id, created_at, visibility) \
                 VALUES ('bf-uni', 'BF', '', ?1, ?2, 'private')",
                rusqlite::params![owner_id, now],
            )
            .unwrap();

        // First run: seeds the new universe (and any others that lack a room).
        let n1 = storage.backfill_default_rooms();
        assert!(n1 >= 1, "first run must insert at least 1 room (got {n1})");

        // Second run: everything already seeded → no-op.
        let n2 = storage.backfill_default_rooms();
        assert_eq!(n2, 0, "second run must be a no-op");
    }

    // --- 16. GET /rooms — subscriber can read rooms ---

    #[tokio::test]
    async fn test_subscriber_can_read_rooms() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner16@example.com");
        let sub_id = insert_user(dir.path(), "sub16@example.com");
        insert_universe(dir.path(), "uni16", &owner_id);
        add_subscriber(dir.path(), "uni16", &sub_id);
        let token = make_jwt(&sub_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/uni16/chat/rooms")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    fn make_state_inner(dir: &std::path::Path) -> crate::server::AppState {
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
        Arc::new(crate::server::AppStateInner {
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
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
            event_bus: crate::events::Bus::new(),
            worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
        })
    }

    // --- 17. POST /messages — rate limit 429 ---

    #[tokio::test]
    async fn test_post_message_rate_limit() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner17@example.com");
        insert_universe(dir.path(), "uni17", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        // Send 21 messages — the 21st should hit rate limit
        let mut last_status = StatusCode::CREATED;
        for _ in 0..21 {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/universes/uni17/chat/rooms/general/messages")
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::from(r#"{"body":"ping"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            last_status = resp.status();
        }

        assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);
    }

    // =========================================================================
    // CO-196 — message edit + delete moderation
    // =========================================================================

    // --- 18. PATCH within edit window by author → 200 ---

    #[tokio::test]
    async fn test_edit_within_window_by_author_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner18@example.com");
        insert_universe(dir.path(), "uni18", &owner_id);

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni18", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &owner_id, "original", None)
                .unwrap()
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!(
                        "/api/v1/universes/uni18/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"edited text"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["body"], "edited text");
        assert!(json["edited_at"].is_string());
    }

    // --- 19. PATCH outside edit window → 403 ---

    #[tokio::test]
    async fn test_edit_outside_window_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner19@example.com");
        insert_universe(dir.path(), "uni19", &owner_id);

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni19", "general")
                .expect("room");
            let mid = storage
                .post_chat_message(&room.id, &owner_id, "old msg", None)
                .unwrap();
            let old_ts = (chrono::Utc::now() - chrono::Duration::minutes(16)).to_rfc3339();
            storage
                .conn()
                .execute(
                    "UPDATE chat_messages SET created_at = ?1 WHERE id = ?2",
                    rusqlite::params![old_ts, mid],
                )
                .unwrap();
            mid
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!(
                        "/api/v1/universes/uni19/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"try edit"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 20. PATCH by non-author → 403 ---

    #[tokio::test]
    async fn test_edit_by_non_author_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner20@example.com");
        let member_id = insert_user(dir.path(), "member20@example.com");
        insert_universe(dir.path(), "uni20", &owner_id);
        add_member(dir.path(), "uni20", &member_id, "member");

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni20", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &owner_id, "owner msg", None)
                .unwrap()
        };

        let token = make_jwt(&member_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!(
                        "/api/v1/universes/uni20/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"hacked edit"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 21. PATCH on deleted message → 410 ---

    #[tokio::test]
    async fn test_edit_deleted_message_410() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner21@example.com");
        insert_universe(dir.path(), "uni21", &owner_id);

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni21", "general")
                .expect("room");
            let mid = storage
                .post_chat_message(&room.id, &owner_id, "msg", None)
                .unwrap();
            let now = chrono::Utc::now().to_rfc3339();
            storage
                .conn()
                .execute(
                    "UPDATE chat_messages SET deleted_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, mid],
                )
                .unwrap();
            mid
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!(
                        "/api/v1/universes/uni21/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"edit deleted"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::GONE);
    }

    // --- 22. PATCH with empty body → 400 ---

    #[tokio::test]
    async fn test_edit_empty_body_400() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner22@example.com");
        insert_universe(dir.path(), "uni22", &owner_id);

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni22", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &owner_id, "msg", None)
                .unwrap()
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!(
                        "/api/v1/universes/uni22/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- 23. DELETE by author → 200 ---

    #[tokio::test]
    async fn test_delete_by_author_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner23@example.com");
        insert_universe(dir.path(), "uni23", &owner_id);

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni23", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &owner_id, "my msg", None)
                .unwrap()
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/universes/uni23/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert!(json["deleted_at"].is_string());
    }

    // --- 24. DELETE by owner of other member's msg → 200 ---

    #[tokio::test]
    async fn test_delete_by_owner_of_other_msg_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner24@example.com");
        let member_id = insert_user(dir.path(), "member24@example.com");
        insert_universe(dir.path(), "uni24", &owner_id);
        add_member(dir.path(), "uni24", &member_id, "member");

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni24", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &member_id, "member msg", None)
                .unwrap()
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/universes/uni24/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    // --- 25. DELETE by admin of other member's msg → 200 ---

    #[tokio::test]
    async fn test_delete_by_admin_of_other_msg_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner25@example.com");
        let admin_id = insert_user(dir.path(), "admin25@example.com");
        let member_id = insert_user(dir.path(), "member25@example.com");
        insert_universe(dir.path(), "uni25", &owner_id);
        add_member(dir.path(), "uni25", &admin_id, "admin");
        add_member(dir.path(), "uni25", &member_id, "member");

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni25", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &member_id, "member msg", None)
                .unwrap()
        };

        let token = make_jwt(&admin_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/universes/uni25/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    // --- 26. DELETE by member of someone else's msg → 403 ---

    #[tokio::test]
    async fn test_delete_by_member_of_other_msg_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner26@example.com");
        let member_id = insert_user(dir.path(), "member26@example.com");
        insert_universe(dir.path(), "uni26", &owner_id);
        add_member(dir.path(), "uni26", &member_id, "member");

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni26", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &owner_id, "owner msg", None)
                .unwrap()
        };

        let token = make_jwt(&member_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/universes/uni26/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 27. DELETE by viewer of someone else's msg → 403 ---

    #[tokio::test]
    async fn test_delete_by_viewer_of_other_msg_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner27@example.com");
        let viewer_id = insert_user(dir.path(), "viewer27@example.com");
        insert_universe(dir.path(), "uni27", &owner_id);
        add_member(dir.path(), "uni27", &viewer_id, "viewer");

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni27", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &owner_id, "owner msg", None)
                .unwrap()
        };

        let token = make_jwt(&viewer_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/universes/uni27/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 28. DELETE on already-deleted message → 410 ---

    #[tokio::test]
    async fn test_delete_already_deleted_410() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner28@example.com");
        insert_universe(dir.path(), "uni28", &owner_id);

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni28", "general")
                .expect("room");
            let mid = storage
                .post_chat_message(&room.id, &owner_id, "msg", None)
                .unwrap();
            let now = chrono::Utc::now().to_rfc3339();
            storage
                .conn()
                .execute(
                    "UPDATE chat_messages SET deleted_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, mid],
                )
                .unwrap();
            mid
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/universes/uni28/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::GONE);
    }

    // --- 29. PATCH broadcasts message.edited ---

    #[tokio::test]
    async fn test_edit_broadcasts_message_edited_event() {
        use tokio::sync::broadcast;
        use tokio::time::Duration;
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner29@example.com");
        insert_universe(dir.path(), "uni29", &owner_id);

        let state = make_state_inner(dir.path());

        let (msg_id, room_id) = {
            let storage = state.storage.lock();
            let room = storage
                .get_chat_room_by_slug("uni29", "general")
                .expect("room");
            let mid = storage
                .post_chat_message(&room.id, &owner_id, "original", None)
                .unwrap();
            (mid, room.id.clone())
        };

        let (tx, mut rx) = broadcast::channel::<crate::chat_ws::ChatEvent>(64);
        {
            let mut map = state.chat_rooms_broadcast.lock().unwrap();
            map.insert(room_id.clone(), tx);
        }

        let app = crate::server::build_router(Arc::clone(&state), None);
        let token = make_jwt(&owner_id);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!(
                        "/api/v1/universes/uni29/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"edited!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let evt = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("timeout waiting for broadcast")
            .unwrap();

        assert!(
            matches!(evt, crate::chat_ws::ChatEvent::MessageEdited { .. }),
            "expected MessageEdited, got {evt:?}"
        );
    }

    // --- 30. DELETE broadcasts message.deleted ---

    #[tokio::test]
    async fn test_delete_broadcasts_message_deleted_event() {
        use tokio::sync::broadcast;
        use tokio::time::Duration;
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner30@example.com");
        insert_universe(dir.path(), "uni30", &owner_id);

        let state = make_state_inner(dir.path());

        let (msg_id, room_id) = {
            let storage = state.storage.lock();
            let room = storage
                .get_chat_room_by_slug("uni30", "general")
                .expect("room");
            let mid = storage
                .post_chat_message(&room.id, &owner_id, "to delete", None)
                .unwrap();
            (mid, room.id.clone())
        };

        let (tx, mut rx) = broadcast::channel::<crate::chat_ws::ChatEvent>(64);
        {
            let mut map = state.chat_rooms_broadcast.lock().unwrap();
            map.insert(room_id.clone(), tx);
        }

        let app = crate::server::build_router(Arc::clone(&state), None);
        let token = make_jwt(&owner_id);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/universes/uni30/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let evt = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("timeout waiting for broadcast")
            .unwrap();

        assert!(
            matches!(evt, crate::chat_ws::ChatEvent::MessageDeleted { .. }),
            "expected MessageDeleted, got {evt:?}"
        );
    }

    // --- 31. GET messages returns tombstone after DELETE ---

    #[tokio::test]
    async fn test_get_messages_returns_tombstone_for_deleted() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner31@example.com");
        insert_universe(dir.path(), "uni31", &owner_id);

        let msg_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let room = storage
                .get_chat_room_by_slug("uni31", "general")
                .expect("room");
            storage
                .post_chat_message(&room.id, &owner_id, "secret msg", None)
                .unwrap()
        };

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let del_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/universes/uni31/chat/rooms/general/messages/{msg_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::OK);

        let get_resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/uni31/chat/rooms/general/messages")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(get_resp.status(), StatusCode::OK);
        let json = body_json(get_resp.into_body()).await;
        let msgs = json["messages"].as_array().unwrap();
        let msg = msgs
            .iter()
            .find(|m| m["id"] == msg_id)
            .expect("msg in list");
        assert_eq!(msg["body"], "[mensagem removida]");
        assert!(msg["deleted_at"].is_string());
    }
}
