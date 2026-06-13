//! CO-54 — idempotency + conflict resolution for concurrent entry editing.
//!
//! These integration tests exercise the web write path end-to-end through the
//! real router (in-process, no ports — `tower::ServiceExt::oneshot`):
//!
//! * field-level merge — two clients editing *different* frontmatter fields
//!   merge instead of clobbering (Scenario 1);
//! * same-field edits resolve last-write-wins with the prior value preserved in
//!   the version history (no data loss);
//! * identical re-writes are idempotent (no version bump — Scenario 3);
//! * `GET …/entries/versions` returns the pre-overwrite snapshot history.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
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

fn seed_owned_universe(dir: &std::path::Path, owner_id: &str, slug: &str) {
    let storage = Storage::new(dir);
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
             VALUES (?1, ?2, ?3, '2026-01-01')",
            rusqlite::params![
                owner_id,
                format!("{owner_id}@test.local"),
                format!("Test {owner_id}")
            ],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_public, visibility) \
             VALUES (?1, ?2, 'owned by test', ?3, '2026-01-01', 1, 'public-subscribable')",
            rusqlite::params![slug, format!("Owned-{slug}"), owner_id],
        )
        .unwrap();
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
            rate_limiter: std::sync::Mutex::new(co_web::rate_limit::InProcessRateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: co_web::infra::workers::InProcessExecutor::new_arc(),
        }),
    });
    build_router(state, None)
}

async fn body_to_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn create_task(app: &axum::Router, slug: &str, path: &str, fm: Value, body: &str) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{slug}/entries"))
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"path": path, "frontmatter": fm, "body": body}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "create should succeed; got {}",
        res.status()
    );
}

async fn put_entry(app: &axum::Router, slug: &str, path: &str, patch: Value) -> Value {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/universes/{slug}/entries/{path}"))
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(patch.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "PUT should succeed; got {}",
        res.status()
    );
    body_to_json(res.into_body()).await
}

async fn get_entry(app: &axum::Router, slug: &str, path: &str) -> Value {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/{slug}/entries/{path}"))
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    body_to_json(res.into_body()).await
}

async fn get_versions(app: &axum::Router, slug: &str, path: &str) -> Value {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/universes/{slug}/entries/versions?path={path}"
                ))
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    body_to_json(res.into_body()).await
}

/// Scenario 1 — two clients editing *different* fields must both survive.
#[tokio::test]
async fn concurrent_field_edits_merge() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "u1");
    let app = build_test_router(dir.path());

    create_task(
        &app,
        "u1",
        "task.md",
        json!({"type": "task", "title": "Original", "description": "orig"}),
        "body",
    )
    .await;

    // User A edits only the description; User B edits only the title.
    put_entry(
        &app,
        "u1",
        "task.md",
        json!({"frontmatter": {"description": "A-edit"}}),
    )
    .await;
    put_entry(
        &app,
        "u1",
        "task.md",
        json!({"frontmatter": {"title": "B-edit"}}),
    )
    .await;

    let entry = get_entry(&app, "u1", "task.md").await;
    let fm = &entry["frontmatter"];
    assert_eq!(fm["title"], "B-edit", "B's title edit survives");
    assert_eq!(fm["description"], "A-edit", "A's description edit survives");
    assert_eq!(fm["type"], "task", "untouched field preserved");
}

/// Scenario 1 (same field) — last-write-wins, prior value kept in history.
#[tokio::test]
async fn same_field_last_write_wins_no_data_loss() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "u1");
    let app = build_test_router(dir.path());

    create_task(
        &app,
        "u1",
        "task.md",
        json!({"type": "task", "title": "Original"}),
        "body",
    )
    .await;

    put_entry(
        &app,
        "u1",
        "task.md",
        json!({"frontmatter": {"title": "First"}}),
    )
    .await;
    put_entry(
        &app,
        "u1",
        "task.md",
        json!({"frontmatter": {"title": "Second"}}),
    )
    .await;

    // Live entry reflects the last write.
    let entry = get_entry(&app, "u1", "task.md").await;
    assert_eq!(entry["frontmatter"]["title"], "Second");

    // No data loss: both prior titles survive in the version history.
    let versions = get_versions(&app, "u1", "task.md").await;
    let arr = versions["versions"].as_array().expect("versions array");
    assert_eq!(
        arr.len(),
        2,
        "two overwrites → two snapshots; got {versions}"
    );
    let titles: Vec<String> = arr
        .iter()
        .map(|v| {
            let fm: Value = serde_json::from_str(v["frontmatter_json"].as_str().unwrap()).unwrap();
            fm["title"].as_str().unwrap().to_string()
        })
        .collect();
    assert!(titles.contains(&"Original".to_string()));
    assert!(titles.contains(&"First".to_string()));
}

/// Scenario 3 — idempotent convergence: re-applying the same content is a no-op.
#[tokio::test]
async fn idempotent_put_no_version_bump() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "u1");
    let app = build_test_router(dir.path());

    create_task(
        &app,
        "u1",
        "task.md",
        json!({"type": "task", "title": "T", "status": "todo"}),
        "body",
    )
    .await;

    // First status change bumps a version (snapshot of the pre-done state).
    put_entry(
        &app,
        "u1",
        "task.md",
        json!({"frontmatter": {"status": "done"}}),
    )
    .await;
    // Setting status:done again when already done is a no-op.
    put_entry(
        &app,
        "u1",
        "task.md",
        json!({"frontmatter": {"status": "done"}}),
    )
    .await;
    put_entry(
        &app,
        "u1",
        "task.md",
        json!({"frontmatter": {"status": "done"}}),
    )
    .await;

    let versions = get_versions(&app, "u1", "task.md").await;
    assert_eq!(
        versions["total"], 1,
        "idempotent re-writes must not grow history; got {versions}"
    );
    // State is still correct.
    let entry = get_entry(&app, "u1", "task.md").await;
    assert_eq!(entry["frontmatter"]["status"], "done");
}

/// Version history is accessible via the API, with actor + hash + version.
#[tokio::test]
async fn versions_api_exposes_audit_fields() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "u1");
    let app = build_test_router(dir.path());

    create_task(
        &app,
        "u1",
        "doc.md",
        json!({"type": "page", "title": "v1"}),
        "b1",
    )
    .await;
    put_entry(&app, "u1", "doc.md", json!({"body": "b2"})).await;
    put_entry(&app, "u1", "doc.md", json!({"body": "b3"})).await;

    let versions = get_versions(&app, "u1", "doc.md").await;
    let arr = versions["versions"].as_array().expect("versions array");
    assert_eq!(arr.len(), 2, "two overwrites → two snapshots");
    // Newest first, version numbers descending and contiguous from 1.
    assert_eq!(arr[0]["version"], 2);
    assert_eq!(arr[1]["version"], 1);
    for v in arr {
        assert_eq!(v["actor"], "test-user", "authed actor recorded");
        assert_eq!(
            v["hash"].as_str().unwrap().len(),
            64,
            "hash is a 64-char SHA-256 hex"
        );
        assert!(!v["timestamp"].as_str().unwrap().is_empty());
    }
}
