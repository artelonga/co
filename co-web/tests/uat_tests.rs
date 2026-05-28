//! CO-44 + CO-45: UAT environment tests
//!
//! CO-44:
//! - Migration v17 adds password_hash column
//! - `seed_uat_user` is idempotent
//! - `cleanup_anon_universes` removes anon-* universes
//! - `uat_login` handler returns 404 in prod mode
//! - `uat_login` handler works in UAT mode
//!
//! CO-45:
//! - Migration v19 adds uat_mutations table
//! - `log_uat_mutation` / `get_uat_mutations_since` roundtrip
//! - `create_snapshot` writes a JSON file with watermark
//! - GET /api/v1/uat/changes returns 404 in prod mode
//! - GET /api/v1/uat/changes returns mutations since last snapshot in UAT mode
//! - POST /api/v1/uat/export-patch generates a valid tar.gz tarball

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::auth::AuthStore;
use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
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
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
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
            rate_limiter: std::sync::Mutex::new(co_web::rate_limit::RateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: co_web::infra::workers::InProcessExecutor::new_arc(),
        }),
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
    // Exclude system-seeded universes that get inserted at boot
    // (template, default, yggdrasil, etc.); only the test-inserted
    // 'usr-kept' should remain after anon cleanup.
    let kept: i64 = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM universes WHERE key = 'usr-kept'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1, "usr-kept universe should remain");
    let anon_left: i64 = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM universes WHERE key LIKE 'anon-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(anon_left, 0, "anon universes should be gone");
}

#[test]
fn test_get_all_users_with_hashes_and_restore() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());

    let hash = "$argon2id$v=19$m=19456,t=2,p=1$fakesalt$fakehash";
    storage.seed_uat_user(hash).unwrap();

    let backup = storage.get_all_users_with_hashes();
    let backup_count = backup.len();
    assert!(!backup.is_empty());

    // Clear users.
    storage.conn().execute("DELETE FROM users", []).unwrap();
    let count_after_delete: i64 = storage
        .conn()
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count_after_delete, 0);

    // Restore — count should match the backup count (don't pin to 1
    // because boot-time auto-seeded users like a yggdrasil bot or
    // system user may be present in addition to the UAT user).
    storage.restore_users_with_hashes(&backup);
    let count_after_restore: i64 = storage
        .conn()
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count_after_restore, backup_count as i64);
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
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
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

// ---------------------------------------------------------------------------
// CO-45: UAT mutation tracking unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_migration_v19_adds_uat_mutations_table() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());
    let table_exists: bool = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='uat_mutations'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    assert!(
        table_exists,
        "uat_mutations table should exist after migration v19"
    );
}

#[test]
fn test_log_and_retrieve_uat_mutations() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());

    // No mutations yet — watermark should be 0.
    assert_eq!(storage.get_uat_mutations_watermark(), 0);

    storage
        .log_uat_mutation(
            "entry.create",
            "template:projects/CO/1.md",
            None,
            Some("{\"title\":\"Task 1\"}"),
            None,
            None,
        )
        .unwrap();
    storage
        .log_uat_mutation(
            "entry.update",
            "template:projects/CO/1.md",
            Some("{\"title\":\"Task 1\"}"),
            Some("{\"title\":\"Task 1 Updated\"}"),
            None,
            None,
        )
        .unwrap();

    // Watermark should now be 2.
    assert_eq!(storage.get_uat_mutations_watermark(), 2);

    // All mutations since id 0.
    let all = storage.get_uat_mutations_since(0);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].action, "entry.create");
    assert_eq!(all[1].action, "entry.update");

    // Mutations since id 1 — only the update.
    let since_first = storage.get_uat_mutations_since(1);
    assert_eq!(since_first.len(), 1);
    assert_eq!(since_first[0].action, "entry.update");

    // Mutations since id 2 — none.
    let since_last = storage.get_uat_mutations_since(2);
    assert!(since_last.is_empty());
}

#[test]
fn test_create_snapshot_writes_json_file() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());

    // Log a mutation before snapshot so watermark > 0.
    storage
        .log_uat_mutation(
            "entry.create",
            "template:test.md",
            None,
            Some("{}"),
            None,
            None,
        )
        .unwrap();

    let data_dir = dir.path().to_str().unwrap();
    let snap = co_web::uat_routes::create_snapshot(data_dir, &storage).unwrap();

    assert_eq!(snap.version, "v1");
    assert_eq!(snap.mutation_watermark, 1);
    assert!(snap.schema_version >= 19);

    // File must exist on disk.
    let snap_path = dir.path().join("uat-snapshots").join("v1.json");
    assert!(
        snap_path.exists(),
        "snapshot file should be written to disk"
    );

    // Second snapshot increments version.
    storage
        .log_uat_mutation(
            "entry.update",
            "template:test.md",
            Some("{}"),
            Some("{\"v\":2}"),
            None,
            None,
        )
        .unwrap();
    let snap2 = co_web::uat_routes::create_snapshot(data_dir, &storage).unwrap();
    assert_eq!(snap2.version, "v2");
    assert_eq!(snap2.mutation_watermark, 2);
}

// ---------------------------------------------------------------------------
// CO-45: HTTP integration tests
// ---------------------------------------------------------------------------

/// Helper: sign a JWT for test use and return the Authorization header value.
fn test_auth_header() -> String {
    // SAFETY: single-threaded test context; no concurrent env reads.
    unsafe {
        std::env::set_var("JWT_SECRET", "test-secret");
    }
    let (token, _) =
        co_web::auth::sign_jwt("user-test", "test@test.com", "admin", "test-secret").unwrap();
    format!("Bearer {token}")
}

#[tokio::test]
async fn test_uat_changes_returns_404_in_prod() {
    let dir = tempdir().unwrap();
    let app = build_app(dir.path(), false); // prod mode

    let auth = test_auth_header();
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/uat/changes")
        .header("authorization", auth)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_uat_changes_requires_auth() {
    let dir = tempdir().unwrap();
    let app = build_app(dir.path(), true); // UAT mode

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/uat/changes")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_uat_changes_returns_mutations_since_snapshot() {
    let dir = tempdir().unwrap();

    // Log a mutation and create a snapshot before building the app.
    {
        let storage = Storage::new(dir.path().to_str().unwrap());
        storage
            .log_uat_mutation(
                "entry.create",
                "template:projects/CO/1.md",
                None,
                Some("{\"title\":\"Task 1\"}"),
                None,
                None,
            )
            .unwrap();
        co_web::uat_routes::create_snapshot(dir.path().to_str().unwrap(), &storage).unwrap();

        // Log a second mutation after the snapshot (this is what we expect back).
        storage
            .log_uat_mutation(
                "entry.update",
                "template:projects/CO/1.md",
                Some("{\"title\":\"Task 1\"}"),
                Some("{\"title\":\"Task 1 Updated\"}"),
                None,
                None,
            )
            .unwrap();
    }

    let app = build_app(dir.path(), true);
    let auth = test_auth_header();

    // Changes since snapshot v1 — should return only the second mutation.
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/uat/changes?since=v1")
        .header("authorization", &auth)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(body["snapshot"], "v1");
    let mutations = body["mutations"].as_array().unwrap();
    assert_eq!(mutations.len(), 1);
    assert_eq!(mutations[0]["action"], "entry.update");

    let content_diff = &body["content_diff"];
    assert_eq!(content_diff["modified"].as_array().unwrap().len(), 1);
    assert!(content_diff["added"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_uat_export_patch_returns_404_in_prod() {
    let dir = tempdir().unwrap();
    let app = build_app(dir.path(), false); // prod mode

    let auth = test_auth_header();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/uat/export-patch")
        .header("authorization", auth)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_uat_export_patch_generates_tarball() {
    use flate2::read::GzDecoder;

    let dir = tempdir().unwrap();

    // Create a universe and entry file so the tarball can include it.
    {
        let universe_dir = dir.path().join("universes").join("template");
        std::fs::create_dir_all(universe_dir.join("projects/CO")).unwrap();
        std::fs::write(
            universe_dir.join("projects/CO/test.md"),
            "---\ntitle: Test Task\n---\nBody content\n",
        )
        .unwrap();

        let storage = Storage::new(dir.path().to_str().unwrap());
        // Snapshot first (watermark = 0).
        co_web::uat_routes::create_snapshot(dir.path().to_str().unwrap(), &storage).unwrap();
        // Then log the create mutation.
        storage
            .log_uat_mutation(
                "entry.create",
                "template:projects/CO/test.md",
                None,
                Some("{\"title\":\"Test Task\"}"),
                None,
                None,
            )
            .unwrap();
    }

    let app = build_app(dir.path(), true);
    let auth = test_auth_header();

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/uat/export-patch")
        .header("authorization", &auth)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("gzip") || content_type.contains("octet-stream"),
        "expected gzip content type, got: {content_type}"
    );

    // Decompress and inspect the tarball.
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let gz = GzDecoder::new(bytes.as_ref());
    let mut archive = tar::Archive::new(gz);

    let mut found_mutations = false;
    let mut found_entry = false;
    let mut found_readme = false;

    for file in archive.entries().unwrap() {
        let file = file.unwrap();
        let path = file.path().unwrap().to_string_lossy().to_string();
        if path == "mutations.json" {
            found_mutations = true;
        }
        if path.contains("projects/CO/test.md") {
            found_entry = true;
        }
        if path == "README.md" {
            found_readme = true;
        }
    }

    assert!(found_mutations, "tarball must contain mutations.json");
    assert!(found_entry, "tarball must contain the promoted entry file");
    assert!(found_readme, "tarball must contain README.md");
}
