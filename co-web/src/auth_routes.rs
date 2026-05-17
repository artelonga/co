//! CO-214 — Token exchange for cross-universe API calls.
//!
//! `POST /api/v1/auth/exchange-session` trades a valid short-lived ES256
//! handover JWT (CO-186, 60s TTL) or a session cookie for a 7-day ES256 JWT
//! that the caller can carry as `Authorization: Bearer …` on co's APIs.
//!
//! Use case: downstream universe runtimes (quilombo's SvelteKit `co-client`)
//! that receive the handover token via the SSO redirect but need to keep
//! making API calls past the 60s window, with no cookie-sharing possible
//! across distinct apex domains (`.artelonga.com.br` ↔ `quilomboaraucaria.com.br`).

use axum::{Json, Router, extract::State, http::HeaderMap, response::IntoResponse, routing::post};
use serde::Serialize;

use crate::auth::{decode_claims_any, extract_session_cookie, jwt_secret, sign_jwt_es256};
use crate::error::AppError;
use crate::server::AppState;

pub fn auth_routes_router() -> Router<AppState> {
    Router::new().route("/v1/auth/exchange-session", post(exchange_session_handler))
}

#[derive(Debug, Serialize)]
struct ExchangeSessionResponse {
    token: String,
    expires_at: String,
}

async fn exchange_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let token = extract_bearer_or_cookie(&headers)
        .ok_or_else(|| AppError::Unauthorized("Missing Bearer token or session cookie".into()))?;

    let claims = decode_claims_any(&token, &state.jwt_key, &jwt_secret())
        .map_err(|_| AppError::Unauthorized("Invalid or expired token".into()))?;

    // Reload user from storage so the new token reflects current email/tier,
    // not whatever was frozen in the incoming token. Also confirms the user
    // still exists (defends against tokens for since-deleted accounts).
    let user = {
        let storage = state.storage.lock();
        storage
            .get_user_by_id(&claims.sub)
            .ok_or_else(|| AppError::Unauthorized("User no longer exists".into()))?
    };

    let (new_token, expires_at) = sign_jwt_es256(&state.jwt_key, &user.id, &user.email, &user.tier)
        .map_err(|e| AppError::Internal(format!("failed to sign exchanged JWT: {e}")))?;

    Ok(Json(ExchangeSessionResponse {
        token: new_token,
        expires_at: expires_at.to_rfc3339(),
    }))
}

fn extract_bearer_or_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(headers))
}
