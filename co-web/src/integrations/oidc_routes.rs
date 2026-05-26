//! CO-166: OIDC provider endpoints.
//!
//! Implements a minimal but spec-compliant OpenID Connect authorization server
//! with PKCE (S256). Relying parties register via the gestão admin API; end
//! users authorize via the standard code flow.
//!
//! Endpoints
//! ---------
//! Public discovery:
//!   GET  /.well-known/openid-configuration  — OIDC discovery document
//!   GET  /.well-known/jwks.json             — JWK Set (EC P-256 public key)
//!
//! Authorization code flow:
//!   GET  /oauth/authorize    — authorization endpoint (redirects with ?code=)
//!   POST /oauth/token        — exchange code for tokens (validates PKCE)
//!   GET  /oauth/userinfo     — returns claims from access_token
//!
//! Admin (gestão-authenticated):
//!   POST /api/v1/gestao/oauth/clients  — register a new OIDC client
//!   GET  /api/v1/gestao/oauth/clients  — list all OIDC clients

use axum::Router;
use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::sync::Arc;

use crate::auth::UserId;
use crate::error::AppError;
use crate::github_auth::GitHubAdmin;
use crate::server::{AppState, CoreState, IntegrationsState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn base64url_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Verify PKCE S256: SHA-256(code_verifier) == base64url(code_challenge)
fn verify_pkce_s256(code_verifier: &str, code_challenge: &str) -> bool {
    let hash = Sha256::digest(code_verifier.as_bytes());
    base64url_encode(&hash) == code_challenge
}

// ---------------------------------------------------------------------------
// Storage helpers — operate directly on the shared SQLite connection.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OauthClient {
    pub id: String,
    pub client_id: String,
    pub client_secret: String,
    pub name: String,
    /// JSON array of allowed redirect URIs.
    pub redirect_uris: String,
    /// Space-separated list of allowed scopes.
    pub scopes: String,
    pub created_at: String,
}

fn insert_oauth_client(
    conn: &rusqlite::Connection,
    client: &OauthClient,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO oauth_clients (id, client_id, client_secret, name, redirect_uris, scopes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            client.id,
            client.client_id,
            client.client_secret,
            client.name,
            client.redirect_uris,
            client.scopes,
            client.created_at
        ],
    )?;
    Ok(())
}

fn list_oauth_clients(conn: &rusqlite::Connection) -> Vec<OauthClient> {
    let mut stmt = conn
        .prepare(
            "SELECT id, client_id, client_secret, name, redirect_uris, scopes, created_at
             FROM oauth_clients ORDER BY created_at",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok(OauthClient {
            id: row.get(0)?,
            client_id: row.get(1)?,
            client_secret: row.get(2)?,
            name: row.get(3)?,
            redirect_uris: row.get(4)?,
            scopes: row.get(5)?,
            created_at: row.get(6)?,
        })
    })
    .unwrap()
    .flatten()
    .collect()
}

fn get_oauth_client_by_client_id(
    conn: &rusqlite::Connection,
    client_id: &str,
) -> Option<OauthClient> {
    conn.query_row(
        "SELECT id, client_id, client_secret, name, redirect_uris, scopes, created_at
         FROM oauth_clients WHERE client_id = ?1",
        rusqlite::params![client_id],
        |row| {
            Ok(OauthClient {
                id: row.get(0)?,
                client_id: row.get(1)?,
                client_secret: row.get(2)?,
                name: row.get(3)?,
                redirect_uris: row.get(4)?,
                scopes: row.get(5)?,
                created_at: row.get(6)?,
            })
        },
    )
    .ok()
}

fn insert_auth_code(conn: &rusqlite::Connection, row: &AuthCodeRow) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO oauth_auth_codes (code, client_id, user_id, redirect_uri, scope,
             code_challenge, code_challenge_method, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            row.code,
            row.client_id,
            row.user_id,
            row.redirect_uri,
            row.scope,
            row.code_challenge,
            row.code_challenge_method,
            row.expires_at,
            row.created_at
        ],
    )?;
    Ok(())
}

struct AuthCodeRow {
    code: String,
    client_id: String,
    user_id: String,
    redirect_uri: String,
    scope: String,
    code_challenge: String,
    code_challenge_method: String,
    expires_at: String,
    created_at: String,
}

fn consume_auth_code(conn: &rusqlite::Connection, code: &str) -> Option<AuthCodeRow> {
    let row = conn
        .query_row(
            "SELECT code, client_id, user_id, redirect_uri, scope,
                    code_challenge, code_challenge_method, expires_at, created_at
             FROM oauth_auth_codes WHERE code = ?1",
            rusqlite::params![code],
            |row| {
                Ok(AuthCodeRow {
                    code: row.get(0)?,
                    client_id: row.get(1)?,
                    user_id: row.get(2)?,
                    redirect_uri: row.get(3)?,
                    scope: row.get(4)?,
                    code_challenge: row.get(5)?,
                    code_challenge_method: row.get(6)?,
                    expires_at: row.get(7)?,
                    created_at: row.get(8)?,
                })
            },
        )
        .ok()?;

    // Delete (single-use code).
    conn.execute(
        "DELETE FROM oauth_auth_codes WHERE code = ?1",
        rusqlite::params![code],
    )
    .ok();
    Some(row)
}

fn insert_access_token(
    conn: &rusqlite::Connection,
    token: &str,
    client_id: &str,
    user_id: &str,
    scope: &str,
    expires_at: &str,
    created_at: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO oauth_access_tokens (token, client_id, user_id, scope, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![token, client_id, user_id, scope, expires_at, created_at],
    )?;
    Ok(())
}

fn get_access_token_row(
    conn: &rusqlite::Connection,
    token: &str,
) -> Option<(String, String, String, String)> {
    // Returns (user_id, scope, client_id, expires_at)
    conn.query_row(
        "SELECT user_id, scope, client_id, expires_at FROM oauth_access_tokens WHERE token = ?1",
        rusqlite::params![token],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .ok()
}

// ---------------------------------------------------------------------------
// Gestão admin routes — register and list OAuth clients
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegisterClientRequest {
    pub name: String,
    /// Array of allowed redirect URIs (at least one required).
    pub redirect_uris: Vec<String>,
    /// Space-separated scopes this client may request (default: "openid profile email").
    #[serde(default = "default_scopes")]
    pub scopes: String,
}

fn default_scopes() -> String {
    "openid profile email".to_string()
}

#[derive(Debug, Serialize)]
pub struct RegisterClientResponse {
    pub id: String,
    pub client_id: String,
    pub client_secret: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub scopes: String,
    pub created_at: String,
}

/// POST /api/v1/gestao/oauth/clients — register a new OIDC client (admin only)
pub async fn register_oauth_client(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Json(body): Json<RegisterClientRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("Client name is required".into()));
    }
    if body.redirect_uris.is_empty() {
        return Err(AppError::BadRequest(
            "At least one redirect_uri is required".into(),
        ));
    }
    for uri in &body.redirect_uris {
        if uri.trim().is_empty() {
            return Err(AppError::BadRequest(
                "redirect_uri must not be empty".into(),
            ));
        }
    }

    let id = format!("oac_{}", nanoid::nanoid!(16));
    let client_id = format!("co_{}", nanoid::nanoid!(20));
    let client_secret = nanoid::nanoid!(40);
    let now = Utc::now().to_rfc3339();
    let redirect_uris_json =
        serde_json::to_string(&body.redirect_uris).unwrap_or_else(|_| "[]".into());

    let client = OauthClient {
        id: id.clone(),
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
        name: body.name.clone(),
        redirect_uris: redirect_uris_json,
        scopes: body.scopes.clone(),
        created_at: now.clone(),
    };

    {
        let storage = state.core.storage.lock();
        insert_oauth_client(storage.conn(), &client)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    Ok((
        StatusCode::CREATED,
        axum::Json(RegisterClientResponse {
            id,
            client_id,
            client_secret,
            name: body.name,
            redirect_uris: body.redirect_uris,
            scopes: body.scopes,
            created_at: now,
        }),
    ))
}

/// GET /api/v1/gestao/oauth/clients — list OIDC clients (admin only)
pub async fn list_oauth_clients_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
) -> Result<axum::Json<Vec<RegisterClientResponse>>, AppError> {
    let storage = state.core.storage.lock();
    let clients = list_oauth_clients(storage.conn());
    let resp: Vec<RegisterClientResponse> = clients
        .into_iter()
        .map(|c| {
            let uris: Vec<String> = serde_json::from_str(&c.redirect_uris).unwrap_or_default();
            RegisterClientResponse {
                id: c.id,
                client_id: c.client_id,
                client_secret: c.client_secret,
                name: c.name,
                redirect_uris: uris,
                scopes: c.scopes,
                created_at: c.created_at,
            }
        })
        .collect();
    Ok(axum::Json(resp))
}

pub fn gestao_oauth_router() -> Router<AppState> {
    Router::new()
        .route("/clients", post(register_oauth_client))
        .route("/clients", get(list_oauth_clients_handler))
}

// ---------------------------------------------------------------------------
// OIDC discovery + JWKS
// ---------------------------------------------------------------------------

/// GET /.well-known/openid-configuration — OIDC discovery document
pub async fn openid_configuration(State(core): State<Arc<CoreState>>) -> impl IntoResponse {
    // Derive the issuer URL from environment or fall back to localhost.
    let base = std::env::var("CO_PUBLIC_URL")
        .unwrap_or_else(|_| format!("http://localhost:{}", core.config.port));

    axum::Json(serde_json::json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "userinfo_endpoint": format!("{base}/oauth/userinfo"),
        "jwks_uri": format!("{base}/.well-known/jwks.json"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["ES256"],
        "scopes_supported": ["openid", "profile", "email"],
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
        "claims_supported": ["sub", "email", "tier", "iat", "exp"],
    }))
}

/// GET /.well-known/jwks.json — JWK Set containing the public ES256 key
pub async fn jwks_json(State(integrations): State<Arc<IntegrationsState>>) -> impl IntoResponse {
    let jwk = integrations.jwt_key.public_jwk();
    axum::Json(serde_json::json!({ "keys": [jwk] }))
}

// ---------------------------------------------------------------------------
// Authorization endpoint — GET /oauth/authorize
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub state: Option<String>,
    pub code_challenge: String,
    pub code_challenge_method: Option<String>,
}

/// GET /oauth/authorize — issue an authorization code for an already-logged-in user.
///
/// In a full web app this would show a consent screen. Here we require the
/// caller to be authenticated (session cookie / Bearer JWT) and immediately
/// issue the code. The browser redirect includes `code` and optionally `state`.
pub async fn authorize_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    user_id: UserId,
    Query(params): Query<AuthorizeQuery>,
) -> Result<Response, AppError> {
    // Validate response_type.
    if params.response_type != "code" {
        return Err(AppError::BadRequest(
            "Only response_type=code is supported".into(),
        ));
    }

    // Validate code_challenge_method.
    let method = params.code_challenge_method.as_deref().unwrap_or("S256");
    if method != "S256" {
        return Err(AppError::BadRequest(
            "Only code_challenge_method=S256 is supported".into(),
        ));
    }

    // Look up client.
    let client = {
        let storage = state.core.storage.lock();
        get_oauth_client_by_client_id(storage.conn(), &params.client_id)
            .ok_or_else(|| AppError::BadRequest("Unknown client_id".into()))?
    };

    // Validate redirect_uri against allowlist.
    let allowed_uris: Vec<String> = serde_json::from_str(&client.redirect_uris).unwrap_or_default();
    if !allowed_uris.contains(&params.redirect_uri) {
        return Err(AppError::BadRequest("redirect_uri not allowed".into()));
    }

    // Validate requested scopes are a subset of the client's scopes.
    let client_scopes: std::collections::HashSet<&str> = client.scopes.split_whitespace().collect();
    for s in params.scope.split_whitespace() {
        if s != "openid" && !client_scopes.contains(s) {
            return Err(AppError::BadRequest(format!(
                "Scope '{s}' not allowed for this client"
            )));
        }
    }

    // Mint auth code (valid 5 minutes).
    let code = nanoid::nanoid!(40);
    let now = Utc::now();
    let expires_at = (now + chrono::Duration::seconds(300)).to_rfc3339();

    {
        let storage = state.core.storage.lock();
        insert_auth_code(
            storage.conn(),
            &AuthCodeRow {
                code: code.clone(),
                client_id: params.client_id.clone(),
                user_id: user_id.0.clone(),
                redirect_uri: params.redirect_uri.clone(),
                scope: params.scope.clone(),
                code_challenge: params.code_challenge.clone(),
                code_challenge_method: method.to_string(),
                expires_at,
                created_at: now.to_rfc3339(),
            },
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // Build redirect URL with code (and optional state).
    let mut redirect = format!("{}?code={}", params.redirect_uri, code);
    if let Some(s) = &params.state {
        redirect.push_str(&format!(
            "&state={}",
            percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC)
        ));
    }

    Ok(Redirect::to(&redirect).into_response())
}

// ---------------------------------------------------------------------------
// Token endpoint — POST /oauth/token
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: String,
    pub redirect_uri: String,
    pub client_id: String,
    pub client_secret: String,
    pub code_verifier: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub id_token: String,
    pub scope: String,
}

/// POST /oauth/token — exchange authorization code for tokens (PKCE S256)
pub async fn token_handler(
    State(state): State<AppState>,
    axum::Form(params): axum::Form<TokenRequest>,
) -> Result<axum::Json<TokenResponse>, AppError> {
    if params.grant_type != "authorization_code" {
        return Err(AppError::BadRequest(
            "Only grant_type=authorization_code is supported".into(),
        ));
    }

    // Fetch and consume the auth code.
    let code_row = {
        let storage = state.core.storage.lock();

        // Validate client credentials.
        let client = get_oauth_client_by_client_id(storage.conn(), &params.client_id)
            .ok_or_else(|| AppError::Unauthorized("Unknown client_id".into()))?;
        if client.client_secret != params.client_secret {
            return Err(AppError::Unauthorized("Invalid client_secret".into()));
        }

        consume_auth_code(storage.conn(), &params.code)
            .ok_or_else(|| AppError::Unauthorized("Invalid or expired code".into()))?
    };

    // Validate code belongs to this client.
    if code_row.client_id != params.client_id {
        return Err(AppError::Unauthorized(
            "Code was not issued to this client".into(),
        ));
    }

    // Validate redirect_uri matches.
    if code_row.redirect_uri != params.redirect_uri {
        return Err(AppError::BadRequest("redirect_uri mismatch".into()));
    }

    // Validate code has not expired.
    let expires_at = chrono::DateTime::parse_from_rfc3339(&code_row.expires_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now() - chrono::Duration::seconds(1));
    if Utc::now() > expires_at {
        return Err(AppError::Unauthorized(
            "Authorization code has expired".into(),
        ));
    }

    // Verify PKCE.
    if !verify_pkce_s256(&params.code_verifier, &code_row.code_challenge) {
        return Err(AppError::Unauthorized("PKCE verification failed".into()));
    }

    // Mint access token.
    let access_token = format!("co_at_{}", nanoid::nanoid!(40));
    let now = Utc::now();
    let at_expires_at = (now + chrono::Duration::seconds(3600)).to_rfc3339();

    {
        let storage = state.core.storage.lock();
        insert_access_token(
            storage.conn(),
            &access_token,
            &code_row.client_id,
            &code_row.user_id,
            &code_row.scope,
            &at_expires_at,
            &now.to_rfc3339(),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // Mint ES256 ID token.
    let id_token = {
        let user_email = {
            let storage = state.core.storage.lock();
            storage
                .get_user_by_id(&code_row.user_id)
                .map(|u| u.email)
                .unwrap_or_default()
        };

        crate::auth::sign_jwt_es256(
            &state.integrations.jwt_key,
            &code_row.user_id,
            &user_email,
            "player",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?
        .0
    };

    Ok(axum::Json(TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: 3600,
        id_token,
        scope: code_row.scope,
    }))
}

// ---------------------------------------------------------------------------
// Userinfo endpoint — GET /oauth/userinfo
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct UserinfoResponse {
    pub sub: String,
    pub email: String,
    pub name: String,
    pub tier: String,
}

/// GET /oauth/userinfo — return user profile from access_token
pub async fn userinfo_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<axum::Json<UserinfoResponse>, AppError> {
    // Extract Bearer access token.
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Missing Bearer token".into()))?;

    // Validate access token and look up user.
    let (user_id, _scope, _client_id, expires_at_str) = {
        let storage = state.core.storage.lock();
        get_access_token_row(storage.conn(), token)
            .ok_or_else(|| AppError::Unauthorized("Invalid access token".into()))?
    };

    // Check expiry.
    let expires_at = chrono::DateTime::parse_from_rfc3339(&expires_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now() - chrono::Duration::seconds(1));
    if Utc::now() > expires_at {
        return Err(AppError::Unauthorized("Access token has expired".into()));
    }

    // Fetch user.
    let user = {
        let storage = state.core.storage.lock();
        storage
            .get_user_by_id(&user_id)
            .ok_or_else(|| AppError::NotFound("User not found".into()))?
    };

    Ok(axum::Json(UserinfoResponse {
        sub: user.id,
        email: user.email,
        name: user.display_name,
        tier: user.tier,
    }))
}

// ---------------------------------------------------------------------------
// OAuth sub-router (mounted at /oauth)
// ---------------------------------------------------------------------------

pub fn oauth_router(state: AppState) -> Router<AppState> {
    // /authorize requires authentication (session cookie or Bearer JWT).
    // /token and /userinfo use their own credential validation (client_secret / access_token).
    let authorized = Router::new()
        .route("/authorize", get(authorize_handler))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::require_auth,
        ));

    Router::new()
        .merge(authorized)
        .route("/token", post(token_handler))
        .route("/userinfo", get(userinfo_handler))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{JwtKey, sign_jwt};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        use crate::config::WebConfig;
        use crate::experiment::ExperimentStore;
        use crate::server::{
            AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
            build_router,
        };
        use crate::storage::Storage;
        use std::sync::Mutex;

        let config = WebConfig {
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
            bypass_rate_limit: false,
        };
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path)
                .expect("Failed to open test game storage"),
        );
        let state: AppState = AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx: {
                    let (tx, _) = crate::embedding_worker::channel();
                    tx
                },
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(JwtKey::load_or_generate()),
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
            }),
        });
        build_router(state, None)
    }

    async fn body_str(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// GET /.well-known/jwks.json returns 200 with a `keys` array
    /// containing an EC P-256 public key.
    #[tokio::test]
    async fn test_jwks_endpoint_returns_valid_json() {
        let dir = TempDir::new().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/jwks.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "JWKS endpoint must return 200"
        );
        let body = body_str(resp.into_body()).await;
        let json: serde_json::Value = serde_json::from_str(&body).expect("JWKS body must be JSON");

        let keys = json["keys"]
            .as_array()
            .expect("JWKS must have 'keys' array");
        assert!(!keys.is_empty(), "JWKS must have at least one key");

        let key = &keys[0];
        assert_eq!(key["kty"], "EC", "key type must be EC");
        assert_eq!(key["crv"], "P-256", "curve must be P-256");
        assert_eq!(key["alg"], "ES256", "algorithm must be ES256");
        assert!(key["x"].is_string(), "x coordinate must be present");
        assert!(key["y"].is_string(), "y coordinate must be present");
        assert!(key["kid"].is_string(), "kid must be present");
    }

    /// GET /.well-known/openid-configuration returns a valid discovery document.
    #[tokio::test]
    async fn test_openid_configuration_endpoint() {
        let dir = TempDir::new().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/openid-configuration")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_str(resp.into_body()).await;
        let json: serde_json::Value =
            serde_json::from_str(&body).expect("openid-configuration body must be JSON");

        assert!(json["issuer"].is_string());
        assert!(json["authorization_endpoint"].is_string());
        assert!(json["token_endpoint"].is_string());
        assert!(json["userinfo_endpoint"].is_string());
        assert!(json["jwks_uri"].is_string());
        assert_eq!(json["code_challenge_methods_supported"][0], "S256");
    }

    /// Cookie domain logic: build_session_cookie with CO_COOKIE_DOMAIN set.
    #[test]
    fn test_cookie_domain_appended_when_configured() {
        let cookie =
            crate::auth::build_session_cookie("mytoken", Some(".artelonga.com.br"), 604800);
        assert!(
            cookie.contains("Domain=.artelonga.com.br"),
            "Domain attribute must be appended: {cookie}"
        );
        assert!(
            cookie.contains("session=mytoken"),
            "token must be in cookie"
        );
    }

    /// Without CO_COOKIE_DOMAIN set, no Domain attribute is added.
    #[test]
    fn test_cookie_no_domain_when_not_configured() {
        let cookie = crate::auth::build_session_cookie("mytoken", None, 604800);
        assert!(
            !cookie.contains("Domain="),
            "Domain attribute must NOT be present when unset: {cookie}"
        );
    }

    /// OIDC authorize → token flow (happy path with PKCE S256).
    #[tokio::test]
    async fn test_oidc_authorize_token_flow() {
        use crate::storage::Storage;
        use sha2::{Digest, Sha256};

        let dir = TempDir::new().unwrap();
        let app = build_test_router(dir.path());

        // Create a user and get a session token.
        // Use jwt_secret() so the token matches whatever the middleware will use
        // (avoids env-var races with concurrent tests that call set_var("JWT_SECRET", ...)).
        let mut storage = Storage::new(dir.path().to_str().unwrap());
        let user = storage.create_user("oidc@test.local", "OIDC Test").unwrap();
        let secret = crate::auth::jwt_secret();
        let (session_token, _) = sign_jwt(&user.id, "oidc@test.local", "player", &secret).unwrap();

        // Register an OAuth client directly in storage.
        let client_id = "test-client";
        let client_secret = "test-secret";
        let redirect_uri = "https://app.example.com/callback";
        storage
            .conn()
            .execute(
                "INSERT INTO oauth_clients (id, client_id, client_secret, name, redirect_uris, scopes, created_at)
                 VALUES ('c1', ?1, ?2, 'Test', ?3, 'openid email profile', datetime('now'))",
                rusqlite::params![
                    client_id,
                    client_secret,
                    serde_json::to_string(&vec![redirect_uri]).unwrap()
                ],
            )
            .unwrap();

        // Generate PKCE pair.
        let code_verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge_bytes = Sha256::digest(code_verifier.as_bytes());
        let code_challenge = base64url_encode(&challenge_bytes);

        // GET /oauth/authorize (authenticated).
        let auth_url = format!(
            "/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&scope=openid%20email&code_challenge={code_challenge}&code_challenge_method=S256",
            percent_encoding::utf8_percent_encode(redirect_uri, percent_encoding::NON_ALPHANUMERIC)
        );

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&auth_url)
                    .header("cookie", format!("session={session_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::SEE_OTHER,
            "authorize must redirect"
        );
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .expect("must have Location header");
        assert!(
            location.starts_with(redirect_uri),
            "must redirect to redirect_uri: {location}"
        );
        let code = location
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        assert!(!code.is_empty(), "auth code must be non-empty");

        // POST /oauth/token.
        let token_body = format!(
            "grant_type=authorization_code&code={code}&redirect_uri={}&client_id={client_id}&client_secret={client_secret}&code_verifier={code_verifier}",
            percent_encoding::utf8_percent_encode(redirect_uri, percent_encoding::NON_ALPHANUMERIC)
        );

        let token_resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(token_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            token_resp.status(),
            StatusCode::OK,
            "token exchange must succeed"
        );
        let token_json_str = body_str(token_resp.into_body()).await;
        let token_json: serde_json::Value =
            serde_json::from_str(&token_json_str).expect("token response must be JSON");

        assert!(
            token_json["access_token"].is_string(),
            "must have access_token"
        );
        assert!(token_json["id_token"].is_string(), "must have id_token");
        assert_eq!(token_json["token_type"], "Bearer");
    }

    /// Quilombo legacy login returns 410 when disabled.
    #[tokio::test]
    async fn test_quilombo_legacy_login_disabled_returns_410() {
        use crate::config::WebConfig;
        use crate::experiment::ExperimentStore;
        use crate::server::{
            AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
            build_router,
        };
        use crate::storage::Storage;
        use std::sync::Mutex;

        let dir = TempDir::new().unwrap();
        let config = WebConfig {
            port: 3000,
            data_dir: dir.path().to_str().unwrap().to_string(),
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
            quilombo_legacy_login: false, // <-- disabled
            bypass_rate_limit: false,
        };
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir.path()).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.path().join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path)
                .expect("Failed to open test game storage"),
        );
        let state: AppState = AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx: {
                    let (tx, _) = crate::embedding_worker::channel();
                    tx
                },
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(JwtKey::load_or_generate()),
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
            }),
        });
        let app = build_router(state, None);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/quilombo/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"usuario":"anyone","senha":"anypassword"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::GONE,
            "legacy quilombo login must return 410 when disabled"
        );
    }
}
