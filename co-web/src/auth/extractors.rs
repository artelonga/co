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

use axum::{
    Json,
    extract::MatchedPath,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};

use super::UserId;
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
            let storage = state.storage.lock();
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
            let storage = state.storage.lock();
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
