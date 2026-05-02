//! CO-146 — integration tests for the binary asset endpoint.
//!
//! Covers: round-trip upload → GET, sha256 dedupe, ETag/304, oversize rejection,
//! anonymous-on-private 401, anonymous-on-public 200.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{AppState, AppStateInner, build_router};
use co_web::storage::Storage;

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
    }
}

fn build_test_app(dir: &std::path::Path) -> (axum::Router, AppState) {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
    );
    let state: AppState = Arc::new(AppStateInner {
        storage: Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config,
        auth_store: Mutex::new(auth_store),
        mail,
        game_storage,
        plugin_registry: game_core::plugin::PluginRegistry::new(),
        doc_rooms: co_web::ws::new_room_manager(),
        cache: co_web::cache::CacheLayer::new(),
        rate_limiter: std::sync::Mutex::new(co_web::rate_limit::RateLimiter::new()),
        wae: co_web::wae::WaeEmitter::new(None, None),
    });
    let router = build_router(state.clone(), None);
    (router, state)
}

fn user_bearer(user_id: &str) -> String {
    let (token, _) = co_web::auth::sign_jwt(
        user_id,
        &format!("{user_id}@example.com"),
        "player",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

fn make_universe(state: &AppState, key: &str, owner: &str, public: bool) {
    let mut storage = state.storage.lock().unwrap();
    storage
        .create_universe(
            co_web::models::CreateUniverse {
                key: key.into(),
                name: key.into(),
                description: "".into(),
            },
            owner,
        )
        .unwrap();
    if public {
        storage
            .conn()
            .execute(
                "UPDATE universes SET is_public = 1 WHERE key = ?1",
                rusqlite::params![key],
            )
            .unwrap();
    }
}

const SHA_HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

#[tokio::test]
async fn upload_then_get_round_trip() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    // POST raw bytes
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "text/plain")
                .body(Body::from("hello"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["sha256"].as_str().unwrap(), SHA_HELLO);
    assert_eq!(json["size"].as_i64().unwrap(), 5);
    assert_eq!(json["mime"].as_str().unwrap(), "text/plain");

    // GET back the bytes
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u1/assets/{SHA_HELLO}"))
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let etag = resp
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(etag, format!("\"{SHA_HELLO}\""));
    let cc = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("immutable"));
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"hello");
}

#[tokio::test]
async fn upload_dedupes_by_sha256() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    for _ in 0..3 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/u1/assets")
                    .header("authorization", user_bearer("owner-1"))
                    .header("content-type", "text/plain")
                    .body(Body::from("hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Exactly one row in the assets table for this content.
    let conn_arc = {
        let storage = state.storage.lock().unwrap();
        storage.universe_conn("u1")
    };
    let conn = conn_arc.lock().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "sha256 should dedupe writes");
}

#[tokio::test]
async fn if_none_match_returns_304() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    // Upload first
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "text/plain")
                .body(Body::from("hello"))
                .unwrap(),
        )
        .await
        .unwrap();

    // GET with If-None-Match
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u1/assets/{SHA_HELLO}"))
                .header("authorization", user_bearer("owner-1"))
                .header("if-none-match", format!("\"{SHA_HELLO}\""))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn anonymous_blocked_on_private_universe() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    // Upload as owner
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "text/plain")
                .body(Body::from("hello"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Anonymous read fails
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u1/assets/{SHA_HELLO}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn anonymous_allowed_on_public_universe() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u-pub", "owner-1", true);

    // Upload as owner
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u-pub/assets")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "text/plain")
                .body(Body::from("hello"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Anonymous read succeeds
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u-pub/assets/{SHA_HELLO}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn upload_oversize_rejected() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    // 51 MB payload (just over the 50 MB cap).
    let big = vec![0u8; 51 * 1024 * 1024];
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "application/octet-stream")
                .body(Body::from(big))
                .unwrap(),
        )
        .await
        .unwrap();
    // axum's default body size limit may surface as 413 or our handler's 400 — both are correct rejections.
    let s = resp.status();
    assert!(
        s == StatusCode::BAD_REQUEST || s == StatusCode::PAYLOAD_TOO_LARGE,
        "expected oversize rejection, got {s}"
    );
}

#[tokio::test]
async fn delete_removes_blob_when_unreferenced() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "text/plain")
                .body(Body::from("hello"))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/universes/u1/assets/{SHA_HELLO}"))
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET now returns 404
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u1/assets/{SHA_HELLO}"))
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
