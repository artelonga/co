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
pub fn forgot_password_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/forgot-password", post(forgot_password_handler))
        .route(
            "/forgot-password/verify",
            post(forgot_password_verify_handler),
        )
        .route("/reset-password", post(reset_password_handler))
        .route(
            "/change-password",
            post(change_password_handler).layer(axum::middleware::from_fn_with_state(
                state,
                crate::auth::require_auth,
            )),
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
    /// Primary identifier — usuario, email, or phone-as-channel-value.
    /// Existing API consumers may still send only this field.
    username_or_channel_value: String,
    /// CO-176: when provided, both `username_or_channel_value` and `email`
    /// must resolve to the **same** user — otherwise the request silently
    /// returns `sent_to: []`. UI sends both since 1.91.3 to disambiguate
    /// quilombo users (one email may exist on CO + quilombo with different
    /// usernames).
    #[serde(default)]
    email: String,
}

#[derive(Debug, Serialize)]
struct ForgotPasswordResponse {
    sent_to: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ForgotPasswordVerifyRequest {
    username_or_channel_value: String,
    #[serde(default)]
    email: String,
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
    /// Optional. When a quilombo user (or any signed-in user) has no email
    /// on file and wants to add one as part of the change-password flow,
    /// they pass it here. Server validates uniqueness, sets `users.email`,
    /// promotes the address to a verified recovery channel, and mirrors it
    /// to any linked `quilombo_usuarios` row. Username is NEVER touched.
    #[serde(default)]
    email: String,
}

// -------------------------------------------------------------------------
// CO-172: return_to safelist
// -------------------------------------------------------------------------

/// Validate a `return_to` URL for the `/recover` redirect.
/// Accepts `*.artelonga.com.br`, `quilomboaraucaria.com.br`, and localhost
/// (for `co serve` local distribution — CO-282).
/// Called server-side from `server::serve_recover` and mirrored client-side
/// in `login.js::isAllowedReturnTo`.
pub(crate) fn is_allowed_return_to(url: &str) -> bool {
    fn extract_host(url: &str) -> Option<&str> {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let host_port = rest.split(['/', '?', '#']).next()?;
        Some(host_port.split(':').next().unwrap_or(host_port))
    }
    let Some(host) = extract_host(url) else {
        return false;
    };
    host == "localhost"
        || host == "127.0.0.1"
        || host == "quilomboaraucaria.com.br"
        || host == "quilomboaraucaria.org"
        || host == "artelonga.com.br"
        || host.ends_with(".artelonga.com.br")
        || host == "yggdrasil-artelonga.fly.dev"
        || host == "yggdrasil.artelonga.com.br"
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

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.core.storage.lock()
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

    let lookup_hash =
        crate::recovery_crypto::compute_lookup_hash(&normalized, &*state.core.secrets);

    let (ciphertext, nonce) =
        crate::recovery_crypto::encrypt_channel_value(normalized.as_bytes(), &*state.core.secrets)
            .map_err(AppError::Internal)?;

    let channel_id = {
        let storage = lock_storage(&state);

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
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
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
            let storage = lock_storage(&state);
            storage.increment_verification_attempts(&verification.id)?
        };

        if attempts >= 5 {
            let storage = lock_storage(&state);
            storage.expire_verification(&verification.id)?;
        } else if attempts >= 3 {
            let lockout = (Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();
            let storage = lock_storage(&state);
            storage.set_channel_lockout(&req.channel_id, &lockout)?;
        }

        return Err(AppError::Unauthorized("Invalid verification code".into()));
    }

    let now = Utc::now().to_rfc3339();
    {
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
        storage.get_recovery_channels_for_user(&user_id)
    };

    let mut responses = Vec::with_capacity(channels.len());
    for ch in &channels {
        let plaintext = crate::recovery_crypto::decrypt_channel_value(
            &ch.value_ciphertext,
            &ch.value_nonce,
            &*state.core.secrets,
        )
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
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
        storage
            .get_recovery_channel(&channel_id)
            .ok_or_else(|| AppError::NotFound("Channel not found".into()))?
    };

    if channel.user_id != user_id {
        return Err(AppError::Forbidden("Not your channel".into()));
    }

    {
        let storage = lock_storage(&state);
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
    let email_companion = req.email.trim().to_lowercase();
    let mut sent_to = Vec::new();

    // CO-176: when both `username_or_channel_value` and `email` are sent
    // (1.91.3 UI), both must resolve to the **same** user. Otherwise the
    // request returns 202 with `sent_to: []` — same shape as "user not
    // found", so callers can't distinguish a wrong-pair attempt from a
    // non-existent account.
    let user_opt = find_user_for_recovery_pair(&state, &identifier, &email_companion).await;

    if let Some(user_id) = user_opt {
        let channels = {
            let storage = lock_storage(&state);
            storage.get_recovery_channels_for_user(&user_id)
        };

        let verified_count = channels.iter().filter(|c| c.verified_at.is_some()).count();
        if verified_count == 0 {
            tracing::info!(
                "Recovery request matched user {} but no verified channel; \
                 silent 202 (no enumeration). identifier={} email_supplied={}",
                user_id,
                redact_email(&identifier),
                !email_companion.is_empty(),
            );
        }

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
                let storage = lock_storage(&state);
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
                &*state.core.secrets,
            )
            .unwrap_or_else(|_| b"???".to_vec());
            let value_str = String::from_utf8_lossy(&plaintext).to_string();
            let masked = crate::recovery_crypto::mask_channel_value(&ch.channel_type, &value_str);

            send_verification_code(&ch.channel_type, &value_str, &code);
            sent_to.push(masked);
        }
    } else {
        tracing::info!(
            "Recovery request no-match: silent 202 (no enumeration). \
             identifier={} email={}",
            redact_email(&identifier),
            if email_companion.is_empty() {
                "<none>".to_string()
            } else {
                redact_email(&email_companion)
            },
        );
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
    let email_companion = req.email.trim().to_lowercase();

    // CO-176: pair lookup so the verify step honours the same identity gate
    // as forgot-password — a code intercepted with a *different* identifier
    // can't be used to reset another account.
    let user_id = find_user_for_recovery_pair(&state, &identifier, &email_companion)
        .await
        .ok_or_else(|| AppError::Unauthorized("Invalid or expired code".into()))?;

    // Find active reset_password verifications for any of the user's channels.
    let (verification, channel_id) = {
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
        let attempts = storage.increment_verification_attempts(&verification.id)?;
        if attempts >= 5 {
            storage.expire_verification(&verification.id)?;
        }
        return Err(AppError::Unauthorized("Invalid or expired code".into()));
    }

    // Consume verification and issue reset token.
    {
        let storage = lock_storage(&state);
        storage.consume_verification(&verification.id)?;
    }

    // Generate a cryptographically random reset token.
    let token = format!("{}", uuid::Uuid::new_v4().as_simple());
    let token_hash = sha256_hex(&token);
    let expires_at = (Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();

    {
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
        storage.update_password_hash(&reset_token.user_id, &new_hash)?;
        storage.consume_reset_token(&token_hash)?;
        // CO-172 Phase 4: propagate new hash to all linked quilombo users
        match storage.mirror_password_to_quilombo(&reset_token.user_id, &new_hash) {
            Ok(n) if n > 0 => {
                tracing::info!(
                    user_id = %reset_token.user_id,
                    "reset-password: mirrored hash to {n} quilombo_usuarios row(s)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    user_id = %reset_token.user_id,
                    "mirror_password_to_quilombo failed: {e}"
                );
            }
            _ => {}
        }
    }

    // Issue a new session JWT.
    let user = {
        let storage = lock_storage(&state);
        storage
            .get_user_by_id(&reset_token.user_id)
            .ok_or_else(|| AppError::Internal("User not found".into()))?
    };

    let jwt_secret = crate::auth::jwt_secret();
    let (token, _expires_at) =
        crate::auth::sign_jwt(&user.id, &user.email, &user.tier, &jwt_secret)
            .map_err(|e| AppError::Internal(e.to_string()))?;

    let cookie = format!("session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800");

    // CO-186: short-lived (60s) ES256-signed handover token in the body.
    // SPA appends it to the redirect when `return_to` ends in
    // `/auth/co-handover`, letting the receiving deployment mint its own
    // session cookie cross-apex. Validated via CO's JWKS — no shared secret.
    let co_token = crate::auth::sign_handover_jwt_es256(
        &state.integrations.jwt_key,
        &user.id,
        &user.email,
        &user.tier,
    )
    .ok();

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        axum::Json(serde_json::json!({
            "ok": true,
            "co_token": co_token,
        })),
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
        let storage = lock_storage(&state);
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

    // Optionally attach (or update) the user's email — typically a
    // quilombo user who never had one and is adding it as part of this
    // change-password flow. Username is preserved.
    let email_to_attach = req.email.trim().to_lowercase();
    if !email_to_attach.is_empty() {
        if !email_to_attach.contains('@') || email_to_attach.len() > 254 {
            return Err(AppError::BadRequest("Invalid email format".into()));
        }
        let storage = lock_storage(&state);
        match storage.set_user_email(&user_id, &email_to_attach) {
            Ok(_) => {
                let _ = storage.ensure_email_recovery_channel(&user_id, &email_to_attach);
                let _ = storage.mirror_email_to_quilombo(&user_id, &email_to_attach);
                tracing::info!(
                    "change-password: attached email {} to user {} (recovery channel + quilombo mirror)",
                    redact_email(&email_to_attach),
                    user_id,
                );
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already in use") {
                    return Err(AppError::Conflict("Email já em uso por outra conta".into()));
                }
                return Err(AppError::Internal(format!("Failed to set email: {msg}")));
            }
        }
    }

    // Hash the new password.
    let new_password = req.new_password.clone();
    let new_hash = tokio::task::spawn_blocking(move || hash_code(&new_password))
        .await
        .map_err(|_| AppError::Internal("Task join error".into()))?
        .map_err(|e| AppError::Internal(e.to_string()))?;

    {
        let storage = lock_storage(&state);
        storage.update_password_hash(&user_id, &new_hash)?;
        // Also propagate to any linked quilombo rows so the user keeps a
        // single canonical password across both identities.
        let _ = storage.mirror_password_to_quilombo(&user_id, &new_hash);
    }

    // Issue a refreshed JWT.
    let user = {
        let storage = lock_storage(&state);
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

/// CO-176 paired lookup. When `email` is non-empty: resolves the user via
/// `find_user_for_recovery(identifier)`, verifies that user has a verified
/// email channel matching `email` (or a `users.email` matching `email`),
/// and returns `Some(user_id)` only when both match. When `email` is empty,
/// falls back to the legacy single-identifier lookup so older API consumers
/// keep working.
async fn find_user_for_recovery_pair(
    state: &AppState,
    identifier: &str,
    email: &str,
) -> Option<String> {
    // Primary: try the identifier (could be CO usuario, quilombo usuario,
    // verified email channel, or trigger a lazy-bridge by email).
    let mut user_id = find_user_for_recovery(state, identifier).await;

    // Fallback: if the identifier didn't resolve and the caller separately
    // supplied an email, run the lazy-bridge through that email. This lets
    // a legacy quilombo user with a quilombo email but no CO account / no
    // CO recovery channel get their CO identity provisioned just by typing
    // the email in the email field.
    if user_id.is_none() && email.contains('@') && !email.is_empty() {
        user_id = find_user_for_recovery(state, email).await;
    }

    let user_id = user_id?;

    // Whenever a user_id is resolved AND the caller supplied an email,
    // ensure that email exists as a verified recovery channel for them.
    // This covers two important cases:
    //   - lazy-bridge by usuario succeeded but the bridge didn't propagate
    //     a recovery channel (the quilombo row may have empty email);
    //   - the user typed an email different from the one already on file —
    //     they want to recover via the new one going forward.
    // Idempotent for already-existing channels.
    if !email.is_empty() && email.contains('@') {
        let storage = state.core.storage.lock();
        let _ = storage.ensure_email_recovery_channel(&user_id, email);
    }

    if email.is_empty() {
        return Some(user_id);
    }

    // Hash the supplied email and compare against any verified email
    // channel for this user. Quilombo users get a verified channel via the
    // boot-time backfill or lazy-bridge step, so this catches both paths.
    let email_lookup = crate::recovery_crypto::compute_lookup_hash(
        &crate::recovery_crypto::normalize_channel_value("email", email),
        &*state.core.secrets,
    );

    let storage = state.core.storage.lock();
    let channels = storage.get_recovery_channels_for_user(&user_id);
    let channel_match = channels
        .iter()
        .filter(|c| c.channel_type == "email" && c.verified_at.is_some())
        .any(|c| c.value_lookup_hash == email_lookup);
    if channel_match {
        return Some(user_id);
    }

    // Fallback: if the user has `users.email` set and matches what was typed,
    // accept it. Belt-and-suspenders for the brief window between bridge
    // creation and recovery-channel auto-promotion.
    if let Some(user) = storage.get_user_by_id(&user_id)
        && !user.email.is_empty()
        && user.email.eq_ignore_ascii_case(email)
    {
        return Some(user_id);
    }

    None
}

async fn find_user_for_recovery(state: &AppState, identifier: &str) -> Option<String> {
    let storage = state.core.storage.lock();
    let trimmed = identifier.trim();
    tracing::info!(
        "find_user_for_recovery: input={} (len={})",
        redact_email(trimmed),
        trimmed.len()
    );

    // Try CO username first.
    if let Some(user) = storage.get_user_by_usuario(trimmed) {
        tracing::info!(
            "find_user_for_recovery: matched CO users.usuario → {}",
            user.id
        );
        return Some(user.id);
    }

    // CO-172/QB-9 follow-up: also try quilombo username. A quilombo user
    // might have a different `usuario` in `quilombo_usuarios` than in
    // `users` (the bridge in `quilombo_bridge.rs` falls back to NULL on
    // unique-index collision), so hitting only `users.usuario` would miss
    // them. The lookup walks `quilombo_usuarios.usuario` and resolves
    // through `linked_co_user_id` to the canonical CO user id.
    let normalized = trimmed.to_lowercase();
    if let Ok(linked) = storage.conn().query_row(
        "SELECT linked_co_user_id FROM quilombo_usuarios \
         WHERE usuario = ?1 AND linked_co_user_id IS NOT NULL",
        rusqlite::params![normalized],
        |row| row.get::<_, Option<String>>(0),
    ) && let Some(co_id) = linked
    {
        tracing::info!("find_user_for_recovery: matched linked quilombo usuario → {co_id}");
        return Some(co_id);
    }

    // Lazy bridge for unlinked legacy quilombo users by usuario. A quilombo
    // user who pre-dates CO-172 — or whose bridge silently failed at
    // signup — has a `quilombo_usuarios` row with no `linked_co_user_id`,
    // so the linked-only query above misses them. Provision the CO side on
    // demand using the same chain CO-172 Phase 1 runs at signup.
    if let Ok(q_id) = storage.conn().query_row(
        "SELECT id FROM quilombo_usuarios \
         WHERE usuario = ?1 AND (linked_co_user_id IS NULL OR linked_co_user_id = '')",
        rusqlite::params![normalized],
        |row| row.get::<_, String>(0),
    ) {
        tracing::info!(
            "find_user_for_recovery: matched unlinked quilombo usuario → q_id={q_id}, bridging"
        );
        if let Ok(co_id) = storage.ensure_co_user_for_quilombo(&q_id) {
            let _ = storage.link_quilombo_to_co(&q_id, &co_id);
            tracing::info!("find_user_for_recovery: bridge complete → co_id={co_id}");
            return Some(co_id);
        } else {
            tracing::warn!(
                "find_user_for_recovery: ensure_co_user_for_quilombo failed for q_id={q_id}"
            );
        }
    }

    // Try as email through CO's verified-channel index (post-bridge users).
    let email_normalized = crate::recovery_crypto::normalize_channel_value("email", trimmed);
    let email_hash =
        crate::recovery_crypto::compute_lookup_hash(&email_normalized, &*state.core.secrets);
    if let Some(ch) = storage.find_verified_channel_by_lookup_hash("email", &email_hash) {
        return Some(ch.user_id);
    }

    // CO-176: lazy bridge for legacy quilombo users. A quilombo user who
    // signed up before CO-172 (or whose bridge silently failed at email-set
    // time) may have `quilombo_usuarios.email` populated but no
    // `linked_co_user_id` and no CO recovery channel — typing their email
    // would have hit a dead end. Bridge them on demand: lookup by
    // `quilombo_usuarios.email`, run the same `ensure_co_user_for_quilombo`
    // → `link_quilombo_to_co` → `ensure_email_recovery_channel` chain that
    // CO-172 Phase 1 runs at signup time, then return the new CO user id.
    // Idempotent: if the bridge already happened, the lookup above caught
    // it and we never reach here.
    if !email_normalized.is_empty() && email_normalized.contains('@') {
        let q_user_id: Option<String> = storage
            .conn()
            .query_row(
                "SELECT id FROM quilombo_usuarios WHERE email = ?1",
                rusqlite::params![email_normalized],
                |row| row.get(0),
            )
            .ok();
        if let Some(q_id) = q_user_id {
            tracing::info!(
                "find_user_for_recovery: matched quilombo email → q_id={q_id}, bridging"
            );
            if let Ok(co_id) = storage.ensure_co_user_for_quilombo(&q_id) {
                let _ = storage.link_quilombo_to_co(&q_id, &co_id);
                let _ = storage.ensure_email_recovery_channel(&co_id, &email_normalized);
                tracing::info!("find_user_for_recovery: bridge complete via email → co_id={co_id}");
                return Some(co_id);
            } else {
                tracing::warn!(
                    "find_user_for_recovery: ensure_co_user_for_quilombo failed for q_id={q_id}"
                );
            }
        } else {
            tracing::info!(
                "find_user_for_recovery: no quilombo_usuarios row with email={}",
                redact_email(&email_normalized)
            );
        }
    }

    // Try as phone (SMS or WhatsApp).
    for ct in ["sms", "whatsapp"] {
        let phone_normalized = crate::recovery_crypto::normalize_channel_value(ct, trimmed);
        if !phone_normalized.is_empty() {
            let phone_hash = crate::recovery_crypto::compute_lookup_hash(
                &phone_normalized,
                &*state.core.secrets,
            );
            if let Some(ch) = storage.find_verified_channel_by_lookup_hash(ct, &phone_hash) {
                return Some(ch.user_id);
            }
        }
    }

    tracing::info!("find_user_for_recovery: all paths exhausted, returning None");
    None
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests;
