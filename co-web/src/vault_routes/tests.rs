use super::*;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use crate::auth::sign_jwt;
use crate::config::WebConfig;
use crate::experiment::ExperimentStore;
use crate::models::CreateUniverse;
use crate::server::{AppState, AppStateInner, build_router};
use crate::storage::Storage;

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

/// Sign a test JWT with the CURRENT JWT_SECRET env var (or fallback).
/// This avoids overriding the global env var and breaking concurrent tests
/// (universe_routes and ws tests also mutate JWT_SECRET).
fn test_bearer() -> String {
    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".to_string());
    let (token, _) = sign_jwt("test-user", "test@example.com", "player", &secret).unwrap();
    format!("Bearer {token}")
}

fn build_test_router(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = crate::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
    );
    let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
    let state: AppState = Arc::new(AppStateInner {
        storage: parking_lot::Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config,
        auth_store: Mutex::new(auth_store),
        mail,
        game_storage,
        plugin_registry: game_core::plugin::PluginRegistry::new(),
        doc_rooms: crate::ws::new_room_manager(),
        sync_rooms: crate::sync_ws::new_sync_room_manager(),
        cache: crate::cache::CacheLayer::new(),
        rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
        wae: crate::wae::WaeEmitter::new(None, None),
        jwt_key: std::sync::Arc::new(crate::auth::JwtKey::load_or_generate()),
        embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
        embedding_tx,
        chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
        chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
        event_bus: crate::events::Bus::new(),
        worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
    });
    build_router(state, None)
}

fn seed_universe(dir: &std::path::Path, slug: &str) {
    let mut storage = Storage::new(dir.to_str().unwrap());
    let _ = storage.create_universe(
        CreateUniverse {
            key: slug.to_string(),
            name: slug.to_string(),
            description: String::new(),
        },
        "test-user",
    );
}

async fn body_str(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// -----------------------------------------------------------------------
// CRUD cycle
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_vault_crud_cycle() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "test-vault");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    let content = "---\ntitle: Test Note\ntags: [rust, test]\ntype: note\n---\n\nHello vault!";

    // PUT — create file
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(content))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(json["path"], "notes/hello.md");
    assert_eq!(json["frontmatter"]["title"], "Test Note");

    // GET — read file
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(json["frontmatter"]["title"], "Test Note");
    assert!(
        json["tags"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("rust"))
    );

    // GET / — list files
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/test-vault/vault/")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let files: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(
        files
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "notes/hello.md")
    );

    // POST — append
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("Appended line."))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET — verify append
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(json["content"].as_str().unwrap().contains("Appended line."));

    // POST /search
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/test-vault/vault/search")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"query":"vault"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let results: serde_json::Value =
        serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(!results.as_array().unwrap().is_empty());

    // DELETE — soft
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET after delete — 404
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// -----------------------------------------------------------------------
// PATCH targeted edits
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_vault_patch_frontmatter() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "patch-test");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    let content = "---\ntitle: Patch Me\ntags: [old]\ntype: note\n---\n\nBody.";
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/patch-test/vault/patch.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(content))
                .unwrap(),
        )
        .await
        .unwrap();

    // PATCH frontmatter — replace tags
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/universes/patch-test/vault/patch.md")
                .header(header::AUTHORIZATION, &bearer)
                .header("target-type", "frontmatter")
                .header("target", "tags")
                .header("operation", "replace")
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(r#"["new","updated"]"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/patch-test/vault/patch.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    let tags: Vec<&str> = json["frontmatter"]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(tags.contains(&"new"), "Expected 'new' in {tags:?}");
    assert!(tags.contains(&"updated"), "Expected 'updated' in {tags:?}");
    assert!(
        !tags.contains(&"old"),
        "Should not contain 'old' in {tags:?}"
    );
}

#[tokio::test]
async fn test_vault_patch_heading() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "heading-test");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    let content = "---\ntitle: Doc\ntype: note\n---\n\n## Introduction\n\nOld intro text.\n\n## Other\n\nOther content.";
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/heading-test/vault/doc.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(content))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/universes/heading-test/vault/doc.md")
                .header(header::AUTHORIZATION, &bearer)
                .header("target-type", "heading")
                .header("target", "## Introduction")
                .header("operation", "replace")
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("New intro text."))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/heading-test/vault/doc.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    let c = json["content"].as_str().unwrap();
    assert!(c.contains("New intro text."), "Expected new text in: {c}");
    assert!(
        !c.contains("Old intro text."),
        "Should not contain old: {c}"
    );
    assert!(c.contains("## Other"), "Should keep other sections: {c}");
}

// -----------------------------------------------------------------------
// Clipper format
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_vault_clipper_post() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "clipper-test");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    let clip_req = serde_json::json!({
        "content": "---\ntitle: \"Article Title\"\nsource: \"https://example.com/article\"\nauthor: \"Author Name\"\npublished: \"2026-04-06\"\ncreated: \"2026-04-06T13:00:00Z\"\ntags: [web-clip, topic]\n---\n\n# Article Title\n\nClipped content here..."
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/clipper-test/vault/clip")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(clip_req.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let result: serde_json::Value =
        serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(
        result["path"]
            .as_str()
            .unwrap()
            .starts_with("content/clips/"),
        "Path should be in content/clips/: {}",
        result["path"]
    );
    assert!(
        result["slug"].as_str().unwrap().contains("article-title"),
        "Slug should contain 'article-title': {}",
        result["slug"]
    );

    // Read back the clip and verify fields
    let path = result["path"].as_str().unwrap().to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/clipper-test/vault/{path}"))
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let file: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(file["frontmatter"]["type"], "clip");
    assert_eq!(file["frontmatter"]["source"], "https://example.com/article");
    assert!(
        file["tags"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("web-clip"))
    );
}

// -----------------------------------------------------------------------
// Auth — unauthenticated request rejected
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_vault_requires_auth() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "auth-test");
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/auth-test/vault/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// -----------------------------------------------------------------------
// API token lifecycle
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_api_token_lifecycle() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    // Create token
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/token")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"obsidian-plugin"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: serde_json::Value =
        serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    let token_val = created["token"].as_str().unwrap().to_string();
    let token_id = created["id"].as_str().unwrap().to_string();
    assert!(
        token_val.starts_with("co_"),
        "Token should have co_ prefix: {token_val}"
    );

    // List tokens
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/tokens")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(list.as_array().unwrap().iter().any(|t| t["id"] == token_id));

    // Use the API token to access vault
    seed_universe(dir.path(), "token-test");
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/token-test/vault/")
                .header(header::AUTHORIZATION, format!("Bearer {token_val}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Revoke token
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/auth/tokens/{token_id}"))
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Revoked token should no longer work
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/token-test/vault/")
                .header(header::AUTHORIZATION, format!("Bearer {token_val}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// -----------------------------------------------------------------------
// Integration: Obsidian plugin mock → vault API → verify SQLite state
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_obsidian_plugin_mock_flow() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "obs-test");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    // Simulate plugin writing a note
    let note = "---\ntitle: Plugin Note\ntype: note\ntags: [from-plugin]\n---\n\nPlugin body.";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/obs-test/vault/notes/plugin-note.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(note))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "PUT should succeed");

    // Verify via vault listing — entry should appear
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/obs-test/vault/")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let files: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(
        files
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "notes/plugin-note.md"),
        "Entry should appear in vault listing (SQLite indexed): {files}"
    );

    // Verify content preserved correctly
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/obs-test/vault/notes/plugin-note.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let file: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(
        file["frontmatter"]["title"], "Plugin Note",
        "SQLite should have the correct title"
    );
    assert!(
        file["tags"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("from-plugin")),
        "Tags should be preserved in SQLite"
    );
}

// -----------------------------------------------------------------------
// Rate limiter unit test (no network)
// -----------------------------------------------------------------------

#[test]
fn test_rate_limiter_allows_up_to_60() {
    // Use a unique id per test run to avoid cross-test interference
    let id = format!("rate-test-{}", nanoid::nanoid!(12));
    for i in 0..60 {
        assert!(check_rate_limit(&id), "Request {i} should be within limit");
    }
    assert!(
        !check_rate_limit(&id),
        "61st request should be rate-limited"
    );
}

// -----------------------------------------------------------------------
// Helper unit tests (no I/O)
// -----------------------------------------------------------------------

#[test]
fn test_slugify() {
    assert_eq!(slugify("Hello World!"), "hello-world");
    assert_eq!(slugify("Article Title"), "article-title");
    assert_eq!(slugify("  spaces  "), "spaces");
    assert_eq!(slugify("CO v1.0 Release"), "co-v10-release");
}

#[test]
fn test_patch_frontmatter_replace() {
    let fm = serde_json::json!({"tags": ["old"], "title": "Test"});
    let result = patch_frontmatter(fm, "tags", r#"["new","updated"]"#, "replace");
    let tags = result["tags"].as_array().unwrap();
    assert!(tags.contains(&serde_json::json!("new")));
    assert!(!tags.contains(&serde_json::json!("old")));
}

#[test]
fn test_patch_frontmatter_append() {
    let fm = serde_json::json!({"tags": ["existing"]});
    let result = patch_frontmatter(fm, "tags", r#""new-tag""#, "append");
    let tags = result["tags"].as_array().unwrap();
    assert!(tags.contains(&serde_json::json!("existing")));
    assert!(tags.contains(&serde_json::json!("new-tag")));
}

#[test]
fn test_extract_matches() {
    let text = "Hello vault world, vault is great";
    let matches = extract_matches(text, "vault", 10);
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_patch_heading_replace() {
    let body = "## Introduction\n\nOld text.\n\n## Other\n\nKeep this.";
    let result = patch_heading(body, "## Introduction", "New text.", "replace");
    assert!(result.contains("New text."), "Got: {result}");
    assert!(!result.contains("Old text."), "Got: {result}");
    assert!(result.contains("## Other"), "Got: {result}");
}

#[test]
fn test_patch_block_replace() {
    let body = "Some paragraph with block ref. ^myblock\n\nOther content.";
    let result = patch_block(body, "^myblock", "Replaced paragraph.", "replace");
    assert!(
        result.contains("Replaced paragraph. ^myblock"),
        "Got: {result}"
    );
    assert!(!result.contains("Some paragraph"), "Got: {result}");
}
