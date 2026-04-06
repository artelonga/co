use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::Serialize;

use crate::auth::UserId;
use crate::error::AppError;
use crate::models::*;
use crate::server::AppState;

/// Public universe info returned by GET /:slug — no sensitive owner_id.
#[derive(Debug, Serialize)]
pub struct UniverseInfo {
    pub key: String,
    pub name: String,
    pub description: String,
    pub content_count: i64,
    pub is_anonymous: bool,
    pub is_template: bool,
}

fn lock_storage(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::storage::Storage>, AppError> {
    state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("Storage lock failed".into()))
}

fn validate_universe_key(key: &str) -> Result<(), AppError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Universe key cannot be empty".into()));
    }
    if key.len() < 2 || key.len() > 40 {
        return Err(AppError::BadRequest(
            "Universe key must be 2–40 characters".into(),
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(AppError::BadRequest(
            "Universe key must contain only lowercase letters, digits, and hyphens".into(),
        ));
    }
    Ok(())
}

// GET /api/v1/universes — list universes the caller belongs to
pub async fn list_universes(
    State(state): State<AppState>,
    user_id: UserId,
) -> Result<Json<Vec<Universe>>, AppError> {
    let storage = lock_storage(&state)?;
    let universes = storage.list_universes_for_user(&user_id.0);
    Ok(Json(universes))
}

// POST /api/v1/universes — create a universe (caller becomes owner)
pub async fn create_universe(
    State(state): State<AppState>,
    user_id: UserId,
    Json(body): Json<CreateUniverse>,
) -> Result<impl IntoResponse, AppError> {
    validate_universe_key(&body.key)?;
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("Universe name cannot be empty".into()));
    }
    if body.name.len() > 100 {
        return Err(AppError::BadRequest(
            "Universe name must be 100 characters or fewer".into(),
        ));
    }
    let universe = lock_storage(&state)?.create_universe(body, &user_id.0)?;
    Ok((StatusCode::CREATED, Json(universe)))
}

// GET /api/v1/universes/:key/members — list members (must be a member)
pub async fn list_members(
    State(state): State<AppState>,
    Path(key): Path<String>,
    user_id: UserId,
) -> Result<Json<Vec<UniverseMember>>, AppError> {
    let storage = lock_storage(&state)?;
    if storage.get_universe(&key).is_none() {
        return Err(AppError::NotFound(format!("Universe '{}' not found", key)));
    }
    if !storage.is_universe_member(&key, &user_id.0) {
        return Err(AppError::Forbidden("Not a member of this universe".into()));
    }
    Ok(Json(storage.list_universe_members(&key)))
}

// POST /api/v1/universes/:key/members — add a member (must be a member)
pub async fn add_member(
    State(state): State<AppState>,
    Path(key): Path<String>,
    user_id: UserId,
    Json(body): Json<AddMember>,
) -> Result<impl IntoResponse, AppError> {
    let valid_roles = ["owner", "admin", "member", "coordenacao"];
    if !valid_roles.contains(&body.role.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Invalid role '{}'. Must be one of: {}",
            body.role,
            valid_roles.join(", ")
        )));
    }
    {
        let storage = lock_storage(&state)?;
        if !storage.is_universe_member(&key, &user_id.0) {
            return Err(AppError::Forbidden("Not a member of this universe".into()));
        }
    }
    let member = lock_storage(&state)?.add_universe_member(&key, &body.user_id, &body.role)?;
    Ok((StatusCode::CREATED, Json(member)))
}

// DELETE /api/v1/universes/:key/members/:user_id — remove a member
pub async fn remove_member(
    State(state): State<AppState>,
    Path((key, member_id)): Path<(String, String)>,
    user_id: UserId,
) -> Result<StatusCode, AppError> {
    {
        let storage = lock_storage(&state)?;
        if !storage.is_universe_member(&key, &user_id.0) {
            return Err(AppError::Forbidden("Not a member of this universe".into()));
        }
    }
    lock_storage(&state)?.remove_universe_member(&key, &member_id)?;
    Ok(StatusCode::NO_CONTENT)
}

// GET /api/v1/universes/:slug/projects — public for public universes
pub async fn list_universe_projects(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<Project>>, AppError> {
    let storage = lock_storage(&state)?;
    let projects = storage
        .list_projects_for_public_universe(&slug)
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                AppError::NotFound(msg)
            } else {
                AppError::Forbidden(msg)
            }
        })?;
    Ok(Json(projects))
}

// POST /api/v1/universes/:slug/clone — clone a universe (no auth required)
pub async fn clone_universe(
    State(state): State<AppState>,
    Path(source_slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CloneUniverse>,
) -> Result<impl IntoResponse, AppError> {
    validate_universe_key(&body.key)?;
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("Universe name cannot be empty".into()));
    }
    if body.name.len() > 100 {
        return Err(AppError::BadRequest(
            "Universe name must be 100 characters or fewer".into(),
        ));
    }

    // Verify source universe exists and is clonable (must be public or template)
    {
        let storage = lock_storage(&state)?;
        match storage.get_universe(&source_slug) {
            None => {
                return Err(AppError::NotFound(format!(
                    "Universe '{}' not found",
                    source_slug
                )));
            }
            Some(u) if !u.is_public && !u.is_template => {
                return Err(AppError::Forbidden("Source universe is not public".into()));
            }
            _ => {}
        }
    }

    // Use authenticated user ID if Bearer token is present and valid, else anon
    let maybe_auth_id = extract_optional_user_id(&headers, &state);
    let is_anon = maybe_auth_id.is_none();
    let anon_id = format!("anon-{}", nanoid::nanoid!(10));
    let owner_id = maybe_auth_id.unwrap_or_else(|| anon_id.clone());

    let universe = lock_storage(&state)?.clone_universe(
        &source_slug,
        &body.key,
        &body.name,
        &body.description,
        &owner_id,
    )?;

    let mut response_headers = axum::http::HeaderMap::new();
    if is_anon {
        // Issue an anon JWT as session cookie so the browser can make write requests.
        let secret =
            std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".to_string());
        if let Ok((token, _)) = crate::auth::sign_jwt(&anon_id, "", "anon", &secret) {
            let cookie =
                format!("session={token}; Path=/; SameSite=Lax; HttpOnly; Max-Age=2592000");
            if let Ok(val) = axum::http::HeaderValue::from_str(&cookie) {
                response_headers.append(header::SET_COOKIE, val);
            }
        }
        // Also set co_universe_owner cookie (for the claim endpoint)
        let owner_cookie =
            format!("co_universe_owner={anon_id}; Path=/; SameSite=Lax; HttpOnly; Max-Age=2592000");
        if let Ok(val) = axum::http::HeaderValue::from_str(&owner_cookie) {
            response_headers.append(header::SET_COOKIE, val);
        }
    }

    Ok((StatusCode::CREATED, response_headers, Json(universe)))
}

// GET /api/v1/universes/:slug — public universe info (content_count, no owner_id)
pub async fn get_universe_info(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<UniverseInfo>, AppError> {
    let storage = lock_storage(&state)?;
    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
    Ok(Json(UniverseInfo {
        key: universe.key,
        name: universe.name,
        description: universe.description,
        content_count: universe.content_count,
        is_anonymous: universe.owner_id.starts_with("anon-"),
        is_template: universe.is_template,
    }))
}

// POST /api/v1/universes/:slug/claim — authenticated user claims an anonymous universe
pub async fn claim_universe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
    headers: HeaderMap,
) -> Result<Json<Universe>, AppError> {
    // Read co_universe_owner cookie
    let anon_id = extract_cookie(&headers, "co_universe_owner")
        .ok_or_else(|| AppError::BadRequest("Missing co_universe_owner cookie".into()))?;

    let universe = lock_storage(&state)?
        .claim_universe(&slug, &user_id.0, &anon_id)
        .map_err(|e| AppError::Forbidden(e.to_string()))?;

    Ok(Json(universe))
}

/// Extract a named cookie value from the Cookie header.
fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        let prefix = format!("{name}=");
        if let Some(val) = part.strip_prefix(&prefix) {
            let val = val.trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Try to extract a user ID from the Authorization header without hard-failing.
fn extract_optional_user_id(headers: &HeaderMap, _state: &AppState) -> Option<String> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))?;

    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".to_string());

    crate::auth::decode_user_id(token, &secret).ok()
}

pub fn router() -> Router<AppState> {
    // Public routes (no auth layer)
    let public_routes = Router::new()
        .route("/{slug}", get(get_universe_info))
        .route("/{slug}/projects", get(list_universe_projects))
        .route("/{slug}/clone", post(clone_universe));

    // Protected routes (auth required)
    let protected_routes = Router::new()
        .route("/", get(list_universes).post(create_universe))
        .route("/{key}/members", get(list_members).post(add_member))
        .route("/{key}/members/{user_id}", delete(remove_member))
        .route("/{slug}/claim", post(claim_universe))
        .layer(axum::middleware::from_fn(crate::auth::require_auth));

    Router::new().merge(public_routes).merge(protected_routes)
}
