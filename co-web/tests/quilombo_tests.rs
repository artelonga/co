// CO-41: quilomboaraucaria universe — seed, stats endpoint, idempotent import
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::models::QuilomboStats;
use co_web::server::{AppStateInner, build_router};
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
    }
}

fn build_quilombo_app(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    storage.seed_quilombo_universe();
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
    );
    let state: co_web::server::AppState = Arc::new(AppStateInner {
        storage: Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config,
        auth_store: Mutex::new(auth_store),
        mail,
        game_storage,
        plugin_registry: game_core::plugin::PluginRegistry::new(),
        doc_rooms: co_web::ws::new_room_manager(),
    });
    build_router(state, None)
}

/// Seeding is idempotent — calling it twice leaves exactly one universe.
#[tokio::test]
async fn test_quilombo_seed_idempotent() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());

    storage.seed_quilombo_universe();
    storage.seed_quilombo_universe(); // second call should be a no-op

    assert!(storage.quilombo_universe_exists());
    let u = storage.get_universe("quilomboaraucaria").unwrap();
    assert_eq!(u.key, "quilomboaraucaria");
    assert!(u.is_public);
    assert_eq!(u.owner_id, "system");
}

/// Universe uses the quilombo theme preset.
#[tokio::test]
async fn test_quilombo_theme_preset() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_quilombo_universe();

    let config = storage
        .get_universe_form_config("quilomboaraucaria")
        .expect("quilomboaraucaria must have form config");

    assert_eq!(config.theme_preset, "quilombo");
    assert_eq!(config.font_headline.as_deref(), Some("Playfair Display"));
    assert_eq!(config.font_body.as_deref(), Some("Inter"));
}

/// GET /api/v1/universes/quilomboaraucaria returns 200 for public universe.
#[tokio::test]
async fn test_quilombo_universe_info_endpoint() {
    let dir = tempdir().unwrap();
    let app = build_quilombo_app(dir.path());

    let req = Request::builder()
        .uri("/api/v1/universes/quilomboaraucaria")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["key"], "quilomboaraucaria");
    assert_eq!(json["is_template"], false);
}

/// GET /api/v1/universes/quilomboaraucaria/stats returns QuilomboStats JSON.
#[tokio::test]
async fn test_quilombo_stats_endpoint() {
    let dir = tempdir().unwrap();
    let app = build_quilombo_app(dir.path());

    let req = Request::builder()
        .uri("/api/v1/universes/quilomboaraucaria/stats")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let stats: QuilomboStats =
        serde_json::from_slice(&body).expect("Response should be valid QuilomboStats JSON");

    // Fresh universe has no content yet — all zeros, no ultimaSync
    assert_eq!(stats.total_publicacoes, 0);
    assert_eq!(stats.total_eventos, 0);
    assert_eq!(stats.total_missoes, 0);
    assert!(stats.ultima_sync.is_none());
}

/// stats() counts entries by type correctly.
#[tokio::test]
async fn test_quilombo_stats_counts_entries() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_quilombo_universe();

    // Insert test entries directly
    let conn = storage.conn();
    let now = "2026-04-10T00:00:00Z";
    for (path, etype, title) in &[
        ("posts/post-1.md", "post", "Post 1"),
        ("posts/post-2.md", "post", "Post 2"),
        ("events/event-1.md", "event", "Evento 1"),
        ("missions/1.md", "mission", "Missao 1"),
    ] {
        conn.execute(
            "INSERT OR REPLACE INTO entries \
             (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
              created_at, updated_at) \
             VALUES (?1, 'quilomboaraucaria', ?2, ?3, '{}', '', 'deadbeef', ?4, ?4)",
            rusqlite::params![path, etype, title, now],
        )
        .unwrap();
    }

    let stats = storage.quilombo_stats();
    assert_eq!(stats.total_publicacoes, 2);
    assert_eq!(stats.total_eventos, 1);
    assert_eq!(stats.total_missoes, 1);
}

/// The quilombo theme preset contains the required brand tokens.
#[test]
fn test_quilombo_theme_has_brand_tokens() {
    let preset =
        co_web::theme_engine::ThemePreset::by_name("quilombo").expect("quilombo preset must exist");

    assert_eq!(
        preset.tokens.get("--accent").map(String::as_str),
        Some("#2d4a22"),
        "--accent must be folha green"
    );
    assert_eq!(
        preset.tokens.get("--bg").map(String::as_str),
        Some("#f5f0e8"),
        "--bg must be areia sand"
    );
    assert_eq!(
        preset.tokens.get("--card-bg").map(String::as_str),
        Some("#faf6ef"),
        "--card-bg must match quilombo card color"
    );
    assert_eq!(
        preset.tokens.get("--border").map(String::as_str),
        Some("#c8b48e"),
        "--border must match quilombo border color"
    );
    assert_eq!(
        preset.font_headline.as_deref(),
        Some("Playfair Display"),
        "headline font must be Playfair Display"
    );
    assert_eq!(
        preset.font_body.as_deref(),
        Some("Inter"),
        "body font must be Inter"
    );
}

/// All required theme tokens are present in the quilombo preset.
#[test]
fn test_quilombo_preset_has_all_required_tokens() {
    // Mirror the required token list from theme_engine tests
    const REQUIRED: &[&str] = &[
        "--bg",
        "--sidebar-bg",
        "--card-bg",
        "--text-primary",
        "--text-secondary",
        "--accent",
        "--border",
        "--status-todo",
        "--status-in_progress",
        "--status-in_review",
        "--status-done",
        "--status-todo-bg",
        "--status-in_progress-bg",
        "--status-in_review-bg",
        "--status-done-bg",
        "--status-todo-text",
        "--status-in_progress-text",
        "--status-in_review-text",
        "--status-done-text",
        "--priority-low",
        "--priority-medium",
        "--priority-high",
        "--priority-critical",
        "--font",
        "--font-mono",
        "--radius-sm",
        "--radius-md",
        "--radius-lg",
        "--shadow-sm",
        "--shadow-md",
        "--shadow-lg",
    ];

    let preset = co_web::theme_engine::ThemePreset::by_name("quilombo").unwrap();
    for token in REQUIRED {
        assert!(
            preset.tokens.contains_key(*token),
            "quilombo preset missing required token '{token}'"
        );
    }
}
