//! Integration tests for `DELETE /api/v1/universes/:slug`.
//!
//! The route is documented in `co-web/src/universe_routes.rs:441`:
//!   - any authenticated user may delete any universe they can see
//!   - refuses to delete `template` (the seed)
//!   - cascades: entries, members, subscriptions, on-disk dir

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use co_web::storage::{Storage, seed_data};

extern crate co;

fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 3000,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: true,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: "quilomboaraucaria".to_string(),
        gestao_github_admins: vec!["artelonga".to_string()],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: true,
    }
}

fn test_bearer() -> String {
    let (token, _) = co_web::auth::sign_jwt(
        "test-user",
        "test@example.com",
        "player",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

fn build_test_router(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    seed_data(&mut storage);
    let experiment = ExperimentStore::new(&config.data_dir);

    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("game storage"),
    );
    let state: AppState = AppState::new(AppStateInner {
        core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
        realtime: Arc::new(RealtimeState {
            doc_rooms: co_web::ws::new_room_manager(),
            sync_rooms: co_web::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: co_web::cache::CacheLayer::new(),
            embeddings: std::sync::Arc::new(co_web::embedding::EmbeddingService::disabled()),
            embedding_tx: {
                let (tx, _) = co_web::embedding_worker::channel();
                tx
            },
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: std::sync::Arc::new(co_web::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: co_web::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(co_web::auth::JwtKey::load_or_generate()),
            rate_limiter: std::sync::Mutex::new(co_web::rate_limit::RateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: co_web::worker_supervisor::WorkerSupervisor::new(),
        }),
    });

    build_router(state, None)
}

async fn body_to_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Create a sacrificial universe via the universes API for later deletion.
async fn create_test_universe(app: &axum::Router, key: &str) {
    let body = serde_json::json!({
        "key": key,
        "name": format!("Test {key}"),
        "description": "ephemeral universe for delete-route coverage",
        "is_public": true,
        "visibility": "public",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes")
        .header(header::AUTHORIZATION, test_bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    assert!(
        status.is_success() || status == StatusCode::CONFLICT,
        "creating test universe should succeed; got {status}"
    );
}

#[tokio::test]
async fn delete_universe_succeeds_when_authenticated() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    create_test_universe(&app, "deleteme").await;

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/deleteme")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "DELETE should succeed for authed caller; got {} body={}",
        res.status(),
        body_to_string(res.into_body()).await,
    );

    // Subsequent GET should return 404.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/deleteme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_universe_refuses_template() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let res = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/template")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::BAD_REQUEST,
        "template is protected from deletion"
    );
}

#[tokio::test]
async fn delete_universe_requires_auth() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    create_test_universe(&app, "needs-auth").await;

    let res = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/needs-auth")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_universe_404_when_absent() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let res = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/does-not-exist")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
