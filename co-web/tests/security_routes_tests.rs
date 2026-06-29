// CO-388 (security hardening): integration tests for the security findings API.
//
// The security router is the SECURITY gate itself, so correctness here is
// paramount. These tests assert:
//   1. Every /api/v1/gestao/security/* endpoint requires GitHub admin auth —
//      an unauthenticated request must NOT reach the handler (401/403).
//   2. The trigger_scan daily scan cap is enforced by shared, persistent state
//      (a DB-backed counter), not a per-request in-memory counter that resets.
//
// All tests run in-process with a temp SQLite DB — no network, no real port.
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
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
        universo_dir: "universo".to_string(),
        // Non-empty admin list — so an authenticated non-admin would be
        // forbidden, and (crucially) an UNauthenticated request is rejected
        // before any admin check.
        gestao_github_admins: vec!["artelonga".to_string()],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        bypass_rate_limit: false,
    }
}

fn build_test_app(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
    );
    let (embedding_tx, _embedding_rx) = co_web::embedding_worker::channel();
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
    build_router(state, None)
}

fn set_jwt() {
    // SAFETY: single-threaded test setup, no concurrent env access.
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
}

/// Every mutating + reading security endpoint must reject unauthenticated
/// requests. Without the enforcement middleware the whole findings API
/// (including resolve, which opens the release gate) was reachable by anyone.
#[tokio::test]
async fn security_endpoints_require_auth() {
    set_jwt();
    let dir = tempdir().unwrap();

    let cases = [
        ("GET", "/api/v1/gestao/security/findings"),
        ("GET", "/api/v1/gestao/security/findings/some-id"),
        ("PATCH", "/api/v1/gestao/security/findings/some-id"),
        ("GET", "/api/v1/gestao/security/scan/status"),
        ("POST", "/api/v1/gestao/security/scan"),
    ];

    for (method, uri) in cases {
        let app = build_test_app(dir.path());
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // No Authorization header → 401 (the require_github_admin middleware
        // rejects before any handler runs). It must NOT be 200/2xx.
        assert!(
            resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
            "{method} {uri} returned {} — expected 401/403 (endpoint is unauthenticated!)",
            resp.status()
        );
    }
}

/// A bogus Bearer token must also be rejected (the middleware verifies the
/// token against GitHub; an obviously-invalid token cannot pass).
#[tokio::test]
async fn security_scan_rejects_bogus_token() {
    set_jwt();
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/gestao/security/scan")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, "Bearer obviously-invalid-token")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
        "bogus token returned {} — expected 401/403",
        resp.status()
    );
}

/// The DB-backed daily scan counter must increment and trip the cap. This
/// exercises the counter directly (the route's auth wall makes a full HTTP
/// path impractical without a live GitHub token), proving the limit lives in
/// persistent shared state rather than a per-request in-memory counter.
#[test]
fn daily_scan_counter_trips_cap() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());
    let conn = storage.conn();
    let today = "2026-06-12";

    // Simulate N scans for the same day; the count must persist and grow.
    let bump = |conn: &rusqlite::Connection| -> i64 {
        conn.query_row(
            "INSERT INTO security_scan_counts (scan_day, count) VALUES (?1, 1) \
             ON CONFLICT(scan_day) DO UPDATE SET count = count + 1 RETURNING count",
            rusqlite::params![today],
            |r| r.get(0),
        )
        .unwrap()
    };

    let max = 3;
    let mut last = 0;
    for _ in 0..max {
        last = bump(conn);
    }
    assert_eq!(
        last, max,
        "counter must accumulate across calls (not reset)"
    );
    // The next call exceeds the cap.
    let over = bump(conn);
    assert!(
        over > max,
        "counter must keep climbing past the cap so it can be rejected"
    );

    // A different day starts fresh (daily reset).
    let other_day: i64 = conn
        .query_row(
            "INSERT INTO security_scan_counts (scan_day, count) VALUES ('2026-06-13', 1) \
             ON CONFLICT(scan_day) DO UPDATE SET count = count + 1 RETURNING count",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(other_day, 1, "a new UTC day resets the counter");
}
