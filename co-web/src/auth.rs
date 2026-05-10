use std::path::Path;

use axum::{
    Json,
    body::Body,
    http::{Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use p256::ecdsa::SigningKey;
use p256::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use redb::{Database, TableDefinition};
use rusqlite;
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

// ---------------------------------------------------------------------------
// CO-166: ES256 asymmetric JWT key (used for OIDC + cross-domain SSO)
// ---------------------------------------------------------------------------

/// EC P-256 key pair used for ES256 JWT signing and the JWKS endpoint.
///
/// Keys are loaded from `CO_JWT_PRIVATE_KEY` (PEM) or generated fresh each
/// boot. On boot-generated keys, tokens are only valid for the lifetime of
/// the process — this is fine for dev. In production, set `CO_JWT_PRIVATE_KEY`
/// so the key survives restarts and multiple instances can verify tokens.
pub struct JwtKey {
    /// Key ID — included in JWTs so verifiers can select the right JWK.
    pub kid: String,
    signing_pem: String,
    verifying_pem: String,
    x_bytes: Vec<u8>,
    y_bytes: Vec<u8>,
}

impl JwtKey {
    /// Load from `CO_JWT_PRIVATE_KEY` env var (PKCS8 PEM) or generate a new key.
    pub fn load_or_generate() -> Self {
        if let Ok(pem) = std::env::var("CO_JWT_PRIVATE_KEY") {
            if let Ok(key) = Self::from_pem(&pem) {
                tracing::info!("CO-166: loaded EC key from CO_JWT_PRIVATE_KEY");
                return key;
            }
            tracing::warn!("CO-166: CO_JWT_PRIVATE_KEY set but invalid PEM, generating new key");
        }
        let key = Self::generate();
        tracing::info!("CO-166: generated fresh EC P-256 key (kid={})", key.kid);
        key
    }

    fn generate() -> Self {
        let signing_key = SigningKey::random(&mut rand::thread_rng());
        Self::from_signing_key(signing_key)
    }

    fn from_pem(pem: &str) -> anyhow::Result<Self> {
        use p256::pkcs8::DecodePrivateKey;
        let signing_key = SigningKey::from_pkcs8_pem(pem)?;
        Ok(Self::from_signing_key(signing_key))
    }

    fn from_signing_key(signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        let private_pem = signing_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("PKCS8 PEM encode failed")
            .to_string();
        let public_pem = verifying_key
            .to_public_key_pem(LineEnding::LF)
            .expect("SubjectPublicKeyInfo PEM encode failed");

        let point = verifying_key.to_encoded_point(false); // uncompressed (x, y)
        let x_bytes = point.x().expect("EC point x coordinate").to_vec();
        let y_bytes = point.y().expect("EC point y coordinate").to_vec();

        // kid = first 16 hex chars of blake3 of x_bytes||y_bytes
        let mut combined = x_bytes.clone();
        combined.extend_from_slice(&y_bytes);
        let hash = blake3::hash(&combined);
        // Format first 8 bytes as lowercase hex (16 chars).
        let kid = hash.as_bytes()[..8].iter().fold(String::new(), |mut s, b| {
            s.push_str(&format!("{b:02x}"));
            s
        });

        Self {
            kid,
            signing_pem: private_pem,
            verifying_pem: public_pem,
            x_bytes,
            y_bytes,
        }
    }

    /// Build an `EncodingKey` for signing JWTs with ES256.
    pub fn encoding_key(&self) -> anyhow::Result<EncodingKey> {
        Ok(EncodingKey::from_ec_pem(self.signing_pem.as_bytes())?)
    }

    /// Build a `DecodingKey` for verifying ES256 JWTs.
    pub fn decoding_key(&self) -> anyhow::Result<DecodingKey> {
        Ok(DecodingKey::from_ec_pem(self.verifying_pem.as_bytes())?)
    }

    /// Build the JWK representation of the public key for `/.well-known/jwks.json`.
    pub fn public_jwk(&self) -> serde_json::Value {
        let x_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&self.x_bytes);
        let y_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&self.y_bytes);
        serde_json::json!({
            "kty": "EC",
            "crv": "P-256",
            "use": "sig",
            "alg": "ES256",
            "kid": self.kid,
            "x": x_b64,
            "y": y_b64,
        })
    }
}

/// Sign a JWT using ES256 (asymmetric). Returns (token, expires_at).
///
/// Used for OIDC flows and cross-domain tokens where the verifier can fetch
/// the public key from JWKS rather than sharing a secret.
pub fn sign_jwt_es256(
    key: &JwtKey,
    user_id: &str,
    email: &str,
    tier: &str,
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

    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key.kid.clone());

    let token = encode(&header, &claims, &key.encoding_key()?)?;
    Ok((token, expires_at))
}

/// Decode and validate a JWT, trying ES256 first then falling back to HS256.
///
/// This lets the server accept both old HS256 session tokens and new ES256
/// tokens transparently during the migration period.
pub fn decode_claims_any(
    token: &str,
    jwt_key: &JwtKey,
    hs256_secret: &str,
) -> anyhow::Result<Claims> {
    // Try ES256 first.
    if let Ok(decoding_key) = jwt_key.decoding_key() {
        let mut validation = Validation::new(Algorithm::ES256);
        validation.validate_exp = true;
        if let Ok(data) = decode::<Claims>(token, &decoding_key, &validation) {
            return Ok(data.claims);
        }
    }

    // Fall back to HS256.
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(hs256_secret.as_bytes()),
        &validation,
    )?;
    Ok(data.claims)
}

/// Build a `Set-Cookie` header value for the session token.
///
/// When `domain` is `Some(".artelonga.com.br")` the cookie is shared across
/// all subdomains. When `None` (dev default), no `Domain` attribute is added
/// and the cookie only works for the current host.
pub fn build_session_cookie(token: &str, domain: Option<&str>, max_age: u64) -> String {
    let mut cookie = format!("session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if let Some(d) = domain {
        cookie.push_str("; Domain=");
        cookie.push_str(d);
    }
    cookie
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

/// CO-185 / CO-186: short-lived handover JWT for cross-apex SSO.
///
/// Signed with the server's P-256 private key. Receivers validate via
/// the public key at `/.well-known/jwks.json` — no shared secret
/// distribution required.
///
/// 60-second TTL — long enough for the browser redirect, short enough
/// that a leaked URL is useless.
pub fn sign_handover_jwt_es256(
    key: &JwtKey,
    user_id: &str,
    email: &str,
    tier: &str,
) -> anyhow::Result<String> {
    let now = Utc::now();
    let iat = now.timestamp() as usize;
    let exp = (now.timestamp() + 60) as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        email: email.to_string(),
        tier: tier.to_string(),
        usuario: String::new(),
        papel: String::new(),
        exp,
        iat,
    };
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key.kid.clone());
    let token = encode(&header, &claims, &key.encoding_key()?)?;
    Ok(token)
}

/// CO-186: when a redirect target points at a `/auth/co-handover` endpoint
/// on a safelisted host, append a short-lived `co_token` query param so the
/// receiving SvelteKit (or other framework) can establish its own session
/// cookie cross-apex. Token signed ES256 — receivers validate via JWKS,
/// no shared secret to distribute. Otherwise returns `return_to` unchanged.
pub fn maybe_attach_co_handover_token(
    return_to: &str,
    user_id: &str,
    email: &str,
    tier: &str,
    jwt_key: &JwtKey,
) -> String {
    if !return_to.contains("/auth/co-handover") {
        return return_to.to_string();
    }
    let token = match sign_handover_jwt_es256(jwt_key, user_id, email, tier) {
        Ok(t) => t,
        Err(_) => return return_to.to_string(),
    };
    let separator = if return_to.contains('?') { '&' } else { '?' };
    format!("{return_to}{separator}co_token={token}")
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

// ---------------------------------------------------------------------------
// CO-161: Universe-scoped visibility middleware
// ---------------------------------------------------------------------------

/// Extract the universe slug from a request path that has had the
/// `/api/v1/universes` prefix stripped by axum's nest(), e.g. `/<slug>/entries`.
fn extract_universe_slug(path: &str) -> Option<String> {
    let after_slash = path.strip_prefix('/')?;
    let slug = after_slash.split('/').next()?;
    if slug.is_empty() {
        None
    } else {
        Some(slug.to_string())
    }
}

fn err_response(status: StatusCode, error: &str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": error,
            "message_en": message,
        })),
    )
        .into_response()
}

/// Tower middleware — applied once on the combined `/api/v1/universes/{slug}/…`
/// sub-router. Public and template universes pass through; private universes
/// require an authenticated user who is the owner or a `universe_members` row.
///
/// Replaces 13 per-handler `check_reader_for_entries` calls and the
/// `asset_routes::check_reader` duplicate (CO-161).
pub async fn universe_visibility_gate(
    axum::extract::State(state): axum::extract::State<crate::server::AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    let slug = match extract_universe_slug(req.uri().path()) {
        Some(s) => s,
        None => return Ok(next.run(req).await),
    };

    let universe = {
        let storage = state.storage.lock().map_err(|_| {
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Storage lock failed",
            )
        })?;
        match storage.get_universe(&slug) {
            Some(u) => u,
            None => {
                return Err(err_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    &format!("Universe '{slug}' not found"),
                ));
            }
        }
    };

    if universe.is_public || universe.is_template {
        return Ok(next.run(req).await);
    }

    let user_id = resolve_user_id(&state, req.headers())
        .ok_or_else(|| err_response(StatusCode::UNAUTHORIZED, "unauthorized", "Login required"))?;

    let is_allowed = {
        let storage = state.storage.lock().map_err(|_| {
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Storage lock failed",
            )
        })?;
        let universe = storage.get_universe(&slug).ok_or_else(|| {
            err_response(
                StatusCode::NOT_FOUND,
                "not_found",
                &format!("Universe '{slug}' not found"),
            )
        })?;
        if universe.owner_id == user_id {
            true
        } else {
            storage
                .conn()
                .query_row(
                    "SELECT 1 FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
                    rusqlite::params![&slug, &user_id],
                    |_| Ok(true),
                )
                .unwrap_or(false)
        }
    };

    if is_allowed {
        Ok(next.run(req).await)
    } else {
        Err(err_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Not authorized to access this universe",
        ))
    }
}

/// Tower middleware — applied once on the combined universe sub-router for all
/// mutating methods (POST / PUT / PATCH / DELETE). Passes GET/HEAD/OPTIONS
/// through unchanged (handled by `universe_visibility_gate` above).
///
/// Requires the caller to be the universe owner or a `universe_members` row.
pub async fn universe_writer_gate(
    axum::extract::State(state): axum::extract::State<crate::server::AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    let is_write = matches!(
        req.method(),
        &Method::POST | &Method::PUT | &Method::PATCH | &Method::DELETE
    );
    if !is_write {
        return Ok(next.run(req).await);
    }

    let slug = match extract_universe_slug(req.uri().path()) {
        Some(s) => s,
        None => return Ok(next.run(req).await),
    };

    let user_id = resolve_user_id(&state, req.headers())
        .ok_or_else(|| err_response(StatusCode::UNAUTHORIZED, "unauthorized", "Login required"))?;

    let is_allowed = {
        let storage = state.storage.lock().map_err(|_| {
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Storage lock failed",
            )
        })?;
        let universe = storage.get_universe(&slug).ok_or_else(|| {
            err_response(
                StatusCode::NOT_FOUND,
                "not_found",
                &format!("Universe '{slug}' not found"),
            )
        })?;
        if universe.owner_id == user_id {
            true
        } else {
            storage
                .conn()
                .query_row(
                    "SELECT 1 FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
                    rusqlite::params![&slug, &user_id],
                    |_| Ok(true),
                )
                .unwrap_or(false)
        }
    };

    if is_allowed {
        Ok(next.run(req).await)
    } else {
        Err(err_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Not authorized to write to this universe",
        ))
    }
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

    // -----------------------------------------------------------------------
    // CO-166: JwtKey, JWKS, cookie domain, ES256 tests
    // -----------------------------------------------------------------------

    /// JwtKey::load_or_generate() produces a key with a non-empty kid.
    #[test]
    fn test_jwt_key_generates_kid() {
        let key = JwtKey::load_or_generate();
        assert!(!key.kid.is_empty(), "kid must be non-empty");
        assert_eq!(key.kid.len(), 16, "kid should be 16 hex chars");
    }

    /// public_jwk() returns a JSON object with the required EC key fields.
    #[test]
    fn test_jwt_key_public_jwk_has_required_fields() {
        let key = JwtKey::load_or_generate();
        let jwk = key.public_jwk();
        assert_eq!(jwk["kty"], "EC", "kty must be EC");
        assert_eq!(jwk["crv"], "P-256", "crv must be P-256");
        assert_eq!(jwk["alg"], "ES256", "alg must be ES256");
        assert_eq!(jwk["use"], "sig", "use must be sig");
        assert_eq!(jwk["kid"], key.kid, "kid must match");
        assert!(jwk["x"].is_string(), "x must be a string (base64url)");
        assert!(jwk["y"].is_string(), "y must be a string (base64url)");
        // x and y must be non-empty base64url strings (32-byte P-256 coords = 43 chars)
        let x = jwk["x"].as_str().unwrap();
        let y = jwk["y"].as_str().unwrap();
        assert!(
            x.len() >= 40,
            "x must be at least 40 chars: got {}",
            x.len()
        );
        assert!(
            y.len() >= 40,
            "y must be at least 40 chars: got {}",
            y.len()
        );
    }

    /// sign_jwt_es256 produces a token that can be verified with the matching public key.
    #[test]
    fn test_sign_jwt_es256_verifiable() {
        let key = JwtKey::load_or_generate();
        let (token, _expires_at) =
            sign_jwt_es256(&key, "user-123", "test@example.com", "player").unwrap();

        // Must decode successfully with ES256.
        let decoding_key = key.decoding_key().unwrap();
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.validate_exp = true;
        let data = jsonwebtoken::decode::<Claims>(&token, &decoding_key, &validation).unwrap();
        assert_eq!(data.claims.sub, "user-123");
        assert_eq!(data.claims.email, "test@example.com");
        assert_eq!(data.claims.tier, "player");
    }

    /// decode_claims_any falls back to HS256 for old tokens.
    #[test]
    fn test_decode_claims_any_accepts_hs256_fallback() {
        let key = JwtKey::load_or_generate();
        let secret = "test-hs256-secret";
        let (hs256_token, _) =
            sign_jwt("legacy-user", "legacy@example.com", "free", secret).unwrap();

        let claims = decode_claims_any(&hs256_token, &key, secret).unwrap();
        assert_eq!(claims.sub, "legacy-user");
    }

    /// decode_claims_any accepts ES256 tokens.
    #[test]
    fn test_decode_claims_any_accepts_es256() {
        let key = JwtKey::load_or_generate();
        let (es256_token, _) = sign_jwt_es256(&key, "es-user", "es@example.com", "player").unwrap();

        let claims = decode_claims_any(&es256_token, &key, "any-hs256-secret").unwrap();
        assert_eq!(claims.sub, "es-user");
    }

    /// build_session_cookie without domain does not include Domain attribute.
    #[test]
    fn test_build_session_cookie_no_domain() {
        let cookie = build_session_cookie("tok123", None, 604800);
        assert!(
            cookie.starts_with("session=tok123;"),
            "cookie must start with session=<token>;"
        );
        assert!(
            !cookie.contains("Domain="),
            "cookie must not include Domain when domain is None"
        );
        assert!(
            cookie.contains("Max-Age=604800"),
            "cookie must include Max-Age"
        );
        assert!(cookie.contains("HttpOnly"), "cookie must include HttpOnly");
        assert!(
            cookie.contains("SameSite=Lax"),
            "cookie must include SameSite=Lax"
        );
    }

    /// build_session_cookie with domain appends Domain attribute.
    #[test]
    fn test_build_session_cookie_with_domain() {
        let cookie = build_session_cookie("tok456", Some(".artelonga.com.br"), 604800);
        assert!(
            cookie.contains("Domain=.artelonga.com.br"),
            "cookie must include Domain attribute: {cookie}"
        );
    }
}
