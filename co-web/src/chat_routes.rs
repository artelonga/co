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
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
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
            "/{slug}/chat/rooms/{room_slug}/messages",
            get(list_messages_handler).post(post_message_handler),
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

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn lock_storage(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::storage::Storage>, AppError> {
    state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("Storage lock failed".into()))
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
    user_id: UserId,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let include_archived = params
        .get("include_archived")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let storage = lock_storage(&state)?;

    storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let role = resolve_role(&storage, &slug, &user_id.0)
        .ok_or_else(|| AppError::Forbidden("Chat is only available to universe members".into()))?;

    if !can_read(&role) {
        return Err(AppError::Forbidden("Insufficient role to read chat".into()));
    }

    let rooms = storage.list_chat_rooms(&slug, include_archived);
    Ok(axum::Json(ListRoomsResponse { rooms }))
}

/// POST /api/v1/universes/:slug/chat/rooms
pub async fn create_room_handler(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
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

    let storage = lock_storage(&state)?;

    storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let role = resolve_role(&storage, &slug, &user_id.0)
        .ok_or_else(|| AppError::Forbidden("Chat is only available to universe members".into()))?;

    if !can_manage_rooms(&role) {
        return Err(AppError::Forbidden(
            "Only universe owners and admins can create rooms".into(),
        ));
    }

    let room = storage
        .create_chat_room(&slug, &name, body.description.as_deref(), &user_id.0)
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

    let storage = lock_storage(&state)?;

    storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let role = resolve_role(&storage, &slug, &user_id.0)
        .ok_or_else(|| AppError::Forbidden("Chat is only available to universe members".into()))?;

    if !can_read(&role) {
        return Err(AppError::Forbidden("Insufficient role to read chat".into()));
    }

    let room = storage
        .get_chat_room_by_slug(&slug, &room_slug)
        .ok_or_else(|| AppError::NotFound(format!("Room '{room_slug}' not found")))?;

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

    let storage = lock_storage(&state)?;

    storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let role = resolve_role(&storage, &slug, &user_id.0)
        .ok_or_else(|| AppError::Forbidden("Chat is only available to universe members".into()))?;

    if !can_post(&role) {
        return Err(AppError::Forbidden(
            "Viewers and subscribers cannot post messages".into(),
        ));
    }

    let room = storage
        .get_chat_room_by_slug(&slug, &room_slug)
        .ok_or_else(|| AppError::NotFound(format!("Room '{room_slug}' not found")))?;

    let msg_id = storage
        .post_chat_message(&room.id, &user_id.0, &trimmed, body.reply_to_id.as_deref())
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        axum::Json(PostMessageResponse { id: msg_id }),
    ))
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
            storage: Mutex::new(storage),
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
}
