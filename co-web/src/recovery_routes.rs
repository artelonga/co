//! CO-165 — Forgot password / change password with verified recovery channels.
//!
//! ## Endpoints
//!
//! **Recovery channels (requires auth)**:
//! - GET  /api/v1/auth/recovery/channels         — list channels
//! - POST /api/v1/auth/recovery/channels         — add channel
//! - POST /api/v1/auth/recovery/channels/verify  — verify channel with code
//! - DELETE /api/v1/auth/recovery/channels/:id   — remove channel
//!
//! **Password recovery (public)**:
//! - POST /api/v1/auth/forgot-password         — request code to all verified channels
//! - POST /api/v1/auth/forgot-password/verify  — verify code, get reset token
//! - POST /api/v1/auth/reset-password          — exchange token for new password
//! - POST /api/v1/auth/change-password         — change password (auth required)

use axum::{
    Router,
    extract::{Json, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
use crate::error::AppError;
use crate::server::AppState;

// -------------------------------------------------------------------------
// Router construction
// -------------------------------------------------------------------------

/// Routes for managing recovery channels. Mount behind `require_auth`.
pub fn recovery_router() -> Router<AppState> {
    Router::new()
        .route(
            "/channels",
            get(list_channels_handler).post(add_channel_handler),
        )
        .route("/channels/verify", post(verify_channel_handler))
        .route("/channels/{id}", delete(delete_channel_handler))
}

/// Public routes for forgot/reset password. No auth required.
pub fn forgot_password_router() -> Router<AppState> {
    Router::new()
        .route("/forgot-password", post(forgot_password_handler))
        .route(
            "/forgot-password/verify",
            post(forgot_password_verify_handler),
        )
        .route("/reset-password", post(reset_password_handler))
        .route(
            "/change-password",
            post(change_password_handler)
                .layer(axum::middleware::from_fn(crate::auth::require_auth)),
        )
}

// -------------------------------------------------------------------------
// Request / Response types
// -------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct AddChannelRequest {
    channel_type: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct AddChannelResponse {
    channel_id: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct VerifyChannelRequest {
    channel_id: String,
    code: String,
}

#[derive(Debug, Deserialize)]
struct DeleteChannelRequest {
    current_password: String,
}

#[derive(Debug, Deserialize)]
struct ForgotPasswordRequest {
    username_or_channel_value: String,
}

#[derive(Debug, Serialize)]
struct ForgotPasswordResponse {
    sent_to: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ForgotPasswordVerifyRequest {
    username_or_channel_value: String,
    code: String,
}

#[derive(Debug, Serialize)]
struct ForgotPasswordVerifyResponse {
    reset_token: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct ResetPasswordRequest {
    reset_token: String,
    new_password: String,
}

#[derive(Debug, Deserialize)]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

/// Argon2id hash a short code / password.
fn hash_code(code: &str) -> anyhow::Result<String> {
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(code.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .to_string();
    Ok(hash)
}

/// Verify an Argon2id-hashed code.
fn verify_code(code: &str, hash: &str) -> bool {
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    PasswordHash::new(hash)
        .ok()
        .and_then(|h| Argon2::default().verify_password(code.as_bytes(), &h).ok())
        .is_some()
}

/// SHA-256 hex digest of a string — used to hash reset tokens before DB storage.
fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(input.as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// Deliver a verification code over the appropriate channel.
/// Phase 1: always logs. Phase 2: SMTP / WhatsApp / Twilio.
fn send_verification_code(channel_type: &str, value: &str, code: &str) {
    match channel_type {
        "email" => {
            // TODO Phase 2: send via SMTP when CO_SMTP_HOST is set.
            tracing::info!(
                "Recovery code for {}: {} [channel={}]",
                value,
                code,
                channel_type
            );
        }
        "whatsapp" => {
            // TODO Phase 2: WhatsApp Business Cloud API
            // (CO_WHATSAPP_ACCESS_TOKEN, CO_WHATSAPP_PHONE_ID).
            tracing::info!(
                "Recovery code for WhatsApp {}: {} [STUB - Phase 2]",
                value,
                code
            );
        }
        "sms" => {
            // TODO Phase 2: Twilio (CO_TWILIO_SID, CO_TWILIO_TOKEN, CO_TWILIO_FROM).
            tracing::info!("Recovery code for SMS {}: {} [STUB - Phase 2]", value, code);
        }
        _ => {}
    }
}

fn lock_storage(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::storage::Storage>, AppError> {
    state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("Storage lock failed".into()))
}

// -------------------------------------------------------------------------
// Handlers: Recovery channels
// -------------------------------------------------------------------------

/// POST /api/v1/auth/recovery/channels — add a new recovery channel.
/// Rate-limited to 5 codes/hour per channel. Returns 201 with channel_id.
async fn add_channel_handler(
    State(state): State<AppState>,
    UserId(user_id): UserId,
    Json(req): Json<AddChannelRequest>,
) -> Result<Response, AppError> {
    let channel_type = req.channel_type.trim().to_string();
    if !["email", "sms", "whatsapp"].contains(&channel_type.as_str()) {
        return Err(AppError::BadRequest(
            "channel_type must be email, sms, or whatsapp".into(),
        ));
    }

    let normalized = crate::recovery_crypto::normalize_channel_value(&channel_type, &req.value);
    if normalized.is_empty() {
        return Err(AppError::BadRequest("Channel value cannot be empty".into()));
    }

    let lookup_hash = crate::recovery_crypto::compute_lookup_hash(&normalized);

    let (ciphertext, nonce) = crate::recovery_crypto::encrypt_channel_value(normalized.as_bytes())
        .map_err(AppError::Internal)?;

    let channel_id = {
        let storage = lock_storage(&state)?;

        // Rate-limit: at most 5 add_channel verifications per hour for this
        // user (across all channels of same type to prevent enumeration).
        let one_hour_ago = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();

        // Check existing channels for this user to gather channel IDs for rate check.
        let channels = storage.get_recovery_channels_for_user(&user_id);
        let recent_count: i64 = channels
            .iter()
            .filter(|c| c.channel_type == channel_type)
            .map(|c| storage.count_recent_verifications_for_channel(&c.id, &one_hour_ago))
            .sum();

        if recent_count >= 5 {
            return Err(AppError::TooManyRequests(
                "Too many channel verification attempts. Try again in an hour.".into(),
            ));
        }

        storage.create_recovery_channel(&user_id, &channel_type, ciphertext, nonce, &lookup_hash)?
    };

    // Generate a 6-digit code and hash it.
    let code = crate::auth::generate_code();
    let code_hash = tokio::task::spawn_blocking({
        let code = code.clone();
        move || hash_code(&code)
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))?
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let expires_at = (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();

    {
        let storage = lock_storage(&state)?;
        storage.create_recovery_verification(
            &channel_id,
            &user_id,
            "add_channel",
            &code_hash,
            &expires_at,
        )?;
    }

    // Send code (Phase 1: log only).
    send_verification_code(&channel_type, &normalized, &code);

    Ok((
        StatusCode::CREATED,
        axum::Json(AddChannelResponse {
            channel_id,
            expires_at,
        }),
    )
        .into_response())
}

/// POST /api/v1/auth/recovery/channels/verify — verify a channel with the code.
async fn verify_channel_handler(
    State(state): State<AppState>,
    UserId(user_id): UserId,
    Json(req): Json<VerifyChannelRequest>,
) -> Result<Response, AppError> {
    let channel = {
        let storage = lock_storage(&state)?;
        storage
            .get_recovery_channel(&req.channel_id)
            .ok_or_else(|| AppError::NotFound("Channel not found".into()))?
    };

    if channel.user_id != user_id {
        return Err(AppError::Forbidden("Not your channel".into()));
    }

    // Check lockout.
    if let Some(ref lockout) = channel.lockout_until {
        let now = Utc::now().to_rfc3339();
        if lockout.as_str() > now.as_str() {
            return Err(AppError::TooManyRequests(
                "Channel is temporarily locked. Try again later.".into(),
            ));
        }
    }

    let verification = {
        let storage = lock_storage(&state)?;
        storage
            .get_active_verification(&req.channel_id, "add_channel")
            .ok_or_else(|| AppError::Gone("Verification code expired".into()))?
    };

    // Verify code (blocking — argon2).
    let code = req.code.clone();
    let code_hash = verification.code_hash.clone();
    let code_ok = tokio::task::spawn_blocking(move || verify_code(&code, &code_hash))
        .await
        .map_err(|_| AppError::Internal("Task join error".into()))?;

    if !code_ok {
        let attempts = {
            let storage = lock_storage(&state)?;
            storage.increment_verification_attempts(&verification.id)?
        };

        if attempts >= 5 {
            let storage = lock_storage(&state)?;
            storage.expire_verification(&verification.id)?;
        } else if attempts >= 3 {
            let lockout = (Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();
            let storage = lock_storage(&state)?;
            storage.set_channel_lockout(&req.channel_id, &lockout)?;
        }

        return Err(AppError::Unauthorized("Invalid verification code".into()));
    }

    let now = Utc::now().to_rfc3339();
    {
        let storage = lock_storage(&state)?;
        storage.consume_verification(&verification.id)?;
        storage.verify_recovery_channel(&req.channel_id, &now)?;
    }

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({"verified": true})),
    )
        .into_response())
}

/// GET /api/v1/auth/recovery/channels — list channels for the current user.
async fn list_channels_handler(
    State(state): State<AppState>,
    UserId(user_id): UserId,
) -> Result<Response, AppError> {
    let channels = {
        let storage = lock_storage(&state)?;
        storage.get_recovery_channels_for_user(&user_id)
    };

    let mut responses = Vec::with_capacity(channels.len());
    for ch in &channels {
        let plaintext =
            crate::recovery_crypto::decrypt_channel_value(&ch.value_ciphertext, &ch.value_nonce)
                .unwrap_or_else(|_| b"???".to_vec());
        let value_str = String::from_utf8_lossy(&plaintext).to_string();
        let masked = crate::recovery_crypto::mask_channel_value(&ch.channel_type, &value_str);
        responses.push(crate::models::RecoveryChannelResponse {
            id: ch.id.clone(),
            channel_type: ch.channel_type.clone(),
            masked_value: masked,
            verified_at: ch.verified_at.clone(),
            created_at: ch.created_at.clone(),
        });
    }

    Ok((StatusCode::OK, axum::Json(responses)).into_response())
}

/// DELETE /api/v1/auth/recovery/channels/:id — remove a channel.
/// Requires current password for confirmation.
async fn delete_channel_handler(
    State(state): State<AppState>,
    UserId(user_id): UserId,
    Path(channel_id): Path<String>,
    Json(req): Json<DeleteChannelRequest>,
) -> Result<Response, AppError> {
    // Verify current password.
    let hash = {
        let storage = lock_storage(&state)?;
        let (_, hash_opt) = storage
            .get_user_by_id_with_hash(&user_id)
            .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;
        hash_opt.ok_or_else(|| AppError::Unauthorized("No password set".into()))?
    };

    let password = req.current_password.clone();
    let hash_clone = hash.clone();
    let pw_ok = tokio::task::spawn_blocking(move || verify_code(&password, &hash_clone))
        .await
        .map_err(|_| AppError::Internal("Task join error".into()))?;

    if !pw_ok {
        return Err(AppError::Unauthorized("Invalid password".into()));
    }

    // Check ownership and delete.
    let channel = {
        let storage = lock_storage(&state)?;
        storage
            .get_recovery_channel(&channel_id)
            .ok_or_else(|| AppError::NotFound("Channel not found".into()))?
    };

    if channel.user_id != user_id {
        return Err(AppError::Forbidden("Not your channel".into()));
    }

    {
        let storage = lock_storage(&state)?;
        storage.delete_recovery_channel(&channel_id, &user_id)?;
    }

    Ok((StatusCode::OK, axum::Json(serde_json::json!({"ok": true}))).into_response())
}

// -------------------------------------------------------------------------
// Handlers: Forgot / reset password
// -------------------------------------------------------------------------

/// POST /api/v1/auth/forgot-password — send reset codes to all verified channels.
/// ALWAYS returns 202 — never reveals whether the identifier is known.
async fn forgot_password_handler(
    State(state): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> Result<Response, AppError> {
    let identifier = req.username_or_channel_value.trim().to_string();
    let mut sent_to = Vec::new();

    // Try to find the user by username or by a verified channel value.
    let user_opt = find_user_for_recovery(&state, &identifier).await;

    if let Some(user_id) = user_opt {
        let channels = {
            let storage = lock_storage(&state)?;
            storage.get_recovery_channels_for_user(&user_id)
        };

        for ch in channels.iter().filter(|c| c.verified_at.is_some()) {
            let code = crate::auth::generate_code();
            let code_hash = {
                let code_clone = code.clone();
                tokio::task::spawn_blocking(move || hash_code(&code_clone))
                    .await
                    .map_err(|_| AppError::Internal("Task join error".into()))?
                    .map_err(|e| AppError::Internal(e.to_string()))?
            };

            let expires_at = (Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();

            {
                let storage = lock_storage(&state)?;
                let _ = storage.create_recovery_verification(
                    &ch.id,
                    &user_id,
                    "reset_password",
                    &code_hash,
                    &expires_at,
                );
            }

            // Decrypt value for delivery and masking.
            let plaintext = crate::recovery_crypto::decrypt_channel_value(
                &ch.value_ciphertext,
                &ch.value_nonce,
            )
            .unwrap_or_else(|_| b"???".to_vec());
            let value_str = String::from_utf8_lossy(&plaintext).to_string();
            let masked = crate::recovery_crypto::mask_channel_value(&ch.channel_type, &value_str);

            send_verification_code(&ch.channel_type, &value_str, &code);
            sent_to.push(masked);
        }
    }

    // Always 202 — no enumeration.
    Ok((
        StatusCode::ACCEPTED,
        axum::Json(ForgotPasswordResponse { sent_to }),
    )
        .into_response())
}

/// POST /api/v1/auth/forgot-password/verify — verify code, get reset token.
async fn forgot_password_verify_handler(
    State(state): State<AppState>,
    Json(req): Json<ForgotPasswordVerifyRequest>,
) -> Result<Response, AppError> {
    let identifier = req.username_or_channel_value.trim().to_string();

    // Find the user.
    let user_id = find_user_for_recovery(&state, &identifier)
        .await
        .ok_or_else(|| AppError::Unauthorized("Invalid or expired code".into()))?;

    // Find active reset_password verifications for any of the user's channels.
    let (verification, channel_id) = {
        let storage = lock_storage(&state)?;
        let channels = storage.get_recovery_channels_for_user(&user_id);
        let mut found = None;
        for ch in channels.iter().filter(|c| c.verified_at.is_some()) {
            if let Some(v) = storage.get_active_verification(&ch.id, "reset_password") {
                found = Some((v, ch.id.clone()));
                break;
            }
        }
        found.ok_or_else(|| AppError::Unauthorized("Invalid or expired code".into()))?
    };

    // Verify code.
    let code = req.code.clone();
    let code_hash = verification.code_hash.clone();
    let code_ok = tokio::task::spawn_blocking(move || verify_code(&code, &code_hash))
        .await
        .map_err(|_| AppError::Internal("Task join error".into()))?;

    if !code_ok {
        let storage = lock_storage(&state)?;
        let attempts = storage.increment_verification_attempts(&verification.id)?;
        if attempts >= 5 {
            storage.expire_verification(&verification.id)?;
        }
        return Err(AppError::Unauthorized("Invalid or expired code".into()));
    }

    // Consume verification and issue reset token.
    {
        let storage = lock_storage(&state)?;
        storage.consume_verification(&verification.id)?;
    }

    // Generate a cryptographically random reset token.
    let token = format!("{}", uuid::Uuid::new_v4().as_simple());
    let token_hash = sha256_hex(&token);
    let expires_at = (Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();

    {
        let storage = lock_storage(&state)?;
        storage.create_reset_token(&token_hash, &user_id, &channel_id, &expires_at)?;
    }

    Ok((
        StatusCode::OK,
        axum::Json(ForgotPasswordVerifyResponse {
            reset_token: token,
            expires_at,
        }),
    )
        .into_response())
}

/// POST /api/v1/auth/reset-password — exchange a reset token for a new password.
async fn reset_password_handler(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Response, AppError> {
    let token_hash = sha256_hex(&req.reset_token);

    let reset_token = {
        let storage = lock_storage(&state)?;
        storage
            .get_reset_token(&token_hash)
            .ok_or_else(|| AppError::Unauthorized("Invalid or expired reset token".into()))?
    };

    if reset_token.consumed_at.is_some() {
        return Err(AppError::Unauthorized("Reset token already used".into()));
    }

    let now = Utc::now().to_rfc3339();
    if reset_token.expires_at.as_str() <= now.as_str() {
        return Err(AppError::Unauthorized("Reset token expired".into()));
    }

    // Hash the new password (blocking — CPU-intensive).
    let new_password = req.new_password.clone();
    let new_hash = tokio::task::spawn_blocking(move || hash_code(&new_password))
        .await
        .map_err(|_| AppError::Internal("Task join error".into()))?
        .map_err(|e| AppError::Internal(e.to_string()))?;

    {
        let storage = lock_storage(&state)?;
        storage.update_password_hash(&reset_token.user_id, &new_hash)?;
        storage.consume_reset_token(&token_hash)?;
    }

    // Issue a new session JWT.
    let user = {
        let storage = lock_storage(&state)?;
        storage
            .get_user_by_id(&reset_token.user_id)
            .ok_or_else(|| AppError::Internal("User not found".into()))?
    };

    let jwt_secret = crate::auth::jwt_secret();
    let (token, _expires_at) =
        crate::auth::sign_jwt(&user.id, &user.email, &user.tier, &jwt_secret)
            .map_err(|e| AppError::Internal(e.to_string()))?;

    let cookie = format!("session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800");

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        axum::Json(serde_json::json!({"ok": true})),
    )
        .into_response())
}

/// POST /api/v1/auth/change-password — change password when already logged in.
async fn change_password_handler(
    State(state): State<AppState>,
    UserId(user_id): UserId,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Response, AppError> {
    let hash = {
        let storage = lock_storage(&state)?;
        let (_, hash_opt) = storage
            .get_user_by_id_with_hash(&user_id)
            .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;
        hash_opt.ok_or_else(|| AppError::Unauthorized("No password set".into()))?
    };

    // Verify current password.
    let current_password = req.current_password.clone();
    let hash_clone = hash.clone();
    let pw_ok = tokio::task::spawn_blocking(move || verify_code(&current_password, &hash_clone))
        .await
        .map_err(|_| AppError::Internal("Task join error".into()))?;

    if !pw_ok {
        return Err(AppError::Unauthorized("Invalid current password".into()));
    }

    // Hash the new password.
    let new_password = req.new_password.clone();
    let new_hash = tokio::task::spawn_blocking(move || hash_code(&new_password))
        .await
        .map_err(|_| AppError::Internal("Task join error".into()))?
        .map_err(|e| AppError::Internal(e.to_string()))?;

    {
        let storage = lock_storage(&state)?;
        storage.update_password_hash(&user_id, &new_hash)?;
    }

    // Issue a refreshed JWT.
    let user = {
        let storage = lock_storage(&state)?;
        storage
            .get_user_by_id(&user_id)
            .ok_or_else(|| AppError::Internal("User not found".into()))?
    };

    let jwt_secret = crate::auth::jwt_secret();
    let (token, _expires_at) =
        crate::auth::sign_jwt(&user.id, &user.email, &user.tier, &jwt_secret)
            .map_err(|e| AppError::Internal(e.to_string()))?;

    let cookie = format!("session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800");

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        axum::Json(serde_json::json!({"ok": true})),
    )
        .into_response())
}

// -------------------------------------------------------------------------
// Lookup helper
// -------------------------------------------------------------------------

/// Find a user_id for password recovery by:
///   1. Exact `usuario` match in the users table.
///   2. Verified recovery channel with matching lookup hash (email/sms/whatsapp).
async fn find_user_for_recovery(state: &AppState, identifier: &str) -> Option<String> {
    let storage = state.storage.lock().ok()?;

    // Try username first.
    if let Some(user) = storage.get_user_by_usuario(identifier) {
        return Some(user.id);
    }

    // Try as email.
    let email_normalized = crate::recovery_crypto::normalize_channel_value("email", identifier);
    let email_hash = crate::recovery_crypto::compute_lookup_hash(&email_normalized);
    if let Some(ch) = storage.find_verified_channel_by_lookup_hash("email", &email_hash) {
        return Some(ch.user_id);
    }

    // Try as phone (SMS or WhatsApp).
    for ct in ["sms", "whatsapp"] {
        let phone_normalized = crate::recovery_crypto::normalize_channel_value(ct, identifier);
        if !phone_normalized.is_empty() {
            let phone_hash = crate::recovery_crypto::compute_lookup_hash(&phone_normalized);
            if let Some(ch) = storage.find_verified_channel_by_lookup_hash(ct, &phone_hash) {
                return Some(ch.user_id);
            }
        }
    }

    None
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
            game_core::storage::Storage::open(&game_db_path)
                .expect("Failed to open test game storage"),
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
            crate::auth::sign_jwt(user_id, "test@example.com", "player", "test-jwt-secret")
                .unwrap();
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
            .create_recovery_verification(
                &channel_id,
                &user_id,
                "add_channel",
                &code_hash,
                expires_at,
            )
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
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(b"user7@example.com").unwrap();
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
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(b"user9@example.com").unwrap();
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
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(b"user10@example.com").unwrap();
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
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(b"user11@example.com").unwrap();
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
                .create_recovery_verification(
                    &ch_id,
                    &user_id,
                    "add_channel",
                    "fakehash",
                    &expires_at,
                )
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
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(b"user15@example.com").unwrap();
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
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(b"user16@example.com").unwrap();
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
        let (ct, nonce) =
            crate::recovery_crypto::encrypt_channel_value(b"user17@example.com").unwrap();
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
}
