//! Shared helpers for the review route tests (CO-215 pattern).

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value as JsonValue, json};
use tower::ServiceExt;

use crate::config::WebConfig;
use crate::experiment::ExperimentStore;
use crate::models::CreateUniverse;
use crate::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use crate::storage::Storage;

pub const OWNER: &str = "owner-user";
pub const SLUG: &str = "rev-test";

pub fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 0,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: false,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: String::new(),
        gestao_github_admins: vec![],
        universe_key: None,
        // Skip the in-handler rate limiter so the e2e is deterministic.
        co_env: "test".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        bypass_rate_limit: true,
    }
}

/// Mint a long-lived API token for `user_id`. Resolved via the
/// `api_tokens` table — independent of the global `JWT_SECRET` env var,
/// so these tests stay deterministic when run in parallel with other
/// suites that mutate `JWT_SECRET`.
pub fn bearer_for(dir: &std::path::Path, user_id: &str) -> String {
    let storage = Storage::new(dir.to_str().unwrap());
    let token = storage
        .create_api_token(user_id, "review-test")
        .unwrap()
        .token
        .unwrap();
    format!("Bearer {token}")
}

pub fn owner_bearer(dir: &std::path::Path) -> String {
    bearer_for(dir, OWNER)
}

/// Seed a PUBLIC universe owned by `OWNER` so anon callers pass the
/// visibility gate.
pub fn seed_public_universe(dir: &std::path::Path) {
    let mut storage = Storage::new(dir.to_str().unwrap());
    let _ = storage.create_universe(
        CreateUniverse {
            key: SLUG.to_string(),
            name: SLUG.to_string(),
            description: String::new(),
        },
        OWNER,
    );
    storage
        .conn()
        .execute(
            "UPDATE universes SET is_public = 1, visibility = 'public' WHERE key = ?1",
            rusqlite::params![SLUG],
        )
        .unwrap();
}

pub fn build_test_router(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
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
            jwt_key: std::sync::Arc::new(crate::auth::JwtKey::load_or_generate()),
            rate_limiter: std::sync::Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
        }),
    });
    build_router(state, None)
}

pub async fn body_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(JsonValue::Null)
}

/// Anon POST /suggest, identified by IP so the submitter differs from a
/// no-cookie public reader.
pub async fn anon_suggest(app: &axum::Router, ip: &str, title: &str) -> serde_json::Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{SLUG}/suggest"))
                .header("x-forwarded-for", ip)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "title": title, "body": "proposed content", "entry_type": "note" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "suggest should 201");
    body_json(resp.into_body()).await
}

pub async fn list_entry_paths(app: &axum::Router, xff: Option<&str>) -> Vec<String> {
    let mut builder = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/universes/{SLUG}/entries"));
    if let Some(ip) = xff {
        builder = builder.header("x-forwarded-for", ip);
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp.into_body()).await;
    v["entries"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| e["path"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}
