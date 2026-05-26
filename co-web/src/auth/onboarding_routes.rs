//! CO-190 — Passwordless onboarding via email: magic-code sign-in or signup.
//!
//! ## Endpoints (no auth required)
//!
//! - `POST /api/v1/auth/onboard-with-email`        — request 6-digit code
//! - `POST /api/v1/auth/onboard-with-email/verify` — verify code → session

use axum::{
    Router,
    extract::{Json, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::server::AppState;

// -------------------------------------------------------------------------
// Router
// -------------------------------------------------------------------------

pub fn onboarding_router() -> Router<AppState> {
    Router::new()
        .route("/onboard-with-email", post(onboard_handler))
        .route("/onboard-with-email/verify", post(onboard_verify_handler))
}

// -------------------------------------------------------------------------
// Request / Response types
// -------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OnboardRequest {
    email: String,
    #[serde(default)]
    preferred_usuario: Option<String>,
    #[serde(default)]
    return_to: Option<String>,
    #[serde(default)]
    origin: Option<String>,
}

#[derive(Debug, Serialize)]
struct OnboardResponse {
    sent: bool,
    expires_at: String,
    /// CO-303: populated in non-prod envs so the SPA can display the code
    /// inline and auto-fill the code input. Never set in production.
    #[serde(skip_serializing_if = "Option::is_none")]
    dev_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OnboardVerifyRequest {
    email: String,
    code: String,
}

#[derive(Debug, Serialize)]
struct OnboardVerifyResponse {
    user_id: String,
    email: String,
    display_name: String,
    expires_at: DateTime<Utc>,
    return_to: Option<String>,
}

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.core.storage.lock()
}

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

fn verify_code(code: &str, hash: &str) -> bool {
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    PasswordHash::new(hash)
        .ok()
        .and_then(|h| Argon2::default().verify_password(code.as_bytes(), &h).ok())
        .is_some()
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

/// Send the onboarding code email (Resend → SMTP → log fallback).
async fn send_onboard_email(to: &str, code: &str) {
    use crate::notification_providers::{ChannelProvider, ResendProvider};

    let subject = "Seu código de acesso ao CO";
    let body = format!(
        "Olá,\n\n\
         Seu código de acesso é {code}.\n\n\
         Se você ainda não tem conta, ela será criada automaticamente quando \
         você confirmar este código.\n\n\
         O código expira em 10 minutos.\n\n\
         — CO\n"
    );

    if let Some(provider) = ResendProvider::from_env() {
        let payload = format!("{subject}\n---\n{body}");
        let client = reqwest::Client::new();
        match provider.send(&client, to, &payload).await {
            Ok(()) => {
                tracing::info!("Onboarding code emailed to {} via Resend", redact_email(to));
                return;
            }
            Err(e) => {
                tracing::warn!(
                    "Resend onboard delivery to {} failed: {e}. Trying SMTP fallback.",
                    redact_email(to)
                );
            }
        }
    }

    match crate::email_smtp::send_recovery_code(to, code).await {
        Ok(true) => {
            tracing::info!("Onboarding code emailed to {} via SMTP", redact_email(to));
        }
        Ok(false) => {
            tracing::info!(
                "Onboarding code for {}: {} [no email provider configured — code logged]",
                redact_email(to),
                code
            );
        }
        Err(e) => {
            tracing::warn!(
                "SMTP onboard delivery to {} failed: {e}. Code: {} (dev fallback)",
                redact_email(to),
                code
            );
        }
    }
}

// -------------------------------------------------------------------------
// Handler: POST /api/v1/auth/onboard-with-email
// -------------------------------------------------------------------------

async fn onboard_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OnboardRequest>,
) -> Result<Response, AppError> {
    let email = req.email.trim().to_lowercase();

    // 1. Validate email format.
    if !email.contains('@') || email.len() > 254 {
        return Err(AppError::BadRequest("Email inválido.".into()));
    }

    let email_normalized = crate::recovery_crypto::normalize_channel_value("email", &email);
    let email_hash =
        crate::recovery_crypto::compute_lookup_hash(&email_normalized, &*state.core.secrets);

    // 2. Rate limit: 5 codes per email per hour.
    let one_hour_ago = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    {
        let storage = lock_storage(&state);
        let count = storage.count_onboarding_codes_for_email(&email_hash, &one_hour_ago);
        if count >= 5 {
            return Err(AppError::TooManyRequests(
                "Muitas tentativas. Tente novamente em uma hora.".into(),
            ));
        }
    }

    // Rate limit: 20 codes per IP per hour.
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .unwrap_or("unknown")
        .trim()
        .to_string();
    let ip_hash = {
        use sha2::{Digest, Sha256};
        let h = Sha256::digest(format!("onboard:ip:{ip}").as_bytes());
        let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
        format!("ip:{hex}")
    };
    {
        let storage = lock_storage(&state);
        let count = storage.count_onboarding_codes_for_ip(&ip_hash, &one_hour_ago);
        if count >= 20 {
            return Err(AppError::TooManyRequests(
                "Muitas tentativas por este IP. Tente novamente em uma hora.".into(),
            ));
        }
    }

    // 3. Determine intent: login (known email) or create (unknown).
    let intent = {
        let storage = lock_storage(&state);
        if storage.get_user_by_email(&email_normalized).is_some() {
            "login"
        } else {
            "create"
        }
    };

    // 4. Mint 6-digit code, hash, store.
    let code = crate::auth::generate_code();
    let code_hash = {
        let code_clone = code.clone();
        tokio::task::spawn_blocking(move || hash_code(&code_clone))
            .await
            .map_err(|_| AppError::Internal("Task join error".into()))?
            .map_err(|e| AppError::Internal(e.to_string()))?
    };

    let expires_at = (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();

    let preferred_usuario = req
        .preferred_usuario
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let return_to = req
        .return_to
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let origin = crate::auth::sanitize_origin(req.origin);

    {
        let storage = lock_storage(&state);
        storage.create_onboarding_code(
            &email_hash,
            intent,
            &code_hash,
            preferred_usuario.as_deref(),
            return_to.as_deref(),
            origin.as_deref(),
            &expires_at,
        )?;
    }

    // Record IP rate-limit sentinel.
    {
        let storage = lock_storage(&state);
        let _ = storage.record_ip_onboarding_request(&ip_hash);
    }

    // 5. Send email (detached — response returns immediately).
    let email_for_send = email_normalized.clone();
    let code_for_send = code.clone();
    tokio::spawn(async move {
        send_onboard_email(&email_for_send, &code_for_send).await;
    });

    tracing::info!(
        "onboard-with-email: intent={intent}, email={}, expires_at={expires_at}",
        redact_email(&email)
    );

    // CO-303: surface code inline in non-prod envs so localhost devs can
    // complete login through the UI without email delivery.
    let dev_code = state.core.config.is_local_or_test().then(|| code.clone());

    // 6. Return 202 with sent=true (even for unknown emails).
    Ok((
        StatusCode::ACCEPTED,
        axum::Json(OnboardResponse {
            sent: true,
            expires_at,
            dev_code,
        }),
    )
        .into_response())
}

// -------------------------------------------------------------------------
// Handler: POST /api/v1/auth/onboard-with-email/verify
// -------------------------------------------------------------------------

async fn onboard_verify_handler(
    State(state): State<AppState>,
    Json(req): Json<OnboardVerifyRequest>,
) -> Result<Response, AppError> {
    let email = req.email.trim().to_lowercase();
    if !email.contains('@') {
        return Err(AppError::BadRequest("Email inválido.".into()));
    }

    let email_normalized = crate::recovery_crypto::normalize_channel_value("email", &email);
    let email_hash =
        crate::recovery_crypto::compute_lookup_hash(&email_normalized, &*state.core.secrets);

    // 1. Look up active code.
    let oc = {
        let storage = lock_storage(&state);
        storage.get_onboarding_code(&email_hash).ok_or_else(|| {
            AppError::Gone("Código não encontrado, expirado ou já utilizado.".into())
        })?
    };

    // 2. Check lockout (>= 5 wrong attempts).
    if oc.attempts >= 5 {
        return Err(AppError::Gone(
            "Código bloqueado. Solicite um novo código.".into(),
        ));
    }

    // 3. Verify code (Argon2id, CPU-bound).
    let code = req.code.trim().to_string();
    let code_hash = oc.code_hash.clone();
    let code_ok = tokio::task::spawn_blocking(move || verify_code(&code, &code_hash))
        .await
        .map_err(|_| AppError::Internal("Task join error".into()))?;

    if !code_ok {
        let attempts = {
            let storage = lock_storage(&state);
            storage.increment_onboarding_attempts(&oc.id)?
        };
        if attempts >= 5 {
            return Err(AppError::Gone(
                "Código bloqueado após 5 tentativas incorretas. Solicite um novo código.".into(),
            ));
        }
        return Err(AppError::Unauthorized("Código inválido.".into()));
    }

    // 4. Branch on intent.
    let user = match oc.intent.as_str() {
        "login" => {
            let storage = lock_storage(&state);
            storage
                .get_user_by_email(&email_normalized)
                .ok_or_else(|| AppError::Internal("User not found for login intent".into()))?
        }
        _ => {
            // "create" — or any unexpected intent is treated as create.

            // Check global signup rate cap (100 new accounts / 24h).
            {
                let storage = lock_storage(&state);
                let count = storage.count_users_created_since(24 * 60 * 60);
                if count >= 100 {
                    return Err(AppError::TooManyRequests(
                        "Limite diário de cadastros atingido. Tente novamente em algumas horas."
                            .into(),
                    ));
                }
            }

            // Derive usuario.
            let usuario = {
                let storage = lock_storage(&state);
                if let Some(ref preferred) = oc.preferred_usuario {
                    let p = preferred.trim().to_lowercase();
                    if !p.is_empty() && storage.get_user_by_usuario(&p).is_none() {
                        p
                    } else {
                        crate::storage::derive_usuario_from_email(&email_normalized, &storage)
                    }
                } else {
                    crate::storage::derive_usuario_from_email(&email_normalized, &storage)
                }
            };

            // INSERT into users (no password_hash).
            let id = format!("usr_{}", nanoid::nanoid!(10));
            let now_str = Utc::now().to_rfc3339();
            {
                let storage = lock_storage(&state);
                storage
                    .conn()
                    .execute(
                        "INSERT INTO users \
                         (id, usuario, email, display_name, tier, created_at, origin) \
                         VALUES (?1, ?2, ?3, ?2, 'player', ?4, ?5)",
                        rusqlite::params![id, usuario, email_normalized, now_str, oc.origin],
                    )
                    .map_err(|e| AppError::Internal(format!("INSERT users: {e}")))?;

                if let Err(e) = storage.subscribe_user_to_default_universes(&id) {
                    tracing::warn!("onboard-create: default subscriptions failed for {id}: {e}");
                }

                // CO-184 reverse bridge.
                if let Err(e) = storage.ensure_quilombo_user_for_co(&id) {
                    tracing::warn!(
                        "onboard-create: ensure_quilombo_user_for_co failed for {id}: {e}"
                    );
                }

                // ensure_email_recovery_channel so /forgot-password works.
                if let Err(e) = storage.ensure_email_recovery_channel(&id, &email_normalized) {
                    tracing::warn!(
                        "onboard-create: ensure_email_recovery_channel failed for {id}: {e}"
                    );
                }
            }

            tracing::info!(
                "onboard-create: created user {id} (usuario={usuario}) for email {}",
                redact_email(&email)
            );

            crate::telemetry::emit_crud_event(
                &state,
                crate::telemetry::CrudEvent {
                    kind: "auth.signup",
                    universe: String::new(),
                    list: Some("onboard-email".to_string()),
                    key: Some(id.clone()),
                    actor: Some(id.clone()),
                    session_id: None,
                    extra: None,
                },
            );

            let storage = lock_storage(&state);
            storage
                .get_user_by_id(&id)
                .ok_or_else(|| AppError::Internal("User not found after insert".into()))?
        }
    };

    // 5. Mark code consumed.
    {
        let storage = lock_storage(&state);
        storage.consume_onboarding_code(&oc.id)?;
    }

    // 6. Mint session JWT + set cookie.
    let jwt_secret = crate::auth::jwt_secret();
    let (token, expires_at) = crate::auth::sign_jwt(&user.id, &user.email, &user.tier, &jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let cookie = crate::auth::build_session_cookie(
        &token,
        state.core.config.cookie_domain.as_deref(),
        604800,
    );

    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.login",
            universe: String::new(),
            list: Some("onboard-email".to_string()),
            key: Some(user.id.clone()),
            actor: Some(user.id.clone()),
            session_id: None,
            extra: None,
        },
    );

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        axum::Json(OnboardVerifyResponse {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            expires_at,
            return_to: oc.return_to,
        }),
    )
        .into_response())
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests;
