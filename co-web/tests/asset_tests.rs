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
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
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
        event_bus: co_web::events::Bus::new(),
        worker_supervisor: co_web::worker_supervisor::WorkerSupervisor::new(),
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
    let mut storage = state.storage.lock();
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
        let storage = state.storage.lock();
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
async fn if_none_match_on_nonexistent_returns_404_not_304() {
    // CO-145 regression: a probe with If-None-Match echoing the URL sha
    // must return 404 when the row doesn't exist — not 304. Earlier
    // ordering short-circuited to 304 before the existence check, breaking
    // client-side idempotency probes (a missing blob looked "already there").
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    let fake_sha = "0000000000000000000000000000000000000000000000000000000000000000";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u1/assets/{fake_sha}"))
                .header("authorization", user_bearer("owner-1"))
                .header("if-none-match", format!("\"{fake_sha}\""))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
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
async fn blob_on_disk_is_ciphertext() {
    // CO-148: bytes on disk must NOT contain the plaintext.
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    unsafe { std::env::set_var("CO_ASSETS_MASTER_KEY", "test-master-key-1") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    let plaintext = b"super-secret-marker-XYZ";
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "application/octet-stream")
                .body(Body::from(plaintext.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();

    let universe_dir = {
        let storage = state.storage.lock();
        storage.universe_pool.universe_dir("u1")
    };
    let blob_root = universe_dir.join("blobs");
    let mut found = false;
    for aa in std::fs::read_dir(&blob_root).unwrap() {
        let aa = aa.unwrap().path();
        for bb in std::fs::read_dir(&aa).unwrap() {
            let bb = bb.unwrap().path();
            for f in std::fs::read_dir(&bb).unwrap() {
                let p = f.unwrap().path();
                let bytes = std::fs::read(&p).unwrap();
                assert!(
                    !bytes.windows(plaintext.len()).any(|w| w == plaintext),
                    "ciphertext on disk leaked plaintext marker"
                );
                found = true;
            }
        }
    }
    assert!(found, "expected at least one blob on disk");
}

#[tokio::test]
async fn http_range_returns_206_with_slice() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    unsafe { std::env::set_var("CO_ASSETS_MASTER_KEY", "test-master-key-2") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    let payload = vec![b'A'; 100];
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "application/octet-stream")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let sha = json["sha256"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u1/assets/{sha}"))
                .header("authorization", user_bearer("owner-1"))
                .header("range", "bytes=10-19")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let cr = resp
        .headers()
        .get(header::CONTENT_RANGE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cr, "bytes 10-19/100");
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.len(), 10);
    assert!(body.iter().all(|b| *b == b'A'));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u1/assets/{sha}"))
                .header("authorization", user_bearer("owner-1"))
                .header("range", "bytes=-5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.len(), 5);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/u1/assets/{sha}"))
                .header("authorization", user_bearer("owner-1"))
                .header("range", "bytes=200-300")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
}

#[tokio::test]
async fn tag_crud_round_trip() {
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
                .method("POST")
                .uri(format!("/api/v1/universes/u1/assets/{SHA_HELLO}/tags"))
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"tags":["foo","bar"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let tags = json["assets"][0]["tags"].as_array().unwrap();
    let tag_strs: Vec<&str> = tags.iter().filter_map(|t| t.as_str()).collect();
    assert!(tag_strs.contains(&"foo"));
    assert!(tag_strs.contains(&"bar"));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/u1/assets?tag=foo")
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"].as_i64().unwrap(), 1);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/u1/assets/tags")
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 2);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/universes/u1/assets/{SHA_HELLO}/tags/foo"))
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
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

// ---------------------------------------------------------------------------
// CO-150: list assets endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_assets_returns_uploaded_assets() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    // Upload two assets with different MIME types
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets?filename=hello.txt")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "text/plain")
                .body(Body::from("hello"))
                .unwrap(),
        )
        .await
        .unwrap();

    // sha256 of b"\x89PNG\r\n\x1A\n" prefix (minimal PNG-like bytes for testing)
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets?filename=img.png")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "image/png")
                .body(Body::from(b"\x89PNG\r\n\x1A\nfakepng".to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();

    // List all
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/u1/assets")
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["total"].as_u64().unwrap(), 2);
    assert_eq!(json["assets"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn list_assets_mime_filter() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u1", "owner-1", false);

    // Upload text + image
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets?filename=doc.txt")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "text/plain")
                .body(Body::from("text content"))
                .unwrap(),
        )
        .await
        .unwrap();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/u1/assets?filename=photo.png")
                .header("authorization", user_bearer("owner-1"))
                .header("content-type", "image/png")
                .body(Body::from(b"\x89PNG\r\n\x1A\ndata".to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Filter by image/
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/u1/assets?mime=image/")
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["total"].as_u64().unwrap(), 1);
    let mime = json["assets"][0]["mime"].as_str().unwrap();
    assert!(
        mime.starts_with("image/"),
        "expected image mime, got {mime}"
    );
}

#[tokio::test]
async fn list_assets_empty_universe() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());
    make_universe(&state, "u-empty", "owner-1", false);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/u-empty/assets")
                .header("authorization", user_bearer("owner-1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["total"].as_u64().unwrap(), 0);
    assert_eq!(json["assets"].as_array().unwrap().len(), 0);
}
