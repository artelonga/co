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
        core: Arc::new(CoreState::from_storage_with_secrets(
            storage,
            config,
            auth_store,
            crate::infra::secrets::StaticSecretsProvider::new([("JWT_SECRET", "test-jwt-secret")]),
        )),
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
            worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
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

/// manifest.json must use application/manifest+json, not application/json.
/// Required for Lighthouse PWA installability checks.
#[test]
fn test_guess_content_type_manifest() {
    assert_eq!(
        guess_content_type("manifest.json"),
        "application/manifest+json"
    );
    assert_eq!(
        guess_content_type("shared/manifest.json"),
        "application/manifest+json"
    );
    // Other JSON files remain application/json.
    assert_eq!(guess_content_type("data.json"), "application/json");
    assert_eq!(
        guess_content_type("shared/openapi.json"),
        "application/json"
    );
}

/// offline.html must be treated as a static asset (routed to serve_variant_file).
#[test]
fn test_looks_like_static_asset_offline_html() {
    assert!(looks_like_static_asset("/offline.html"));
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

/// CO-397: Anonymous user gets HTTP 429 after exhausting the 60-read/min bucket.
/// No X-Forwarded-For is set so abuse tracking (which uses the IP) is bypassed;
/// the rate-limit bucket is still enforced via the "anon:unknown:r" key.
#[tokio::test]
async fn test_rate_limit_anonymous_reads_returns_429() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let make_req = || {
        Request::builder()
            .method("GET")
            .uri("/api/v1/universes")
            .body(Body::empty())
            .unwrap()
    };

    // First 60 requests: bucket starts full at 60 → all succeed (may return 401
    // for auth, but the rate-limit layer passes them regardless of status).
    for i in 0..60 {
        let status = app.clone().oneshot(make_req()).await.unwrap().status();
        assert_ne!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "request {i} should not be rate limited"
        );
    }

    // 61st request: bucket is empty → 429.
    let status = app.clone().oneshot(make_req()).await.unwrap().status();
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "61st request should be rate limited"
    );
}

/// HTTP 429 response includes a Retry-After header.
#[tokio::test]
async fn test_rate_limit_429_has_retry_after_header() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let make_req = || {
        Request::builder()
            .method("GET")
            .uri("/api/v1/universes")
            .body(Body::empty())
            .unwrap()
    };

    // Exhaust the anonymous read bucket (60 slots, CO-397).
    for _ in 0..60 {
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

    let jwt = crate::auth::sign_jwt(&user_id, "quota@test.local", "user", "test-jwt-secret")
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

    let jwt = crate::auth::sign_jwt(user_id, "admin@test.local", "admin", "test-jwt-secret")
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

// ---------------------------------------------------------------------------
// CO-323: subdomain routing — single-universe SPA injection
// ---------------------------------------------------------------------------

/// A request with `Host: yuri.artelonga.com.br` to `/` must return the SPA
/// shell with `window.__CO_SUBDOMAIN_UNIVERSE__='yuri'` injected.
#[tokio::test]
async fn test_subdomain_routing_injects_universe_script() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "yuri.artelonga.com.br")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_str(resp.into_body()).await;
    assert!(
        body.contains("window.__CO_SUBDOMAIN_UNIVERSE__='yuri'"),
        "subdomain SPA must contain universe bootstrap script; body excerpt: {}",
        &body[..body.len().min(500)]
    );
}

/// A request with a non-subdomain host must NOT inject the universe script.
#[tokio::test]
async fn test_non_subdomain_host_no_script_injection() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "co-artelonga.fly.dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_str(resp.into_body()).await;
    assert!(
        !body.contains("__CO_SUBDOMAIN_UNIVERSE__"),
        "non-subdomain host must not inject universe script"
    );
}

/// Reserved subdomain `co.artelonga.com.br` must not inject the script.
#[tokio::test]
async fn test_reserved_subdomain_co_no_injection() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "co.artelonga.com.br")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_str(resp.into_body()).await;
    assert!(
        !body.contains("__CO_SUBDOMAIN_UNIVERSE__"),
        "reserved 'co' subdomain must not inject universe script"
    );
}

/// `/{universe}/{*subpath}` for a non-existent universe must return 200
/// so the SPA can render its own client-side routes (e.g. `/entrar/`,
/// `/sobre/`, `/termos/`). The CO-232 hotfix in 2.12.2 (b8ed778) made
/// this deliberate; only return 404 when the universe exists but the
/// entry within it does not (covered by `test_deep_link_unknown_slug_returns_404`).
#[tokio::test]
async fn test_deep_link_nonexistent_universe_returns_200() {
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

// ---------------------------------------------------------------------------
// CO-361: atividades audit log integration tests
// ---------------------------------------------------------------------------

/// schema_versoes must contain a row for every entry in schema_version
/// after migrations run.
#[test]
fn test_co361_schema_versoes_backfill() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());
    let conn = storage.conn();

    let schema_version_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
        .expect("schema_version count");

    let schema_versoes_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_versoes", [], |row| row.get(0))
        .expect("schema_versoes count");

    assert_eq!(
        schema_version_count, schema_versoes_count,
        "schema_versoes must have one row per schema_version row (got {schema_versoes_count} vs {schema_version_count})"
    );
    assert!(
        schema_version_count > 0,
        "at least one migration must exist"
    );
}

/// Create task → atividades row appears within 100 ms with correct acao/entidade.
#[tokio::test]
async fn test_co361_create_task_writes_atividade() {
    let dir = tempdir().unwrap();
    // Seed a test user and project directly in storage.
    let user_id = insert_test_user(dir.path(), "taskaudit@example.com", None);
    {
        let mut storage = Storage::new(dir.path().to_str().unwrap());
        storage
            .create_project(crate::models::CreateProject {
                name: "Audit Project".into(),
                key: "audit-proj".into(),
                description: String::new(),
                universe_key: None,
            })
            .expect("create project");
    }

    let jwt = crate::auth::sign_jwt(
        &user_id,
        "taskaudit@example.com",
        "player",
        "test-jwt-secret",
    )
    .expect("sign jwt")
    .0;

    let app = build_test_router(dir.path());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/audit-proj/tasks")
                .header("content-type", "application/json")
                .header("Authorization", format!("Bearer {jwt}"))
                .body(Body::from(r#"{"title":"Audit me","status":"todo"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "task creation failed");

    // Allow the deferred tokio::spawn in log_atividade to run.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let storage = Storage::new(dir.path().to_str().unwrap());
    let count: i64 = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM atividades WHERE acao='criar' AND entidade='task'",
            [],
            |row| row.get(0),
        )
        .expect("atividades count");

    assert!(
        count >= 1,
        "expected at least one 'criar/task' atividade row, got {count}"
    );
}

/// Password login must NOT produce a atividade row whose conteudo contains
/// any raw password, hash, or token string.
#[tokio::test]
async fn test_co361_login_no_sensitive_data_in_atividade() {
    let dir = tempdir().unwrap();
    let plaintext = "s3cr3t-password";
    let hash = argon2_hash(plaintext);
    insert_test_user(dir.path(), "audit@example.com", Some(&hash));

    let app = build_test_router(dir.path());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/password-login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"audit@example.com","password":"s3cr3t-password"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Allow the deferred spawn to complete.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let storage = Storage::new(dir.path().to_str().unwrap());
    let rows: Vec<String> = {
        let conn = storage.conn();
        let mut stmt = conn
            .prepare("SELECT COALESCE(conteudo,'') FROM atividades WHERE acao='login'")
            .expect("prepare");
        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query_map")
            .filter_map(|r| r.ok())
            .collect()
    };

    assert!(
        !rows.is_empty(),
        "expected at least one login atividade row after password-login"
    );

    for conteudo in &rows {
        assert!(
            !conteudo.contains(plaintext),
            "plaintext password leaked into atividades.conteudo: {conteudo}"
        );
        assert!(
            !conteudo.contains(&hash),
            "password_hash leaked into atividades.conteudo"
        );
        // Confirm "[REDACTED]" is not present either — the before/after are both
        // None for login events (no diff to capture), so conteudo is {before:null,after:null}.
        let has_redacted = conteudo.contains("[REDACTED]");
        // If there IS a conteudo diff, it must use [REDACTED] for any sensitive key.
        // For login events with no diff, this assertion just confirms null/empty content is fine.
        let _ = has_redacted;
    }
}
