//! Integration tests for `POST /api/v1/universes/:slug/proposals/inline`.
//!
//! Scenario 3 from the editing matrix: a logged-in user wants to
//! change content in a public universe they don't own. PUT on the
//! entry returns 403 (owner check); the SPA falls back to this
//! endpoint to land the change in the target universe's
//! `_proposals/` inbox.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
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
        bypass_rate_limit: false,
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
    // The endpoint requires the target universe to exist; seed it.
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
async fn inline_proposal_lands_in_target_proposals_folder() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // template universe is seeded by seed_data; any authed user can
    // propose into it even though they don't own it.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/template/proposals/inline")
        .header(header::AUTHORIZATION, test_bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "target_path": "content/sobre.md",
                "body": "# Proposed rewrite\n\nThis is the proposed body.",
                "note": "tiny tweak",
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = body_to_json(res.into_body()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "POST inline proposal should succeed; body: {body}"
    );

    let proposal_path = body["proposal_path"].as_str().expect("proposal_path");
    assert!(
        proposal_path.starts_with("_proposals/"),
        "proposal must land under _proposals/, got: {proposal_path}"
    );
    assert!(
        proposal_path.ends_with(".md"),
        "proposal path must end .md, got: {proposal_path}"
    );
    assert_eq!(body["target_universe"], "template");
    assert_eq!(body["target_path"], "content/sobre.md");
    assert_eq!(body["author"], "test-user");
    assert_eq!(body["status"], "open");

    // Verify the entry is fetchable from the target universe.
    // Path segments only contain timestamps + author id + nanoid +
    // ".md" — no chars that need URL-encoding — so a plain join works.
    let encoded = proposal_path.to_string();
    let fetch_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/universes/template/entries/{encoded}"))
        .body(Body::empty())
        .unwrap();
    let fetch_res = app.clone().oneshot(fetch_req).await.unwrap();
    assert_eq!(fetch_res.status(), StatusCode::OK);
    let entry = body_to_json(fetch_res.into_body()).await;
    let fm = &entry["frontmatter"];
    assert_eq!(fm["type"], "proposal");
    assert_eq!(fm["kind"], "inline");
    assert_eq!(fm["target_path"], "content/sobre.md");
    assert_eq!(fm["target_universe"], "template");
    assert_eq!(fm["status"], "open");
    assert_eq!(fm["author"], "test-user");
    assert_eq!(fm["note"], "tiny tweak");
}

#[tokio::test]
async fn inline_proposal_requires_auth() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/template/proposals/inline")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"target_path": "content/sobre.md", "body": "x"}).to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn inline_proposal_rejects_traversal_in_target_path() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/template/proposals/inline")
        .header(header::AUTHORIZATION, test_bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"target_path": "../../etc/passwd", "body": "x"}).to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn inline_proposal_server_overrides_smuggled_frontmatter() {
    // The caller can only set body, target_path, and (optional) note.
    // Even if they tried to send extra frontmatter to forge author or
    // status, the endpoint accepts only the three known fields — so
    // this test confirms the request schema rejects unknown ones, or
    // (if it ignores them) the resulting entry still has the right
    // server-controlled author/status.
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/template/proposals/inline")
        .header(header::AUTHORIZATION, test_bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "target_path": "content/sobre.md",
                "body": "x",
                "author": "attacker",            // ignored
                "status": "merged",              // ignored
                "frontmatter": {"type": "page"}, // ignored
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_to_json(res.into_body()).await;
    assert_eq!(body["author"], "test-user", "author is server-set");
    assert_eq!(body["status"], "open", "status is server-set");
}
