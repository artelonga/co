//! Shared helpers for the vault route tests (CO-215 pattern).

use std::sync::{Arc, Mutex};

use axum::body::Body;
use http_body_util::BodyExt;

use crate::auth::sign_jwt;
use crate::config::WebConfig;
use crate::experiment::ExperimentStore;
use crate::models::CreateUniverse;
use crate::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use crate::storage::Storage;

pub fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 3000,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: true,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: "quilomboaraucaria".to_string(),
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
    }
}

/// Sign a test JWT with the CURRENT JWT_SECRET env var (or fallback).
/// This avoids overriding the global env var and breaking concurrent tests
/// (universe_routes and ws tests also mutate JWT_SECRET).
pub fn test_bearer() -> String {
    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".to_string());
    let (token, _) = sign_jwt("test-user", "test@example.com", "player", &secret).unwrap();
    format!("Bearer {token}")
}

pub fn build_test_router(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = crate::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
    );
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
            jwt_key: std::sync::Arc::new(crate::auth::JwtKey::load_or_generate()),
            rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
        }),
    });
    build_router(state, None)
}

pub fn seed_universe(dir: &std::path::Path, slug: &str) {
    let mut storage = Storage::new(dir.to_str().unwrap());
    let _ = storage.create_universe(
        CreateUniverse {
            key: slug.to_string(),
            name: slug.to_string(),
            description: String::new(),
        },
        "test-user",
    );
}

pub async fn body_str(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}
