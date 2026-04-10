//! CO-44: UAT environment tests
//!
//! Covers:
//! - Migration v17 adds password_hash column
//! - `seed_uat_user` is idempotent
//! - `cleanup_anon_universes` removes anon-* universes
//! - `uat_login` handler returns 404 in prod mode
//! - `uat_login` handler works in UAT mode

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::auth::AuthStore;
use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{AppState, AppStateInner, build_router};
use co_web::storage::Storage;

extern crate co;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_config(dir: &std::path::Path, is_uat: bool) -> WebConfig {
    WebConfig {
        port: 3000,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: false,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: dir.join("universes").to_str().unwrap().to_string(),
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: if is_uat { "uat".into() } else { "prod".into() },
    }
}

fn build_app(dir: &std::path::Path, is_uat: bool) -> axum::Router {
    let config = test_config(dir, is_uat);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db = dir.join("game_test.db");
    let game_storage = Arc::new(game_core::storage::Storage::open(&game_db).expect("game storage"));
    let state: AppState = Arc::new(AppStateInner {
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

// ---------------------------------------------------------------------------
// Storage unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_migration_v17_adds_password_hash() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());
    // Migration v17 must have run — verify the column exists via pragma_table_info.
    let col_exists: bool = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name = 'password_hash'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    assert!(
        col_exists,
        "password_hash column should exist after migration v17"
    );
}

#[test]
fn test_seed_uat_user_idempotent() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());

    let hash = "$argon2id$v=19$m=19456,t=2,p=1$fakesalt$fakehash";
    storage.seed_uat_user(hash).unwrap();
    storage.seed_uat_user(hash).unwrap(); // second call must not fail

    // Exactly one yuri@uat.local user.
    let count: i64 = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM users WHERE email = 'yuri@uat.local'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "exactly one UAT user should exist after idempotent seed"
    );

    // Tier must be admin.
    let tier: String = storage
        .conn()
        .query_row(
            "SELECT tier FROM users WHERE email = 'yuri@uat.local'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tier, "admin");
}

#[test]
fn test_get_user_by_email_with_hash() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());

    let hash = "$argon2id$v=19$m=19456,t=2,p=1$fakesalt$fakehash";
    storage.seed_uat_user(hash).unwrap();

    let (user, stored_hash) = storage
        .get_user_by_email_with_hash("yuri@uat.local")
        .expect("should find yuri");

    assert_eq!(user.email, "yuri@uat.local");
    assert_eq!(user.tier, "admin");
    assert_eq!(stored_hash.as_deref(), Some(hash));
}

#[test]
fn test_cleanup_anon_universes() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());

    // Insert an anon-* universe and a regular universe.
    storage
        .conn()
        .execute(
            "INSERT INTO universes (key, name, description, owner_id, created_at, \
             is_template, is_public, content_count) \
             VALUES ('anon-abc', 'Anon', '', 'anon-abc', datetime('now'), 0, 0, 0)",
            [],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT INTO universes (key, name, description, owner_id, created_at, \
             is_template, is_public, content_count) \
             VALUES ('usr-kept', 'Kept', '', 'user-1', datetime('now'), 0, 0, 0)",
            [],
        )
        .unwrap();

    let removed = storage.cleanup_anon_universes();
    assert_eq!(removed, 1, "should have removed the anon-* universe");

    // Verify only the regular universe remains.
    let remaining: i64 = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM universes WHERE key NOT IN ('template', 'default')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 1, "non-anon universe should remain");
}

#[test]
fn test_get_all_users_with_hashes_and_restore() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());

    let hash = "$argon2id$v=19$m=19456,t=2,p=1$fakesalt$fakehash";
    storage.seed_uat_user(hash).unwrap();

    let backup = storage.get_all_users_with_hashes();
    assert!(!backup.is_empty());

    // Clear users.
    storage.conn().execute("DELETE FROM users", []).unwrap();
    let count_after_delete: i64 = storage
        .conn()
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count_after_delete, 0);

    // Restore.
    storage.restore_users_with_hashes(&backup);
    let count_after_restore: i64 = storage
        .conn()
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count_after_restore, 1);
}

// ---------------------------------------------------------------------------
// HTTP integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_uat_login_returns_404_in_prod_mode() {
    let dir = tempdir().unwrap();
    let app = build_app(dir.path(), false); // prod mode

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/uat-login")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"email":"yuri@uat.local","password":"uat"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "uat-login should return 404 in prod mode"
    );
}

#[tokio::test]
async fn test_uat_login_works_in_uat_mode() {
    let dir = tempdir().unwrap();

    // Pre-seed yuri@uat.local with a real Argon2 hash of "uat".
    {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
        let mut storage = Storage::new(dir.path().to_str().unwrap());
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(b"uat", &salt)
            .unwrap()
            .to_string();
        storage.seed_uat_user(&hash).unwrap();
    }

    let app = build_app(dir.path(), true); // UAT mode

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/uat-login")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"email":"yuri@uat.local","password":"uat"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "uat-login should succeed in UAT mode"
    );

    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["email"], "yuri@uat.local");
    assert!(body["user_id"].is_string());
}

#[tokio::test]
async fn test_uat_login_wrong_password_returns_401() {
    let dir = tempdir().unwrap();

    {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
        let mut storage = Storage::new(dir.path().to_str().unwrap());
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(b"uat", &salt)
            .unwrap()
            .to_string();
        storage.seed_uat_user(&hash).unwrap();
    }

    let app = build_app(dir.path(), true);

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/uat-login")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"email":"yuri@uat.local","password":"wrong"}"#,
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn test_is_uat_config() {
    let uat_config = WebConfig {
        port: 3000,
        data_dir: "/data".into(),
        static_dir: "static".into(),
        default_variant: "a".into(),
        experiments: false,
        plugins_dir: "plugins".into(),
        game_db_path: None,
        universo_dir: "/data/universes".into(),
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: "uat".into(),
    };
    assert!(uat_config.is_uat());

    let prod_config = WebConfig {
        co_env: "prod".into(),
        ..uat_config
    };
    assert!(!prod_config.is_uat());

    let default_config = WebConfig {
        co_env: "".into(),
        ..prod_config
    };
    assert!(!default_config.is_uat());
}
