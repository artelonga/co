//! CO-233: Sync pipeline regression tests.
//!
//! Verifies that a content write (vault PUT or entries POST) is immediately
//! visible on the GET list endpoint, and that the response carries
//! `Cache-Control: no-store` to prevent CDN / browser caching from hiding
//! the change.
//!
//! Root cause (CO-233): `list_entries`, `list_entry_tags`, and `entry_tree`
//! returned no `Cache-Control` header, allowing Cloudflare (CO-117) to cache
//! mutable API responses. Additionally, `entry_cache_control` used
//! `stale-while-revalidate=300`, creating a 5-minute stale window.

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
        core: Arc::new(CoreState {
            storage: parking_lot::Mutex::new(storage),
            config,
            auth_store: Mutex::new(auth_store),
            event_bus: co_web::events::Bus::new(),
        }),
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

async fn body_to_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// CO-233: write-then-read pipeline
// ---------------------------------------------------------------------------

/// Vault PUT followed immediately by GET list must reflect the change within 2s,
/// and the list response must carry Cache-Control: no-store.
#[tokio::test]
async fn vault_write_appears_in_list_immediately() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "sync-test");
    let app = build_test_router(dir.path());

    let start = std::time::Instant::now();

    // Write a new entry via the vault API (mirrors a real editor workflow).
    let put_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/sync-test/vault/tasks/my-task.md")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "body": "---\ntype: task\ntitle: Sync pipeline test\nstatus: todo\n---\nBody text."
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        put_res.status().is_success(),
        "vault PUT must succeed; got {}",
        put_res.status()
    );

    // Immediately list entries — must reflect the write.
    let list_res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/sync-test/entries")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let elapsed = start.elapsed();

    // Write + read must complete within 2 seconds.
    assert!(
        elapsed.as_secs() < 2,
        "write+read took {}ms — must be < 2 000ms",
        elapsed.as_millis()
    );

    // Response must carry Cache-Control: no-store so CDN/browser never caches.
    let cc = list_res
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        cc, "no-store",
        "list_entries must return Cache-Control: no-store; got '{cc}'"
    );

    assert_eq!(list_res.status(), StatusCode::OK);
    let body = body_to_json(list_res.into_body()).await;
    let entries = body["entries"].as_array().expect("entries array");
    let found = entries
        .iter()
        .any(|e| e["path"].as_str() == Some("tasks/my-task.md"));
    assert!(
        found,
        "newly written entry must appear in list immediately; entries = {entries:?}"
    );
}

/// Entry tags endpoint must also carry Cache-Control: no-store.
#[tokio::test]
async fn entry_tags_carries_no_store_header() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "tag-test");
    let app = build_test_router(dir.path());

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/tag-test/entries/tags")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let cc = res
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        cc, "no-store",
        "list_entry_tags must return Cache-Control: no-store; got '{cc}'"
    );
}

/// Entry tree endpoint must also carry Cache-Control: no-store.
#[tokio::test]
async fn entry_tree_carries_no_store_header() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "tree-test");
    let app = build_test_router(dir.path());

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/tree-test/entries/tree")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let cc = res
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        cc, "no-store",
        "entry_tree must return Cache-Control: no-store; got '{cc}'"
    );
}

/// Update an existing entry then re-read — the new content must be
/// visible immediately (in-process query cache is correctly invalidated).
#[tokio::test]
async fn entry_update_visible_immediately() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "update-test");
    let app = build_test_router(dir.path());

    // Create entry — raw markdown body (vault PUT takes the raw file content, not JSON).
    let create_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/update-test/vault/pages/about.md")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::from(
                    "---\ntype: page\ntitle: About v1\n---\nOriginal.",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create_res.status().is_success(), "initial PUT must succeed");

    // Update the same entry with a new title.
    let update_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/update-test/vault/pages/about.md")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::from(
                    "---\ntype: page\ntitle: About v2\n---\nUpdated.",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(update_res.status().is_success(), "update PUT must succeed");

    // Read back the single entry — must show the updated title.
    let get_res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/update-test/entries/pages/about.md")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_res.status(), StatusCode::OK);
    let body = body_to_json(get_res.into_body()).await;
    let title = body["frontmatter"]["title"].as_str().unwrap_or("(missing)");
    assert_eq!(
        title, "About v2",
        "GET must reflect the updated title immediately; got '{title}'"
    );
}
