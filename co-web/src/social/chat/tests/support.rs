//! Shared REST test helpers.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use rusqlite::params;

use crate::config::WebConfig;
use crate::experiment::ExperimentStore;
use crate::server::{CoreState, IndexState, IntegrationsState, RealtimeState};
use crate::storage::Storage;

pub fn isolate_env() {
    unsafe {
        std::env::set_var("JWT_SECRET", "test-jwt-secret");
    }
}

pub fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 3000,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: false,
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

pub fn build_test_router(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = crate::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage =
        Arc::new(game_core::storage::Storage::open(&game_db_path).expect("open test game storage"));
    let (embedding_tx, _rx) = crate::embedding_worker::channel();
    let state: crate::server::AppState =
        crate::server::AppState::new(crate::server::AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
    crate::server::build_router(state, None)
}

pub fn make_state_inner(dir: &std::path::Path) -> crate::server::AppState {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = crate::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage =
        Arc::new(game_core::storage::Storage::open(&game_db_path).expect("open test game storage"));
    let (embedding_tx, _rx) = crate::embedding_worker::channel();
    crate::server::AppState::new(crate::server::AppStateInner {
        core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
        realtime: Arc::new(RealtimeState {
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: crate::cache::CacheLayer::new(),
            embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
            rate_limiter: Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
        }),
    })
}

pub fn insert_user(dir: &std::path::Path, email: &str) -> String {
    let storage = Storage::new(dir.to_str().unwrap());
    let id = format!("usr_test_{}", &nanoid::nanoid!(8));
    let usuario = email.split('@').next().unwrap_or("user").to_lowercase();
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES (?1, ?2, ?3, 'player', ?4, ?5)",
            params![id, email, email, now, usuario],
        )
        .expect("insert test user");
    id
}

pub fn insert_universe(dir: &std::path::Path, key: &str, owner_id: &str) {
    let storage = Storage::new(dir.to_str().unwrap());
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, visibility) \
             VALUES (?1, ?2, '', ?3, ?4, 'private')",
            params![key, key, owner_id, now],
        )
        .expect("insert test universe");
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', ?3)",
            params![key, owner_id, now],
        )
        .expect("insert owner member");
    // seed the default general room
    storage
        .ensure_default_room(key)
        .expect("ensure_default_room");
}

pub fn add_member(dir: &std::path::Path, universe_key: &str, user_id: &str, role: &str) {
    let storage = Storage::new(dir.to_str().unwrap());
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![universe_key, user_id, role, now],
        )
        .expect("insert member");
}

pub fn add_subscriber(dir: &std::path::Path, universe_key: &str, user_id: &str) {
    let storage = Storage::new(dir.to_str().unwrap());
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO subscriptions (user_id, universe_key, subscribed_at) \
             VALUES (?1, ?2, ?3)",
            params![user_id, universe_key, now],
        )
        .expect("insert subscriber");
}

pub fn make_jwt(user_id: &str) -> String {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (token, _) =
        crate::auth::sign_jwt(user_id, "test@example.com", "player", "test-jwt-secret").unwrap();
    token
}

pub async fn body_json(body: Body) -> serde_json::Value {
    use http_body_util::BodyExt;
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}
