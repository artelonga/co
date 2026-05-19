// CO-41: quilomboaraucaria universe — seed, stats endpoint, idempotent import
// CO-167: email collection for quilombo users
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
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
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
        event_bus: co_web::events::Bus::new(),
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

/// CO-66: re-seeding preserves user-edited description (INSERT OR IGNORE, never UPDATE).
#[tokio::test]
async fn test_quilombo_seed_preserves_user_edited_description() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());

    // Seed once (simulates first server boot).
    storage.seed_quilombo_universe();

    // User edits the description via the API (simulated as a direct DB write).
    storage
        .conn()
        .execute(
            "UPDATE universes SET description = 'Descrição editada pelo usuário' \
             WHERE key = 'quilomboaraucaria'",
            rusqlite::params![],
        )
        .unwrap();

    let after_edit = storage.get_universe("quilomboaraucaria").unwrap();
    assert_eq!(after_edit.description, "Descrição editada pelo usuário");

    // Seed again (simulates server restart where the guard is absent or ignored).
    storage.seed_quilombo_universe();

    // Description must still be the user-edited value.
    let after_reseed = storage.get_universe("quilomboaraucaria").unwrap();
    assert_eq!(
        after_reseed.description, "Descrição editada pelo usuário",
        "seed must not overwrite user-edited description"
    );
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

// --- CO-167: email collection ---

/// Login response includes missing_email: true for new user with no email.
#[tokio::test]
async fn test_login_missing_email_flag_true() {
    let dir = tempdir().unwrap();
    let app = build_quilombo_app(dir.path());

    let body = serde_json::json!({
        "usuario": "testuser_email",
        "nome": "Test User",
        "senha": "senha123456"
    })
    .to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/quilombo/auth/cadastro")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let token = json["token"].as_str().unwrap().to_string();

    // Login → missing_email: true
    let login_body =
        serde_json::json!({"usuario": "testuser_email", "senha": "senha123456"}).to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/quilombo/auth/login")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(login_body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        json["missing_email"], true,
        "new user should have missing_email: true"
    );
}

/// Setting an email via PUT /perfil works and login then shows missing_email: false.
#[tokio::test]
async fn test_set_email_works() {
    let dir = tempdir().unwrap();
    let app = build_quilombo_app(dir.path());

    // Register
    let body = serde_json::json!({
        "usuario": "emailuser",
        "nome": "Email User",
        "senha": "senha123456"
    })
    .to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/quilombo/auth/cadastro")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let token = json["token"].as_str().unwrap().to_string();

    // Set email via PUT /perfil
    let perfil_body = serde_json::json!({"email": "emailuser@example.com"}).to_string();
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/quilombo/perfil")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(perfil_body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let perfil: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(perfil["email"], "emailuser@example.com");

    // Login again → missing_email: false
    let login_body =
        serde_json::json!({"usuario": "emailuser", "senha": "senha123456"}).to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/quilombo/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(login_body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        json["missing_email"], false,
        "user with email should have missing_email: false"
    );
}

/// Duplicate email returns 409 Conflict.
#[tokio::test]
async fn test_duplicate_email_returns_409() {
    let dir = tempdir().unwrap();
    let app = build_quilombo_app(dir.path());

    // Register two users
    for usuario in &["dupuser1", "dupuser2"] {
        let body = serde_json::json!({
            "usuario": usuario,
            "nome": "Dup User",
            "senha": "senha123456"
        })
        .to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/quilombo/auth/cadastro")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        app.clone().oneshot(req).await.unwrap();
    }

    // Get token for user1
    let login1 = serde_json::json!({"usuario": "dupuser1", "senha": "senha123456"}).to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/quilombo/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(login1))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let token1 = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // Get token for user2
    let login2 = serde_json::json!({"usuario": "dupuser2", "senha": "senha123456"}).to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/quilombo/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(login2))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let token2 = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    let shared_email = "shared@example.com";

    // user1 sets email
    let body = serde_json::json!({"email": shared_email}).to_string();
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/quilombo/perfil")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token1}"))
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // user2 tries the same email → 409
    let body = serde_json::json!({"email": shared_email}).to_string();
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/quilombo/perfil")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token2}"))
        .body(Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

/// Admin resumo includes com_email and vinculados_co counts.
#[tokio::test]
async fn test_admin_resumo_includes_email_stats() {
    let dir = tempdir().unwrap();
    let app = build_quilombo_app(dir.path());

    // Register an admin and get token
    let body = serde_json::json!({
        "usuario": "admin_stats",
        "nome": "Admin Stats",
        "senha": "senha123456"
    })
    .to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/quilombo/auth/cadastro")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let token = json["token"].as_str().unwrap().to_string();

    // Promote to admin directly via storage
    let storage = Storage::new(dir.path().to_str().unwrap());
    let user_id = json["usuario"]["id"].as_str().unwrap().to_string();
    storage
        .conn()
        .execute(
            "UPDATE quilombo_usuarios SET papel = 'admin' WHERE id = ?1",
            rusqlite::params![user_id],
        )
        .unwrap();

    // Hit admin resumo
    let req = Request::builder()
        .uri("/api/v1/quilombo/admin/resumo")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let stats: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        stats.get("com_email").is_some(),
        "resumo should include com_email"
    );
    assert!(
        stats.get("vinculados_co").is_some(),
        "resumo should include vinculados_co"
    );
    assert_eq!(stats["com_email"], 0);
    // CO-184 reverse bridge auto-links every quilombo signup to a fresh
    // CO user, so vinculados_co counts the one user just created.
    // Previously this asserted 0 (pre-CO-184). The test's intent is
    // "stat field exists and is sensible," not a specific count.
    assert_eq!(stats["vinculados_co"], 1);
}
