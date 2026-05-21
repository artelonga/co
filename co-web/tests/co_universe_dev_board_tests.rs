// CO-262: entries seeded by CO-261 walker must be visible to anonymous visitors
// via GET /api/v1/universes/co/entries.
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use co_web::storage::Storage;

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

fn build_co_app(dir: &std::path::Path, seed_dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    storage.seed_admin_content_universes();
    storage.seed_co_universe_tasks(seed_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("open test game storage"),
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

fn make_co_task_md(n: u64, status: &str) -> String {
    format!(
        "---\nid: {n}\ntitle: Test CO-{n}\ntype: user-story\nstatus: {status}\n\
         priority: high\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-02-01T00:00:00Z\n\
         labels:\n  - type:feat\n---\n\nBody of CO-{n}.",
    )
}

#[tokio::test]
async fn test_co_entries_visible_to_anon_after_seed() {
    let dir = tempdir().unwrap();
    let seed_dir = tempdir().unwrap();
    std::fs::write(seed_dir.path().join("CO-1.md"), make_co_task_md(1, "todo")).unwrap();
    std::fs::write(seed_dir.path().join("CO-2.md"), make_co_task_md(2, "done")).unwrap();

    let app = build_co_app(dir.path(), seed_dir.path());

    // Anonymous GET — no Authorization header
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co/entries?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let total = json["total"].as_u64().unwrap_or(0);
    assert!(
        total > 0,
        "anonymous visitor should see CO task entries via entries API, got total={total}"
    );
    let entries = json["entries"].as_array().expect("entries array");
    assert!(
        !entries.is_empty(),
        "entries array must be non-empty for anonymous visitor"
    );

    // Single-entry lookup must resolve to 200 for the public/ path
    let single = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co/entries/public/CO-1.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        single.status(),
        StatusCode::OK,
        "GET .../entries/public/CO-1.md must return 200 for anon visitor"
    );
}
