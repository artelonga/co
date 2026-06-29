//! CO-326 — Contact form: POST /api/v1/universes/{key}/messages
//!
//! Public endpoint (no auth). Delivers to the universe owner via:
//!   1. Email (MailProvider)
//!   2. In-app notification (user_notifications table, event_type = "direct_message")
//!
//! Rate limit: 5 messages / hour per IP (sliding window, in-process).
//! Honeypot: silently drops requests where `website` field is non-empty.

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn contact_router() -> Router<AppState> {
    Router::new().route("/{universe_key}/messages", post(send_message_handler))
}

// ---------------------------------------------------------------------------
// Per-IP rate limiter: 5 messages / hour
// ---------------------------------------------------------------------------

static MESSAGE_RATE: OnceLock<parking_lot::Mutex<HashMap<String, VecDeque<Instant>>>> =
    OnceLock::new();

fn rate_store() -> &'static parking_lot::Mutex<HashMap<String, VecDeque<Instant>>> {
    MESSAGE_RATE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

fn check_message_rate(ip: &str) -> bool {
    let mut store = rate_store().lock();
    let window = store.entry(ip.to_string()).or_default();
    let cutoff = Instant::now() - Duration::from_secs(3600);
    while window.front().is_some_and(|t| *t < cutoff) {
        window.pop_front();
    }
    if window.len() >= 5 {
        return false;
    }
    window.push_back(Instant::now());
    true
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub from_email: String,
    pub from_name: String,
    pub subject: String,
    pub body: String,
    /// Honeypot — legitimate users leave this empty; bots fill it in.
    pub website: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub ok: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn send_message_handler(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<SendMessageRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Honeypot: non-empty website field → silent 200 so bots don't retry.
    if body.website.as_deref().is_some_and(|s| !s.is_empty()) {
        return Ok((StatusCode::OK, axum::Json(SendMessageResponse { ok: true })));
    }

    let from_email = body.from_email.trim();
    let from_name = body.from_name.trim();
    let subject = body.subject.trim();
    let msg_body = body.body.trim();

    if from_email.is_empty() || from_name.is_empty() || subject.is_empty() || msg_body.is_empty() {
        return Err(AppError::BadRequest("All fields are required".into()));
    }

    let ip = client_ip(&headers);
    if !check_message_rate(&ip) {
        return Err(AppError::TooManyRequests(
            "Rate limit: 5 messages per hour per IP".into(),
        ));
    }

    let (owner_id, owner_email) = {
        let storage = state.core.storage.lock();
        let universe = storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        let owner = storage
            .get_user_by_id(&universe.owner_id)
            .ok_or_else(|| AppError::Internal("Universe owner not found".into()))?;
        (universe.owner_id.clone(), owner.email.clone())
    };

    let email_subject = format!("[CO] {subject}");
    let email_body = format!("From: {from_name} <{from_email}>\nSubject: {subject}\n\n{msg_body}");
    if let Err(e) = state
        .integrations
        .mail
        .send(&owner_email, &email_subject, &email_body)
    {
        tracing::warn!(universe_key = %universe_key, "contact: mail send failed: {e}");
    }

    let object_id = nanoid::nanoid!(16);
    {
        let storage = state.core.storage.lock();
        let _ = storage.create_notification(
            &owner_id,
            "direct_message",
            Some(&universe_key),
            None,
            &format!("external:{from_email}"),
            &object_id,
            "contact.direct_message",
            serde_json::json!({
                "from_email": from_email,
                "from_name": from_name,
                "subject": subject,
            }),
        );
    }

    Ok((StatusCode::OK, axum::Json(SendMessageResponse { ok: true })))
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
    use crate::server::{
        AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
    };
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
        let state = AppState::new(AppStateInner {
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
                geo: Arc::new(crate::geo::GeoDb::disabled()),
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
        let id = format!("usr_{}", nanoid::nanoid!(8));
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
            .expect("ensure default room");
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    // -----------------------------------------------------------------------
    // 1. test_send_message_200 — happy path delivers email + notification
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_send_message_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_c1@example.com");
        insert_universe(dir.path(), "space_c1", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/space_c1/messages")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "198.51.100.10")
                    .header("user-agent", "CO-test/1.0")
                    .body(Body::from(
                        r#"{"from_email":"visitor@example.com","from_name":"Test Visitor","subject":"Hello","body":"This is a test."}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["ok"], true);
    }

    // -----------------------------------------------------------------------
    // 2. test_honeypot_silently_drops — website field non-empty → silent 200
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_honeypot_silently_drops() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_c2@example.com");
        insert_universe(dir.path(), "space_c2", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/space_c2/messages")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "198.51.100.20")
                    .header("user-agent", "CO-test/1.0")
                    .body(Body::from(
                        r#"{"from_email":"bot@spam.com","from_name":"Bot","subject":"Buy now","body":"spam","website":"http://spam.example.com"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["ok"], true);
    }

    // -----------------------------------------------------------------------
    // 3. test_rate_limit_kicks_in_at_6th — 6th request from same IP → 429
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_rate_limit_kicks_in_at_6th() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_c3@example.com");
        insert_universe(dir.path(), "space_c3", &owner_id);

        let app = build_test_router(dir.path());
        // RFC 5737 TEST-NET-3: unique per this test to avoid cross-test interference
        let test_ip = "203.0.113.99";

        for i in 0..5 {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/universes/space_c3/messages")
                        .header("content-type", "application/json")
                        .header("x-forwarded-for", test_ip)
                        .header("user-agent", "CO-test/1.0")
                        .body(Body::from(format!(
                            r#"{{"from_email":"v{i}@example.com","from_name":"V","subject":"Hi {i}","body":"Msg {i}"}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "request {i} should succeed");
        }

        let resp6 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/space_c3/messages")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", test_ip)
                    .header("user-agent", "CO-test/1.0")
                    .body(Body::from(
                        r#"{"from_email":"v@example.com","from_name":"V","subject":"6th","body":"Should be blocked"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp6.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    // -----------------------------------------------------------------------
    // 4. test_missing_fields_400 — empty required field
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_missing_fields_400() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_c4@example.com");
        insert_universe(dir.path(), "space_c4", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/space_c4/messages")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "198.51.100.40")
                    .header("user-agent", "CO-test/1.0")
                    .body(Body::from(
                        r#"{"from_email":"","from_name":"Visitor","subject":"Hello","body":"Hi"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // 5. test_unknown_universe_404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_unknown_universe_404() {
        isolate_env();
        let dir = tempdir().unwrap();

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/does-not-exist/messages")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "198.51.100.50")
                    .header("user-agent", "CO-test/1.0")
                    .body(Body::from(
                        r#"{"from_email":"v@example.com","from_name":"V","subject":"Hi","body":"Hello"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // 6. test_notification_created — in-app notification row inserted
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_notification_created() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_c6@example.com");
        insert_universe(dir.path(), "space_c6", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/space_c6/messages")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "198.51.100.60")
                    .header("user-agent", "CO-test/1.0")
                    .body(Body::from(
                        r#"{"from_email":"sender@example.com","from_name":"Sender","subject":"Test notif","body":"Hello owner"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let storage = Storage::new(dir.path().to_str().unwrap());
        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM user_notifications WHERE user_id = ?1 AND event_type = 'direct_message'",
                params![owner_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "one direct_message notification should be created"
        );
    }
}
