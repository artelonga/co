//! CO-201: HTTP routes for web push subscriptions and VAPID key.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};

use crate::auth::extractors::AuthedUser;
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Routers
// ---------------------------------------------------------------------------

/// Anonymous route: GET /api/v1/notifications/vapid-public-key
pub fn vapid_router() -> Router<AppState> {
    Router::new().route(
        "/api/v1/notifications/vapid-public-key",
        get(vapid_public_key_handler),
    )
}

/// Auth-required routes nested under /api/v1/me
pub fn me_push_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/push-subscriptions", post(subscribe_handler))
        .route("/push-subscriptions", get(list_subscriptions_handler))
        .route(
            "/push-subscriptions/{id}",
            delete(delete_subscription_handler),
        )
        .layer(middleware::from_fn_with_state(
            state,
            crate::auth::require_auth,
        ))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub user_agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubscribeResponse {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct ListSubscriptionsResponse {
    pub subscriptions: Vec<crate::storage::push_subscriptions::PushSubscriptionInfo>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/notifications/vapid-public-key (anonymous)
async fn vapid_public_key_handler(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let key = state
        .core
        .secrets
        .get("VAPID_PUBLIC_KEY")
        .unwrap_or_default();
    if key.is_empty() {
        return Err(AppError::NotFound("VAPID not configured".into()));
    }
    Ok(axum::Json(serde_json::json!({ "key": key })))
}

/// POST /api/v1/me/push-subscriptions
async fn subscribe_handler(
    State(state): State<AppState>,
    user: AuthedUser,
    axum::Json(body): axum::Json<SubscribeRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.endpoint.is_empty() || body.p256dh.is_empty() || body.auth.is_empty() {
        return Err(AppError::BadRequest(
            "endpoint, p256dh, and auth are required".into(),
        ));
    }

    let ua = body
        .user_agent
        .as_deref()
        .map(|s| &s[..s.len().min(256)])
        .map(|s| s.to_string());

    let storage = state.core.storage.lock();

    let id = storage
        .upsert_push_subscription(
            &user.user_id,
            &body.endpoint,
            &body.p256dh,
            &body.auth,
            ua.as_deref(),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, axum::Json(SubscribeResponse { id })))
}

/// DELETE /api/v1/me/push-subscriptions/:id
async fn delete_subscription_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user: AuthedUser,
) -> Result<impl IntoResponse, AppError> {
    let storage = state.core.storage.lock();

    let deleted = storage
        .delete_push_subscription(&id, &user.user_id)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if !deleted {
        return Err(AppError::NotFound("Subscription not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/me/push-subscriptions
async fn list_subscriptions_handler(
    State(state): State<AppState>,
    user: AuthedUser,
) -> Result<impl IntoResponse, AppError> {
    let storage = state.core.storage.lock();

    let subscriptions = storage.list_push_subscriptions_for_user(&user.user_id);

    Ok(axum::Json(ListSubscriptionsResponse { subscriptions }))
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

    fn test_secrets() -> std::sync::Arc<dyn crate::infra::secrets::SecretsProvider> {
        crate::infra::secrets::StaticSecretsProvider::new([
            ("JWT_SECRET", "test-jwt-secret"),
            (
                "VAPID_PUBLIC_KEY",
                "BLxCtest_vapid_public_key_base64url_encoded",
            ),
        ])
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
        let state: crate::server::AppState =
            crate::server::AppState::new(crate::server::AppStateInner {
                core: Arc::new(CoreState::from_storage_with_secrets(
                    storage,
                    config,
                    auth_store,
                    test_secrets(),
                )),
                realtime: Arc::new(RealtimeState {
                    doc_rooms: crate::ws::new_room_manager(),
                    sync_rooms: crate::sync_ws::new_sync_room_manager(),
                    chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                    chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
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
        let id = format!("usr_push_{}", nanoid::nanoid!(8));
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

    fn make_jwt(user_id: &str) -> String {
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
    // test_vapid_endpoint_returns_configured_key
    // =========================================================================

    #[tokio::test]
    async fn test_vapid_endpoint_returns_configured_key() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/notifications/vapid-public-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(
            json["key"].as_str().unwrap(),
            "BLxCtest_vapid_public_key_base64url_encoded"
        );
    }

    // =========================================================================
    // test_subscribe_upsert_returns_201
    // =========================================================================

    #[tokio::test]
    async fn test_subscribe_upsert_returns_201() {
        let dir = tempdir().unwrap();
        let user_id = insert_user(dir.path(), "sub201@example.com");
        let token = make_jwt(&user_id);
        let app = build_test_router(dir.path());

        let body = serde_json::json!({
            "endpoint": "https://push.example.com/test-endpoint-abc",
            "p256dh": "BLxCdummyp256dh",
            "auth": "dummyauthsecret",
            "user_agent": "TestBrowser/1.0"
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/me/push-subscriptions")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let json = body_json(resp.into_body()).await;
        assert!(
            json["id"].as_str().unwrap().starts_with("pushsub_"),
            "id should start with pushsub_"
        );
    }

    // =========================================================================
    // test_subscribe_idempotent_by_endpoint
    // =========================================================================

    #[tokio::test]
    async fn test_subscribe_idempotent_by_endpoint() {
        let dir = tempdir().unwrap();
        let user_id = insert_user(dir.path(), "idem@example.com");
        let token = make_jwt(&user_id);

        let body = serde_json::json!({
            "endpoint": "https://push.example.com/idempotent-endpoint",
            "p256dh": "BLxCdummyp256dh",
            "auth": "dummyauthsecret"
        });

        // First request
        let app1 = build_test_router(dir.path());
        let resp1 = app1
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/me/push-subscriptions")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::CREATED);
        let json1 = body_json(resp1.into_body()).await;
        let id1 = json1["id"].as_str().unwrap().to_string();

        // Second request — same endpoint, same user → same id
        let app2 = build_test_router(dir.path());
        let resp2 = app2
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/me/push-subscriptions")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::CREATED);
        let json2 = body_json(resp2.into_body()).await;
        let id2 = json2["id"].as_str().unwrap().to_string();

        assert_eq!(id1, id2, "same endpoint should return same subscription id");
    }

    // =========================================================================
    // test_delete_subscription_204
    // =========================================================================

    #[tokio::test]
    async fn test_delete_subscription_204() {
        let dir = tempdir().unwrap();
        let user_id = insert_user(dir.path(), "del204@example.com");
        let token = make_jwt(&user_id);

        // Subscribe first
        let app = build_test_router(dir.path());
        let body = serde_json::json!({
            "endpoint": "https://push.example.com/to-delete",
            "p256dh": "BLxCdummyp256dh",
            "auth": "dummyauthsecret"
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/me/push-subscriptions")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp.into_body()).await;
        let sub_id = json["id"].as_str().unwrap().to_string();

        // Delete
        let app2 = build_test_router(dir.path());
        let resp2 = app2
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/me/push-subscriptions/{sub_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::NO_CONTENT);
    }

    // =========================================================================
    // test_list_subscriptions_for_user
    // =========================================================================

    #[tokio::test]
    async fn test_list_subscriptions_for_user() {
        let dir = tempdir().unwrap();
        let user_id = insert_user(dir.path(), "listpush@example.com");
        let token = make_jwt(&user_id);

        // Add two subscriptions
        for i in 0..2 {
            let app = build_test_router(dir.path());
            let body = serde_json::json!({
                "endpoint": format!("https://push.example.com/endpoint-{i}"),
                "p256dh": "BLxCdummyp256dh",
                "auth": "dummyauthsecret"
            });
            let _ = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/me/push-subscriptions")
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/me/push-subscriptions")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let subs = json["subscriptions"].as_array().unwrap();
        assert_eq!(subs.len(), 2, "should list 2 subscriptions");
    }
}
