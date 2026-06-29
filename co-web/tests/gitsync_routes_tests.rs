//! CO-89: HTTP integration tests for git-backed universes.
//!
//! In-process (`tower::ServiceExt::oneshot`) — no real ports. The git source is
//! a local `file://` repo created in the test's tempdir, so the sync path runs
//! fully offline and deterministically (commit dates pinned via `GIT_*_DATE`).

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use co_web::storage::{Storage, seed_data};

extern crate co;

fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok()
}

fn test_config(dir: &Path) -> WebConfig {
    WebConfig {
        port: 3000,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: true,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: "universo".to_string(),
        gestao_github_admins: vec!["artelonga".to_string()],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
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

/// Seed an owned universe `co-dev` whose git_source points at `remote`.
fn seed_git_universe(dir: &Path, remote: &str) {
    let storage = Storage::new(dir);
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
             VALUES ('test-user', 'test-user@test.local', 'Test', '2026-01-01')",
            [],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_public, visibility, git_source, git_branch) \
             VALUES ('co-dev', 'Co Dev', 'dev', 'test-user', '2026-01-01', 0, 'private', ?1, 'main')",
            rusqlite::params![remote],
        )
        .unwrap();
}

/// Create a local git repo with one CO-tagged commit; return its `file://` URL.
fn init_origin(dir: &Path) -> String {
    let g = |args: &[&str]| {
        Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_DATE", "2026-04-26T23:00:00-03:00")
            .env("GIT_COMMITTER_DATE", "2026-04-26T23:00:00-03:00")
            .env("GIT_AUTHOR_NAME", "Yuri")
            .env("GIT_AUTHOR_EMAIL", "yuri@artelonga.com.br")
            .env("GIT_COMMITTER_NAME", "Yuri")
            .env("GIT_COMMITTER_EMAIL", "yuri@artelonga.com.br")
            .output()
            .expect("git");
    };
    std::fs::create_dir_all(dir).unwrap();
    g(&["init", "--initial-branch=main"]);
    g(&["config", "user.email", "yuri@artelonga.com.br"]);
    g(&["config", "user.name", "Yuri"]);
    std::fs::write(dir.join("a.txt"), "x\ny\n").unwrap();
    g(&["add", "."]);
    g(&["commit", "-m", "feat(co-web): CO-65 visibility"]);
    format!("file://{}", dir.display())
}

fn build_test_router(dir: &Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    seed_data(&mut storage);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage =
        Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
    let state: AppState = AppState::new(AppStateInner {
        core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
        realtime: Arc::new(RealtimeState {
            doc_rooms: co_web::ws::new_room_manager(),
            sync_rooms: co_web::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
            chat_presence: Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: co_web::cache::CacheLayer::new(),
            embeddings: Arc::new(co_web::embedding::EmbeddingService::disabled()),
            embedding_tx: {
                let (tx, _) = co_web::embedding_worker::channel();
                tx
            },
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: Arc::new(co_web::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: co_web::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(co_web::auth::JwtKey::load_or_generate()),
            rate_limiter: Mutex::new(co_web::rate_limit::InProcessRateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: co_web::infra::workers::InProcessExecutor::new_arc(),
        }),
    });
    build_router(state, None)
}

async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn git_sync_ingests_commits_then_views_render() {
    if !git_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let remote = init_origin(&dir.path().join("origin"));
    seed_git_universe(dir.path(), &remote);
    let app = build_test_router(dir.path());

    // 1. Trigger git-sync.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/co-dev/git-sync")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "git-sync should succeed");
    let stats = body_json(res.into_body()).await;
    assert_eq!(stats["commits_ingested"], 1);
    assert_eq!(stats["profiles_written"], 1);

    // 2. Commit timeline renders Mermaid with the commit's day.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co-dev/commit-timeline")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let tl = body_json(res.into_body()).await;
    assert_eq!(tl["count"], 1);
    assert!(tl["mermaid"].as_str().unwrap().starts_with("timeline"));
    assert!(tl["mermaid"].as_str().unwrap().contains("2026-04-26"));

    // 3. Recompute analytics covers the one profile.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/co-dev/recompute-analytics")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let r = body_json(res.into_body()).await;
    assert_eq!(r["profiles_updated"], 1);

    // 4. Gantt endpoint renders (no tasks yet ⇒ just the header).
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co-dev/gantt")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let g = body_json(res.into_body()).await;
    assert!(g["mermaid"].as_str().unwrap().starts_with("gantt"));
}

#[tokio::test]
async fn git_sync_without_source_is_bad_request() {
    let dir = tempdir().unwrap();
    // Seed a universe with NO git_source.
    let storage = Storage::new(dir.path());
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
             VALUES ('test-user', 't@l', 'T', '2026-01-01')",
            [],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_public, visibility) \
             VALUES ('plain', 'Plain', '', 'test-user', '2026-01-01', 0, 'private')",
            [],
        )
        .unwrap();
    drop(storage);
    let app = build_test_router(dir.path());

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/plain/git-sync")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::BAD_REQUEST,
        "a markdown-only universe cannot git-sync"
    );
}
