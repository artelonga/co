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

// Set a deterministic env for crypto ops in tests.
fn isolate_env() {
    unsafe {
        std::env::set_var("CO_RECOVERY_KEY", "test-recovery-key-for-tests");
        std::env::set_var("JWT_SECRET", "test-jwt-secret");
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
    let state: AppState = Arc::new(crate::server::AppStateInner {
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

fn argon2_hash(password: &str) -> String {
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("hash failed")
        .to_string()
}

/// Insert a test user and return their ID.
fn insert_test_user(dir: &std::path::Path, email: &str, password: Option<&str>) -> String {
    let storage = Storage::new(dir.to_str().unwrap());
    let id = format!(
        "usr_test_{}",
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
    );
    let now = chrono::Utc::now().to_rfc3339();
    let hash = password.map(argon2_hash);
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, password_hash) \
                 VALUES (?1, ?2, 'Test', 'player', ?3, ?4)",
            rusqlite::params![id, email, now, hash.as_deref()],
        )
        .expect("insert test user");
    id
}

/// Build a JWT for a test user.
fn make_jwt(user_id: &str) -> String {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (token, _) =
        crate::auth::sign_jwt(user_id, "test@example.com", "player", "test-jwt-secret").unwrap();
    token
}

async fn body_str(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// --- 1. add_channel_email_unauthenticated → 401 ---

#[tokio::test]
async fn test_add_channel_email_unauthenticated() {
    isolate_env();
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/recovery/channels")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"channel_type":"email","value":"test@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- 2. add_channel_email_authenticated → 201 ---

#[tokio::test]
async fn test_add_channel_email_authenticated() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user2@example.com", Some("pass"));
    let token = make_jwt(&user_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/recovery/channels")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    r#"{"channel_type":"email","value":"user2@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "body: {}",
        body_str(resp.into_body()).await
    );
}

// --- 3. verify_channel_correct_code → 200 ---

#[tokio::test]
async fn test_verify_channel_correct_code() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user3@example.com", Some("pass"));
    let token = make_jwt(&user_id);

    // Insert channel + verification row directly (avoid argon2 overhead in setup).
    let code = "123456";
    let code_hash = hash_code(code).unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());
    let channel_id = storage
        .create_recovery_channel(
            &user_id,
            "email",
            b"user3@example.com".to_vec(),
            [0u8; 12],
            "deadbeef",
        )
        .unwrap();
    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    storage
        .create_recovery_verification(
            &channel_id,
            &user_id,
            "add_channel",
            &code_hash,
            &expires_at,
        )
        .unwrap();

    let app = build_test_router(dir.path());
    let body = format!(r#"{{"channel_id":"{channel_id}","code":"{code}"}}"#);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/recovery/channels/verify")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_str(resp.into_body()).await;
    assert!(body.contains("true"), "body: {body}");
}

// --- 4. verify_channel_wrong_code → 401 ---

#[tokio::test]
async fn test_verify_channel_wrong_code() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user4@example.com", Some("pass"));
    let token = make_jwt(&user_id);

    let code_hash = hash_code("correct").unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());
    let channel_id = storage
        .create_recovery_channel(
            &user_id,
            "email",
            b"user4@example.com".to_vec(),
            [0u8; 12],
            "deadbeef4",
        )
        .unwrap();
    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    storage
        .create_recovery_verification(
            &channel_id,
            &user_id,
            "add_channel",
            &code_hash,
            &expires_at,
        )
        .unwrap();

    let app = build_test_router(dir.path());
    let body = format!(r#"{{"channel_id":"{channel_id}","code":"wrong!"}}"#);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/recovery/channels/verify")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- 5. verify_channel_expired_code → 410 ---

#[tokio::test]
async fn test_verify_channel_expired_code() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user5@example.com", Some("pass"));
    let token = make_jwt(&user_id);

    let code_hash = hash_code("123456").unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());
    let channel_id = storage
        .create_recovery_channel(
            &user_id,
            "email",
            b"user5@example.com".to_vec(),
            [0u8; 12],
            "deadbeef5",
        )
        .unwrap();
    // Expired: in the past.
    let expires_at = "2000-01-01T00:00:00Z";
    storage
        .create_recovery_verification(&channel_id, &user_id, "add_channel", &code_hash, expires_at)
        .unwrap();

    let app = build_test_router(dir.path());
    let body = format!(r#"{{"channel_id":"{channel_id}","code":"123456"}}"#);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/recovery/channels/verify")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::GONE);
}

// --- 6. list_channels_empty → 200 [] ---

#[tokio::test]
async fn test_list_channels_empty() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user6@example.com", None);
    let token = make_jwt(&user_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/recovery/channels")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_str(resp.into_body()).await;
    assert_eq!(body.trim(), "[]");
}

// --- 7. list_channels_after_add_verify ---

#[tokio::test]
async fn test_list_channels_after_add_verify() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user7@example.com", Some("pass"));
    let token = make_jwt(&user_id);

    // Insert and verify a channel directly.
    let storage = Storage::new(dir.path().to_str().unwrap());
    let (ct, nonce) = crate::recovery_crypto::encrypt_channel_value(b"user7@example.com").unwrap();
    let lhash = crate::recovery_crypto::compute_lookup_hash("user7@example.com");
    let channel_id = storage
        .create_recovery_channel(&user_id, "email", ct, nonce, &lhash)
        .unwrap();
    let verified_at = chrono::Utc::now().to_rfc3339();
    storage
        .verify_recovery_channel(&channel_id, &verified_at)
        .unwrap();

    let app = build_test_router(dir.path());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/recovery/channels")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_str(resp.into_body()).await;
    assert!(body.contains("email"), "body: {body}");
    assert!(body.contains("***"), "should be masked: {body}");
}

// --- 8. forgot_password_always_202 ---

#[tokio::test]
async fn test_forgot_password_always_202() {
    isolate_env();
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // Unknown identifier — should still be 202.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/forgot-password")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"username_or_channel_value":"nobody@unknown.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = body_str(resp.into_body()).await;
    assert!(body.contains("sent_to"), "body: {body}");
}

// --- 9. forgot_password_verify_wrong_code → 401 ---

#[tokio::test]
async fn test_forgot_password_verify_wrong_code() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user9@example.com", Some("pass"));

    // Set up a verified channel and a reset_password verification.
    let storage = Storage::new(dir.path().to_str().unwrap());
    let (ct, nonce) = crate::recovery_crypto::encrypt_channel_value(b"user9@example.com").unwrap();
    let lhash = crate::recovery_crypto::compute_lookup_hash("user9@example.com");
    let channel_id = storage
        .create_recovery_channel(&user_id, "email", ct, nonce, &lhash)
        .unwrap();
    let verified_at = chrono::Utc::now().to_rfc3339();
    storage
        .verify_recovery_channel(&channel_id, &verified_at)
        .unwrap();

    let code_hash = hash_code("654321").unwrap();
    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();
    storage
        .create_recovery_verification(
            &channel_id,
            &user_id,
            "reset_password",
            &code_hash,
            &expires_at,
        )
        .unwrap();

    // Insert the user with a known `usuario` so lookup works.
    storage
        .conn()
        .execute(
            "UPDATE users SET usuario = 'user9' WHERE id = ?1",
            rusqlite::params![user_id],
        )
        .unwrap();

    let app = build_test_router(dir.path());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/forgot-password/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"username_or_channel_value":"user9","code":"wrong!"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- 10. reset_password_with_valid_token → 200 + cookie ---

#[tokio::test]
async fn test_reset_password_with_valid_token() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user10@example.com", Some("oldpass"));

    // Create a reset token directly.
    let raw_token = "test-reset-token-abc123";
    let token_hash = sha256_hex(raw_token);
    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();
    let storage = Storage::new(dir.path().to_str().unwrap());
    let (ct, nonce) = crate::recovery_crypto::encrypt_channel_value(b"user10@example.com").unwrap();
    let lhash = crate::recovery_crypto::compute_lookup_hash("user10@example.com");
    let channel_id = storage
        .create_recovery_channel(&user_id, "email", ct, nonce, &lhash)
        .unwrap();
    storage
        .create_reset_token(&token_hash, &user_id, &channel_id, &expires_at)
        .unwrap();

    let app = build_test_router(dir.path());
    let body = serde_json::json!({
        "reset_token": raw_token,
        "new_password": "newSecurePassword!"
    })
    .to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/reset-password")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    // Should set a session cookie.
    let headers = resp.headers().clone();
    let cookie = headers.get("set-cookie").and_then(|v| v.to_str().ok());
    assert!(
        cookie.map(|c| c.contains("session=")).unwrap_or(false),
        "expected session cookie"
    );
}

// --- 11. reset_password_expired_token → 401 ---

#[tokio::test]
async fn test_reset_password_expired_token() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user11@example.com", Some("pass"));

    let raw_token = "expired-token";
    let token_hash = sha256_hex(raw_token);
    let expires_at = "2000-01-01T00:00:00Z"; // Past.
    let storage = Storage::new(dir.path().to_str().unwrap());
    let (ct, nonce) = crate::recovery_crypto::encrypt_channel_value(b"user11@example.com").unwrap();
    let lhash = crate::recovery_crypto::compute_lookup_hash("user11@example.com");
    let channel_id = storage
        .create_recovery_channel(&user_id, "email", ct, nonce, &lhash)
        .unwrap();
    storage
        .create_reset_token(&token_hash, &user_id, &channel_id, expires_at)
        .unwrap();

    let app = build_test_router(dir.path());
    let body = serde_json::json!({
        "reset_token": raw_token,
        "new_password": "anything"
    })
    .to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/reset-password")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- 12. change_password_correct → 200 ---

#[tokio::test]
async fn test_change_password_correct() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user12@example.com", Some("oldpass"));
    let token = make_jwt(&user_id);
    let app = build_test_router(dir.path());

    let body = r#"{"current_password":"oldpass","new_password":"newpass123"}"#;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/change-password")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// --- 13. change_password_wrong_current → 401 ---

#[tokio::test]
async fn test_change_password_wrong_current() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user13@example.com", Some("correctpass"));
    let token = make_jwt(&user_id);
    let app = build_test_router(dir.path());

    let body = r#"{"current_password":"wrongpass","new_password":"newpass"}"#;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/change-password")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- 14. rate_limit_add_channel_5_per_hour → 429 on 6th request ---
//
// This test inserts 5 existing verifications directly, then attempts
// a 6th via the API to trigger the rate limit.

#[tokio::test]
async fn test_rate_limit_add_channel_5_per_hour() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user14@example.com", Some("pass"));
    let token = make_jwt(&user_id);

    // Insert 5 existing channel+verification rows to saturate the rate limit.
    let storage = Storage::new(dir.path().to_str().unwrap());
    for i in 0..5 {
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(b"extra@example.com").unwrap();
        let lhash = format!("fakehash{i}");
        let ch_id = storage
            .create_recovery_channel(&user_id, "email", ct, nonce, &lhash)
            .unwrap();
        let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
        storage
            .create_recovery_verification(&ch_id, &user_id, "add_channel", "fakehash", &expires_at)
            .unwrap();
    }

    let app = build_test_router(dir.path());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/recovery/channels")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    r#"{"channel_type":"email","value":"new@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

// --- 15. delete_channel_handler — correct password removes channel ---

#[tokio::test]
async fn test_delete_channel_correct_password() {
    isolate_env();
    let dir = tempdir().unwrap();
    let password = "testpassword";
    let user_id = insert_test_user(dir.path(), "user15@example.com", Some(password));
    let token = make_jwt(&user_id);

    let storage = Storage::new(dir.path().to_str().unwrap());
    let (ct, nonce) = crate::recovery_crypto::encrypt_channel_value(b"user15@example.com").unwrap();
    let lhash = crate::recovery_crypto::compute_lookup_hash("user15@example.com");
    let channel_id = storage
        .create_recovery_channel(&user_id, "email", ct, nonce, &lhash)
        .unwrap();

    let app = build_test_router(dir.path());
    let body = serde_json::json!({ "current_password": password }).to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/auth/recovery/channels/{channel_id}"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Channel should be gone.
    let remaining = storage.get_recovery_channels_for_user(&user_id);
    assert!(remaining.is_empty(), "channel should be deleted");
}

// --- 16. delete_channel_handler — wrong password returns 401 ---

#[tokio::test]
async fn test_delete_channel_wrong_password() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user16@example.com", Some("correct"));
    let token = make_jwt(&user_id);

    let storage = Storage::new(dir.path().to_str().unwrap());
    let (ct, nonce) = crate::recovery_crypto::encrypt_channel_value(b"user16@example.com").unwrap();
    let lhash = crate::recovery_crypto::compute_lookup_hash("user16@example.com");
    let channel_id = storage
        .create_recovery_channel(&user_id, "email", ct, nonce, &lhash)
        .unwrap();

    let app = build_test_router(dir.path());
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/auth/recovery/channels/{channel_id}"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"current_password":"wrong"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- 17. lockout after 3 wrong verification attempts ---

#[tokio::test]
async fn test_lockout_after_3_wrong_attempts() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user_id = insert_test_user(dir.path(), "user17@example.com", Some("pass"));
    let _token = make_jwt(&user_id);
    let token = &_token;

    let code_hash = hash_code("correct").unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());
    let (ct, nonce) = crate::recovery_crypto::encrypt_channel_value(b"user17@example.com").unwrap();
    let channel_id = storage
        .create_recovery_channel(&user_id, "email", ct, nonce, "lhash17")
        .unwrap();
    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    storage
        .create_recovery_verification(
            &channel_id,
            &user_id,
            "add_channel",
            &code_hash,
            &expires_at,
        )
        .unwrap();

    let wrong_body = format!(r#"{{"channel_id":"{channel_id}","code":"000000"}}"#);

    // Three wrong attempts trigger lockout.
    for _ in 0..3 {
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/recovery/channels/verify")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(wrong_body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Should be unauthorized (not locked out yet until lockout_until is set).
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // Fourth attempt: channel is locked out → 429.
    let app = build_test_router(dir.path());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/recovery/channels/verify")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(wrong_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "expected lockout after 3 wrong attempts"
    );
}

// --- 18. E2E happy path: add → verify → forgot-password/verify → reset → login ---

#[tokio::test]
async fn test_e2e_reset_password_happy_path() {
    isolate_env();
    let dir = tempdir().unwrap();
    let original_password = "original-pass";
    let user_id = insert_test_user(dir.path(), "e2e@example.com", Some(original_password));
    make_jwt(&user_id); // ensure JWT_SECRET is set in env

    // Steps 1-4: set up DB state then drop the connection before opening
    // the test router (SQLite allows only one writer at a time).
    let code = "112233";
    let code_hash = hash_code(code).unwrap();
    {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let normalized =
            crate::recovery_crypto::normalize_channel_value("email", "e2e@example.com");
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(normalized.as_bytes()).unwrap();
        let lhash = crate::recovery_crypto::compute_lookup_hash(&normalized);
        let channel_id = storage
            .create_recovery_channel(&user_id, "email", ct, nonce, &lhash)
            .unwrap();

        // 2. Mark channel verified.
        let verified_at = chrono::Utc::now().to_rfc3339();
        storage
            .verify_recovery_channel(&channel_id, &verified_at)
            .unwrap();

        // 3. Set usuario so forgot-password lookup works by username.
        storage
            .conn()
            .execute(
                "UPDATE users SET usuario = 'e2euser' WHERE id = ?1",
                rusqlite::params![user_id],
            )
            .unwrap();

        // 4. Insert reset_password verification (pre-hashed code, simulates forgot-password).
        let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();
        storage
            .create_recovery_verification(
                &channel_id,
                &user_id,
                "reset_password",
                &code_hash,
                &expires_at,
            )
            .unwrap();
    } // storage dropped here — connection released before build_test_router

    let app = build_test_router(dir.path());

    // 5. forgot-password/verify with correct code → get reset_token.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/forgot-password/verify")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"username_or_channel_value":"e2euser","code":"{code}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "forgot-password/verify failed"
    );
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let reset_token = body["reset_token"].as_str().unwrap().to_string();

    // 6. reset-password with token → new password + session cookie.
    let new_password = "brand-new-pass!";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/reset-password")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "reset_token": reset_token,
                        "new_password": new_password
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "reset-password failed");
    let has_session = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .map(|c| c.contains("session="))
        .unwrap_or(false);
    assert!(has_session, "reset-password must set session cookie");

    // 7. Login with new password must succeed.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/password-login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "email": "e2e@example.com",
                        "password": new_password
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "login with new password must succeed"
    );

    // 8. Login with OLD password must fail.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/password-login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "email": "e2e@example.com",
                        "password": original_password
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "old password must be rejected after reset"
    );

    // 9. Token is consumed — cannot be reused.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/reset-password")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "reset_token": reset_token,
                        "new_password": "another-pass"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "consumed reset token must not be reusable"
    );
}
