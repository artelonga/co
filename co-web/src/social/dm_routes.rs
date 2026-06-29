//! CO-198 — Private DMs: 1:1 chat with inbox, unread counts, and privacy controls.
//!
//! ## Endpoints (all require auth)
//!
//! - POST /api/v1/dms/with/:user_id        — open (or return existing) DM room
//! - GET  /api/v1/me/dms                   — inbox: threads + unread counts
//! - POST /api/v1/dms/:room_id/read        — mark thread read
//! - POST /api/v1/dms/:room_id/mute        — mute/unmute thread
//! - PUT  /api/v1/me/dm-policy             — update DM privacy policy
//! - POST /api/v1/users/:user_id/block     — block a user
//! - DELETE /api/v1/users/:user_id/block   — unblock a user

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
use crate::error::AppError;
use crate::server::AppState;
use crate::storage::chat::DmError;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn dm_router() -> Router<AppState> {
    Router::new()
        .route("/dms/with/{user_id}", post(open_dm_handler))
        .route("/me/dms", get(list_dms_handler))
        .route("/dms/{room_id}/read", post(mark_read_handler))
        .route("/dms/{room_id}/mute", post(mute_handler))
        .route("/me/dm-policy", put(set_dm_policy_handler))
        .route("/users/{user_id}/block", post(block_user_handler))
        .route("/users/{user_id}/block", delete(unblock_user_handler))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct OpenDmResponse {
    room_id: String,
    slug: String,
    other_user: crate::storage::chat::ChatAuthor,
    created: bool,
}

#[derive(Debug, Serialize)]
struct ListDmsResponse {
    threads: Vec<crate::storage::chat::DmThreadPreview>,
    total_unread: i64,
}

#[derive(Debug, Serialize)]
struct MarkReadResponse {
    last_read_at: String,
}

#[derive(Debug, Deserialize)]
pub struct MuteRequest {
    until: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DmPolicyRequest {
    policy: String,
}

#[derive(Debug, Deserialize)]
pub struct BlockRequest {
    reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.core.storage.lock()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/dms/with/:user_id
///
/// Idempotent: second call returns same room_id with `created: false`.
pub async fn open_dm_handler(
    State(state): State<AppState>,
    Path(other_user_id): Path<String>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    if user_id.0 == other_user_id {
        return Err(AppError::BadRequest(
            "dm.error.self_dm: cannot DM yourself".into(),
        ));
    }

    let storage = lock_storage(&state);

    match storage.open_dm(&user_id.0, &other_user_id) {
        Ok(dm) => Ok((
            StatusCode::OK,
            axum::Json(OpenDmResponse {
                room_id: dm.room_id,
                slug: dm.slug,
                other_user: dm.other_user,
                created: dm.created,
            }),
        )),
        Err(DmError::SelfDm) => Err(AppError::BadRequest("dm.error.self_dm".into())),
        Err(DmError::Blocked) => Err(AppError::Forbidden("dm.error.blocked".into())),
        Err(DmError::PolicyDenied) => Err(AppError::Forbidden("dm.error.policy_denied".into())),
        Err(DmError::UserNotFound) => Err(AppError::NotFound("User not found".into())),
        Err(DmError::Db(e)) => Err(AppError::Internal(e.to_string())),
    }
}

/// GET /api/v1/me/dms
pub async fn list_dms_handler(
    State(state): State<AppState>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);
    let threads = storage.list_my_dms(&user_id.0);
    let total_unread: i64 = threads.iter().map(|t| t.unread_count).sum();
    Ok(axum::Json(ListDmsResponse {
        threads,
        total_unread,
    }))
}

/// POST /api/v1/dms/:room_id/read
pub async fn mark_read_handler(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);

    // Verify membership
    if !storage.is_dm_member(&room_id, &user_id.0) {
        return Err(AppError::Forbidden("Not a member of this DM".into()));
    }

    let last_read_at = storage
        .mark_dm_read(&room_id, &user_id.0)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(axum::Json(MarkReadResponse { last_read_at }))
}

/// POST /api/v1/dms/:room_id/mute
pub async fn mute_handler(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    user_id: UserId,
    axum::Json(body): axum::Json<MuteRequest>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);

    if !storage.is_dm_member(&room_id, &user_id.0) {
        return Err(AppError::Forbidden("Not a member of this DM".into()));
    }

    storage
        .mute_dm(&room_id, &user_id.0, body.until.as_deref())
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(StatusCode::OK)
}

/// PUT /api/v1/me/dm-policy
pub async fn set_dm_policy_handler(
    State(state): State<AppState>,
    user_id: UserId,
    axum::Json(body): axum::Json<DmPolicyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let policy = body.policy.as_str();
    if !matches!(policy, "everyone" | "shared-universe" | "nobody") {
        return Err(AppError::BadRequest(
            "policy must be 'everyone', 'shared-universe', or 'nobody'".into(),
        ));
    }

    let storage = lock_storage(&state);
    storage
        .set_dm_policy(&user_id.0, policy)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(StatusCode::OK)
}

/// POST /api/v1/users/:user_id/block
pub async fn block_user_handler(
    State(state): State<AppState>,
    Path(target_id): Path<String>,
    user_id: UserId,
    body: Option<axum::Json<BlockRequest>>,
) -> Result<impl IntoResponse, AppError> {
    if user_id.0 == target_id {
        return Err(AppError::BadRequest("Cannot block yourself".into()));
    }

    let reason = body.and_then(|b| b.reason.clone());
    let storage = lock_storage(&state);
    storage
        .block_user(&user_id.0, &target_id, reason.as_deref())
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(StatusCode::OK)
}

/// DELETE /api/v1/users/:user_id/block
pub async fn unblock_user_handler(
    State(state): State<AppState>,
    Path(target_id): Path<String>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);
    storage
        .unblock_user(&user_id.0, &target_id)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::server::{CoreState, IndexState, IntegrationsState, RealtimeState};
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
            universo_dir: "universo".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
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
        let state: crate::server::AppState =
            crate::server::AppState::new(crate::server::AppStateInner {
                core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
                realtime: Arc::new(RealtimeState {
                    doc_rooms: crate::ws::new_room_manager(),
                    sync_rooms: crate::sync_ws::new_sync_room_manager(),
                    chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
                    chat_presence: Mutex::new(std::collections::HashMap::new()),
                }),
                index: Arc::new(IndexState {
                    cache: crate::cache::CacheLayer::new(),
                    embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                    embedding_tx,
                }),
                integrations: Arc::new(IntegrationsState {
                    mail,
                    geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                    plugin_registry: game_core::plugin::PluginRegistry::new(),
                    game_storage,
                    wae: crate::wae::WaeEmitter::new(None, None),
                    jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                    rate_limiter: Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                    experiment: Mutex::new(experiment),
                    worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
                }),
            });
        crate::server::build_router(state, None)
    }

    fn insert_user(dir: &std::path::Path, email: &str) -> String {
        let storage = Storage::new(dir.to_str().unwrap());
        let id = format!("usr_{}", &nanoid::nanoid!(8));
        let usuario = email.split('@').next().unwrap_or("u").to_lowercase();
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
                 VALUES (?1, ?2, ?3, 'player', ?4, ?5)",
                params![id, email, email, now, usuario],
            )
            .expect("insert user");
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
            .expect("insert universe");
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, owner_id, now],
            )
            .expect("insert owner");
        storage
            .ensure_default_room(key)
            .expect("ensure general room");
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

    fn set_dm_policy(dir: &std::path::Path, user_id: &str, policy: &str) {
        let storage = Storage::new(dir.to_str().unwrap());
        storage
            .conn()
            .execute(
                "UPDATE users SET dm_policy = ?1 WHERE id = ?2",
                params![policy, user_id],
            )
            .expect("set dm_policy");
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

    // -----------------------------------------------------------------------
    // 1. test_open_dm_creates_room — first call inserts
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_open_dm_creates_room() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a@example.com");
        let b = insert_user(dir.path(), "b@example.com");
        insert_universe(dir.path(), "shared", &a);
        add_member(dir.path(), "shared", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        let token = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{b}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert!(json["room_id"].is_string());
        assert_eq!(json["created"], true);
    }

    // -----------------------------------------------------------------------
    // 2. test_open_dm_idempotent — second call returns same room
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_open_dm_idempotent() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a2@example.com");
        let b = insert_user(dir.path(), "b2@example.com");
        insert_universe(dir.path(), "shared2", &a);
        add_member(dir.path(), "shared2", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp1 = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{b}"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let json1 = body_json(resp1.into_body()).await;
        let room_id1 = json1["room_id"].as_str().unwrap().to_string();
        assert_eq!(json1["created"], true);

        let resp2 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{b}"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let json2 = body_json(resp2.into_body()).await;
        assert_eq!(json2["room_id"].as_str().unwrap(), room_id1);
        assert_eq!(json2["created"], false);
    }

    // -----------------------------------------------------------------------
    // 3. test_open_dm_canonical_pair_ordering — A→B and B→A same slug
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_open_dm_canonical_pair_ordering() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a3@example.com");
        let b = insert_user(dir.path(), "b3@example.com");
        insert_universe(dir.path(), "shared3", &a);
        add_member(dir.path(), "shared3", &b, "member");
        set_dm_policy(dir.path(), &a, "everyone");
        set_dm_policy(dir.path(), &b, "everyone");

        let token_a = make_jwt(&a);
        let token_b = make_jwt(&b);
        let app = build_test_router(dir.path());

        let resp_a = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{b}"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json_a = body_json(resp_a.into_body()).await;
        let slug_a = json_a["slug"].as_str().unwrap().to_string();

        let resp_b = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{a}"))
                    .header("authorization", format!("Bearer {token_b}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json_b = body_json(resp_b.into_body()).await;
        let slug_b = json_b["slug"].as_str().unwrap().to_string();

        assert_eq!(
            slug_a, slug_b,
            "canonical slug must match regardless of direction"
        );
    }

    // -----------------------------------------------------------------------
    // 4. test_open_dm_blocked_403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_open_dm_blocked_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a4@example.com");
        let b = insert_user(dir.path(), "b4@example.com");
        set_dm_policy(dir.path(), &b, "everyone");

        // b blocks a
        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage.block_user(&b, &a, None).unwrap();
        }

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{b}"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // 5. test_open_dm_policy_nobody_403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_open_dm_policy_nobody_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a5@example.com");
        let b = insert_user(dir.path(), "b5@example.com");
        set_dm_policy(dir.path(), &b, "nobody");

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{b}"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // 6. test_open_dm_policy_shared_universe_no_overlap_403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_open_dm_policy_shared_universe_no_overlap_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a6@example.com");
        let b = insert_user(dir.path(), "b6@example.com");
        // b's policy is 'shared-universe' (default), but they share no universe
        set_dm_policy(dir.path(), &b, "shared-universe");

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{b}"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // 7. test_open_dm_policy_shared_universe_with_overlap_200
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_open_dm_policy_shared_universe_with_overlap_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a7@example.com");
        let b = insert_user(dir.path(), "b7@example.com");
        insert_universe(dir.path(), "shared7", &a);
        add_member(dir.path(), "shared7", &b, "member");
        // both default policy 'shared-universe'

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{b}"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // 8. test_open_dm_self_400
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_open_dm_self_400() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a8@example.com");
        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/with/{a}"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // 9. test_post_message_in_dm_member_200
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_post_message_in_dm_member_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a9@example.com");
        let b = insert_user(dir.path(), "b9@example.com");
        insert_universe(dir.path(), "shared9", &a);
        add_member(dir.path(), "shared9", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        // Open DM as A
        let dm_slug = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let dm = storage.open_dm(&a, &b).unwrap();
            dm.slug
        };

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/universes/dm/chat/rooms/{dm_slug}/messages"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::from(r#"{"body":"Hello!"}"#))
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

    // -----------------------------------------------------------------------
    // 10. test_post_message_in_dm_non_member_403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_post_message_in_dm_non_member_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a10@example.com");
        let b = insert_user(dir.path(), "b10@example.com");
        let c = insert_user(dir.path(), "c10@example.com");
        insert_universe(dir.path(), "shared10", &a);
        add_member(dir.path(), "shared10", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        let dm_slug = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let dm = storage.open_dm(&a, &b).unwrap();
            dm.slug
        };

        let token_c = make_jwt(&c);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/universes/dm/chat/rooms/{dm_slug}/messages"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token_c}"))
                    .body(Body::from(r#"{"body":"intruder!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // 11. test_post_message_when_blocked_403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_post_message_when_blocked_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a11@example.com");
        let b = insert_user(dir.path(), "b11@example.com");
        insert_universe(dir.path(), "shared11", &a);
        add_member(dir.path(), "shared11", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        let dm_slug = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let dm = storage.open_dm(&a, &b).unwrap();
            dm.slug
        };

        // b blocks a after opening the DM
        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage.block_user(&b, &a, None).unwrap();
        }

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/universes/dm/chat/rooms/{dm_slug}/messages"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::from(r#"{"body":"hi?"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // 12. test_list_my_dms_returns_threads_ordered_by_last_message
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_my_dms_returns_threads_ordered_by_last_message() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a12@example.com");
        let b = insert_user(dir.path(), "b12@example.com");
        let c = insert_user(dir.path(), "c12@example.com");
        insert_universe(dir.path(), "shared12", &a);
        add_member(dir.path(), "shared12", &b, "member");
        add_member(dir.path(), "shared12", &c, "member");
        set_dm_policy(dir.path(), &b, "everyone");
        set_dm_policy(dir.path(), &c, "everyone");

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let dm_ab = storage.open_dm(&a, &b).unwrap();
            let dm_ac = storage.open_dm(&a, &c).unwrap();
            // Post to ab first, then ac (ac should come first in the list)
            storage
                .post_chat_message(&dm_ab.room_id, &a, "hi b", None, None)
                .unwrap();
            storage
                .post_chat_message(&dm_ac.room_id, &a, "hi c", None, None)
                .unwrap();
        }

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/me/dms")
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let threads = json["threads"].as_array().unwrap();
        assert_eq!(threads.len(), 2);
        // dm_ac was last posted, so it should come first
        assert_eq!(
            threads[0]["last_message"]["body_preview"], "hi c",
            "most recent thread first"
        );
    }

    // -----------------------------------------------------------------------
    // 13. test_unread_count_increments_on_new_message
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_unread_count_increments_on_new_message() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a13@example.com");
        let b = insert_user(dir.path(), "b13@example.com");
        insert_universe(dir.path(), "shared13", &a);
        add_member(dir.path(), "shared13", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let dm = storage.open_dm(&a, &b).unwrap();
            // b sends 3 messages to a
            storage
                .post_chat_message(&dm.room_id, &b, "msg 1", None, None)
                .unwrap();
            storage
                .post_chat_message(&dm.room_id, &b, "msg 2", None, None)
                .unwrap();
            storage
                .post_chat_message(&dm.room_id, &b, "msg 3", None, None)
                .unwrap();
        }

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/me/dms")
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let thread = &json["threads"][0];
        assert_eq!(thread["unread_count"], 3);
        assert_eq!(json["total_unread"], 3);
    }

    // -----------------------------------------------------------------------
    // 14. test_mark_read_clears_unread
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_mark_read_clears_unread() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a14@example.com");
        let b = insert_user(dir.path(), "b14@example.com");
        insert_universe(dir.path(), "shared14", &a);
        add_member(dir.path(), "shared14", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        let room_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let dm = storage.open_dm(&a, &b).unwrap();
            storage
                .post_chat_message(&dm.room_id, &b, "hello", None, None)
                .unwrap();
            dm.room_id
        };

        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        // Mark read
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/dms/{room_id}/read"))
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Inbox should show 0 unread
        let resp2 = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/me/dms")
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let json = body_json(resp2.into_body()).await;
        assert_eq!(json["threads"][0]["unread_count"], 0);
        assert_eq!(json["total_unread"], 0);
    }

    // -----------------------------------------------------------------------
    // 15. test_block_disables_existing_dm
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_block_disables_existing_dm() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a15@example.com");
        let b = insert_user(dir.path(), "b15@example.com");
        insert_universe(dir.path(), "shared15", &a);
        add_member(dir.path(), "shared15", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        let dm_slug = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage.open_dm(&a, &b).unwrap().slug
        };

        // b blocks a
        let token_b = make_jwt(&b);
        let app = build_test_router(dir.path());
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/users/{a}/block"))
                    .header("authorization", format!("Bearer {token_b}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // a can no longer post in the DM
        let token_a = make_jwt(&a);
        let resp2 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/universes/dm/chat/rooms/{dm_slug}/messages"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::from(r#"{"body":"after block"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // 16. test_unblock_restores_dm
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_unblock_restores_dm() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a16@example.com");
        let b = insert_user(dir.path(), "b16@example.com");
        insert_universe(dir.path(), "shared16", &a);
        add_member(dir.path(), "shared16", &b, "member");
        set_dm_policy(dir.path(), &b, "everyone");

        let dm_slug = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage.open_dm(&a, &b).unwrap().slug
        };

        let token_b = make_jwt(&b);
        let token_a = make_jwt(&a);
        let app = build_test_router(dir.path());

        // Block
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/users/{a}/block"))
                    .header("authorization", format!("Bearer {token_b}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Unblock
        let resp_unblock = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/users/{a}/block"))
                    .header("authorization", format!("Bearer {token_b}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp_unblock.status(), StatusCode::OK);

        // a can post again
        let resp2 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/universes/dm/chat/rooms/{dm_slug}/messages"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::from(r#"{"body":"restored"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::CREATED);
    }

    // -----------------------------------------------------------------------
    // 17. test_backfill_inserts_chat_room_members_from_universe_members
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_backfill_inserts_chat_room_members_from_universe_members() {
        isolate_env();
        let dir = tempdir().unwrap();
        let a = insert_user(dir.path(), "a17@example.com");
        let b = insert_user(dir.path(), "b17@example.com");
        insert_universe(dir.path(), "uni17", &a);
        add_member(dir.path(), "uni17", &b, "member");

        let storage = Storage::new(dir.path().to_str().unwrap());

        // Clear any existing chat_room_members to test backfill
        storage
            .conn()
            .execute_batch("DELETE FROM chat_room_members")
            .unwrap();

        let n = storage.backfill_chat_room_members_from_universe_members();
        assert!(n > 0, "backfill should insert at least one row");

        // Idempotent: running again should not panic
        let n2 = storage.backfill_chat_room_members_from_universe_members();
        assert!(n2 >= n, "idempotent backfill");
    }
}
