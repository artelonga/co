//! Shared helpers for the universe route tests (CO-215 pattern).

use tempfile::tempdir;

use crate::storage::Storage;

pub fn make_storage() -> (Storage, tempfile::TempDir) {
    // SAFETY: single-threaded test environment.
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());
    (storage, dir)
}

// --- CO-30: theme.css endpoint ---

/// Build a minimal in-process router for the universe API (no port binding).
pub fn make_universe_router(
    storage: Storage,
    dir: &std::path::Path,
) -> (axum::Router, tempfile::TempDir) {
    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::server::{
        AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
        build_router,
    };
    use std::sync::{Arc, Mutex};

    let config = WebConfig {
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
    };
    let experiment = ExperimentStore::new(dir);
    let auth_store = crate::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage =
        Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
    let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
    let state: AppState = AppState::new(AppStateInner {
        core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
        realtime: Arc::new(RealtimeState {
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: crate::cache::CacheLayer::new(),
            embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
            rate_limiter: std::sync::Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
        }),
    });
    let router = build_router(state, None);
    let tmp = tempdir().unwrap(); // keep alive
    (router, tmp)
}

pub async fn body_bytes(response: axum::http::Response<axum::body::Body>) -> String {
    use http_body_util::BodyExt;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// --- CO-49: deterministic access check ---

/// Helper: set visibility on a universe.
pub fn set_visibility(storage: &Storage, key: &str, visibility: &str) {
    storage
        .conn()
        .execute(
            "UPDATE universes SET visibility = ?1, is_public = ?2, is_template = ?3 WHERE key = ?4",
            rusqlite::params![
                visibility,
                if visibility == "public-subscribable" || visibility == "public" {
                    1i64
                } else {
                    0i64
                },
                if visibility == "template" { 1i64 } else { 0i64 },
                key
            ],
        )
        .unwrap();
}

/// CO-423: helper — create a universe via the API as the given user.
pub async fn create_universe_via_api(
    router: &axum::Router,
    token: &str,
    key: &str,
    name: &str,
) -> axum::http::StatusCode {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let payload = serde_json::json!({ "key": key, "name": name, "description": "" });
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status()
}
