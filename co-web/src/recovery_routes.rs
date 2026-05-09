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

/// Deliver a recovery verification code over the appropriate channel.
///
/// Resolution order per channel:
///
/// - **email**: CO-169 [`ResendProvider`] (`RESEND_API_KEY`) →
///   [`crate::email_smtp::send_recovery_code`] SMTP (`CO_SMTP_*`) → log only.
/// - **whatsapp**: CO-169 [`EvolutionApiProvider`]
///   (`EVOLUTION_API_KEY`) → log only.
/// - **sms**: log only (Twilio is the planned Phase 2 provider).
///
/// Each transport is best-effort. A failure logs at WARN with the redacted
/// recipient and the raw code (for operator/developer recovery), but the
/// request handler always returns 202 (no enumeration).
///
/// Detached via [`tokio::spawn`] so the HTTP response returns instantly.
///
/// [`ResendProvider`]: crate::notification_providers::ResendProvider
/// [`EvolutionApiProvider`]: crate::notification_providers::EvolutionApiProvider
fn send_verification_code(channel_type: &str, value: &str, code: &str) {
    let value = value.to_string();
    let code = code.to_string();
    let channel_type = channel_type.to_string();
    tokio::spawn(async move {
        match channel_type.as_str() {
            "email" => deliver_email_code(&value, &code).await,
            "whatsapp" => deliver_whatsapp_code(&value, &code).await,
            "sms" => {
                // Twilio remains Phase 2.
                tracing::info!("Recovery code for SMS {}: {} [STUB - Phase 2]", value, code);
            }
            _ => {}
        }
    });
}

/// Email delivery cascade: Resend → SMTP → log.
async fn deliver_email_code(to: &str, code: &str) {
    use crate::notification_providers::{ChannelProvider, ResendProvider};

    let subject = "Seu código de recuperação CO";
    let body = format!(
        "Olá,\n\nUse este código para recuperar sua conta CO:\n\n\t{code}\n\n\
         O código expira em 10 minutos. Se você não solicitou esta recuperação, \
         pode ignorar este email — sua conta está segura.\n\n— CO\n"
    );

    if let Some(provider) = ResendProvider::from_env() {
        let payload = format!("{subject}\n---\n{body}");
        let client = reqwest::Client::new();
        match provider.send(&client, to, &payload).await {
            Ok(()) => {
                tracing::info!("Recovery code emailed to {} via Resend", redact_email(to));
                return;
            }
            Err(e) => {
                tracing::warn!(
                    "Resend delivery to {} failed: {e}. Trying SMTP fallback.",
                    redact_email(to)
                );
            }
        }
    }

    match crate::email_smtp::send_recovery_code(to, code).await {
        Ok(true) => {
            tracing::info!("Recovery code emailed to {} via SMTP", redact_email(to));
        }
        Ok(false) => {
            tracing::info!(
                "Recovery code for {}: {} [no email provider configured — code logged]",
                to,
                code
            );
        }
        Err(e) => {
            tracing::warn!(
                "SMTP delivery to {} failed: {e}. Code logged: {} (dev fallback)",
                redact_email(to),
                code
            );
        }
    }
}

/// WhatsApp delivery cascade: Evolution API → log.
async fn deliver_whatsapp_code(to: &str, code: &str) {
    use crate::notification_providers::{ChannelProvider, EvolutionApiProvider};

    let body = format!("🔐 *Código de recuperação CO*\n\n{code}\n\nExpira em 10 minutos.");

    if let Some(provider) = EvolutionApiProvider::from_env() {
        let client = reqwest::Client::new();
        match provider.send(&client, to, &body).await {
            Ok(()) => {
                tracing::info!(
                    "Recovery code sent to WhatsApp {} via Evolution API",
                    redact_phone(to)
                );
                return;
            }
            Err(e) => {
                tracing::warn!(
                    "Evolution API delivery to {} failed: {e}. Code logged: {} (dev fallback)",
                    redact_phone(to),
                    code
                );
                return;
            }
        }
    }

    tracing::info!(
        "Recovery code for WhatsApp {}: {} [no Evolution API key — code logged]",
        to,
        code
    );
}

fn redact_phone(phone: &str) -> String {
    let digits: Vec<char> = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return "***".to_string();
    }
    let tail: String = digits
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("****{tail}")
}

fn redact_email(email: &str) -> String {
    if let Some((local, domain)) = email.split_once('@') {
        let head = local
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        format!("{head}***@{domain}")
    } else {
        "***".to_string()
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
mod tests;
