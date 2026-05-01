// CO-46: Telemetry system integration tests
//
// Tests simulate a user flow and verify events are correctly recorded.
// All tests use in-process app state with temp SQLite databases — no network.
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{AppState, AppStateInner, build_router};
use co_web::storage::Storage;
use co_web::telemetry::{
    EventRow, cleanup_old_events, hash_ip_daily, insert_event, telemetry_summary,
};

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
    let state: AppState = Arc::new(AppStateInner {
        storage: Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config,
        auth_store: Mutex::new(auth_store),
        mail,
        game_storage,
        plugin_registry: game_core::plugin::PluginRegistry::new(),
        doc_rooms: co_web::ws::new_room_manager(),
        rate_limiter: std::sync::Mutex::new(co_web::rate_limit::RateLimiter::new()),
    });
    build_router(state, None)
}

// --- Storage-level tests ---

/// simulate user flow: insert events, verify summary reflects them
#[test]
fn test_simulate_user_flow_events_recorded() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());
    let conn = storage.conn();

    // Step 1: anonymous user views landing page
    insert_event(
        conn,
        &EventRow {
            timestamp: chrono::Utc::now().to_rfc3339(),
            visitor_token: Some("anon-visitor-1".to_string()),
            user_id: None,
            session_id: Some("sess-abc".to_string()),
            event_type: "pageview".to_string(),
            event_name: "page.view".to_string(),
            universe_key: None,
            path: Some("/co".to_string()),
            properties: Some(serde_json::json!({"referrer": "https://google.com"})),
            duration_ms: Some(85),
            ip_hash: Some(hash_ip_daily("10.0.0.1")),
            ua_device: Some("desktop".to_string()),
            ua_browser: Some("chrome".to_string()),
            ua_os: Some("mac".to_string()),
        },
    );

    // Step 2: user creates a task (interaction)
    insert_event(
        conn,
        &EventRow {
            timestamp: chrono::Utc::now().to_rfc3339(),
            visitor_token: Some("anon-visitor-1".to_string()),
            user_id: None,
            session_id: Some("sess-abc".to_string()),
            event_type: "interaction".to_string(),
            event_name: "task.create".to_string(),
            universe_key: Some("template".to_string()),
            path: Some("/co/template".to_string()),
            properties: None,
            duration_ms: None,
            ip_hash: Some(hash_ip_daily("10.0.0.1")),
            ua_device: Some("desktop".to_string()),
            ua_browser: Some("chrome".to_string()),
            ua_os: Some("mac".to_string()),
        },
    );

    // Step 3: second visitor
    insert_event(
        conn,
        &EventRow {
            timestamp: chrono::Utc::now().to_rfc3339(),
            visitor_token: Some("anon-visitor-2".to_string()),
            user_id: None,
            session_id: None,
            event_type: "pageview".to_string(),
            event_name: "page.view".to_string(),
            universe_key: None,
            path: Some("/co".to_string()),
            properties: None,
            duration_ms: Some(120),
            ip_hash: Some(hash_ip_daily("10.0.0.2")),
            ua_device: Some("mobile".to_string()),
            ua_browser: Some("safari".to_string()),
            ua_os: Some("ios".to_string()),
        },
    );

    // Verify summary
    let summary = telemetry_summary(conn);
    assert_eq!(summary.total_events, 3, "all 3 events recorded");
    assert_eq!(summary.unique_visitors, 2, "2 distinct visitors");
    assert_eq!(summary.error_count, 0, "no errors");
    assert!(
        summary.p95_latency_ms.is_some(),
        "p95 latency computed from pageviews"
    );
    assert!(!summary.top_pages.is_empty(), "top pages populated");
    assert_eq!(summary.top_pages[0].path, "/co");
    assert_eq!(summary.top_pages[0].count, 2);
}

#[test]
fn test_error_events_counted() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());
    let conn = storage.conn();

    for i in 0..3 {
        insert_event(
            conn,
            &EventRow {
                timestamp: chrono::Utc::now().to_rfc3339(),
                visitor_token: Some(format!("v{i}")),
                user_id: None,
                session_id: None,
                event_type: "error".to_string(),
                event_name: "js.error".to_string(),
                universe_key: None,
                path: Some("/co".to_string()),
                properties: Some(serde_json::json!({"message": "TypeError"})),
                duration_ms: None,
                ip_hash: None,
                ua_device: None,
                ua_browser: None,
                ua_os: None,
            },
        );
    }

    let summary = telemetry_summary(conn);
    assert_eq!(summary.error_count, 3);
}

#[test]
fn test_retention_cleanup_removes_old_events() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());
    let conn = storage.conn();

    // Insert: 1 recent + 1 old (91 days ago)
    insert_event(
        conn,
        &EventRow {
            timestamp: chrono::Utc::now().to_rfc3339(),
            visitor_token: Some("recent".to_string()),
            user_id: None,
            session_id: None,
            event_type: "pageview".to_string(),
            event_name: "page.view".to_string(),
            universe_key: None,
            path: Some("/co".to_string()),
            properties: None,
            duration_ms: Some(10),
            ip_hash: None,
            ua_device: None,
            ua_browser: None,
            ua_os: None,
        },
    );

    // Insert old event directly (91 days ago)
    conn.execute(
        "INSERT INTO telemetry_events (timestamp, event_type, event_name) \
         VALUES (datetime('now','-91 days'), 'pageview', 'page.view')",
        [],
    )
    .unwrap();

    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM telemetry_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, 2);

    cleanup_old_events(conn);

    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM telemetry_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, 1, "old event purged, recent event kept");
}

#[test]
fn test_ip_hash_no_pii() {
    let raw_ip = "203.0.113.42";
    let hash = hash_ip_daily(raw_ip);

    // 16 hex chars
    assert_eq!(hash.len(), 16);
    // Raw IP not in hash
    assert!(!hash.contains("203.0.113.42"));
    assert!(!hash.contains("203"));
    // Deterministic
    assert_eq!(hash, hash_ip_daily(raw_ip));
    // Different IPs differ
    assert_ne!(hash, hash_ip_daily("198.51.100.1"));
}

// --- HTTP endpoint tests ---

/// POST /api/v1/telemetry/event returns 202 Accepted for valid payload
#[tokio::test]
async fn test_track_event_endpoint_returns_202() {
    // SAFETY: single-threaded test setup, no concurrent env access
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let payload = serde_json::json!({
        "event_name": "task.create",
        "event_type": "interaction",
        "path": "/co/test-universe",
        "universe_key": "test-universe",
        "properties": { "status": "todo" }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/telemetry/event")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
}

/// POST /api/v1/telemetry/event with empty event_name returns 400
#[tokio::test]
async fn test_track_event_endpoint_rejects_empty_name() {
    // SAFETY: single-threaded test setup, no concurrent env access
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let payload = serde_json::json!({ "event_name": "" });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/telemetry/event")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// GET /co/co-dev/telemetria returns the admin dashboard HTML
#[tokio::test]
async fn test_admin_dashboard_page_served() {
    // SAFETY: single-threaded test setup, no concurrent env access
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let req = Request::builder()
        .method("GET")
        .uri("/co/co-dev/telemetria")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
    assert!(ct.to_str().unwrap().contains("text/html"));

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("CO — Telemetria"), "page title present");
    assert!(
        html.contains("/api/v1/admin/telemetry/summary"),
        "summary API URL present"
    );
}

/// GET /api/v1/admin/telemetry/summary returns 401 without auth
#[tokio::test]
async fn test_admin_summary_requires_auth() {
    // SAFETY: single-threaded test setup, no concurrent env access
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/admin/telemetry/summary")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // No auth → 401 Unauthorized
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// GET /api/v1/admin/telemetry/export returns 401 without auth
#[tokio::test]
async fn test_admin_export_requires_auth() {
    // SAFETY: single-threaded test setup, no concurrent env access
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/admin/telemetry/export")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
