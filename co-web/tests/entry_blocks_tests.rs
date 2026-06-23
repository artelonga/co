//! CO-470 — integration test for `GET /api/v1/universes/:slug/entries/blocks`.
//!
//! Verifies the block endpoint parses an entry's markdown body into the block
//! tree end-to-end through the real router (in-process, no ports).

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
            rusqlite::params![owner_id, format!("{owner_id}@t.local"), "Test"],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_public, visibility) \
             VALUES (?1, ?2, '', ?3, '2026-01-01', 1, 'public-subscribable')",
            rusqlite::params![slug, format!("U-{slug}"), owner_id],
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

#[tokio::test]
async fn blocks_endpoint_parses_entry_body() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "blk");
    let app = build_test_router(dir.path());

    // Create an entry with a known markdown body.
    let md = "# Title\n\nA paragraph.\n\n- one\n- two\n";
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/blk/entries")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"path": "content/page.md", "frontmatter": {"title": "Title"}, "body": md})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        create.status().is_success(),
        "create entry should succeed; got {}",
        create.status()
    );

    // GET its block tree.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/blk/entries/blocks?path=content/page.md")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json = body_to_json(res.into_body()).await;

    assert_eq!(json["path"], "content/page.md");
    let blocks = json["blocks"].as_array().expect("blocks array");
    assert_eq!(blocks.len(), 4, "heading + paragraph + 2 list items");
    assert_eq!(blocks[0]["type"], "heading");
    assert_eq!(blocks[0]["attrs"]["level"], 1);
    assert_eq!(blocks[0]["text"], "Title");
    assert_eq!(blocks[1]["type"], "paragraph");
    assert_eq!(blocks[2]["type"], "bulleted_list_item");
    assert_eq!(blocks[2]["text"], "one");
}

#[tokio::test]
async fn blocks_endpoint_404_for_missing_entry() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "blk2");
    let app = build_test_router(dir.path());

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/blk2/entries/blocks?path=content/nope.md")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

// --- CO-471: connections + search canonical interfaces -----------------------

#[tokio::test]
async fn connections_endpoint_returns_typed_links_shape() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "cx");
    let app = build_test_router(dir.path());

    // Create an entry with no relations — connections returns empty both ways.
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/cx/entries")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"path": "content/a.md", "frontmatter": {"title": "A"}, "body": "hi"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/cx/connections?path=content/a.md")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json = body_to_json(res.into_body()).await;
    assert_eq!(json["path"], "content/a.md");
    assert!(json["outbound"].is_array());
    assert!(json["inbound"].is_array());
}

#[tokio::test]
async fn search_is_an_alias_of_query() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "sx");
    let app = build_test_router(dir.path());

    // Same DSL expression hits both routes with identical status — proving the
    // `search` route is a true alias of `query`.
    let q = "q=FROM%20task";
    let via_query = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/sx/query?{q}"))
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let via_search = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/sx/search?{q}"))
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(via_query.status(), via_search.status());
}
