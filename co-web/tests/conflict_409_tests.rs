//! CO-128: optimistic-concurrency conflict detection on entry updates.
//!
//! `PUT /api/v1/universes/:slug/entries/:path` accepts an optional `base_hash`
//! (the `body_hash` the client last observed). When that token is stale — the
//! stored entry has since diverged — the write is rejected with `409 Conflict`
//! and a `ConflictPayload { local, remote, base }` so the SPA can open the
//! Apple-style conflict-resolution modal (CO-128).
//!
//! Without `base_hash` the endpoint stays last-write-wins (backward compatible).

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

/// Create an entry and return its server-assigned `body_hash`.
async fn create_entry(app: &axum::Router, slug: &str, path: &str, body: &str) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{slug}/entries"))
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "path": path,
                        "frontmatter": { "type": "note", "title": "Conflict subject" },
                        "body": body,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "create entry must succeed; got {}",
        res.status()
    );
    let json = body_to_json(res.into_body()).await;
    json["body_hash"]
        .as_str()
        .expect("created entry has body_hash")
        .to_string()
}

async fn put_update(
    app: &axum::Router,
    slug: &str,
    path: &str,
    payload: Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/universes/{slug}/entries/{path}"))
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// Two clients edit the same entry. The second writer (with a stale `base_hash`)
/// is rejected with 409 and a payload carrying both divergent versions.
#[tokio::test]
async fn stale_base_hash_returns_409_with_both_versions() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "conf");
    let app = build_test_router(dir.path());

    // Both clients open the entry at the same base revision.
    let base = create_entry(&app, "conf", "notes/x.md", "Original body.").await;

    // Client B saves first — entry diverges from `base`.
    let res_b = put_update(
        &app,
        "conf",
        "notes/x.md",
        json!({ "body": "Body edited by B.", "base_hash": base }),
    )
    .await;
    assert_eq!(res_b.status(), StatusCode::OK, "B saves on a fresh base");

    // Client A saves second with the now-stale base — must conflict.
    let res_a = put_update(
        &app,
        "conf",
        "notes/x.md",
        json!({ "body": "Body edited by A.", "base_hash": base }),
    )
    .await;
    assert_eq!(
        res_a.status(),
        StatusCode::CONFLICT,
        "A's write on a stale base must 409"
    );

    let payload = body_to_json(res_a.into_body()).await;
    assert_eq!(payload["error"], "conflict");
    let c = &payload["conflict"];
    assert_eq!(c["path"], "notes/x.md");
    assert_eq!(c["kind"], "both_modified");
    // Both versions are present: local = A's attempt, remote = B's stored copy.
    assert_eq!(c["local"]["body"], "Body edited by A.");
    assert_eq!(c["remote"]["body"], "Body edited by B.");
    assert_eq!(c["base"]["body_hash"], base);
    assert!(c["local"]["body_hash"].is_string());
    assert!(c["remote"]["body_hash"].is_string());
    assert_ne!(c["local"]["body_hash"], c["remote"]["body_hash"]);
}

/// A write whose `base_hash` matches the current revision succeeds.
#[tokio::test]
async fn matching_base_hash_succeeds() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "conf2");
    let app = build_test_router(dir.path());

    let base = create_entry(&app, "conf2", "notes/y.md", "Original.").await;

    let res = put_update(
        &app,
        "conf2",
        "notes/y.md",
        json!({ "body": "Clean update.", "base_hash": base }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let json = body_to_json(res.into_body()).await;
    assert_eq!(json["body"], "Clean update.");
}

/// Without a `base_hash` the endpoint stays last-write-wins (backward compat:
/// draft autosave and legacy clients never trigger a conflict).
#[tokio::test]
async fn missing_base_hash_is_last_write_wins() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "conf3");
    let app = build_test_router(dir.path());

    create_entry(&app, "conf3", "notes/z.md", "Original.").await;

    // Two writes with no base_hash — neither conflicts; the last one wins.
    let r1 = put_update(&app, "conf3", "notes/z.md", json!({ "body": "first" })).await;
    assert_eq!(r1.status(), StatusCode::OK);
    let r2 = put_update(&app, "conf3", "notes/z.md", json!({ "body": "second" })).await;
    assert_eq!(r2.status(), StatusCode::OK);
    let json = body_to_json(r2.into_body()).await;
    assert_eq!(json["body"], "second");
}
