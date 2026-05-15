//! Integration tests for the per-universe `entry_events` log (2.7.25).
//!
//! The log is the source-of-truth for time-travel and the future
//! Kafka/Iceberg/Pinot/Flink trajectory documented in
//! `co::public/transaction-log.md`. Each PUT/DELETE on an entry
//! appends one row.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{AppState, AppStateInner, build_router};
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
        rate_limiter: std::sync::Mutex::new(co_web::rate_limit::RateLimiter::new()),
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

async fn body_to_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn put_appends_event_with_body_and_hash() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "log-test");
    let app = build_test_router(dir.path());

    // PUT an entry.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/log-test/vault/note.md")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"body": "first version"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res.status().is_success(), "PUT should succeed");

    // GET history.
    let hist_res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/log-test/entries/history?path=note.md")
// /entries/history uses ?path=, not /entries/{*path}, so it's unaffected by the upsert path.
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hist_res.status(), StatusCode::OK);
    let body = body_to_json(hist_res.into_body()).await;
    let events = body["events"].as_array().expect("events array");
    assert_eq!(events.len(), 1, "one event after one PUT; got {body}");
    let ev = &events[0];
    assert_eq!(ev["op"], "put");
    assert_eq!(ev["path"], "note.md");
    assert!(
        ev["body_hash"].as_str().unwrap().len() == 64,
        "body_hash must be a 64-char hex (SHA-256)"
    );
    assert!(ev["prev_body_hash"].is_null(), "first write has no prev");
    assert!(ev["seq"].as_i64().unwrap() >= 1);
    assert!(ev["ts_micros"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn second_put_records_prev_hash() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "log-test");
    let app = build_test_router(dir.path());

    for v in ["v1", "v2", "v3"] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/universes/log-test/vault/note.md")
                    .header(header::AUTHORIZATION, test_bearer())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({"body": v}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            res.status().is_success(),
            "PUT '{v}' should succeed; got {}",
            res.status()
        );
    }

    let hist_res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/log-test/entries/history?path=note.md")
// /entries/history uses ?path=, not /entries/{*path}, so it's unaffected by the upsert path.
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_to_json(hist_res.into_body()).await;
    let events = body["events"].as_array().unwrap();
    assert_eq!(events.len(), 3, "three PUTs → three events");
    // Newest first.
    let v3 = &events[0];
    let v2 = &events[1];
    let v1 = &events[2];
    assert!(v1["prev_body_hash"].is_null());
    assert_eq!(v2["prev_body_hash"], v1["body_hash"]);
    assert_eq!(v3["prev_body_hash"], v2["body_hash"]);
    // ts_micros + seq strictly increasing.
    assert!(v3["ts_micros"].as_i64().unwrap() >= v2["ts_micros"].as_i64().unwrap());
    assert!(v3["seq"].as_i64().unwrap() > v2["seq"].as_i64().unwrap());
    assert!(v2["seq"].as_i64().unwrap() > v1["seq"].as_i64().unwrap());
}

#[tokio::test]
async fn delete_appends_delete_event_with_prev_hash() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "log-test");
    let app = build_test_router(dir.path());

    // PUT then DELETE.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/log-test/vault/ephemeral.md")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"body": "to be deleted"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let del = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/log-test/vault/ephemeral.md")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(del.status().is_success(), "DELETE should succeed");

    let hist_res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/log-test/entries/history?path=ephemeral.md")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_to_json(hist_res.into_body()).await;
    let events = body["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "PUT + DELETE → 2 events");
    let delete_ev = &events[0];
    let put_ev = &events[1];
    assert_eq!(delete_ev["op"], "delete");
    assert!(delete_ev["body_hash"].is_null(), "delete has no new body_hash");
    assert_eq!(
        delete_ev["prev_body_hash"],
        put_ev["body_hash"],
        "delete's prev_hash must equal the PUT's body_hash"
    );
    assert_eq!(put_ev["op"], "put");
}
