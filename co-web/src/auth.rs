use std::path::Path;

use axum::{
    Json,
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use redb::{Database, TableDefinition};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const VERIFY_CODES: TableDefinition<&str, &[u8]> = TableDefinition::new("verify_codes");
const RATE_LIMITS: TableDefinition<&str, &[u8]> = TableDefinition::new("rate_limits");

/// Rate limit: max 3 requests per 15 minutes.
const RATE_LIMIT_MAX: usize = 3;
const RATE_LIMIT_WINDOW_SECS: i64 = 900; // 15 minutes

/// Code expiry: 5 minutes.
const CODE_EXPIRY_SECS: i64 = 300;

/// JWT expiry: 7 days.
const JWT_EXPIRY_SECS: i64 = 604_800;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyCodeEntry {
    pub code: String,
    pub user_id: Option<String>,
    pub expires_at: DateTime<Utc>,
    /// Remaining attempts (starts at 3).
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitEntry {
    /// Unix timestamps of recent requests.
    pub requests: Vec<i64>,
}

/// JWT claims shared across auth flows.
///
/// Supports both legacy (email+tier) and unified (usuario+papel) flows.
/// All fields except `sub`, `exp`, `iat` have defaults for backwards compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub usuario: String,
    #[serde(default)]
    pub papel: String,
    pub exp: usize,
    pub iat: usize,
}

/// User ID extracted from JWT auth middleware.
///
/// Can be used as an Axum extractor on routes behind `require_auth` middleware.
#[derive(Clone, Debug)]
pub struct UserId(pub String);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for UserId {
    type Rejection = axum::response::Response;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = parts
            .extensions
            .get::<UserId>()
            .cloned()
            .ok_or_else(|| unauthorized("Not authenticated"));
        std::future::ready(result)
    }
}

/// JSON error body returned by the auth middleware.
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

/// Reads `JWT_SECRET` from the environment, falling back to a development default.
pub fn jwt_secret() -> String {
    std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".into())
}

/// Resolve the calling user_id from a request's headers, accepting either:
///   - Bearer JWT (or session cookie)
///   - Long-lived API token (CO-35) — looked up in the `api_tokens` table
///
/// Used by handlers that don't sit behind the JWT-only `require_auth`
/// middleware but still need to identify the caller (e.g., universe
/// duplicate, future co-tools API). Returns `None` if no valid auth is
/// present.
pub fn resolve_user_id(
    state: &crate::server::AppState,
    headers: &axum::http::HeaderMap,
) -> Option<String> {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(headers))?;

    // Try JWT first.
    let secret = jwt_secret();
    let validation = Validation::new(Algorithm::HS256);
    if let Ok(data) = decode::<Claims>(
        &bearer,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    ) {
        return Some(data.claims.sub);
    }

    // Fall back to API token via storage.
    let storage = state.storage.lock().ok()?;
    storage
        .get_api_token_by_value(&bearer)
        .ok()
        .flatten()
        .map(|tok| tok.user_id)
}

/// Extracts the session token from the `Cookie` header, if present.
pub fn extract_session_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(token) = part.strip_prefix("session=") {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// Axum middleware that validates a Bearer JWT or `session` cookie and injects
/// [`UserId`] into request extensions. Returns 401 with a JSON error on failure.
///
/// JWT-only — does NOT accept API tokens. For routes that should accept API
/// tokens too (e.g., paths a long-lived background worker hits), use
/// `require_auth_with_token` (state-aware variant).
pub async fn require_auth(mut req: Request<Body>, next: Next) -> Result<Response, Response> {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(req.headers()))
        .ok_or_else(|| unauthorized("Missing or malformed Authorization header"))?;

    let secret = jwt_secret();
    let validation = Validation::new(Algorithm::HS256);

    let token_data = decode::<Claims>(
        &token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => unauthorized("Token expired"),
        jsonwebtoken::errors::ErrorKind::InvalidSignature => {
            unauthorized("Invalid token signature")
        }
        _ => unauthorized("Invalid token"),
    })?;

    req.extensions_mut().insert(UserId(token_data.claims.sub));
    Ok(next.run(req).await)
}

/// Like [`require_auth`] but also accepts API tokens (CO-35) as a fallback
/// after JWT validation fails. Used for endpoints background workers hit
/// (CO-82 mirror, future external integrations) where a 7-day JWT is
/// inadequate.
///
/// Mount via `axum::middleware::from_fn_with_state(state.clone(), require_auth_with_token)`.
pub async fn require_auth_with_token(
    axum::extract::State(state): axum::extract::State<crate::server::AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(req.headers()))
        .ok_or_else(|| unauthorized("Missing or malformed Authorization header"))?;

    let secret = jwt_secret();
    let validation = Validation::new(Algorithm::HS256);
    if let Ok(data) = decode::<Claims>(
        &token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    ) {
        req.extensions_mut().insert(UserId(data.claims.sub));
        return Ok(next.run(req).await);
    }

    // Lookup + immediately drop the lock before the next.run().await below.
    let user_id = {
        let storage = state
            .storage
            .lock()
            .map_err(|_| unauthorized("Storage lock failed"))?;
        match storage.get_api_token_by_value(&token) {
            Ok(Some(tok)) => tok.user_id.clone(),
            _ => return Err(unauthorized("Invalid or expired token")),
        }
    };
    req.extensions_mut().insert(UserId(user_id));
    Ok(next.run(req).await)
}

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
        .into_response()
}

pub struct AuthStore {
    db: Database,
}

impl AuthStore {
    pub fn new(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("auth.redb");
        let db = Database::create(&db_path)?;

        // Ensure tables exist.
        let write_txn = db.begin_write()?;
        {
            write_txn.open_table(VERIFY_CODES)?;
            write_txn.open_table(RATE_LIMITS)?;
        }
        write_txn.commit()?;

        Ok(Self { db })
    }

    pub fn get_code(&self, email: &str) -> anyhow::Result<Option<VerifyCodeEntry>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(VERIFY_CODES)?;
        let key = format!("verify:{email}");
        match table.get(key.as_str())? {
            Some(bytes) => {
                let entry: VerifyCodeEntry = serde_json::from_slice(bytes.value())?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    pub fn store_code(&self, email: &str, entry: &VerifyCodeEntry) -> anyhow::Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(VERIFY_CODES)?;
            let key = format!("verify:{email}");
            let bytes = serde_json::to_vec(entry)?;
            table.insert(key.as_str(), bytes.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn delete_code(&self, email: &str) -> anyhow::Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(VERIFY_CODES)?;
            let key = format!("verify:{email}");
            table.remove(key.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Returns `true` if the email is within the rate limit (has not exceeded it).
    pub fn check_rate_limit(&self, email: &str) -> anyhow::Result<bool> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(RATE_LIMITS)?;
        let key = format!("rate:{email}");
        match table.get(key.as_str())? {
            Some(bytes) => {
                let entry: RateLimitEntry = serde_json::from_slice(bytes.value())?;
                let now = Utc::now().timestamp();
                let cutoff = now - RATE_LIMIT_WINDOW_SECS;
                let recent = entry.requests.iter().filter(|&&ts| ts >= cutoff).count();
                Ok(recent < RATE_LIMIT_MAX)
            }
            None => Ok(true),
        }
    }

    /// Records a new code request timestamp for rate limiting.
    pub fn record_request(&self, email: &str) -> anyhow::Result<()> {
        let now = Utc::now().timestamp();
        let cutoff = now - RATE_LIMIT_WINDOW_SECS;

        // Read existing entry.
        let mut requests = {
            let read_txn = self.db.begin_read()?;
            let table = read_txn.open_table(RATE_LIMITS)?;
            let key = format!("rate:{email}");
            match table.get(key.as_str())? {
                Some(bytes) => {
                    let entry: RateLimitEntry = serde_json::from_slice(bytes.value())?;
                    entry.requests
                }
                None => vec![],
            }
        };

        // Prune old timestamps and add the new one.
        requests.retain(|&ts| ts >= cutoff);
        requests.push(now);

        let entry = RateLimitEntry { requests };
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(RATE_LIMITS)?;
            let key = format!("rate:{email}");
            let bytes = serde_json::to_vec(&entry)?;
            table.insert(key.as_str(), bytes.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }
}

/// Generates a random 6-digit numeric code.
pub fn generate_code() -> String {
    let bytes = Uuid::new_v4();
    let n = u32::from_le_bytes(bytes.as_bytes()[..4].try_into().unwrap());
    format!("{:06}", n % 1_000_000)
}

/// Signs a JWT for the given user. Returns (token, expires_at).
pub fn sign_jwt(
    user_id: &str,
    email: &str,
    tier: &str,
    secret: &str,
) -> anyhow::Result<(String, DateTime<Utc>)> {
    let now = Utc::now();
    let iat = now.timestamp() as usize;
    let exp = (now.timestamp() + JWT_EXPIRY_SECS) as usize;
    let expires_at = now + chrono::Duration::seconds(JWT_EXPIRY_SECS);

    let claims = Claims {
        sub: user_id.to_string(),
        email: email.to_string(),
        tier: tier.to_string(),
        usuario: String::new(),
        papel: String::new(),
        exp,
        iat,
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok((token, expires_at))
}

/// Signs a unified JWT for quilombo community auth. Returns (token, expires_at).
pub fn sign_jwt_quilombo(
    user_id: &str,
    usuario: &str,
    papel: &str,
    secret: &str,
) -> anyhow::Result<(String, DateTime<Utc>)> {
    let now = Utc::now();
    let iat = now.timestamp() as usize;
    let exp = (now.timestamp() + JWT_EXPIRY_SECS) as usize;
    let expires_at = now + chrono::Duration::seconds(JWT_EXPIRY_SECS);

    let claims = Claims {
        sub: user_id.to_string(),
        email: String::new(),
        tier: papel.to_string(), // tier maps to papel for backwards compat
        usuario: usuario.to_string(),
        papel: papel.to_string(),
        exp,
        iat,
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok((token, expires_at))
}

/// Decode a JWT token and return the subject (user ID) if valid.
pub fn decode_user_id(token: &str, secret: &str) -> anyhow::Result<String> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(data.claims.sub)
}

/// Decode a JWT token and return the full claims if valid.
pub fn decode_claims(token: &str, secret: &str) -> anyhow::Result<Claims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(data.claims)
}

/// Creates a new VerifyCodeEntry for the given user.
pub fn new_code_entry(user_id: Option<String>, code: String) -> VerifyCodeEntry {
    VerifyCodeEntry {
        code,
        user_id,
        expires_at: Utc::now() + chrono::Duration::seconds(CODE_EXPIRY_SECS),
        attempts: 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_code_is_six_digits() {
        for _ in 0..20 {
            let code = generate_code();
            assert_eq!(code.len(), 6, "code should be 6 characters: {code}");
            assert!(
                code.chars().all(|c| c.is_ascii_digit()),
                "code should be all digits: {code}"
            );
        }
    }

    #[test]
    fn test_auth_store_code_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let store = AuthStore::new(tmp.path()).unwrap();

        // Initially no code.
        assert!(store.get_code("user@example.com").unwrap().is_none());

        // Store a code.
        let entry = new_code_entry(Some("user-123".to_string()), "123456".to_string());
        store.store_code("user@example.com", &entry).unwrap();

        // Retrieve it.
        let retrieved = store.get_code("user@example.com").unwrap().unwrap();
        assert_eq!(retrieved.code, "123456");
        assert_eq!(retrieved.user_id, Some("user-123".to_string()));
        assert_eq!(retrieved.attempts, 3);

        // Delete it.
        store.delete_code("user@example.com").unwrap();
        assert!(store.get_code("user@example.com").unwrap().is_none());
    }

    #[test]
    fn test_rate_limit() {
        let tmp = TempDir::new().unwrap();
        let store = AuthStore::new(tmp.path()).unwrap();
        let email = "ratelimit@example.com";

        // Initially within limit.
        assert!(store.check_rate_limit(email).unwrap());

        // Record 3 requests — still at the limit boundary.
        store.record_request(email).unwrap();
        assert!(store.check_rate_limit(email).unwrap());

        store.record_request(email).unwrap();
        assert!(store.check_rate_limit(email).unwrap());

        store.record_request(email).unwrap();
        // Now 3 requests recorded — should be exceeded.
        assert!(!store.check_rate_limit(email).unwrap());
    }
}
