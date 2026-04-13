//! GitHub PAT authentication middleware for gestao (admin) routes.
//!
//! Verifies the Bearer token is a valid GitHub PAT belonging to an allowed admin.
//! Caches verified tokens in memory to avoid repeated GitHub API calls.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Cached GitHub user info (verified PAT → username mapping).
pub type TokenCache = Arc<RwLock<HashMap<String, String>>>;

pub fn new_token_cache() -> TokenCache {
    Arc::new(RwLock::new(HashMap::new()))
}

/// GitHub user info from /user endpoint.
#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}

/// Extracted GitHub admin identity, available in request extensions.
#[derive(Clone, Debug)]
pub struct GitHubAdmin(pub String);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for GitHubAdmin {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = parts
            .extensions
            .get::<GitHubAdmin>()
            .cloned()
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": "GitHub authentication required"})),
                )
                    .into_response()
            });
        std::future::ready(result)
    }
}

/// Middleware that validates a GitHub PAT and checks the user is in the allowed admins list.
pub async fn require_github_admin(
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    // Extract token from Authorization header
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .ok_or_else(|| unauthorized("Missing Authorization header with GitHub token"))?;

    if token.is_empty() {
        return Err(unauthorized("Empty token"));
    }

    // Get config and cache from extensions
    let allowed_admins: Vec<String> = req
        .extensions()
        .get::<AllowedAdmins>()
        .map(|a| a.0.clone())
        .unwrap_or_default();

    let cache = req
        .extensions()
        .get::<TokenCache>()
        .cloned()
        .unwrap_or_else(new_token_cache);

    // Check cache first
    let cached_user = {
        let cache_read = cache.read().unwrap_or_else(|e| e.into_inner());
        cache_read.get(&token).cloned()
    };

    let github_login = if let Some(login) = cached_user {
        login
    } else {
        // Verify against GitHub API
        let login = verify_github_token(&token).await.map_err(|e| {
            tracing::warn!("GitHub auth failed: {e}");
            unauthorized("Invalid GitHub token")
        })?;

        // Cache the verified token
        if let Ok(mut cache_write) = cache.write() {
            cache_write.insert(token.clone(), login.clone());
        }

        login
    };

    // Check if user is in allowed admins
    if !allowed_admins.is_empty() && !allowed_admins.contains(&github_login.to_lowercase()) {
        tracing::warn!(
            "GitHub user '{}' not in allowed admins: {:?}",
            github_login,
            allowed_admins
        );
        return Err(forbidden(&format!(
            "User '{}' is not authorized for gestao",
            github_login
        )));
    }

    req.extensions_mut().insert(GitHubAdmin(github_login));
    Ok(next.run(req).await)
}

/// Verify a GitHub PAT by calling the GitHub API.
async fn verify_github_token(token: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "co-web")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("GitHub API returned {}", response.status()));
    }

    let user: GitHubUser = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub response: {e}"))?;

    Ok(user.login)
}

/// Config wrapper for allowed admin usernames, injected as extension.
#[derive(Clone, Debug)]
pub struct AllowedAdmins(pub Vec<String>);

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn forbidden(msg: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}
