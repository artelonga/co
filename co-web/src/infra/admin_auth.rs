//! CO-435: admin-auth provider — the extension seam for admin authentication.
//!
//! Before CO-435 admin auth was hardwired to GitHub PATs: the
//! `require_github_admin` middleware called the GitHub API directly, and ≥6
//! admin routers injected a `TokenCache` + `AllowedAdmins` pair the middleware
//! read. Swapping in SAML/OIDC meant rewriting every site.
//!
//! This module extracts the [`AdminAuthProvider`] trait. The default impl is
//! [`crate::github_auth::GitHubAdminAuthProvider`]; an `Arc<dyn AdminAuthProvider>`
//! is injected as an Axum `Extension`, so the middleware (and its call-sites)
//! depend on the trait, not on GitHub. A SAML/OIDC backend is now: a new
//! `impl AdminAuthProvider` + injecting it instead of the GitHub one.

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// A verified administrator identity.
#[derive(Clone, Debug)]
pub struct AdminIdentity {
    /// The admin's login/username (GitHub login for the default impl).
    pub login: String,
}

/// Why an admin-auth verification failed.
///
/// The variants map to the same HTTP responses the GitHub middleware has always
/// returned, so behaviour is unchanged.
#[derive(Debug)]
pub enum AdminAuthError {
    /// Missing/empty/invalid token → `401`.
    Unauthorized(String),
    /// Valid token but not an authorized admin → `403`.
    Forbidden(String),
}

impl IntoResponse for AdminAuthError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AdminAuthError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            AdminAuthError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
        };
        (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

/// Verifies that a bearer token belongs to an authorized administrator.
///
/// Default impl: [`crate::github_auth::GitHubAdminAuthProvider`] (GitHub PAT).
/// Future impls (SAML/OIDC) plug in without touching the middleware or routers.
#[async_trait]
pub trait AdminAuthProvider: Send + Sync {
    /// Resolve `token` to an [`AdminIdentity`], or fail with [`AdminAuthError`].
    async fn verify_admin(&self, token: &str) -> Result<AdminIdentity, AdminAuthError>;
}
