//! CO-177: Google OAuth 2.0 sign-in.
//!
//! Two endpoints, full OIDC code-flow:
//!
//! - `GET /api/v1/auth/google/start?return_to=<url>` — generates a state
//!   token (signed JWT containing the `return_to` and a nonce), redirects
//!   the browser to Google's consent screen.
//! - `GET /api/v1/auth/google/callback?code=&state=` — exchanges the code
//!   for a token, calls Google's userinfo endpoint, finds-or-creates a
//!   matching CO user (by `users.google_sub` first, then by email), sets
//!   the session cookie, redirects to the safelisted `return_to`.
//!
//! Env vars (set per deployment):
//! - `GOOGLE_CLIENT_ID` — OAuth client id from Google Cloud Console.
//! - `GOOGLE_CLIENT_SECRET` — OAuth client secret.
//! - `GOOGLE_REDIRECT_URI` — exact redirect URI registered with Google.
//!   Default: `https://co.artelonga.com.br/api/v1/auth/google/callback`.
//!
//! When env vars are missing, both endpoints return 503 — no half-broken
//! state. The login UI hides the "Continuar com Google" button when the
//! `/api/v1/auth/google/start` endpoint reports unconfigured.

use axum::{
    Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::server::AppState;

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const STATE_TTL_SECS: u64 = 600;

#[derive(Debug, Deserialize, Serialize)]
struct StateClaims {
    kind: String,
    return_to: String,
    nonce: String,
    exp: u64,
}

#[derive(Debug, Deserialize)]
pub struct StartParams {
    #[serde(default)]
    pub return_to: String,
}

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfo {
    sub: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    email_verified: bool,
    #[serde(default)]
    name: String,
}

struct GoogleConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

impl GoogleConfig {
    fn from_env() -> Option<Self> {
        let client_id = std::env::var("GOOGLE_CLIENT_ID").ok()?;
        let client_secret = std::env::var("GOOGLE_CLIENT_SECRET").ok()?;
        let redirect_uri = std::env::var("GOOGLE_REDIRECT_URI").unwrap_or_else(|_| {
            "https://co.artelonga.com.br/api/v1/auth/google/callback".to_string()
        });
        Some(Self {
            client_id,
            client_secret,
            redirect_uri,
        })
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/google/start", get(start_handler))
        .route("/google/callback", get(callback_handler))
}

/// `GET /api/v1/auth/google/start?return_to=<url>` — kick off the OAuth flow.
async fn start_handler(
    Query(params): Query<StartParams>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let cfg = GoogleConfig::from_env().ok_or_else(|| {
        AppError::ServiceUnavailable("Google sign-in is not configured on this deployment.".into())
    })?;

    // Validate return_to against the same safelist used for /recover (CO-176).
    let return_to = params.return_to.trim();
    if !return_to.is_empty() && !crate::recovery_routes::is_allowed_return_to(return_to) {
        return Err(AppError::BadRequest(
            "return_to host is not in the safelist".into(),
        ));
    }

    // Sign a state JWT carrying return_to + nonce + short expiry. The
    // callback verifies + extracts return_to from this — never trusts the
    // raw query param at callback time.
    let exp =
        (chrono::Utc::now() + chrono::Duration::seconds(STATE_TTL_SECS as i64)).timestamp() as u64;
    let claims = StateClaims {
        kind: "google_oauth_state".into(),
        return_to: return_to.to_string(),
        nonce: nanoid::nanoid!(16),
        exp,
    };
    let secret = crate::auth::jwt_secret();
    let token = jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("state sign: {e}")))?;

    let _ = &state; // state is only borrowed to keep the State extractor happy.

    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    let enc = |s: &str| utf8_percent_encode(s, NON_ALPHANUMERIC).to_string();
    let google_url = format!(
        "{auth}?client_id={cid}&redirect_uri={uri}&response_type=code&scope={scope}&state={state}&access_type=online&prompt=select_account",
        auth = GOOGLE_AUTH_URL,
        cid = enc(&cfg.client_id),
        uri = enc(&cfg.redirect_uri),
        scope = enc("openid email profile"),
        state = enc(&token),
    );
    Ok(Redirect::to(&google_url).into_response())
}

/// `GET /api/v1/auth/google/callback?code=&state=` — exchange code, finish login.
async fn callback_handler(
    Query(params): Query<CallbackParams>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if !params.error.is_empty() {
        return Err(AppError::BadRequest(format!(
            "Google sign-in canceled or failed: {}",
            params.error
        )));
    }
    if params.code.is_empty() || params.state.is_empty() {
        return Err(AppError::BadRequest(
            "Missing code or state from Google callback".into(),
        ));
    }

    let cfg = GoogleConfig::from_env().ok_or_else(|| {
        AppError::ServiceUnavailable("Google sign-in is not configured on this deployment.".into())
    })?;

    // 1. Verify the state JWT.
    let secret = crate::auth::jwt_secret();
    let claims: StateClaims = jsonwebtoken::decode::<StateClaims>(
        &params.state,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| AppError::Unauthorized("State is invalid or expired".into()))?
    .claims;

    if claims.kind != "google_oauth_state" {
        return Err(AppError::Unauthorized("State has wrong kind".into()));
    }
    let return_to = if claims.return_to.is_empty()
        || !crate::recovery_routes::is_allowed_return_to(&claims.return_to)
    {
        "/".to_string()
    } else {
        claims.return_to
    };

    // 2. Exchange code → access_token.
    let client = reqwest::Client::new();
    let token_resp: TokenResponse = client
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("code", params.code.as_str()),
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("redirect_uri", cfg.redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Google token exchange: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Unauthorized(format!("Google token rejected: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Google token parse: {e}")))?;

    // 3. Fetch userinfo (sub, email, name).
    let userinfo: GoogleUserInfo = client
        .get(GOOGLE_USERINFO_URL)
        .bearer_auth(&token_resp.access_token)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Google userinfo: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Unauthorized(format!("Google userinfo rejected: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Google userinfo parse: {e}")))?;

    if !userinfo.email_verified {
        return Err(AppError::Unauthorized(
            "Google account email is not verified.".into(),
        ));
    }
    if userinfo.email.is_empty() || userinfo.sub.is_empty() {
        return Err(AppError::Unauthorized(
            "Google userinfo missing email or sub.".into(),
        ));
    }

    // 4. Find or create CO user.
    let user = {
        let mut storage = state
            .storage
            .lock()
            .map_err(|_| AppError::Internal("storage lock".into()))?;
        storage.find_or_create_user_by_google(&userinfo.sub, &userinfo.email, &userinfo.name)
    };
    let user = user.map_err(|e| AppError::Internal(format!("link google user: {e}")))?;

    // CO-184 reverse bridge: ensure a quilombo identity exists too. Best-effort —
    // a failure here doesn't block sign-in (user can still use CO routes).
    {
        let storage = state
            .storage
            .lock()
            .map_err(|_| AppError::Internal("storage lock".into()))?;
        if let Err(e) = storage.ensure_quilombo_user_for_co(&user.id) {
            tracing::warn!(
                "CO-184 ensure_quilombo_user_for_co failed for {} (sign-in continues): {e}",
                user.id
            );
        }
    }

    // 5. Issue session cookie.
    let (session_token, _expires_at) = crate::auth::sign_jwt(
        &user.id,
        &user.email,
        &user.tier,
        &crate::auth::jwt_secret(),
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;
    let cookie = crate::auth::build_session_cookie(
        &session_token,
        state.config.cookie_domain.as_deref(),
        604800,
    );

    // Telemetry — same shape as password-login + signup.
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.login",
            universe: String::new(),
            list: Some("google".to_string()),
            key: Some(user.id.clone()),
            actor: Some(user.id.clone()),
            session_id: None,
            extra: None,
        },
    );

    // CO-186: when bouncing to a cross-apex `/auth/co-handover` endpoint,
    // append a short-lived ES256-signed `co_token` so the receiving
    // deployment can mint its own session cookie. Receivers validate via
    // CO's JWKS endpoint — no shared secret. Cookie still set on
    // co.artelonga.com.br so the user is also logged in here.
    let final_redirect = crate::auth::maybe_attach_co_handover_token(
        &return_to,
        &user.id,
        &user.email,
        &user.tier,
        &state.jwt_key,
    );

    use axum::http::HeaderValue;
    let cookie_hv = HeaderValue::from_str(&cookie)
        .map_err(|e| AppError::Internal(format!("cookie header: {e}")))?;
    let location_hv = HeaderValue::from_str(&final_redirect)
        .map_err(|e| AppError::Internal(format!("location header: {e}")))?;
    Ok((
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie_hv),
            (header::LOCATION, location_hv),
        ],
        (),
    )
        .into_response())
}
