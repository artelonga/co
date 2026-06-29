//! CO-78: HTTP-level coverage for the job-queue `/metrics` endpoint and the
//! internal submit API surfacing in the queue immediately.
//!
//! In-process via `tower::ServiceExt::oneshot` — no network ports.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::job_queue::{self, JobKind};
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use co_web::storage::Storage;
extern crate co;

fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 0,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: false,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: "".to_string(),
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        bypass_rate_limit: false,
    }
}

fn build_test_app(dir: &std::path::Path) -> (axum::Router, AppState) {
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_storage = Arc::new(
        game_core::storage::Storage::open(&dir.join("game_test.db")).expect("game storage"),
    );
    let (embedding_tx, _rx) = co_web::embedding_worker::channel();
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
            embedding_tx,
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
    let router = build_router(state.clone(), None);
    (router, state)
}

async fn body_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn metrics_endpoint_reports_queue_depth() {
    let dir = tempdir().unwrap();
    let (app, state) = build_test_app(dir.path());

    // Empty queue first.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    assert_eq!(json["queue_depth"], 0);
    assert_eq!(json["dead_letter"], 0);
    assert!(json["desired_workers"].as_u64().unwrap() >= 4);
    assert!(json["by_kind"].is_array());

    // Submit two jobs via the internal API — they surface immediately.
    {
        let s = state.core.storage.lock();
        job_queue::enqueue_job(s.conn(), "u1", JobKind::ChangelogRegen, "{}", Some("a")).unwrap();
        job_queue::enqueue_job(s.conn(), "u1", JobKind::SyncPull, "{}", Some("b")).unwrap();
    }

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    assert_eq!(json["queue_depth"], 2, "both submitted jobs are queued");

    // The per-kind breakdown carries the changelog_regen pending count.
    let by_kind = json["by_kind"].as_array().unwrap();
    let cr = by_kind
        .iter()
        .find(|k| k["kind"] == "changelog_regen")
        .unwrap();
    assert_eq!(cr["pending"], 1);
}
