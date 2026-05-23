use super::*;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use crate::config::WebConfig;
use crate::experiment::ExperimentStore;
use crate::storage::Storage;

fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 3000,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: false,
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
    let state: AppState = AppState::new(AppStateInner {
        core: Arc::new(CoreState {
            storage: parking_lot::Mutex::new(storage),
            config,
            auth_store: Mutex::new(auth_store),
            event_bus: crate::events::Bus::new(),
        }),
        realtime: Arc::new(RealtimeState {
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: crate::cache::CacheLayer::new(),
            embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
            rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
        }),
    });
    build_router(state, None)
}

fn argon2_hash(password: &str) -> String {
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("hash failed")
        .to_string()
}

fn insert_test_user(dir: &std::path::Path, email: &str, password_hash: Option<&str>) -> String {
    let storage = Storage::new(dir.to_str().unwrap());
    let id = format!(
        "usr_test_{}",
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
    );
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, password_hash) \
             VALUES (?1, ?2, 'Test', 'player', ?3, ?4)",
            rusqlite::params![id, email, now, password_hash],
        )
        .expect("insert test user");
    id
}

async fn body_str(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// --- password-login endpoint tests ---

#[tokio::test]
async fn test_password_login_valid_creds() {
    let dir = tempdir().unwrap();
    let hash = argon2_hash("correctpassword");
    insert_test_user(dir.path(), "alice@example.com", Some(&hash));
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/password-login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"alice@example.com","password":"correctpassword"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_str(resp.into_body()).await;
    assert!(body.contains("alice@example.com"), "body: {body}");
}

#[tokio::test]
async fn test_password_login_wrong_password() {
    let dir = tempdir().unwrap();
    let hash = argon2_hash("correctpassword");
    insert_test_user(dir.path(), "bob@example.com", Some(&hash));
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/password-login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"bob@example.com","password":"wrongpassword"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_password_login_no_password_hash() {
    let dir = tempdir().unwrap();
    insert_test_user(dir.path(), "carol@example.com", None);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/password-login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"carol@example.com","password":"anypassword"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// `/{slug}` must NOT swallow static asset requests. Regression for the
/// 1.43.0 URL refactor where `/style.css` returned the SPA HTML instead
/// of CSS, breaking layout entirely.
#[test]
fn test_looks_like_static_asset_recognizes_filenames_and_prefixes() {
    // Top-level filenames
    assert!(looks_like_static_asset("/style.css"));
    assert!(looks_like_static_asset("/app.js"));
    assert!(looks_like_static_asset("/sw.js"));
    assert!(looks_like_static_asset("/manifest.json"));
    assert!(looks_like_static_asset("/icon.png"));
    // Asset prefixes
    assert!(looks_like_static_asset("/shared/production.css"));
    assert!(looks_like_static_asset("/variants/a/style.css"));
    assert!(looks_like_static_asset("/pdfjs/web/viewer.html"));
    // Universe slugs (no extension) — must be SPA
    assert!(!looks_like_static_asset("/"));
    assert!(!looks_like_static_asset("/co"));
    assert!(!looks_like_static_asset("/mbya"));
    assert!(!looks_like_static_asset("/mbya/refs/foo"));
    assert!(!looks_like_static_asset("/co/telemetria"));
}

// --- seed_admin_user_from_env drift detection tests ---

#[test]
fn test_seed_admin_user_from_env_inserts_new_user() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    let hash = argon2_hash("adminpass");

    storage
        .seed_admin_user_from_env("admin@example.com", &hash)
        .unwrap();

    let (_, stored_hash) = storage
        .get_user_by_email_with_hash("admin@example.com")
        .expect("user should exist");
    assert_eq!(stored_hash.as_deref(), Some(hash.as_str()));
}

#[test]
fn test_seed_admin_user_from_env_same_hash_no_op() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    let hash = argon2_hash("adminpass");

    storage
        .seed_admin_user_from_env("admin@example.com", &hash)
        .unwrap();
    // Second call with same hash — should be a no-op (no error, no change)
    storage
        .seed_admin_user_from_env("admin@example.com", &hash)
        .unwrap();

    let count: i64 = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM users WHERE email = 'admin@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "should not duplicate user on no-op");
}

#[test]
fn test_seed_admin_user_from_env_hash_drift_updates() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    let hash1 = argon2_hash("oldpass");
    let hash2 = argon2_hash("newpass");

    storage
        .seed_admin_user_from_env("admin@example.com", &hash1)
        .unwrap();
    storage
        .seed_admin_user_from_env("admin@example.com", &hash2)
        .unwrap();

    let (_, stored_hash) = storage
        .get_user_by_email_with_hash("admin@example.com")
        .expect("user should exist");
    assert_eq!(
        stored_hash.as_deref(),
        Some(hash2.as_str()),
        "hash should be updated when drift detected"
    );
}

// --- CO-80: rate limiting + quota integration tests ---

/// Anonymous user gets HTTP 429 after exhausting the anonymous read bucket
/// (bumped from 20 to 120 reads/min in 2.7.21 for public-content traffic).
#[tokio::test]
async fn test_rate_limit_anonymous_reads_returns_429() {
    // SAFETY: single-threaded test setup, no concurrent set_var calls.
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let make_req = || {
        Request::builder()
            .method("GET")
            .uri("/api/v1/universes/template")
            .body(Body::empty())
            .unwrap()
    };

    // First 120 requests: rate limiter allows them (bucket starts full at 120).
    for i in 0..120 {
        let status = app.clone().oneshot(make_req()).await.unwrap().status();
        assert_ne!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "request {i} should not be rate limited"
        );
    }

    // 121st request: bucket is empty → 429.
    let status = app.clone().oneshot(make_req()).await.unwrap().status();
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "121st request should be rate limited"
    );
}

/// HTTP 429 response includes a Retry-After header.
#[tokio::test]
async fn test_rate_limit_429_has_retry_after_header() {
    // SAFETY: single-threaded test setup, no concurrent set_var calls.
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let make_req = || {
        Request::builder()
            .method("GET")
            .uri("/api/v1/universes/template")
            .body(Body::empty())
            .unwrap()
    };

    // Exhaust the anonymous read bucket (120 slots).
    for _ in 0..120 {
        let _ = app.clone().oneshot(make_req()).await.unwrap();
    }

    let resp = app.clone().oneshot(make_req()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        resp.headers().contains_key("retry-after"),
        "429 response must include Retry-After header"
    );
}

/// Admin user authenticating via long-lived API token (CO-35) gets the
/// admin tier (unlimited), not the Anonymous-by-IP fallback. Pre-fix the
/// rate-limit middleware decoded only JWTs, so admin API tokens hit the
/// 20-reads/min anonymous bucket — breaking multi-watcher background sync.
#[tokio::test]
async fn test_rate_limit_admin_api_token_resolves_to_admin_tier() {
    // SAFETY: single-threaded test setup, no concurrent set_var calls.
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let api_token = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let user_id = format!(
            "usr_admin_{}",
            &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
        );
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at) \
                     VALUES (?1, 'admin@test.local', 'Admin Test', 'admin', ?2)",
                rusqlite::params![user_id, now],
            )
            .unwrap();
        storage
            .create_api_token(&user_id, "test-rate-limit")
            .unwrap()
            .token
            .unwrap_or_default()
    };

    // 25 reads is past the 20/min anonymous read budget — none should 429.
    for i in 0..25 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/template")
                    .header("Authorization", format!("Bearer {}", api_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "request {i}: admin API token must not be rate-limited"
        );
    }
}

/// 1.45.0: tier collapse removes the storage quota for authenticated
/// users — every authed user is admin, with unlimited storage. The test
/// now verifies the inverse: an authenticated user is NOT 402'd even
/// past the legacy 10k-entry cap. Anonymous quotas (100-entry cap) live
/// on a different code path and remain enforced.
#[tokio::test]
async fn test_authed_user_storage_unlimited_post_tier_collapse() {
    // SAFETY: single-threaded test setup, no concurrent set_var calls.
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let user_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let id = format!(
            "usr_quota_{}",
            &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
        );
        let now = chrono::Utc::now().to_rfc3339();
        // tier='user' is a legacy stored value; Tier::parse maps it to
        // Admin under the 1.45.0 model.
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at) \
                     VALUES (?1, 'quota@test.local', 'Quota Test', 'user', ?2)",
                rusqlite::params![id, now],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universes \
                     (key, name, description, owner_id, created_at, is_template, is_public, \
                      content_count, visibility) \
                     VALUES ('quota-u', 'Quota Universe', '', ?1, ?2, 0, 0, 10001, 'private')",
                rusqlite::params![id, now],
            )
            .unwrap();
        id
    };

    let jwt = crate::auth::sign_jwt(&user_id, "quota@test.local", "user", "test-secret")
        .unwrap()
        .0;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/quota-u/entries")
                .header("Authorization", format!("Bearer {}", jwt))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"path":"test.md","frontmatter":{"type":"page","title":"T"},"body":""}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        resp.status(),
        StatusCode::PAYMENT_REQUIRED,
        "authenticated user must NOT hit storage quota under 1.45.0 collapse"
    );
}

/// Admin user with X-Admin-Override-Quota bypasses quota check (audit logged).
#[tokio::test]
async fn test_admin_override_quota_bypasses_universe_quota() {
    // SAFETY: single-threaded test setup, no concurrent set_var calls.
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // Create an admin user who already has 1 universe (the "user" tier limit is 10,
    // but admin has no limit — this tests the override header is accepted for admin).
    let user_id = "usr_admin_override";
    {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at) \
                     VALUES (?1, 'admin@test.local', 'Admin', 'admin', ?2)",
                rusqlite::params![user_id, now],
            )
            .unwrap();
    }

    let jwt = crate::auth::sign_jwt(user_id, "admin@test.local", "admin", "test-secret")
        .unwrap()
        .0;

    // Admin creating a universe with X-Admin-Override-Quota: true must not get 402.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes")
                .header("Authorization", format!("Bearer {}", jwt))
                .header("X-Admin-Override-Quota", "true")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"key":"admin-override-test","name":"Admin Override Test"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        resp.status(),
        StatusCode::PAYMENT_REQUIRED,
        "admin with override header must not get 402, got: {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// CO-177: CORS + universe_key for artelonga.com.br analytics events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_telemetry_events_preflight_artelonga() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/telemetry/events")
                .header("origin", "https://artelonga.com.br")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "content-type")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.status() == StatusCode::OK || resp.status() == StatusCode::NO_CONTENT,
        "preflight must return 200 or 204, got: {}",
        resp.status()
    );
    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        acao, "https://artelonga.com.br",
        "ACAO header must echo artelonga.com.br origin"
    );
}

#[tokio::test]
async fn test_telemetry_events_post_populates_universe_key() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let batch = serde_json::json!({
        "schema": 1,
        "batch": [{
            "s": 1,
            "site": "artelonga",
            "name": "page_view",
            "sid": "sess-001",
            "vid": "vis-001",
            "path": "/blog/post"
        }]
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/telemetry/events")
                .header("content-type", "application/json")
                .header("origin", "https://artelonga.com.br")
                .body(Body::from(batch.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "POST /api/v1/telemetry/events must return 204"
    );

    // Give the spawned task time to write before we check the DB.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let storage = crate::storage::Storage::new(dir.path().to_str().unwrap());
    let universe_key: Option<String> = storage
        .conn()
        .query_row(
            "SELECT universe_key FROM telemetry_events WHERE event_name = 'page_view' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(None);
    assert_eq!(
        universe_key.as_deref(),
        Some("artelonga"),
        "universe_key must be 'artelonga' from site field"
    );
}

#[tokio::test]
async fn test_telemetry_admin_requires_auth_no_cors_bypass() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // Admin endpoint must be blocked by auth even with artelonga.com.br origin.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/telemetry/summary")
                .header("origin", "https://artelonga.com.br")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
        "admin telemetry must reject unauthenticated request, got: {}",
        resp.status()
    );
}

// --- CO-232: deep-link 404 ---

fn setup_universe_with_entry(dir: &std::path::Path, universe_slug: &str, entry_path: &str) {
    let mut storage = Storage::new(dir.to_str().unwrap());

    // Insert owner user.
    let owner_id = "usr_test_owner";
    let now = chrono::Utc::now().to_rfc3339();
    let _ = storage.conn().execute(
        "INSERT OR IGNORE INTO users (id, email, display_name, tier, created_at) \
         VALUES (?1, ?2, 'Owner', 'player', ?3)",
        rusqlite::params![owner_id, "owner@test.local", now],
    );

    let _ = storage.create_universe(
        crate::models::CreateUniverse {
            key: universe_slug.to_string(),
            name: universe_slug.to_string(),
            description: String::new(),
        },
        owner_id,
    );

    // Open the per-universe DB and upsert the test entry.
    let uc = storage.universe_conn(universe_slug);
    let guard = uc.lock().unwrap();
    let index = crate::entry_index::EntryIndex::new(&guard);
    let entry = crate::entry_index::make_entry(
        entry_path,
        serde_json::json!({"type": "note", "title": "Test entry"}),
        "Test body",
    );
    index.upsert(universe_slug, &entry).unwrap();
}

/// `/{universe}/{unknown-slug}` must return 404.
#[tokio::test]
async fn test_deep_link_unknown_slug_returns_404() {
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    setup_universe_with_entry(dir.path(), "testuniv", "content/existing-page.md");
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/testuniv/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "unknown slug must return 404"
    );
}

/// `/{universe}/{known-slug}` must return 200 and the SPA shell.
#[tokio::test]
async fn test_deep_link_known_slug_returns_200() {
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    setup_universe_with_entry(dir.path(), "testuniv2", "content/existing-page.md");
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/testuniv2/content/existing-page")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "known slug must return 200");
}

// --- CO-270: public-subscribable visibility gate fix ---

/// CO-270: anonymous GET on a public-subscribable universe must return
/// total > 0 AND items matching the requested limit.
///
/// Before the fix, universe_visibility_gate checked only `is_public ||
/// is_template`, so public-subscribable universes (is_public=false,
/// visibility='public-subscribable') returned 401 to anonymous callers.
#[tokio::test]
async fn test_anon_list_entries_public_subscribable_universe() {
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();

    let slug = "pub-sub-test";
    {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();

        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, is_template, is_public, \
                  content_count, visibility) \
                 VALUES (?1, 'Pub-Sub Test', '', 'system', ?2, 0, 0, 5, 'public-subscribable')",
                rusqlite::params![slug, now],
            )
            .unwrap();

        let uc = storage.universe_conn(slug);
        let guard = uc.lock().unwrap();
        let index = crate::entry_index::EntryIndex::new(&guard);
        for i in 1..=5_u32 {
            let entry = crate::entry_index::make_entry(
                &format!("tasks/{i}.md"),
                serde_json::json!({"type": "task", "title": format!("Task {i}")}),
                &format!("body {i}"),
            );
            index.upsert(slug, &entry).unwrap();
        }
    }

    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/universes/{slug}/entries?limit=3"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "anonymous GET on public-subscribable universe must return 200"
    );

    let body = body_str(resp.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("response is not JSON: {e}\nbody: {body}"));

    let total = json["total"].as_u64().unwrap_or(0);
    let items = json["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("'entries' must be an array; body: {body}"));

    assert!(
        total > 0,
        "total must be > 0 for public-subscribable universe; got {total}\nbody: {body}"
    );
    assert_eq!(
        items.len(),
        3,
        "limit=3 must yield exactly 3 items; got {}\nbody: {body}",
        items.len()
    );
}

/// CO-270: private universes must still block anonymous reads after the fix.
#[tokio::test]
async fn test_anon_list_entries_private_universe_blocked() {
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();

    let slug = "private-test";
    {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, is_template, is_public, \
                  content_count, visibility) \
                 VALUES (?1, 'Private Test', '', 'system', ?2, 0, 0, 0, 'private')",
                rusqlite::params![slug, now],
            )
            .unwrap();
    }

    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/universes/{slug}/entries"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "anonymous GET on private universe must return 401"
    );
}

/// `/{universe}/{*subpath}` for a non-existent universe must return 200
/// so the SPA can render its own client-side routes (e.g. `/entrar/`,
/// `/sobre/`, `/termos/`). The CO-232 hotfix in 2.12.2 (b8ed778) made
/// this deliberate; only return 404 when the universe exists but the
/// entry within it does not (covered by `test_deep_link_unknown_slug_returns_404`).
#[tokio::test]
async fn test_deep_link_nonexistent_universe_returns_200() {
    unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ghost-universe/some-entry")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "non-existent universe deep-link must return 200 (SPA route fallback per CO-232 2.12.2)"
    );
}
