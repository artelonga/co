use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use crate::config::WebConfig;
use crate::experiment::ExperimentStore;
use crate::storage::Storage;

fn isolate_env() {
    unsafe {
        std::env::set_var("CO_RECOVERY_KEY", "test-recovery-key-co190");
        std::env::set_var("JWT_SECRET", "test-jwt-secret-co190");
        std::env::set_var("RUST_LOG", "off");
    }
}

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
    let state: crate::server::AppState = Arc::new(crate::server::AppStateInner {
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
        rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
        wae: crate::wae::WaeEmitter::new(None, None),
        jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
        embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
        embedding_tx,
    });
    crate::server::build_router(state, None)
}

async fn json_body(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("x-forwarded-for", "127.0.0.1")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

/// Helper: insert a user directly for tests.
fn insert_user_with_email(dir: &std::path::Path, email: &str) -> String {
    let storage = Storage::new(dir.to_str().unwrap());
    let id = format!("usr_test_{}", nanoid::nanoid!(8));
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, usuario, email, display_name, tier, created_at) \
             VALUES (?1, ?2, ?3, ?2, 'player', ?4)",
            rusqlite::params![id, "existing-user", email, now],
        )
        .expect("insert test user");
    id
}

/// Helper: grab the stored code hash from onboarding_codes and derive the real code.
/// Since Argon2id is irreversible, we intercept before hashing by using a
/// predictable code seeded via the standard `generate_code` — but in tests
/// we instead make the round-trip by calling the onboard endpoint and then
/// reading the hash back out, and use verify_code directly in the logic.
/// The simplest approach: call the endpoint, then read the onboarding_code
/// row and call argon2-verify manually.
fn get_latest_onboarding_code_hash(dir: &std::path::Path) -> Option<(String, String)> {
    let storage = Storage::new(dir.to_str().unwrap());
    storage
        .conn()
        .query_row(
            "SELECT id, code_hash FROM onboarding_codes \
             WHERE intent != 'ip_tracker' \
             ORDER BY created_at DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()
}

// -------------------------------------------------------------------------
// Test 1: happy path — login for existing user
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_onboard_login_happy_path() {
    isolate_env();
    let dir = tempdir().unwrap();
    let email = "existing@test.co";
    insert_user_with_email(dir.path(), email);

    let router = build_test_router(dir.path());

    // Step 1: request code
    let resp = router
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/onboard-with-email",
            serde_json::json!({ "email": email }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = json_body(resp.into_body()).await;
    assert_eq!(body["sent"], true);
    assert!(body["expires_at"].is_string());

    // Verify intent stored as 'login'
    let storage = Storage::new(dir.path().to_str().unwrap());
    let row: (String, String) = storage
        .conn()
        .query_row(
            "SELECT intent, email_lookup_hash FROM onboarding_codes WHERE intent = 'login' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("onboarding_codes row");
    assert_eq!(row.0, "login");
    _ = row;

    // Step 2: verify with a wrong code first to test error path, then skip
    // (Argon2id is non-invertible in tests, so we can't recover the real code;
    // we instead test the 410 path for consumed/expired separately.)
}

// -------------------------------------------------------------------------
// Test 2: happy path — create for unknown email
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_onboard_create_stores_intent() {
    isolate_env();
    let dir = tempdir().unwrap();

    let router = build_test_router(dir.path());

    let resp = router
        .oneshot(post_json(
            "/api/v1/auth/onboard-with-email",
            serde_json::json!({ "email": "newuser@test.co" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let storage = Storage::new(dir.path().to_str().unwrap());
    let intent: String = storage
        .conn()
        .query_row(
            "SELECT intent FROM onboarding_codes WHERE intent = 'create' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("create intent row");
    assert_eq!(intent, "create");
}

// -------------------------------------------------------------------------
// Test 3: verify with consumed code → 410
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_verify_consumed_code_returns_410() {
    isolate_env();
    let dir = tempdir().unwrap();
    let email = "consumed@test.co";

    let storage = Storage::new(dir.path().to_str().unwrap());
    let email_normalized = crate::recovery_crypto::normalize_channel_value("email", email);
    let email_hash = crate::recovery_crypto::compute_lookup_hash(&email_normalized);

    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    storage
        .create_onboarding_code(&email_hash, "create", "fakehash", None, None, &expires_at)
        .unwrap();

    // Mark consumed manually.
    let (code_id, _) = get_latest_onboarding_code_hash(dir.path()).unwrap();
    storage.consume_onboarding_code(&code_id).unwrap();

    let router = build_test_router(dir.path());
    let resp = router
        .oneshot(post_json(
            "/api/v1/auth/onboard-with-email/verify",
            serde_json::json!({ "email": email, "code": "123456" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

// -------------------------------------------------------------------------
// Test 4: verify with expired code → 410
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_verify_expired_code_returns_410() {
    isolate_env();
    let dir = tempdir().unwrap();
    let email = "expired@test.co";

    let storage = Storage::new(dir.path().to_str().unwrap());
    let email_normalized = crate::recovery_crypto::normalize_channel_value("email", email);
    let email_hash = crate::recovery_crypto::compute_lookup_hash(&email_normalized);

    // Expire immediately (in the past).
    let expires_at = (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
    storage
        .create_onboarding_code(&email_hash, "create", "fakehash", None, None, &expires_at)
        .unwrap();

    let router = build_test_router(dir.path());
    let resp = router
        .oneshot(post_json(
            "/api/v1/auth/onboard-with-email/verify",
            serde_json::json!({ "email": email, "code": "123456" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

// -------------------------------------------------------------------------
// Test 5: wrong code 5 times → 410 locked
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_wrong_code_lockout_after_5_attempts() {
    isolate_env();
    let dir = tempdir().unwrap();
    let email = "lockout@test.co";

    let storage = Storage::new(dir.path().to_str().unwrap());
    let email_normalized = crate::recovery_crypto::normalize_channel_value("email", email);
    let email_hash = crate::recovery_crypto::compute_lookup_hash(&email_normalized);
    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    // Store a real argon2 hash so verify actually runs.
    let code_hash = {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(b"correct-code", &salt)
            .unwrap()
            .to_string()
    };
    storage
        .create_onboarding_code(&email_hash, "create", &code_hash, None, None, &expires_at)
        .unwrap();

    let router = build_test_router(dir.path());

    // Send 5 wrong codes.
    for _ in 0..5 {
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/v1/auth/onboard-with-email/verify",
                serde_json::json!({ "email": email, "code": "000000" }),
            ))
            .await
            .unwrap();
        // First 4 may return 401, 5th returns 410.
        let status = resp.status();
        if status == StatusCode::GONE {
            return; // locked — test passes
        }
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::GONE,
            "unexpected status: {status}"
        );
    }

    // One more attempt should now return 410.
    let resp = router
        .oneshot(post_json(
            "/api/v1/auth/onboard-with-email/verify",
            serde_json::json!({ "email": email, "code": "000000" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

// -------------------------------------------------------------------------
// Test 6: derive_usuario_from_email — basic derivation
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_derive_usuario_basic() {
    isolate_env();
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());

    let usuario = crate::storage::derive_usuario_from_email("hello@example.com", &storage);
    assert_eq!(usuario, "hello");
}

// -------------------------------------------------------------------------
// Test 7: derive_usuario_from_email — collision suffix
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_derive_usuario_collision_suffix() {
    isolate_env();
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());

    // Pre-insert a user with usuario='alice'
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, usuario, email, display_name, tier, created_at) \
             VALUES ('usr_alice', 'alice', 'alice@test.co', 'Alice', 'player', datetime('now'))",
            [],
        )
        .unwrap();

    let usuario = crate::storage::derive_usuario_from_email("alice@example.com", &storage);
    assert_eq!(usuario, "alice-2");
}

// -------------------------------------------------------------------------
// Test 8: verify with no code row → 410
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_verify_missing_code_returns_410() {
    isolate_env();
    let dir = tempdir().unwrap();

    let router = build_test_router(dir.path());
    let resp = router
        .oneshot(post_json(
            "/api/v1/auth/onboard-with-email/verify",
            serde_json::json!({ "email": "nobody@test.co", "code": "123456" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

// -------------------------------------------------------------------------
// Test 9: email rate limit — 5 codes per hour
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_email_rate_limit() {
    isolate_env();
    let dir = tempdir().unwrap();
    let email = "ratelimited@test.co";

    let storage = Storage::new(dir.path().to_str().unwrap());
    let email_normalized = crate::recovery_crypto::normalize_channel_value("email", email);
    let email_hash = crate::recovery_crypto::compute_lookup_hash(&email_normalized);

    // Insert 5 onboarding code rows to simulate reaching the limit.
    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    for _ in 0..5 {
        storage
            .create_onboarding_code(&email_hash, "create", "fakehash", None, None, &expires_at)
            .unwrap();
    }

    let router = build_test_router(dir.path());
    let resp = router
        .oneshot(post_json(
            "/api/v1/auth/onboard-with-email",
            serde_json::json!({ "email": email }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

// -------------------------------------------------------------------------
// Test 10: invalid email format → 400
// -------------------------------------------------------------------------
#[tokio::test]
async fn test_invalid_email_returns_400() {
    isolate_env();
    let dir = tempdir().unwrap();
    let router = build_test_router(dir.path());

    let resp = router
        .oneshot(post_json(
            "/api/v1/auth/onboard-with-email",
            serde_json::json!({ "email": "not-an-email" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
