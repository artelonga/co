use rusqlite::params;
use tempfile::tempdir;

use crate::models::UpdateUniverseFormConfig;
use crate::storage::Storage;

fn make_storage() -> (Storage, tempfile::TempDir) {
    // SAFETY: single-threaded test environment.
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());
    (storage, dir)
}

/// After migration v14, a universe gets scholarly-light theme and board layout by default.
#[test]
fn test_universe_form_config_defaults() {
    let (storage, _dir) = make_storage();
    let config = storage
        .get_universe_form_config("default")
        .expect("default universe must exist");
    assert_eq!(config.theme_preset, "scholarly-light");
    assert_eq!(config.layout, "board");
    assert!(config.font_headline.is_none());
    assert!(config.font_body.is_none());
    assert!(config.custom_tokens.is_none());
}

/// Updating theme_preset changes only that field; layout is preserved.
#[test]
fn test_update_form_config_theme() {
    let (mut storage, _dir) = make_storage();
    let updated = storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("relic-dark".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(updated.theme_preset, "relic-dark");
    assert_eq!(updated.layout, "board"); // unchanged

    // Persisted correctly.
    let persisted = storage.get_universe_form_config("default").unwrap();
    assert_eq!(persisted.theme_preset, "relic-dark");
}

/// Cloning a universe copies its form config exactly.
#[test]
fn test_clone_universe_inherits_form_config() {
    let (mut storage, _dir) = make_storage();

    // Give the default universe a custom theme + layout.
    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("scholarly-dark".to_string()),
                layout: Some("calendar".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

    // Make default public so it can be cloned.
    storage
        .conn()
        .execute(
            "UPDATE universes SET is_public = 1 WHERE key = 'default'",
            params![],
        )
        .unwrap();

    storage
        .clone_universe("default", "clone1", "Clone 1", "", "usr_test")
        .unwrap();

    let clone_config = storage
        .get_universe_form_config("clone1")
        .expect("clone must have form config");
    assert_eq!(clone_config.theme_preset, "scholarly-dark");
    assert_eq!(clone_config.layout, "calendar");
}

/// Changing form config does not affect entries in the same universe.
#[test]
fn test_form_config_change_does_not_affect_entries() {
    let (mut storage, _dir) = make_storage();

    // Create a project entry so entries table is non-empty.
    let universe_root = storage.universe_root("default");
    let entry = crate::entry_index::make_entry(
        "projects/TEST/_project.md",
        serde_json::json!({
            "type": "project",
            "key": "TEST",
            "title": "Test",
            "status": "active",
            "next_id": 1,
            "archived": false,
            "tags": []
        }),
        "Test project",
    );
    co::entry::write_entry(&universe_root, &entry).unwrap();
    crate::entry_index::EntryIndex::new(storage.conn())
        .upsert("default", &entry)
        .unwrap();

    // Change theme.
    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("relic".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

    // Entry still present and unmodified.
    let index = crate::entry_index::EntryIndex::new(storage.conn());
    let count = index.count("default", Some("project"));
    assert!(
        count > 0,
        "project entries must still be present after config change"
    );

    // Config changed.
    let config = storage.get_universe_form_config("default").unwrap();
    assert_eq!(config.theme_preset, "relic");
}

/// `.universo.yaml` is written when form config is updated.
#[test]
fn test_universo_yaml_written_on_update() {
    let (mut storage, _dir) = make_storage();

    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("relic-light".to_string()),
                layout: Some("table".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

    let yaml_path = storage.universe_root("default").join(".universo.yaml");
    assert!(yaml_path.exists(), ".universo.yaml must be written");
    let contents = std::fs::read_to_string(yaml_path).unwrap();
    assert!(contents.contains("relic-light"));
    assert!(contents.contains("table"));
}

// --- CO-25: theme gating ---

/// Anonymous user (no auth header) sees 4 free palettes, no variants, no custom editor.
#[tokio::test]
async fn test_themes_available_anonymous() {
    let headers = axum::http::HeaderMap::new();
    let axum::Json(themes) = super::get_available_themes(headers).await;

    assert_eq!(
        themes.palettes,
        vec!["scholarly", "scholarly-dark", "relic", "relic-light"]
    );
    assert!(themes.variants.is_empty());
    assert!(themes.custom.is_none());
}

/// Real logged-in user sees Modern + 4 free palettes + 8 variants + custom editor.
#[tokio::test]
async fn test_themes_available_logged_in() {
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let (token, _) =
        crate::auth::sign_jwt("usr_real", "user@example.com", "player", "test-secret").unwrap();

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    let axum::Json(themes) = super::get_available_themes(headers).await;

    assert_eq!(
        themes.palettes,
        vec!["", "scholarly", "scholarly-dark", "relic", "relic-light"]
    );
    assert_eq!(themes.variants.len(), 8);
    assert_eq!(themes.custom, Some(true));
}

/// Anon-tier user (cookie JWT with tier="anon") sees only free palettes.
#[tokio::test]
async fn test_themes_available_anon_cookie() {
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let (token, _) = crate::auth::sign_jwt("anon-abc123", "", "anon", "test-secret").unwrap();

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        format!("session={token}").parse().unwrap(),
    );
    let axum::Json(themes) = super::get_available_themes(headers).await;

    assert_eq!(
        themes.palettes,
        vec!["scholarly", "scholarly-dark", "relic", "relic-light"]
    );
    assert!(themes.variants.is_empty());
}

/// A premium theme (scholarly, relic) set by an owner persists even if the user logs out —
/// the storage layer always returns the stored preset regardless of auth.
#[test]
fn test_premium_theme_persists_after_owner_sets_it() {
    let (mut storage, _dir) = make_storage();

    // Owner sets a premium theme while logged in.
    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("relic".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

    // Reading config back (as if a new, unauthenticated visitor renders the universe)
    // must still return the premium theme — gating only applies to the switcher UI.
    let config = storage.get_universe_form_config("default").unwrap();
    assert_eq!(config.theme_preset, "relic");
}

// --- CO-30: theme.css endpoint ---

/// Build a minimal in-process router for the universe API (no port binding).
fn make_universe_router(
    storage: Storage,
    dir: &std::path::Path,
) -> (axum::Router, tempfile::TempDir) {
    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::server::{AppState, AppStateInner, build_router};
    use std::sync::{Arc, Mutex};

    let config = WebConfig {
        port: 0,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: false,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: "".to_string(),
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
    };
    let experiment = ExperimentStore::new(dir);
    let auth_store = crate::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage =
        Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
    let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
    let state: AppState = Arc::new(AppStateInner {
        storage: Mutex::new(storage),
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
        jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
        embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
        embedding_tx,
        chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
        chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
    });
    let router = build_router(state, None);
    let tmp = tempdir().unwrap(); // keep alive
    (router, tmp)
}

async fn body_bytes(response: axum::http::Response<axum::body::Body>) -> String {
    use http_body_util::BodyExt;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// GET /api/v1/universes/default/theme.css returns 200 with :root block.
#[tokio::test]
async fn test_theme_css_returns_ok() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let (storage, dir) = make_storage();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let ct = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/css"), "Content-Type must be text/css");
    let body = body_bytes(response).await;
    assert!(body.contains(":root {"), "CSS must contain :root block");
    assert!(body.contains("--bg:"), "CSS must contain --bg token");
    assert!(
        body.contains("--accent:"),
        "CSS must contain --accent token"
    );
}

/// All required tokens are present in the generated CSS.
#[tokio::test]
async fn test_theme_css_all_required_tokens() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let (storage, dir) = make_storage();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_bytes(response).await;

    for token in crate::theme_engine::tests::REQUIRED_TOKENS {
        assert!(
            body.contains(*token),
            "theme.css must contain token '{token}'"
        );
    }
}

/// Changing the theme changes the CSS output.
#[tokio::test]
async fn test_theme_css_changes_when_theme_changes() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let (mut storage, dir) = make_storage();

    // Set theme to scholarly-dark
    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("scholarly-dark".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_bytes(response).await;

    // scholarly-dark --bg is #1c1610
    assert!(
        body.contains("#1c1610"),
        "scholarly-dark --bg must be #1c1610"
    );
    // Must NOT have scholarly-light --bg
    assert!(
        !body.contains("#FFF9ED"),
        "scholarly-dark must not contain scholarly-light --bg"
    );
}

/// GET /theme.css for a missing universe returns 404.
#[tokio::test]
async fn test_theme_css_404_for_missing_universe() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let (storage, dir) = make_storage();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/no-such-universe/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

/// ETag is present and the same ETag triggers 304 Not Modified.
#[tokio::test]
async fn test_theme_css_etag_304() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let (storage, dir) = make_storage();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    // First request: capture ETag.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let etag = response
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();

    // Second request with If-None-Match: expect 304.
    let response2 = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .header(axum::http::header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response2.status(), axum::http::StatusCode::NOT_MODIFIED);
}

// --- CO-49: deterministic access check ---

/// Helper: set visibility on a universe.
fn set_visibility(storage: &Storage, key: &str, visibility: &str) {
    storage
        .conn()
        .execute(
            "UPDATE universes SET visibility = ?1, is_public = ?2, is_template = ?3 WHERE key = ?4",
            rusqlite::params![
                visibility,
                if visibility == "public-subscribable" || visibility == "public" {
                    1i64
                } else {
                    0i64
                },
                if visibility == "template" { 1i64 } else { 0i64 },
                key
            ],
        )
        .unwrap();
}

/// 1. Template universe → READ for everyone (anonymous).
#[test]
fn test_access_template_anonymous() {
    let (storage, _dir) = make_storage();
    set_visibility(&storage, "default", "template");
    let access = storage.check_universe_access(None, "default");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
}

/// 1. Template universe → READ for logged-in user too.
#[test]
fn test_access_template_logged_in() {
    let (storage, _dir) = make_storage();
    set_visibility(&storage, "default", "template");
    let access = storage.check_universe_access(Some("some-user"), "default");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
}

/// 2. Owner → READ+WRITE regardless of visibility.
#[test]
fn test_access_owner_readwrite() {
    let (mut storage, _dir) = make_storage();
    // "default" universe is owned by "system"; create one owned by test-owner.
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "my-uni".into(),
                name: "My Universe".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    let access = storage.check_universe_access(Some("owner-1"), "my-uni");
    assert_eq!(access, crate::models::UniverseAccess::ReadWrite);
}

/// 3. Member with editor role → READ+WRITE.
#[test]
fn test_access_editor_member_readwrite() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "collab".into(),
                name: "Collab".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    storage
        .add_universe_member("collab", "editor-1", "editor")
        .unwrap();
    let access = storage.check_universe_access(Some("editor-1"), "collab");
    assert_eq!(access, crate::models::UniverseAccess::ReadWrite);
}

/// 4. Member with viewer role → READ only.
#[test]
fn test_access_viewer_member_readonly() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "readonly-uni".into(),
                name: "Read Only".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    storage
        .add_universe_member("readonly-uni", "viewer-1", "viewer")
        .unwrap();
    let access = storage.check_universe_access(Some("viewer-1"), "readonly-uni");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
}

/// 5. Subscribed user → READ only.
#[test]
fn test_access_subscribed_readonly() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "pub-uni".into(),
                name: "Public".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "pub-uni", "public-subscribable");
    storage.subscribe_universe("sub-user", "pub-uni").unwrap();
    let access = storage.check_universe_access(Some("sub-user"), "pub-uni");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
}

/// 6. Public-subscribable universe → MetadataOnly for non-subscribed anonymous.
#[test]
fn test_access_public_subscribable_anonymous_metadata_only() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "disco".into(),
                name: "Discoverable".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "disco", "public-subscribable");
    // Anonymous (no user_id)
    let access = storage.check_universe_access(None, "disco");
    assert_eq!(access, crate::models::UniverseAccess::MetadataOnly);
}

/// 1.46.0: public-subscribable → ReadOnly for any logged-in user
/// (including non-subscribers). Anonymous still gets MetadataOnly. The
/// pre-collapse behavior gated content behind subscription; the new
/// model treats any authed user as eligible to read.
#[test]
fn test_access_public_subscribable_logged_in_not_subscribed() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "disco2".into(),
                name: "Discoverable2".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "disco2", "public-subscribable");
    let access = storage.check_universe_access(Some("other-user"), "disco2");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
    // Anonymous still sees only metadata.
    let anon = storage.check_universe_access(None, "disco2");
    assert_eq!(anon, crate::models::UniverseAccess::MetadataOnly);
}

/// 7. Private universe → Denied for non-owner.
#[test]
fn test_access_private_denied_to_non_owner() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "secret".into(),
                name: "Secret".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    let access = storage.check_universe_access(Some("attacker"), "secret");
    assert_eq!(access, crate::models::UniverseAccess::Denied);
}

/// 7. Private universe → Denied for anonymous user.
#[test]
fn test_access_private_denied_anonymous() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "secret2".into(),
                name: "Secret2".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    let access = storage.check_universe_access(None, "secret2");
    assert_eq!(access, crate::models::UniverseAccess::Denied);
}

/// Non-existent universe → Denied.
#[test]
fn test_access_nonexistent_denied() {
    let (storage, _dir) = make_storage();
    let access = storage.check_universe_access(None, "does-not-exist");
    assert_eq!(access, crate::models::UniverseAccess::Denied);
}

/// Subscribe/unsubscribe flow: subscriptions table is correctly updated.
#[test]
fn test_subscribe_unsubscribe_flow() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "pub3".into(),
                name: "Public3".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "pub3", "public-subscribable");

    // Not subscribed yet.
    assert!(!storage.is_subscribed("user-a", "pub3"));

    // Subscribe.
    storage.subscribe_universe("user-a", "pub3").unwrap();
    assert!(storage.is_subscribed("user-a", "pub3"));

    // Appears in user's universe list.
    let universes = storage.list_universes_for_user("user-a");
    assert!(
        universes.iter().any(|u| u.key == "pub3"),
        "subscribed universe must appear in user list"
    );

    // Unsubscribe.
    storage.unsubscribe_universe("user-a", "pub3").unwrap();
    assert!(!storage.is_subscribed("user-a", "pub3"));

    // No longer in user's universe list.
    let universes_after = storage.list_universes_for_user("user-a");
    assert!(
        !universes_after.iter().any(|u| u.key == "pub3"),
        "unsubscribed universe must not appear in user list"
    );
}

/// Cannot subscribe to a private universe.
#[test]
fn test_cannot_subscribe_to_private_universe() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "private-u".into(),
                name: "Private".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    let result = storage.subscribe_universe("user-b", "private-u");
    assert!(
        result.is_err(),
        "subscribing to a private universe must fail"
    );
}

/// Search returns only public-subscribable universes matching the query.
#[test]
fn test_search_public_universes() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "co-dev".into(),
                name: "CO Development".into(),
                description: "The main dev board".into(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "co-dev", "public-subscribable");

    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "private-proj".into(),
                name: "Private Project".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();

    let results = storage.search_public_universes("dev");
    assert!(
        results.iter().any(|u| u.key == "co-dev"),
        "co-dev must appear in search results"
    );
    assert!(
        !results.iter().any(|u| u.key == "private-proj"),
        "private universe must not appear in search results"
    );
}

// --- CO-66: 409 on duplicate universe key ---

/// POST /api/v1/universes with an existing key returns 409 Conflict.
#[tokio::test]
async fn test_create_universe_duplicate_key_returns_409() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let (mut storage, dir) = make_storage();

    // Pre-create the universe directly in storage.
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "dupe-uni".into(),
                name: "Dupe Universe".into(),
                description: String::new(),
            },
            "usr_owner",
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());

    let (token, _) =
        crate::auth::sign_jwt("usr_owner", "owner@example.com", "player", "test-secret").unwrap();

    let payload = serde_json::json!({
        "key": "dupe-uni",
        "name": "Another Universe",
        "description": ""
    });
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let body = body_bytes(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["error"], "conflict");
}
