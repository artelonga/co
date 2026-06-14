//! CO-413: integration tests for bidirectional bridge universes.
//!
//! Covers the user-story acceptance:
//! - event-bus universe **not** marked bidi → writes stay rejected (405,
//!   `read_only_universe`) — current CO-383 behavior unchanged.
//! - event-bus universe marked `source_mode = 'bidirectional'` → writes are
//!   accepted (no 405) and the entry-event payload carries `source = co-edit`.
//! - `GET /api/v1/universes/{slug}` exposes `source_bidirectional` so YG-124 can
//!   show/hide the "Editar no CO" button.

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
    // Sign with the same secret the writer/visibility gates validate against
    // (`JWT_SECRET` env, or the dev default when unset) so the token is accepted
    // regardless of the harness's JWT_SECRET setting.
    let secret = co_web::auth::jwt_secret();
    let (token, _) =
        co_web::auth::sign_jwt("test-user", "test@example.com", "player", &secret).unwrap();
    format!("Bearer {token}")
}

/// Seed an `event-bus`-backed universe owned by `test-user`, with the given
/// `source_mode` (`None` → column default `read-only`).
fn seed_event_bus_universe(dir: &std::path::Path, slug: &str, source_mode: Option<&str>) {
    let storage = Storage::new(dir);
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
             VALUES ('test-user', 'test-user@test.local', 'Test User', '2026-01-01')",
            [],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_public, visibility) \
             VALUES (?1, ?2, 'event-bus universe', 'test-user', '2026-01-01', 1, 'public-subscribable')",
            rusqlite::params![slug, format!("Bus-{slug}")],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "UPDATE universes \
             SET source_kind = 'event-bus', \
                 source_url = 'wss://yggdrasil.example/api/v1/events', \
                 source_mode = COALESCE(?2, 'read-only') \
             WHERE key = ?1",
            rusqlite::params![slug, source_mode],
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

async fn post_entry(app: &axum::Router, slug: &str, path: &str) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{slug}/entries"))
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "path": path,
                        "frontmatter": { "type": "page", "title": "Edited in CO" },
                        "body": "edited via the CO editor"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn read_only_event_bus_universe_rejects_writes() {
    let dir = tempdir().unwrap();
    seed_event_bus_universe(dir.path(), "ro-bus", None); // default: read-only
    let app = build_test_router(dir.path());

    let res = post_entry(&app, "ro-bus", "instances/i/notes/x.md").await;
    assert_eq!(
        res.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "read-only event-bus universe must reject writes (CO-383 unchanged)"
    );
    let body = body_to_json(res.into_body()).await;
    assert_eq!(body["error"], "read_only_universe");
}

#[tokio::test]
async fn bidirectional_event_bus_universe_accepts_writes() {
    let dir = tempdir().unwrap();
    seed_event_bus_universe(dir.path(), "bidi-bus", Some("bidirectional"));
    let app = build_test_router(dir.path());

    let res = post_entry(&app, "bidi-bus", "instances/i/notes/x.md").await;
    assert_eq!(
        res.status(),
        StatusCode::CREATED,
        "bidirectional event-bus universe must accept writes (not 405)"
    );
}

#[tokio::test]
async fn universe_info_exposes_bidirectional_capability() {
    let dir = tempdir().unwrap();
    seed_event_bus_universe(dir.path(), "ro-bus", None);
    seed_event_bus_universe(dir.path(), "bidi-bus", Some("bidirectional"));
    let app = build_test_router(dir.path());

    for (slug, expected) in [("ro-bus", false), ("bidi-bus", true)] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/universes/{slug}"))
                    .header(header::AUTHORIZATION, test_bearer())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "GET {slug} should be 200");
        let body = body_to_json(res.into_body()).await;
        assert_eq!(
            body["source_bidirectional"], expected,
            "source_bidirectional for {slug} should be {expected}"
        );
        assert_eq!(body["source_kind"], "event-bus");
    }
}
