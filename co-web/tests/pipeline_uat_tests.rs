// CO-88: end-to-end pipeline UAT — server-side telemetry, aggregation, admin
// endpoint, and dashboard. All in-process with a temp SQLite DB — no network.
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::pipeline::{PipelineMetrics, pipeline_summary};
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use co_web::storage::Storage;
use co_web::telemetry::{EventRow, insert_event};

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

fn transfer_row(universe: &str, combo: &str, raw: i64, wire: i64) -> EventRow {
    let props = serde_json::json!({
        "combo": combo,
        "co_format": "co/protobuf",
        "compression": if combo == "zstd" { "zstd-3" } else { "raw" },
        "encryption": if combo == "private" { "chacha20" } else { "none" },
        "size_uncompressed": raw,
        "size_on_wire": wire,
        "encode_ns": 12_000,
        "decode_ns": 8_000,
        "op": "put",
    });
    EventRow {
        timestamp: chrono::Utc::now().to_rfc3339(),
        visitor_token: None,
        user_id: None,
        session_id: None,
        event_type: "pipeline.transfer".to_string(),
        event_name: "put".to_string(),
        universe_key: Some(universe.to_string()),
        path: Some("notes/a.md".to_string()),
        properties: Some(props),
        duration_ms: None,
        ip_hash: None,
        ua_device: None,
        ua_browser: None,
        ua_os: None,
        country: None,
        city: None,
    }
}

/// pipeline.transfer events aggregate into the per-universe report shape.
#[test]
fn test_pipeline_summary_aggregates_storage_events() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());
    let conn = storage.conn();

    insert_event(conn, &transfer_row("artelonga", "zstd", 1000, 400));
    insert_event(conn, &transfer_row("artelonga", "zstd", 1000, 400));
    insert_event(conn, &transfer_row("artelonga", "private", 1000, 420));
    insert_event(conn, &transfer_row("rfq", "private", 500, 260));

    let summary = pipeline_summary(conn, None, "");
    assert_eq!(summary.total_events, 4);

    let artelonga = summary
        .per_universe
        .iter()
        .find(|u| u.universe_key == "artelonga")
        .expect("artelonga present");
    assert_eq!(artelonga.files, 3);
    assert_eq!(artelonga.raw_bytes, 3000);
    assert_eq!(artelonga.by_combo.len(), 2, "zstd + private combos");

    let zstd = artelonga
        .by_combo
        .iter()
        .find(|c| c.combo == "zstd")
        .unwrap();
    assert_eq!(zstd.count, 2);
    assert!((zstd.compression_ratio - 2.5).abs() < 0.01);
}

/// X-Co-* headers parse into PipelineMetrics; absence of X-Co-Format => None.
#[test]
fn test_pipeline_metrics_from_headers() {
    use axum::http::HeaderMap;
    let mut h = HeaderMap::new();
    assert!(PipelineMetrics::from_headers(&h, 50).is_none());

    h.insert("x-co-format", "co/protobuf".parse().unwrap());
    h.insert("x-co-compression", "zstd-3".parse().unwrap());
    h.insert("x-co-encryption", "chacha20".parse().unwrap());
    h.insert("x-co-combo", "private".parse().unwrap());
    h.insert("x-co-size-on-wire", "420".parse().unwrap());
    let m = PipelineMetrics::from_headers(&h, 1000).unwrap();
    assert_eq!(m.combo, "private");
    assert_eq!(m.encryption, "chacha20");
    assert_eq!(m.size_on_wire, 420);
    assert_eq!(m.size_uncompressed, 1000);
}

/// GET /co/co-dev/pipeline returns the admin dashboard HTML.
#[tokio::test]
async fn test_pipeline_dashboard_page_served() {
    // SAFETY: single-threaded test setup, no concurrent env access
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let req = Request::builder()
        .method("GET")
        .uri("/co/co-dev/pipeline")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
    assert!(ct.to_str().unwrap().contains("text/html"));

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Content Pipeline"), "dashboard title present");
    assert!(
        html.contains("/api/v1/admin/pipeline/summary"),
        "summary API URL present"
    );
}

/// GET /api/v1/admin/pipeline/summary requires admin auth.
#[tokio::test]
async fn test_pipeline_summary_requires_auth() {
    // SAFETY: single-threaded test setup, no concurrent env access
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/admin/pipeline/summary")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
