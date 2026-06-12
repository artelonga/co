//! CO-415: GitHub OAuth 2.0 sign-in.
//!
//! The GitHub twin of `oauth_google.rs` (CO-177). Two endpoints, OAuth
//! authorization-code flow:
//!
//! - `GET /api/v1/auth/github/start?return_to=<url>` — generates a state
//!   token (signed JWT containing the `return_to` + nonce + origin + expiry),
//!   redirects the browser to GitHub's consent screen.
//! - `GET /api/v1/auth/github/callback?code=&state=` — exchanges the code
//!   for a token, calls GitHub's `/user` + `/user/emails` endpoints, finds the
//!   primary verified email, finds-or-creates a matching CO user (by
//!   `users.github_login` first, then by email), sets the session cookie,
//!   redirects to the safelisted `return_to`.
//!
//! Env vars (set per deployment — DISTINCT from any admin PAT):
//! - `GITHUB_OAUTH_CLIENT_ID` — OAuth app client id from GitHub.
//! - `GITHUB_OAUTH_CLIENT_SECRET` — OAuth app client secret.
//! - `GITHUB_OAUTH_REDIRECT_URI` — exact redirect URI registered with GitHub.
//!   Default: `https://co.artelonga.com.br/api/v1/auth/github/callback`.
//!
//! When env vars are missing, both endpoints return 503 — no half-broken
//! state. The login UI hides the "Entrar com GitHub" button when the
//! `/api/v1/auth/github/start` endpoint reports unconfigured.

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

/// GitHub OAuth token-exchange error body, e.g.
/// `{"error":"bad_verification_code","error_description":"..."}`.
#[derive(Debug, Deserialize)]
struct GithubTokenError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

/// Render GitHub's token-error JSON into a compact, log-safe message.
///
/// Returns `"<error>: <error_description>"` when both fields are present,
/// `"<error>"` when only the code is, and falls back to the raw (trimmed,
/// length-capped) body when it is not the expected JSON shape. Mirrors the
/// CO-408 hardening on the Google path.
fn format_token_error_body(body: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<GithubTokenError>(body)
        && !parsed.error.is_empty()
    {
        if parsed.error_description.is_empty() {
            return parsed.error;
        }
        return format!("{}: {}", parsed.error, parsed.error_description);
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "(empty response body)".to_string();
    }
    // Cap the raw fallback so a giant/HTML error page can't flood the log.
    // Truncate on a char boundary to avoid slicing mid-codepoint.
    if trimmed.len() > 300 {
        let mut end = 300;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &trimmed[..end])
    } else {
        trimmed.to_string()
    }
}

const GITHUB_AUTH_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_URL: &str = "https://api.github.com/user";
const GITHUB_EMAILS_URL: &str = "https://api.github.com/user/emails";
const GITHUB_SCOPE: &str = "read:user user:email";
/// GitHub requires a non-empty, descriptive User-Agent on every API request.
const GITHUB_USER_AGENT: &str = "co-artelonga-oauth";
const STATE_TTL_SECS: u64 = 600;

#[derive(Debug, Deserialize, Serialize)]
struct StateClaims {
    kind: String,
    return_to: String,
    nonce: String,
    exp: u64,
    #[serde(default)]
    origin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StartParams {
    #[serde(default)]
    pub return_to: String,
    #[serde(default)]
    pub origin: Option<String>,
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

/// `GET /user` — we only need the stable `login`, plus optional name/email.
#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

/// One entry from `GET /user/emails`.
#[derive(Debug, Deserialize)]
struct GithubEmail {
    email: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    verified: bool,
}

/// Pick the email to associate with the CO account: the primary *verified*
/// address if present, otherwise the first verified address. Returns `None`
/// when no verified email exists (caller maps that to 401).
fn pick_verified_email(emails: &[GithubEmail]) -> Option<String> {
    emails
        .iter()
        .find(|e| e.primary && e.verified)
        .or_else(|| emails.iter().find(|e| e.verified))
        .map(|e| e.email.clone())
}

struct GithubConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

impl GithubConfig {
    fn from_env() -> Option<Self> {
        let client_id = std::env::var("GITHUB_OAUTH_CLIENT_ID").ok()?;
        let client_secret = std::env::var("GITHUB_OAUTH_CLIENT_SECRET").ok()?;
        let redirect_uri = std::env::var("GITHUB_OAUTH_REDIRECT_URI").unwrap_or_else(|_| {
            "https://co.artelonga.com.br/api/v1/auth/github/callback".to_string()
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
        .route("/github/start", get(start_handler))
        .route("/github/callback", get(callback_handler))
}

/// `GET /api/v1/auth/github/start?return_to=<url>` — kick off the OAuth flow.
async fn start_handler(
    Query(params): Query<StartParams>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let cfg = GithubConfig::from_env().ok_or_else(|| {
        AppError::ServiceUnavailable("GitHub sign-in is not configured on this deployment.".into())
    })?;

    // Validate return_to against the same safelist used for /recover (CO-176).
    let return_to = params.return_to.trim();
    if !return_to.is_empty() && !crate::recovery_routes::is_allowed_return_to(return_to) {
        return Err(AppError::BadRequest(
            "return_to host is not in the safelist".into(),
        ));
    }

    // Sign a state JWT carrying return_to + nonce + origin + short expiry.
    // The callback verifies + extracts fields from this — never trusts raw
    // query params at callback time.
    let exp =
        (chrono::Utc::now() + chrono::Duration::seconds(STATE_TTL_SECS as i64)).timestamp() as u64;
    let origin = crate::auth::sanitize_origin(params.origin);
    let claims = StateClaims {
        kind: "github_oauth_state".into(),
        return_to: return_to.to_string(),
        nonce: nanoid::nanoid!(16),
        exp,
        origin,
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
    let github_url = format!(
        "{auth}?client_id={cid}&redirect_uri={uri}&scope={scope}&state={state}&allow_signup=true",
        auth = GITHUB_AUTH_URL,
        cid = enc(&cfg.client_id),
        uri = enc(&cfg.redirect_uri),
        scope = enc(GITHUB_SCOPE),
        state = enc(&token),
    );
    Ok(Redirect::to(&github_url).into_response())
}

/// `GET /api/v1/auth/github/callback?code=&state=` — exchange code, finish login.
async fn callback_handler(
    Query(params): Query<CallbackParams>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if !params.error.is_empty() {
        return Err(AppError::BadRequest(format!(
            "GitHub sign-in canceled or failed: {}",
            params.error
        )));
    }
    if params.code.is_empty() || params.state.is_empty() {
        return Err(AppError::BadRequest(
            "Missing code or state from GitHub callback".into(),
        ));
    }

    let cfg = GithubConfig::from_env().ok_or_else(|| {
        AppError::ServiceUnavailable("GitHub sign-in is not configured on this deployment.".into())
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

    if claims.kind != "github_oauth_state" {
        return Err(AppError::Unauthorized("State has wrong kind".into()));
    }
    let return_to = if claims.return_to.is_empty()
        || !crate::recovery_routes::is_allowed_return_to(&claims.return_to)
    {
        "/".to_string()
    } else {
        claims.return_to
    };

    // 2. Exchange code → access_token. GitHub returns form-encoded by default;
    //    `Accept: application/json` makes it return the JSON `TokenResponse`.
    let client = reqwest::Client::new();
    let resp = client
        .post(GITHUB_TOKEN_URL)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, GITHUB_USER_AGENT)
        .form(&[
            ("code", params.code.as_str()),
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("redirect_uri", cfg.redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub token exchange: {e}")))?;

    // On non-2xx, surface the body in the 401 + WARN-log it (mirrors CO-408).
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let detail = format_token_error_body(&body);
        tracing::warn!(
            "CO-415 GitHub token exchange rejected: HTTP {status} — {detail}",
            status = status.as_u16(),
        );
        return Err(AppError::Unauthorized(format!(
            "GitHub token rejected ({status}): {detail}",
            status = status.as_u16(),
        )));
    }

    // GitHub also returns HTTP 200 with `{"error":"bad_verification_code",...}`
    // when the code is bad. Read the body once and detect that shape.
    let token_body = resp
        .text()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub token read: {e}")))?;
    let token_resp: TokenResponse =
        serde_json::from_str::<TokenResponse>(&token_body).map_err(|_| {
            let detail = format_token_error_body(&token_body);
            tracing::warn!("CO-415 GitHub token exchange error body: {detail}");
            AppError::Unauthorized(format!("GitHub token rejected: {detail}"))
        })?;

    // 3. Fetch the GitHub user (stable `login`, name, public email).
    let gh_user: GithubUser = client
        .get(GITHUB_USER_URL)
        .bearer_auth(&token_resp.access_token)
        .header(header::USER_AGENT, GITHUB_USER_AGENT)
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub user: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Unauthorized(format!("GitHub user rejected: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub user parse: {e}")))?;

    if gh_user.login.trim().is_empty() {
        return Err(AppError::Unauthorized("GitHub user missing login.".into()));
    }

    // 4. Fetch verified emails — the primary verified address is the CO email.
    //    `user:email` scope is required; the public `/user.email` alone is not
    //    enough (it can be null, and isn't a verification signal).
    let emails: Vec<GithubEmail> = client
        .get(GITHUB_EMAILS_URL)
        .bearer_auth(&token_resp.access_token)
        .header(header::USER_AGENT, GITHUB_USER_AGENT)
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub emails: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Unauthorized(format!("GitHub emails rejected: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub emails parse: {e}")))?;

    let email = pick_verified_email(&emails)
        .ok_or_else(|| AppError::Unauthorized("GitHub account has no verified email.".into()))?;

    let name = gh_user
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&gh_user.login)
        .to_string();
    let _ = &gh_user.email; // public email field is unused — we trust verified list.

    // 5. Find or create CO user.
    let user = {
        let mut storage = state.core.storage.lock();
        storage.find_or_create_user_by_github(
            &gh_user.login,
            &email,
            &name,
            claims.origin.as_deref(),
        )
    };
    let user = user.map_err(|e| AppError::Internal(format!("link github user: {e}")))?;

    // CO-184 reverse bridge: ensure a quilombo identity exists too. Best-effort —
    // a failure here doesn't block sign-in.
    {
        let storage = state.core.storage.lock();
        if let Err(e) = storage.ensure_quilombo_user_for_co(&user.id) {
            tracing::warn!(
                "CO-184 ensure_quilombo_user_for_co failed for {} (sign-in continues): {e}",
                user.id
            );
        }
    }

    // 6. Issue session cookie.
    let session_token = state
        .core
        .auth_provider
        .issue_token(
            crate::infra::auth::UserClaims {
                user_id: user.id.clone(),
                email: user.email.clone(),
                tier: user.tier.clone(),
                usuario: String::new(),
                papel: String::new(),
            },
            crate::infra::auth::DEFAULT_TOKEN_TTL,
        )
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let cookie = crate::auth::build_session_cookie(
        session_token.as_str(),
        state.core.config.cookie_domain.as_deref(),
        604800,
    );

    // Telemetry — same shape as the Google path.
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.login",
            universe: String::new(),
            list: Some("github".to_string()),
            key: Some(user.id.clone()),
            actor: Some(user.id.clone()),
            session_id: None,
            extra: None,
        },
    );

    // CO-186: cross-apex handover token when bouncing to /auth/co-handover.
    let final_redirect = crate::auth::maybe_attach_co_handover_token(
        &return_to,
        &user.id,
        &user.email,
        &user.tier,
        &state.integrations.jwt_key,
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

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};

    /// State JWT round-trips: encode → decode preserves all claims, and the
    /// kind tag is `github_oauth_state` (distinct from Google's).
    #[test]
    fn state_jwt_round_trip() {
        let secret = "test-secret";
        let exp = (chrono::Utc::now() + chrono::Duration::seconds(STATE_TTL_SECS as i64))
            .timestamp() as u64;
        let claims = StateClaims {
            kind: "github_oauth_state".into(),
            return_to: "https://quilomboaraucaria.org/x".into(),
            nonce: "abc123".into(),
            exp,
            origin: Some("https://quilomboaraucaria.org".into()),
        };
        let token = jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();
        let decoded: StateClaims = jsonwebtoken::decode::<StateClaims>(
            &token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        )
        .unwrap()
        .claims;
        assert_eq!(decoded.kind, "github_oauth_state");
        assert_eq!(decoded.return_to, "https://quilomboaraucaria.org/x");
        assert_eq!(decoded.nonce, "abc123");
        assert_eq!(
            decoded.origin.as_deref(),
            Some("https://quilomboaraucaria.org")
        );
    }

    /// A state signed with one secret must NOT verify under a different secret.
    #[test]
    fn state_jwt_rejects_wrong_secret() {
        let exp = (chrono::Utc::now() + chrono::Duration::seconds(STATE_TTL_SECS as i64))
            .timestamp() as u64;
        let claims = StateClaims {
            kind: "github_oauth_state".into(),
            return_to: "/".into(),
            nonce: "n".into(),
            exp,
            origin: None,
        };
        let token = jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(b"secret-a"),
        )
        .unwrap();
        let res = jsonwebtoken::decode::<StateClaims>(
            &token,
            &DecodingKey::from_secret(b"secret-b"),
            &Validation::default(),
        );
        assert!(res.is_err());
    }

    #[test]
    fn pick_primary_verified_email() {
        let emails = vec![
            GithubEmail {
                email: "alt@example.com".into(),
                primary: false,
                verified: true,
            },
            GithubEmail {
                email: "primary@example.com".into(),
                primary: true,
                verified: true,
            },
        ];
        assert_eq!(
            pick_verified_email(&emails).as_deref(),
            Some("primary@example.com")
        );
    }

    #[test]
    fn pick_falls_back_to_first_verified_when_no_primary() {
        let emails = vec![
            GithubEmail {
                email: "unverified@example.com".into(),
                primary: true,
                verified: false,
            },
            GithubEmail {
                email: "verified@example.com".into(),
                primary: false,
                verified: true,
            },
        ];
        assert_eq!(
            pick_verified_email(&emails).as_deref(),
            Some("verified@example.com")
        );
    }

    #[test]
    fn pick_none_when_no_verified_email() {
        let emails = vec![GithubEmail {
            email: "x@example.com".into(),
            primary: true,
            verified: false,
        }];
        assert_eq!(pick_verified_email(&emails), None);
    }

    #[test]
    fn token_error_bad_verification_code_is_readable() {
        let body = r#"{"error":"bad_verification_code","error_description":"The code passed is incorrect."}"#;
        assert_eq!(
            format_token_error_body(body),
            "bad_verification_code: The code passed is incorrect."
        );
    }

    #[test]
    fn token_error_non_json_falls_back_to_raw() {
        let body = "  <html>500</html>  ";
        assert_eq!(format_token_error_body(body), "<html>500</html>");
    }

    #[test]
    fn token_error_empty_body() {
        assert_eq!(format_token_error_body("   "), "(empty response body)");
    }
}
