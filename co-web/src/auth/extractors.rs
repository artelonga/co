//! CO-222 — Typed extractor hierarchy for auth gating.
//!
//! Replaces ad-hoc `.layer(require_auth)` / `.layer(require_auth_with_token)` /
//! in-handler `resolve_role()` with four typed axum extractors that express
//! auth requirements directly in handler signatures.
//!
//! | Extractor          | Requirement                                                |
//! |--------------------|------------------------------------------------------------|
//! | [`AuthedUser`]     | Any authenticated user (JWT or session cookie)             |
//! | [`OwnerOf`]        | Authenticated + owns the `{slug}` universe                 |
//! | [`AdminUser`]      | Authenticated + `tier == "admin"` in JWT claims            |
//! | [`TokenOrJwtUser`] | Authenticated via JWT **or** long-lived API token (CO-35)  |
//!
//! Existing middleware (`require_auth`, `require_auth_with_token`) keep working
//! in parallel: [`AuthedUser`] and [`TokenOrJwtUser`] check the `UserId`
//! extension set by middleware as a fast path before decoding the JWT.

use std::marker::PhantomData;

use axum::{
    Json,
    extract::MatchedPath,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};

use super::UserId;
use super::capabilities;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

fn forbidden(msg: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

fn internal_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Shared extraction helpers
// ---------------------------------------------------------------------------

/// Decode the caller's user_id from request parts.
///
/// Fast path: if `UserId` is already in extensions (injected by `require_auth`
/// middleware), return it directly without re-decoding the JWT. Otherwise
/// decodes the Bearer JWT or `session` cookie.
fn resolve_authed_user_id(parts: &Parts) -> Option<String> {
    if let Some(uid) = parts.extensions.get::<UserId>() {
        return Some(uid.0.clone());
    }
    let token = super::extract_bearer_or_cookie(&parts.headers)?;
    let secret = super::jwt_secret();
    super::decode_user_id(&token, &secret).ok()
}

/// Extract the value of the `{slug}` path parameter from the matched route.
///
/// Reads the [`MatchedPath`] extension (the registered route pattern, e.g.
/// `/api/v1/universes/{slug}/chat`) and the raw request URI to find which
/// segment corresponds to `{slug}`. Returns `None` when:
///
/// - `MatchedPath` is absent in extensions (unusual for normal axum routes).
/// - The route pattern has no `{slug}` segment (programmer error — the calling
///   extractor surfaces this as `500 Internal Server Error`).
fn extract_slug_from_parts(parts: &Parts) -> Option<String> {
    let matched = parts.extensions.get::<MatchedPath>()?;
    let pattern = matched.as_str();
    let url_path = parts.uri.path();

    let pattern_segs: Vec<&str> = pattern.split('/').collect();
    let path_segs: Vec<&str> = url_path.split('/').collect();

    if pattern_segs.len() != path_segs.len() {
        return None;
    }

    for (pat_seg, path_seg) in pattern_segs.iter().zip(path_segs.iter()) {
        if *pat_seg == "{slug}" {
            let slug = *path_seg;
            if !slug.is_empty() {
                return Some(slug.to_string());
            }
        }
    }
    None
}

// ===========================================================================
// AuthedUser
// ===========================================================================

/// Any authenticated user.
///
/// Accepts a Bearer JWT or `session` cookie. On routes already behind
/// `require_auth` middleware the `UserId` extension is used as a fast path —
/// the JWT is not re-decoded.
#[derive(Clone, Debug)]
pub struct AuthedUser {
    pub user_id: String,
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for AuthedUser {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = match resolve_authed_user_id(parts) {
            Some(user_id) => Ok(AuthedUser { user_id }),
            None => Err(unauthorized("Not authenticated")),
        };
        std::future::ready(result)
    }
}

// ===========================================================================
// OwnerOf
// ===========================================================================

/// Authenticated user who owns the `{slug}` universe.
///
/// # Route contract
///
/// The route pattern MUST include a `{slug}` path parameter. Using `OwnerOf`
/// on a route without `{slug}` returns `500 Internal Server Error` with a
/// diagnostic message — the missing parameter is immediately visible rather
/// than silently allowing the request.
#[derive(Clone, Debug)]
pub struct OwnerOf {
    pub user_id: String,
    pub slug: String,
}

impl axum::extract::FromRequestParts<AppState> for OwnerOf {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let user_id = match resolve_authed_user_id(parts) {
            Some(uid) => uid,
            None => return std::future::ready(Err(unauthorized("Not authenticated"))),
        };

        let slug = match extract_slug_from_parts(parts) {
            Some(s) => s,
            None => {
                return std::future::ready(Err(internal_error(
                    "OwnerOf extractor requires a {slug} path parameter in the route pattern",
                )));
            }
        };

        let result = {
            let storage = state.core.storage.lock();
            match storage.get_universe(&slug) {
                Some(universe) if universe.owner_id == user_id => Ok(OwnerOf { user_id, slug }),
                Some(_) => Err(forbidden("Not authorized: you do not own this universe")),
                None => Err(not_found(&format!("Universe '{slug}' not found"))),
            }
        };
        std::future::ready(result)
    }
}

// ===========================================================================
// AdminUser
// ===========================================================================

/// CO platform administrator (`tier == "admin"` in JWT claims).
///
/// Returns `403 Forbidden` if the user is authenticated but not an admin.
#[derive(Clone, Debug)]
pub struct AdminUser {
    pub user_id: String,
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for AdminUser {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let Some(token) = super::extract_bearer_or_cookie(&parts.headers) else {
            return std::future::ready(Err(unauthorized(
                "Missing or malformed Authorization header",
            )));
        };
        let secret = super::jwt_secret();
        let claims = match super::decode_claims(&token, &secret) {
            Ok(c) => c,
            Err(_) => return std::future::ready(Err(unauthorized("Invalid or expired token"))),
        };
        if claims.tier != "admin" {
            return std::future::ready(Err(forbidden("Admin access required")));
        }
        std::future::ready(Ok(AdminUser {
            user_id: claims.sub,
        }))
    }
}

// ===========================================================================
// TokenOrJwtUser
// ===========================================================================

/// Authenticated via JWT or long-lived API token (CO-35).
///
/// Tries JWT decode first; falls back to API-token lookup in the database.
/// Replaces `require_auth_with_token` middleware for handlers that migrate
/// to the extractor model.
///
/// On routes behind `require_auth` or `require_auth_with_token` middleware the
/// `UserId` extension is returned directly (fast path, no re-decoding).
#[derive(Clone, Debug)]
pub struct TokenOrJwtUser {
    pub user_id: String,
}

impl axum::extract::FromRequestParts<AppState> for TokenOrJwtUser {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        // Fast path: middleware already authenticated.
        if let Some(uid) = parts.extensions.get::<UserId>() {
            return std::future::ready(Ok(TokenOrJwtUser {
                user_id: uid.0.clone(),
            }));
        }

        let Some(token) = super::extract_bearer_or_cookie(&parts.headers) else {
            return std::future::ready(Err(unauthorized(
                "Missing or malformed Authorization header",
            )));
        };
        let secret = super::jwt_secret();
        // Try JWT first.
        if let Ok(user_id) = super::decode_user_id(&token, &secret) {
            return std::future::ready(Ok(TokenOrJwtUser { user_id }));
        }
        // Fall back to API token lookup.
        let result = {
            let storage = state.core.storage.lock();
            storage
                .get_api_token_by_value(&token)
                .ok()
                .flatten()
                .map(|tok| TokenOrJwtUser {
                    user_id: tok.user_id,
                })
                .ok_or_else(|| unauthorized("Invalid or expired token"))
        };
        std::future::ready(result)
    }
}

// ===========================================================================
// CO-448 — Scoped<C>: capability-gated extractor
// ===========================================================================

/// A capability an endpoint requires, declared as a marker type so the gate is
/// expressed in the handler signature (`Scoped<ChatRead>`) and checked at the
/// type level.
///
/// `REQUIRED` is the `recurso:ação` capability string. `ADMIN_SURFACE` marks
/// endpoints that today require admin tier: for those, a JWT caller must be
/// admin and a NULL-scope (inherit-tier) token must be owned by an admin —
/// preserving the pre-CO-448 admin gate — while a least-privilege token that
/// *explicitly* carries the capability is admitted without being admin-pleno.
pub trait Capability {
    const REQUIRED: &'static str;
    const ADMIN_SURFACE: bool;
}

/// `chat:read` — admin chat telemetry (CO-204 origin breakdown). Admin surface.
pub struct ChatRead;
impl Capability for ChatRead {
    const REQUIRED: &'static str = "chat:read";
    const ADMIN_SURFACE: bool = true;
}

/// `entries:read` — read content entries. Not an admin surface.
pub struct EntriesRead;
impl Capability for EntriesRead {
    const REQUIRED: &'static str = "entries:read";
    const ADMIN_SURFACE: bool = false;
}

/// `entries:write` — create/update content entries. Not an admin surface.
pub struct EntriesWrite;
impl Capability for EntriesWrite {
    const REQUIRED: &'static str = "entries:write";
    const ADMIN_SURFACE: bool = false;
}

/// Authenticated caller authorized for capability `C`.
///
/// The capability gate (CO-448) resolves the caller and **denies with 403**
/// when a scoped token lacks `C::REQUIRED`, and **admits** when it is present.
/// Resolution rules:
///
/// - **JWT / session** — a full user session. Passes (subject to the admin-tier
///   check on `ADMIN_SURFACE` endpoints). Capabilities only restrict tokens.
/// - **API token, scopes present** — admitted iff the resolved set contains
///   `C::REQUIRED`; otherwise 403. No escalation: a token never gains a
///   capability outside its persisted set.
/// - **API token, scopes NULL** — inherits the owner's tier (pre-CO-448
///   behavior): passes, except on an `ADMIN_SURFACE` endpoint where the owner
///   must be an admin.
#[derive(Clone, Debug)]
pub struct Scoped<C: Capability> {
    pub user_id: String,
    // `fn() -> C` so `Scoped<C>` is always `Send`/`Sync` regardless of `C`
    // (the axum `FromRequestParts` future must be `Send`).
    _marker: PhantomData<fn() -> C>,
}

impl<C: Capability> axum::extract::FromRequestParts<AppState> for Scoped<C> {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(resolve_scoped::<C>(parts, state))
    }
}

// The `Err` variant is an axum `Response` (the established rejection type for
// every extractor in this module); boxing it would diverge from that pattern.
#[allow(clippy::result_large_err)]
fn resolve_scoped<C: Capability>(parts: &Parts, state: &AppState) -> Result<Scoped<C>, Response> {
    let Some(token) = super::extract_bearer_or_cookie(&parts.headers) else {
        return Err(unauthorized("Missing or malformed Authorization header"));
    };

    // JWT / session: full user authority. Capabilities only restrict tokens;
    // an ADMIN_SURFACE endpoint still requires admin tier here.
    let secret = super::jwt_secret();
    if let Ok(claims) = super::decode_claims(&token, &secret) {
        if C::ADMIN_SURFACE && claims.tier != "admin" {
            return Err(forbidden("Admin access required"));
        }
        return Ok(Scoped {
            user_id: claims.sub,
            _marker: PhantomData,
        });
    }

    // API token: enforce the capability (or inherit the owner's tier on NULL).
    let storage = state.core.storage.lock();
    let Some(tok) = storage.get_api_token_by_value(&token).ok().flatten() else {
        return Err(unauthorized("Invalid or expired token"));
    };

    match tok.scopes {
        Some(scopes) => {
            let resolved: std::collections::BTreeSet<String> = scopes.into_iter().collect();
            if capabilities::grants(&resolved, C::REQUIRED) {
                Ok(Scoped {
                    user_id: tok.user_id,
                    _marker: PhantomData,
                })
            } else {
                Err(forbidden(&format!(
                    "Token missing required capability '{}'",
                    C::REQUIRED
                )))
            }
        }
        None => {
            // NULL scopes → inherit owner tier (pre-CO-448 behavior).
            if C::ADMIN_SURFACE {
                let owner_is_admin = storage
                    .get_user_by_id(&tok.user_id)
                    .map(|u| u.tier == "admin")
                    .unwrap_or(false);
                if !owner_is_admin {
                    return Err(forbidden("Admin access required"));
                }
            }
            Ok(Scoped {
                user_id: tok.user_id,
                _marker: PhantomData,
            })
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        response::IntoResponse,
        routing::get,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::auth::{UserId, sign_jwt};

    use super::{AdminUser, AuthedUser};

    const TEST_SECRET: &str = "test-jwt-secret";

    fn set_jwt_secret() {
        unsafe { std::env::set_var("JWT_SECRET", TEST_SECRET) };
    }

    async fn echo_authed(user: AuthedUser) -> impl IntoResponse {
        axum::Json(serde_json::json!({ "user_id": user.user_id }))
    }

    async fn echo_admin(admin: AdminUser) -> impl IntoResponse {
        axum::Json(serde_json::json!({ "user_id": admin.user_id }))
    }

    // ------------------------------------------------------------------
    // AuthedUser
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn authed_user_accepts_valid_jwt() {
        set_jwt_secret();
        let app = Router::new().route("/test", get(echo_authed));
        let (token, _) = sign_jwt("user-1", "u@example.com", "free", TEST_SECRET).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["user_id"], "user-1");
    }

    #[tokio::test]
    async fn authed_user_rejects_missing_token() {
        set_jwt_secret();
        let app = Router::new().route("/test", get(echo_authed));

        let resp = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authed_user_rejects_invalid_token() {
        set_jwt_secret();
        let app = Router::new().route("/test", get(echo_authed));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("Authorization", "Bearer not-a-jwt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authed_user_uses_middleware_extension_fast_path() {
        set_jwt_secret();
        // Middleware injects UserId — no JWT token in request needed.
        async fn inject_uid(
            mut req: axum::http::Request<Body>,
            next: middleware::Next,
        ) -> axum::response::Response {
            req.extensions_mut()
                .insert(UserId("injected-uid".to_string()));
            next.run(req).await
        }

        let app = Router::new()
            .route("/test", get(echo_authed))
            .layer(middleware::from_fn(inject_uid));

        let resp = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["user_id"], "injected-uid");
    }

    // ------------------------------------------------------------------
    // AdminUser
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn admin_user_accepts_admin_tier() {
        set_jwt_secret();
        let app = Router::new().route("/admin", get(echo_admin));
        let (token, _) = sign_jwt("admin-1", "a@example.com", "admin", TEST_SECRET).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["user_id"], "admin-1");
    }

    #[tokio::test]
    async fn admin_user_rejects_non_admin_tier() {
        set_jwt_secret();
        let app = Router::new().route("/admin", get(echo_admin));
        let (token, _) = sign_jwt("user-1", "u@example.com", "free", TEST_SECRET).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_user_rejects_missing_token() {
        set_jwt_secret();
        let app = Router::new().route("/admin", get(echo_admin));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

// ===========================================================================
// CO-448 — capability gate integration tests (in-process, tower::ServiceExt)
// ===========================================================================

#[cfg(test)]
mod capability_gate_tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use tempfile::{TempDir, tempdir};
    use tower::ServiceExt;

    use super::{EntriesRead, EntriesWrite, Scoped};
    use crate::server::AppState;
    use crate::storage::Storage;

    const SECRET: &str = "test-jwt-secret";

    /// Build a full `AppState` over a fresh temp DB (no port binding).
    fn make_state() -> (AppState, TempDir) {
        unsafe { std::env::set_var("JWT_SECRET", SECRET) };
        use crate::config::WebConfig;
        use crate::experiment::ExperimentStore;
        use crate::server::{
            AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
        };
        use std::sync::Mutex;

        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path());
        let config = WebConfig {
            port: 0,
            data_dir: dir.path().to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        };
        let experiment = ExperimentStore::new(dir.path());
        let auth_store = crate::auth::AuthStore::new(dir.path()).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&dir.path().join("game_test.db")).expect("game db"),
        );
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let state = AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        (state, dir)
    }

    async fn read_handler(_cap: Scoped<EntriesRead>) -> impl IntoResponse {
        StatusCode::OK
    }
    async fn write_handler(_cap: Scoped<EntriesWrite>) -> impl IntoResponse {
        StatusCode::OK
    }

    /// Small router exposing one `entries:read` GET and one `entries:write` POST.
    fn gated_router(state: AppState) -> Router {
        Router::new()
            .route("/read", get(read_handler))
            .route("/write", post(write_handler))
            .with_state(state)
    }

    fn bearer(method: &str, uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    /// Mint a least-privilege token for `user` from a bundle/capability list.
    fn scoped_token(state: &AppState, user_id: &str, scopes: &[&str]) -> String {
        let requested: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
        let resolved = crate::auth::capabilities::resolve_scopes(&requested).unwrap();
        let storage = state.core.storage.lock();
        storage
            .create_api_token_with_scopes(user_id, "test", &resolved)
            .unwrap()
            .token
            .expect("raw token")
    }

    /// Acceptance (a): a `read`-bundle token reads (200) but cannot write (403).
    #[tokio::test]
    async fn read_token_reads_but_cannot_write() {
        let (state, _dir) = make_state();
        let user = state
            .core
            .storage
            .lock()
            .create_user("r@x.com", "R")
            .unwrap();
        let token = scoped_token(&state, &user.id, &["read"]);
        let app = gated_router(state);

        let read = app
            .clone()
            .oneshot(bearer("GET", "/read", &token))
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::OK, "read bundle must read");

        let write = app.oneshot(bearer("POST", "/write", &token)).await.unwrap();
        assert_eq!(
            write.status(),
            StatusCode::FORBIDDEN,
            "read bundle must NOT write (least-privilege)"
        );
    }

    /// Acceptance (c): a capability absent from the token → 403.
    #[tokio::test]
    async fn missing_capability_is_forbidden() {
        let (state, _dir) = make_state();
        let user = state
            .core
            .storage
            .lock()
            .create_user("c@x.com", "C")
            .unwrap();
        // Only entries:read — write capability is absent.
        let token = scoped_token(&state, &user.id, &["entries:read"]);
        let app = gated_router(state);

        let resp = app.oneshot(bearer("POST", "/write", &token)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// A `write`-bundle token writes (write ⊇ read, so the cap is present).
    #[tokio::test]
    async fn write_bundle_can_write() {
        let (state, _dir) = make_state();
        let user = state
            .core
            .storage
            .lock()
            .create_user("w@x.com", "W")
            .unwrap();
        let token = scoped_token(&state, &user.id, &["write"]);
        let app = gated_router(state);

        let resp = app.oneshot(bearer("POST", "/write", &token)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Acceptance (e): a NULL-scope token (legacy `create_api_token`) keeps the
    /// pre-CO-448 behavior — it can both read and write (inherits owner tier).
    #[tokio::test]
    async fn null_scope_token_inherits_tier_backward_compat() {
        let (state, _dir) = make_state();
        let user = state
            .core
            .storage
            .lock()
            .create_user("n@x.com", "N")
            .unwrap();
        let token = {
            let storage = state.core.storage.lock();
            storage
                .create_api_token(&user.id, "legacy")
                .unwrap()
                .token
                .expect("raw token")
        };
        let app = gated_router(state);

        let read = app
            .clone()
            .oneshot(bearer("GET", "/read", &token))
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::OK, "NULL token still reads");
        let write = app.oneshot(bearer("POST", "/write", &token)).await.unwrap();
        assert_eq!(
            write.status(),
            StatusCode::OK,
            "NULL token still writes (compat)"
        );
    }

    /// A plain JWT session passes the (non-admin) capability gate — capabilities
    /// only restrict API tokens, not full user sessions.
    #[tokio::test]
    async fn jwt_session_passes_non_admin_gate() {
        let (state, _dir) = make_state();
        let (jwt, _) = crate::auth::sign_jwt("u1", "u@x.com", "free", SECRET).unwrap();
        let app = gated_router(state);

        let resp = app.oneshot(bearer("POST", "/write", &jwt)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // --- Acceptance (b): admin surface via the real chat origin-breakdown route ---

    fn admin_app(state: AppState) -> Router {
        crate::server::build_router(state, None)
    }

    const ADMIN_URI: &str = "/api/v1/admin/chat/origin-breakdown";

    /// Acceptance (b): an `admin`-bundle token (which contains `chat:read`)
    /// reaches the admin chat telemetry surface — without being admin-pleno.
    #[tokio::test]
    async fn admin_bundle_token_reaches_admin_surface() {
        let (state, _dir) = make_state();
        let user = state
            .core
            .storage
            .lock()
            .create_user("a@x.com", "A")
            .unwrap();
        let token = scoped_token(&state, &user.id, &["admin"]);
        let app = admin_app(state);

        let resp = app.oneshot(bearer("GET", ADMIN_URI, &token)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "admin bundle grants chat:read"
        );
    }

    /// Acceptance (c) on the real surface: a `read`-bundle token lacks
    /// `chat:read` → 403 on the admin chat endpoint.
    #[tokio::test]
    async fn read_bundle_token_denied_on_admin_surface() {
        let (state, _dir) = make_state();
        let user = state
            .core
            .storage
            .lock()
            .create_user("ra@x.com", "RA")
            .unwrap();
        let token = scoped_token(&state, &user.id, &["read"]);
        let app = admin_app(state);

        let resp = app.oneshot(bearer("GET", ADMIN_URI, &token)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// On an admin surface, a NULL-scope token is admitted only when its owner is
    /// an admin (the pre-CO-448 admin gate is preserved for inherit-tier tokens).
    #[tokio::test]
    async fn null_scope_token_admin_surface_requires_admin_owner() {
        let (state, _dir) = make_state();
        // `create_user` defaults tier to 'admin'; demote `plain` to a non-admin
        // tier so the admin-surface inherit-tier check is meaningfully exercised.
        let admin = state
            .core
            .storage
            .lock()
            .create_user("adm@x.com", "Adm")
            .unwrap();
        let plain = state
            .core
            .storage
            .lock()
            .create_user("plain@x.com", "Pl")
            .unwrap();
        state
            .core
            .storage
            .lock()
            .conn()
            .execute(
                "UPDATE users SET tier = 'free' WHERE id = ?1",
                rusqlite::params![plain.id],
            )
            .unwrap();
        let admin_token = {
            let s = state.core.storage.lock();
            s.create_api_token(&admin.id, "legacy")
                .unwrap()
                .token
                .unwrap()
        };
        let plain_token = {
            let s = state.core.storage.lock();
            s.create_api_token(&plain.id, "legacy")
                .unwrap()
                .token
                .unwrap()
        };
        let app = admin_app(state);

        let ok = app
            .clone()
            .oneshot(bearer("GET", ADMIN_URI, &admin_token))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK, "admin-owned NULL token passes");

        let denied = app
            .oneshot(bearer("GET", ADMIN_URI, &plain_token))
            .await
            .unwrap();
        assert_eq!(
            denied.status(),
            StatusCode::FORBIDDEN,
            "non-admin NULL token denied on admin surface"
        );
    }
}
