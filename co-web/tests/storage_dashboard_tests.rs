//! Integration tests for `GET /api/v1/admin/storage` (2.7.27).
//!
//! The endpoint summarises per-universe disk + table footprints so
//! the owner can spot storage hotspots before they bite. Admin-only.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{AppState, AppStateInner, build_router};
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

fn admin_bearer() -> String {
    // CO admin email allowlist key is yuri@artelonga.com.br; sign a
    // JWT for that subject.
    let (token, _) = co_web::auth::sign_jwt(
        "admin-user",
        "yuri@artelonga.com.br",
        "admin",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

fn non_admin_bearer() -> String {
    let (token, _) = co_web::auth::sign_jwt(
        "regular-user",
        "regular@example.com",
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
    // Seed template so there's at least one universe row.
    storage.seed_template_universe();
    let experiment = ExperimentStore::new(&config.data_dir);

    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("game storage"),
    );
    let state: AppState = Arc::new(AppStateInner {
        storage: parking_lot::Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config,
        auth_store: Mutex::new(auth_store),
        mail,
        game_storage,
        plugin_registry: game_core::plugin::PluginRegistry::new(),
        doc_rooms: co_web::ws::new_room_manager(),
        sync_rooms: co_web::sync_ws::new_sync_room_manager(),
        cache: co_web::cache::CacheLayer::new(),
        rate_limiter: std::sync::Mutex::new(co_web::rate_limit::RateLimiter::new()),
        wae: co_web::wae::WaeEmitter::new(None, None),
        jwt_key: Arc::new(co_web::auth::JwtKey::load_or_generate()),
        embeddings: std::sync::Arc::new(co_web::embedding::EmbeddingService::disabled()),
        embedding_tx: {
            let (tx, _) = co_web::embedding_worker::channel();
            tx
        },
        chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
        chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        geo: std::sync::Arc::new(co_web::geo::GeoDb::disabled()),
    });
    build_router(state, None)
}

async fn body_to_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn storage_dashboard_requires_admin() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // No auth -> 401.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/storage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Non-admin auth -> 403.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/storage")
                .header(header::AUTHORIZATION, non_admin_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn storage_dashboard_returns_shape() {
    unsafe {
        std::env::set_var("JWT_SECRET", "dev-secret-change-me");
        std::env::set_var("CO_SEED_ADMIN_EMAIL", "yuri@artelonga.com.br");
    }
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/storage")
                .header(header::AUTHORIZATION, admin_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = body_to_json(res.into_body()).await;
    assert_eq!(status, StatusCode::OK, "expected 200; got {status}: {body}");

    assert!(body["generated_at"].is_string());
    assert!(body["host"]["data_dir"].is_string());
    assert!(body["totals"]["universes"].is_number());
    let universes = body["universes"].as_array().expect("universes array");
    assert!(!universes.is_empty(), "should have at least template");

    let t = universes
        .iter()
        .find(|u| u["key"] == "template")
        .expect("template universe in dashboard");
    assert!(t["data_db_bytes"].as_u64().is_some());
    assert!(t["md_bytes"].as_u64().is_some());
    assert!(t["tables"]["entries"]["rows"].is_number());
    assert!(t["tables"]["entry_events"]["rows"].is_number());
}
