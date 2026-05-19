use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn me_notifications_router() -> Router<AppState> {
    Router::new()
        .route("/notifications", get(list_notifications_handler))
        .route(
            "/notifications/read-all",
            post(read_all_notifications_handler),
        )
        .route(
            "/notifications/{id}/read",
            post(mark_notification_read_handler),
        )
        .route("/notification-preferences", get(get_preferences_handler))
        .route("/notification-preferences", put(put_preferences_handler))
        .layer(middleware::from_fn(crate::auth::require_auth))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ListNotificationsQuery {
    pub since: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ListNotificationsResponse {
    pub notifications: Vec<crate::storage::notifications::NotificationWithActor>,
    pub unread_count: usize,
    pub has_more: bool,
}

#[derive(Debug, Serialize)]
pub struct ReadAllResponse {
    pub count: usize,
}

/// Typed request body for `PUT /api/v1/me/notification-preferences`.
///
/// All fields are optional — omitted fields are left unchanged (partial update).
#[derive(Debug, Deserialize, Serialize)]
pub struct UpdatePreferencesRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_chat_message: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_chat_dm: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_invitation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_mention: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_chat_message: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_chat_dm: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_invitation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_mention: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_app_chat_message: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_app_chat_dm: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_app_invitation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_app_mention: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_digest_freq: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_hours_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_hours_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_hours_tz: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.storage.lock()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/me/notifications
async fn list_notifications_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Query(q): Query<ListNotificationsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let storage = lock_storage(&state);
    let (notifications, has_more) =
        storage.list_my_notifications(&user_id.0, q.since.as_deref(), limit);
    let unread_count = storage.get_unread_count(&user_id.0);
    Ok(axum::Json(ListNotificationsResponse {
        notifications,
        unread_count,
        has_more,
    }))
}

/// POST /api/v1/me/notifications/read-all
async fn read_all_notifications_handler(
    State(state): State<AppState>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);
    let count = storage.mark_all_read(&user_id.0)?;
    Ok(axum::Json(ReadAllResponse { count }))
}

/// POST /api/v1/me/notifications/{id}/read
async fn mark_notification_read_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);
    storage
        .mark_notification_read(&id, &user_id.0)
        .map_err(|e| {
            if e.to_string().contains("not found") {
                AppError::NotFound("Notification not found".into())
            } else {
                AppError::Internal(e.to_string())
            }
        })?;

    // Fetch the updated notification to return it.
    let (notifications, _) = storage.list_my_notifications(&user_id.0, None, 200);
    let notif = notifications
        .into_iter()
        .find(|n| n.id == id)
        .ok_or_else(|| AppError::NotFound("Notification not found".into()))?;

    Ok((StatusCode::OK, axum::Json(notif)))
}

/// GET /api/v1/me/notification-preferences
async fn get_preferences_handler(
    State(state): State<AppState>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);
    let prefs = storage.get_preferences(&user_id.0);
    Ok(axum::Json(prefs))
}

/// PUT /api/v1/me/notification-preferences
async fn put_preferences_handler(
    State(state): State<AppState>,
    user_id: UserId,
    axum::Json(body): axum::Json<UpdatePreferencesRequest>,
) -> Result<impl IntoResponse, AppError> {
    let valid_digest_freqs = ["instant", "hourly", "daily", "weekly", "never"];
    if let Some(freq) = &body.email_digest_freq
        && !valid_digest_freqs.contains(&freq.as_str())
    {
        return Err(AppError::BadRequest(
            "email_digest_freq must be one of: instant, hourly, daily, weekly, never".into(),
        ));
    }
    for val in [&body.quiet_hours_start, &body.quiet_hours_end]
        .iter()
        .filter_map(|o| o.as_ref())
    {
        if !is_hhmm(val) {
            return Err(AppError::BadRequest(
                "quiet_hours_start/end must match HH:MM format".into(),
            ));
        }
    }

    let patch = serde_json::to_value(&body).map_err(|e| AppError::Internal(e.to_string()))?;
    let storage = lock_storage(&state);
    storage
        .update_preferences(&user_id.0, &patch)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let prefs = storage.get_preferences(&user_id.0);
    Ok(axum::Json(prefs))
}

fn is_hhmm(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 5 {
        return false;
    }
    bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b':'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
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
        unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
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
        });
        crate::server::build_router(state, None)
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
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, owner_id, now],
            )
            .expect("insert owner member");
        storage
            .ensure_default_room(key)
            .expect("ensure_default_room");
    }

    fn add_member(dir: &std::path::Path, universe_key: &str, user_id: &str) {
        let storage = Storage::new(dir.to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'member', ?3)",
                params![universe_key, user_id, now],
            )
            .expect("insert member");
        // Also add to chat_room_members for the general room
        let room_id: Option<String> = storage
            .conn()
            .query_row(
                "SELECT id FROM chat_rooms WHERE universe_key = ?1 AND slug = 'general'",
                params![universe_key],
                |r| r.get(0),
            )
            .ok();
        if let Some(rid) = room_id {
            storage
                .conn()
                .execute(
                    "INSERT OR IGNORE INTO chat_room_members (room_id, user_id, joined_at) \
                     VALUES (?1, ?2, ?3)",
                    params![rid, user_id, now],
                )
                .expect("insert chat_room_member");
        }
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

    // =========================================================================
    // Storage-level tests (run directly against Storage without HTTP)
    // =========================================================================

    // --- test_create_notification_idempotent_within_5s ---
    #[test]
    fn test_create_notification_idempotent_within_5s() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_dedup";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        let id1 = storage
            .create_notification(
                user_id,
                "chat.message",
                None,
                None,
                "actor1",
                "msg_1",
                "notif.chat.message",
                serde_json::json!({}),
            )
            .unwrap();
        let id2 = storage
            .create_notification(
                user_id,
                "chat.message",
                None,
                None,
                "actor1",
                "msg_1",
                "notif.chat.message",
                serde_json::json!({}),
            )
            .unwrap();

        assert_eq!(id1, id2, "duplicate within 5s should return same id");

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM user_notifications WHERE user_id = ?1",
                params![user_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "only one row should exist");
    }

    // --- test_create_notification_inserts_row_with_correct_fields ---
    #[test]
    fn test_create_notification_inserts_row_with_correct_fields() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_fields";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        let id = storage
            .create_notification(
                user_id,
                "universe.invitation",
                Some("my-universe"),
                None,
                "actor_inv",
                "inv_tok_abc",
                "notif.universe.invitation",
                serde_json::json!({"name": "Maria", "universe": "my-universe"}),
            )
            .unwrap();

        let (event_type, universe_key, actor_id, object_id, summary): (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        ) = storage
            .conn()
            .query_row(
                "SELECT event_type, universe_key, actor_id, object_id, summary \
                 FROM user_notifications WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();

        assert_eq!(event_type, "universe.invitation");
        assert_eq!(universe_key.as_deref(), Some("my-universe"));
        assert_eq!(actor_id.as_deref(), Some("actor_inv"));
        assert_eq!(object_id.as_deref(), Some("inv_tok_abc"));
        assert!(summary.contains("notif.universe.invitation"));
    }

    // --- test_list_my_notifications_pagination_via_since_cursor ---
    #[test]
    fn test_list_my_notifications_pagination_via_since_cursor() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_page";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        // Insert 5 notifications with different timestamps
        for i in 0..5u64 {
            let ts = (chrono::Utc::now() - chrono::Duration::seconds(10 - i as i64)).to_rfc3339();
            let id = format!("notif_pg_{i}");
            let summary = serde_json::json!({"key":"notif.chat.message","params":{}}).to_string();
            storage
                .conn()
                .execute(
                    "INSERT INTO user_notifications \
                     (id, user_id, event_type, actor_id, object_id, summary, created_at) \
                     VALUES (?1, ?2, 'chat.message', 'a', ?3, ?4, ?5)",
                    params![id, user_id, format!("obj_{i}"), summary, ts],
                )
                .unwrap();
        }

        // Get first 3 (newest first)
        let (notifs, has_more) = storage.list_my_notifications(user_id, None, 3);
        assert_eq!(notifs.len(), 3);
        assert!(has_more);

        // Use since cursor from the oldest fetched
        let since_ts = &notifs[2].created_at;
        let (page2, _) = storage.list_my_notifications(user_id, Some(since_ts), 10);
        assert!(
            page2.iter().all(|n| n.created_at > *since_ts),
            "since cursor should only return newer items"
        );
    }

    // --- test_unread_count_correct ---
    #[test]
    fn test_unread_count_correct() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_unread";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        for i in 0..3u64 {
            storage
                .create_notification(
                    user_id,
                    "chat.message",
                    None,
                    None,
                    "a",
                    &format!("obj_{i}"),
                    "notif.chat.message",
                    serde_json::json!({}),
                )
                .unwrap();
        }

        assert_eq!(storage.get_unread_count(user_id), 3);
    }

    // --- test_mark_read_clears_unread ---
    #[test]
    fn test_mark_read_clears_unread() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_mr";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        let id = storage
            .create_notification(
                user_id,
                "chat.dm",
                None,
                None,
                "actor",
                "msg_x",
                "notif.chat.dm",
                serde_json::json!({}),
            )
            .unwrap();

        assert_eq!(storage.get_unread_count(user_id), 1);
        storage.mark_notification_read(&id, user_id).unwrap();
        assert_eq!(storage.get_unread_count(user_id), 0);
    }

    // --- test_mark_all_read_returns_count ---
    #[test]
    fn test_mark_all_read_returns_count() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_mar";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        for i in 0..4u64 {
            storage
                .create_notification(
                    user_id,
                    "chat.message",
                    None,
                    None,
                    "a",
                    &format!("obj_m_{i}"),
                    "notif.chat.message",
                    serde_json::json!({}),
                )
                .unwrap();
        }

        let count = storage.mark_all_read(user_id).unwrap();
        assert_eq!(count, 4);
        assert_eq!(storage.get_unread_count(user_id), 0);
    }

    // --- test_preferences_defaults_inserted_on_first_access ---
    #[test]
    fn test_preferences_defaults_inserted_on_first_access() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_prefs_default";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        let prefs = storage.get_preferences(user_id);
        assert_eq!(
            prefs.email_chat_message, false,
            "default OFF for chat message email"
        );
        assert!(prefs.email_chat_dm, "default ON for DM email");
        assert!(
            prefs.in_app_chat_message,
            "default ON for in-app chat message"
        );
        assert_eq!(prefs.email_digest_freq, "daily");
    }

    // --- test_preferences_patch_partial_update ---
    #[test]
    fn test_preferences_patch_partial_update() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_prefs_patch";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        // Initialize defaults
        storage.get_preferences(user_id);

        let patch = serde_json::json!({
            "email_chat_dm": false,
            "email_digest_freq": "hourly"
        });
        storage.update_preferences(user_id, &patch).unwrap();

        let prefs = storage.get_preferences(user_id);
        assert!(!prefs.email_chat_dm, "should be updated to false");
        assert_eq!(prefs.email_digest_freq, "hourly");
        // Untouched fields remain at defaults
        assert!(
            prefs.in_app_chat_dm,
            "in_app_chat_dm should still be default true"
        );
    }

    // --- test_preferences_invalid_digest_freq_400 ---
    #[tokio::test]
    async fn test_preferences_invalid_digest_freq_400() {
        isolate_env();
        let dir = tempdir().unwrap();
        let user_id = insert_user(dir.path(), "pref400@example.com");
        let token = make_jwt(&user_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/me/notification-preferences")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"email_digest_freq":"hourly_invalid"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- test_list_pending_email_notifications_excludes_already_delivered ---
    #[test]
    fn test_list_pending_email_notifications_excludes_already_delivered() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = "usr_email_pending";
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) VALUES (?1, 'X', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();

        // One pending, one already delivered
        let id_pending = storage
            .create_notification(
                user_id,
                "chat.dm",
                None,
                None,
                "a",
                "msg_pend",
                "notif.chat.dm",
                serde_json::json!({}),
            )
            .unwrap();
        let id_delivered = storage
            .create_notification(
                user_id,
                "chat.dm",
                None,
                None,
                "a",
                "msg_del",
                "notif.chat.dm",
                serde_json::json!({}),
            )
            .unwrap();
        storage
            .mark_notifications_email_delivered(&[id_delivered])
            .unwrap();

        let pending = storage.list_pending_email_notifications(user_id, 100);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id_pending);
    }

    // =========================================================================
    // HTTP-level integration tests
    // =========================================================================

    // --- test_post_chat_message_creates_notif_for_each_other_member ---
    #[tokio::test]
    async fn test_post_chat_message_creates_notif_for_each_other_member() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_notif@example.com");
        let member1_id = insert_user(dir.path(), "member1_notif@example.com");
        let member2_id = insert_user(dir.path(), "member2_notif@example.com");
        insert_universe(dir.path(), "uni_notif", &owner_id);
        add_member(dir.path(), "uni_notif", &member1_id);
        add_member(dir.path(), "uni_notif", &member2_id);

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni_notif/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"hello everyone"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);

        let storage = Storage::new(dir.path().to_str().unwrap());
        let c1 = storage.get_unread_count(&member1_id);
        let c2 = storage.get_unread_count(&member2_id);
        let c_owner = storage.get_unread_count(&owner_id);
        assert_eq!(c1, 1, "member1 should have 1 notification");
        assert_eq!(c2, 1, "member2 should have 1 notification");
        assert_eq!(c_owner, 0, "author should not be notified");
    }

    // --- test_post_chat_message_does_NOT_notify_author ---
    #[tokio::test]
    async fn test_post_chat_message_does_not_notify_author() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_noself@example.com");
        insert_universe(dir.path(), "uni_noself", &owner_id);

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni_noself/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"just me here"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let storage = Storage::new(dir.path().to_str().unwrap());
        assert_eq!(storage.get_unread_count(&owner_id), 0);
    }

    // --- test_post_chat_message_in_dm_notifies_only_other_party ---
    #[tokio::test]
    async fn test_post_chat_message_in_dm_notifies_only_other_party() {
        isolate_env();
        let dir = tempdir().unwrap();
        let user_a = insert_user(dir.path(), "dm_a@example.com");
        let user_b = insert_user(dir.path(), "dm_b@example.com");

        // Both users need to share a universe for DM policy='shared-universe' to allow DMs
        let storage_setup = Storage::new(dir.path().to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage_setup.conn().execute(
            "INSERT OR IGNORE INTO universes (key, name, description, owner_id, created_at) VALUES ('shared_uni', 'S', '', ?1, ?2)",
            params![user_a, now],
        ).unwrap();
        for uid in &[&user_a, &user_b] {
            storage_setup.conn().execute(
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) VALUES ('shared_uni', ?1, 'member', ?2)",
                params![uid, now],
            ).unwrap();
        }

        // Open the DM (creates room)
        let dm_room = storage_setup.open_dm(&user_a, &user_b).unwrap();
        let room_slug = dm_room.slug.clone();

        let token_a = make_jwt(&user_a);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/universes/dm/chat/rooms/{room_slug}/messages"
                    ))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token_a}"))
                    .body(Body::from(r#"{"body":"hey!"}"#))
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

        let storage = Storage::new(dir.path().to_str().unwrap());
        let count_b = storage.get_unread_count(&user_b);
        let count_a = storage.get_unread_count(&user_a);
        assert_eq!(count_b, 1, "user_b should get 1 DM notification");
        assert_eq!(count_a, 0, "sender should not be notified");

        // Verify event type is chat.dm
        let (notifs, _) = storage.list_my_notifications(&user_b, None, 10);
        assert_eq!(notifs[0].event_type, "chat.dm");
    }

    // --- test_create_invitation_emits_notif_for_invitee_with_co_account ---
    #[tokio::test]
    async fn test_create_invitation_emits_notif_for_invitee_with_co_account() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_inv@example.com");
        let guest_id = insert_user(dir.path(), "guest_inv@example.com");
        insert_universe(dir.path(), "uni_inv", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni_inv/invitations")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"email":"guest_inv@example.com"}"#))
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

        let storage = Storage::new(dir.path().to_str().unwrap());
        let count = storage.get_unread_count(&guest_id);
        assert_eq!(
            count, 1,
            "invitee with CO account should have 1 notification"
        );

        let (notifs, _) = storage.list_my_notifications(&guest_id, None, 10);
        assert_eq!(notifs[0].event_type, "universe.invitation");
    }

    // --- test_mention_parser_emits_notif_only_for_room_members ---
    #[tokio::test]
    async fn test_mention_parser_emits_notif_only_for_room_members() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "own_mention@example.com");
        let member_id = insert_user(dir.path(), "mem_mention@example.com");
        let outsider_id = insert_user(dir.path(), "out_mention@example.com");
        insert_universe(dir.path(), "uni_mention", &owner_id);
        add_member(dir.path(), "uni_mention", &member_id);

        // outsider_id is NOT a member

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        // Post a message that mentions both member and outsider
        let body = format!(r#"{{"body":"hello @mem_mention and @out_mention"}}"#);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni_mention/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);

        let storage = Storage::new(dir.path().to_str().unwrap());
        // member gets a chat.message notification AND a chat.mention notification
        let (notifs_m, _) = storage.list_my_notifications(&member_id, None, 10);
        let mention_notifs: Vec<_> = notifs_m
            .iter()
            .filter(|n| n.event_type == "chat.mention")
            .collect();
        assert_eq!(
            mention_notifs.len(),
            1,
            "room member should get mention notification"
        );

        // outsider gets NO notification
        let (notifs_o, _) = storage.list_my_notifications(&outsider_id, None, 10);
        let _ = outsider_id; // suppress unused warning
        assert!(
            notifs_o.is_empty(),
            "non-member should not get any notifications"
        );
    }

    // --- test_update_preferences_request_parses_typed_struct ---
    #[test]
    fn test_update_preferences_request_parses_typed_struct() {
        let json =
            r#"{"email_digest_freq":"hourly","email_chat_dm":false,"quiet_hours_start":"22:00"}"#;
        let req: super::UpdatePreferencesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.email_digest_freq.as_deref(), Some("hourly"));
        assert_eq!(req.email_chat_dm, Some(false));
        assert_eq!(req.quiet_hours_start.as_deref(), Some("22:00"));
        assert!(req.email_chat_message.is_none());
    }

    // --- test_update_preferences_put_returns_200 ---
    #[tokio::test]
    async fn test_update_preferences_put_returns_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let user_id = insert_user(dir.path(), "prefs200@example.com");
        let token = make_jwt(&user_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/me/notification-preferences")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        r#"{"email_digest_freq":"weekly","email_chat_dm":false}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }
}
