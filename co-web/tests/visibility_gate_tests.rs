//! CO-161: Integration tests for universe_visibility_gate middleware.
//!
//! Four cases:
//!   1. Anonymous request on a public universe → 200
//!   2. Anonymous request on a private universe → 401
//!   3. Owner JWT on a private universe → 200
//!   4. Non-member authenticated user on a private universe → 403

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::auth::sign_jwt;
use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::models::CreateUniverse;
use co_web::server::{AppState, AppStateInner, build_router};
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
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
    }
}

fn bearer(user_id: &str) -> String {
    let (token, _) = sign_jwt(
        user_id,
        &format!("{user_id}@test.local"),
        "player",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

/// Build a router with two universes pre-seeded:
///   "pub-uni"  — public (`is_public = true`), owned by owner-a
///   "priv-uni" — private (default), owned by owner-a
///
/// Returns the router and the owner-a user_id.
fn build_test_router(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);

    // Create owner user
    let owner = storage.create_user("owner@test.local", "Owner").unwrap();

    // Private universe (default visibility)
    storage
        .create_universe(
            CreateUniverse {
                key: "priv-uni".into(),
                name: "Private".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();

    // Public universe: create then flip to public via direct SQL
    storage
        .create_universe(
            CreateUniverse {
                key: "pub-uni".into(),
                name: "Public".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "UPDATE universes SET is_public = 1, visibility = 'public-subscribable' WHERE key = 'pub-uni'",
            [],
        )
        .unwrap();

    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
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
        rate_limiter: Mutex::new(co_web::rate_limit::RateLimiter::new()),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// CO-161 case 1: anonymous user on a public universe — should get 200.
#[tokio::test]
async fn test_anon_on_public_universe_gets_200() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/pub-uni/entries")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Anonymous reads on a public universe must return 200"
    );
}

/// CO-161 case 2: anonymous user on a private universe — should get 401.
#[tokio::test]
async fn test_anon_on_private_universe_gets_401() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/priv-uni/entries")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "Anonymous reads on a private universe must return 401"
    );
}

/// CO-161 case 3: universe owner on a private universe — should get 200.
#[tokio::test]
async fn test_owner_on_private_universe_gets_200() {
    let dir = tempdir().unwrap();
    let config = test_config(dir.path());
    let mut storage = Storage::new(&config.data_dir);

    let owner = storage.create_user("owner2@test.local", "Owner2").unwrap();
    storage
        .create_universe(
            CreateUniverse {
                key: "priv-owner".into(),
                name: "PrivOwner".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();

    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir.path()).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.path().join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
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
        rate_limiter: Mutex::new(co_web::rate_limit::RateLimiter::new()),
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

    let app = build_router(state, None);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/priv-owner/entries")
                .header("authorization", bearer(&owner.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Universe owner must be able to read their private universe"
    );
}

/// CO-161 case 4: non-member authenticated user on a private universe — should get 403.
#[tokio::test]
async fn test_non_member_on_private_universe_gets_403() {
    let dir = tempdir().unwrap();
    let config = test_config(dir.path());
    let mut storage = Storage::new(&config.data_dir);

    let owner = storage.create_user("owner3@test.local", "Owner3").unwrap();
    let stranger = storage
        .create_user("stranger@test.local", "Stranger")
        .unwrap();

    storage
        .create_universe(
            CreateUniverse {
                key: "priv-stranger".into(),
                name: "PrivStranger".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();

    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir.path()).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.path().join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
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
        rate_limiter: Mutex::new(co_web::rate_limit::RateLimiter::new()),
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

    let app = build_router(state, None);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/priv-stranger/entries")
                .header("authorization", bearer(&stranger.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Non-member authenticated users must get 403 on a private universe"
    );
}
