use std::hash::{Hash, Hasher};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde::Serialize;

use crate::auth::UserId;
use crate::error::AppError;
use crate::models::*;
use crate::server::AppState;
use rusqlite;

// ---------------------------------------------------------------------------
// Theme tier constants
// ---------------------------------------------------------------------------

/// Palette keys available to all users (anonymous or logged-in).
const FREE_PALETTES: &[&str] = &[
    "scholarly",
    "scholarly-light", // backward-compat alias stored in older DB rows
    "scholarly-dark",
    "relic",
    "relic-light",
];

/// Additional palette keys available only to real logged-in (non-anon) users.
const PREMIUM_PALETTES: &[&str] = &["", "modern"];

/// Variant keys (a–h) available only to real logged-in users.
const PREMIUM_VARIANTS: &[&str] = &["a", "b", "c", "d", "e", "f", "g", "h"];

/// Public universe info returned by GET /:slug — no sensitive owner_id.
#[derive(Debug, Serialize)]
pub struct UniverseInfo {
    pub key: String,
    pub name: String,
    pub description: String,
    pub content_count: i64,
    pub is_anonymous: bool,
    pub is_template: bool,
    /// CO-38: signals to the frontend that login is required to use this universe.
    pub requires_login: bool,
    /// CO-49: single visibility field replacing the boolean flags.
    pub visibility: String,
    /// CO-98: optional parent universe key for hierarchical grouping in the
    /// sidebar (e.g. timeline trio under `template`). `None` for top-level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_key: Option<String>,
}

/// Query params for universe search.
#[derive(Debug, serde::Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
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
    let mut storage = lock_storage(&state)?;
    if storage.get_universe(&body.key).is_some() {
        return Err(AppError::Conflict(format!(
            "Universe key '{}' is already taken",
            body.key
        )));
    }
    let universe = storage.create_universe(body, &user_id.0)?;
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
    let valid_roles = [
        "owner",
        "admin",
        "editor",
        "viewer",
        "member",
        "coordenacao",
    ];
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
    headers: HeaderMap,
) -> Result<Json<Vec<Project>>, AppError> {
    let storage = lock_storage(&state)?;

    // First try public access (template or public universes)
    match storage.list_projects_for_public_universe(&slug) {
        Ok(projects) => Ok(Json(projects)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                return Err(AppError::NotFound(msg));
            }
            // Not public — check if caller is the owner
            let caller_id = extract_optional_user_id(&headers, &state);
            if let Some(uid) = caller_id
                && let Some(universe) = storage.get_universe(&slug)
                && (universe.owner_id == uid || storage.is_universe_member(&slug, &uid))
            {
                let projects = storage.list_projects_for_universe(&slug);
                return Ok(Json(projects));
            }
            Err(AppError::Forbidden(msg))
        }
    }
}

// PUT /api/v1/universes/:slug — update universe name/description (owner only)
pub async fn update_universe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let caller_id = extract_optional_user_id(&headers, &state)
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".into()))?;

    let storage = lock_storage(&state)?;
    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

    if universe.owner_id != caller_id {
        return Err(AppError::Forbidden(
            "Only the owner can update this universe".into(),
        ));
    }

    drop(storage);

    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        let name = name.trim();
        if name.is_empty() || name.len() > 100 {
            return Err(AppError::BadRequest("Name must be 1-100 characters".into()));
        }
        let storage = lock_storage(&state)?;
        storage
            .conn()
            .execute(
                "UPDATE universes SET name = ?1 WHERE key = ?2",
                rusqlite::params![name, slug],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    if let Some(desc) = body.get("description").and_then(|v| v.as_str()) {
        let storage = lock_storage(&state)?;
        storage
            .conn()
            .execute(
                "UPDATE universes SET description = ?1 WHERE key = ?2",
                rusqlite::params![desc, slug],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    if let Some(vis) = body.get("visibility").and_then(|v| v.as_str()) {
        // Owners may flip between these three; "template" is system-only.
        let (is_public, requires_login) = match vis {
            "private" => (0, 0),
            "public-subscribable" => (1, 0),
            "requires_login" => (0, 1),
            _ => {
                return Err(AppError::BadRequest(format!(
                    "Invalid visibility '{}'. Must be: private, public-subscribable, requires_login",
                    vis
                )));
            }
        };
        let storage = lock_storage(&state)?;
        storage
            .conn()
            .execute(
                "UPDATE universes SET visibility = ?1, is_public = ?2, requires_login = ?3 \
                 WHERE key = ?4",
                rusqlite::params![vis, is_public, requires_login, slug],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    let storage = lock_storage(&state)?;
    let updated = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::Internal("Universe disappeared".into()))?;

    Ok(Json(serde_json::json!({
        "key": updated.key,
        "name": updated.name,
        "description": updated.description,
        "visibility": updated.visibility,
    })))
}

/// POST /api/v1/universes/:slug/duplicate — create an owner-controlled copy of
/// a universe (CO-95 preview). Unlike `/clone` (which is anon-friendly and
/// gated on the source being public/template), `/duplicate` requires
/// authentication and lets the caller copy ANY universe they have read access
/// to (owner, member, or public).
///
/// The new universe is owned by the caller, defaults to `private`, and is a
/// snapshot of the source's entries at the moment of duplication. Future ops
/// on either side are independent — no shared state. (Lineage tracking via
/// `parent_universe_key` is CO-95 territory and not in this preview.)
///
/// Use case: `quilomboaraucaria` → `quilombo-blog` for parallel testing,
/// scalability analysis, or a materialized "dev branch" of any universe.
pub async fn duplicate_universe(
    State(state): State<AppState>,
    Path(source_slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CloneUniverse>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = crate::auth::resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    validate_universe_key(&body.key)?;
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("Universe name cannot be empty".into()));
    }
    if body.name.len() > 100 {
        return Err(AppError::BadRequest(
            "Universe name must be 100 characters or fewer".into(),
        ));
    }

    // Verify source exists and caller has read access (owner, member,
    // template, or public-subscribable).
    {
        let storage = lock_storage(&state)?;
        let source = storage
            .get_universe(&source_slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", source_slug)))?;
        let has_access = source.owner_id == user_id
            || source.is_template
            || source.is_public
            || storage.is_universe_member(&source_slug, &user_id);
        if !has_access {
            return Err(AppError::Forbidden(
                "Not authorized to duplicate this universe".into(),
            ));
        }
        if storage.get_universe(&body.key).is_some() {
            return Err(AppError::Conflict(format!(
                "Universe key '{}' is already taken",
                body.key
            )));
        }
    }

    let universe = lock_storage(&state)?.clone_universe(
        &source_slug,
        &body.key,
        &body.name,
        &body.description,
        &user_id,
    )?;

    Ok((StatusCode::CREATED, Json(universe)))
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

// GET /api/v1/universes/search?q=... — search public-subscribable universes
pub async fn search_universes(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Vec<Universe>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(storage.search_public_universes(&params.q)))
}

// POST /api/v1/universes/:slug/subscribe — subscribe to a public universe (auth required)
pub async fn subscribe_universe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
) -> Result<StatusCode, AppError> {
    lock_storage(&state)?
        .subscribe_universe(&user_id.0, &slug)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/v1/universes/:slug/subscribe — unsubscribe (auth required)
pub async fn unsubscribe_universe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
) -> Result<StatusCode, AppError> {
    lock_storage(&state)?
        .unsubscribe_universe(&user_id.0, &slug)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// GET /api/v1/universes/:slug/subscribers — list subscribers (owner only)
pub async fn list_subscribers(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
) -> Result<Json<Vec<Subscription>>, AppError> {
    let storage = lock_storage(&state)?;
    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
    if universe.owner_id != user_id.0 {
        return Err(AppError::Forbidden(
            "Only the owner can list subscribers".into(),
        ));
    }
    Ok(Json(storage.list_universe_subscribers(&slug)))
}

// GET /api/v1/universes/:slug — public universe info (content_count, no owner_id)
//
// CO-49: uses deterministic 7-step access check.
// Anonymous universes remain visible only to their cookie-identified owner.
pub async fn get_universe_info(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<UniverseInfo>, AppError> {
    let caller_id = extract_optional_user_id(&headers, &state);

    // For anonymous universes (owned by anon-*), we still use the cookie gate
    // because those don't have a visibility flag yet — they're always private.
    let storage = lock_storage(&state)?;
    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

    if universe.owner_id.starts_with("anon-") {
        let cookie_owner = extract_cookie(&headers, "co_universe_owner");
        if cookie_owner.as_deref() != Some(universe.owner_id.as_str()) {
            return Err(AppError::NotFound(format!("Universe '{}' not found", slug)));
        }
    }

    // CO-49: deterministic access check.
    let access = storage.check_universe_access(caller_id.as_deref(), &slug);
    match access {
        crate::models::UniverseAccess::Denied => {
            return Err(AppError::NotFound(format!("Universe '{}' not found", slug)));
        }
        crate::models::UniverseAccess::LoginRequired => {
            return Err(AppError::Unauthorized(
                "Login required to access this universe".into(),
            ));
        }
        crate::models::UniverseAccess::MetadataOnly => {
            // Public-subscribable: return metadata only (no content access).
        }
        _ => {
            // ReadOnly or ReadWrite: full info.
        }
    }

    Ok(Json(UniverseInfo {
        key: universe.key,
        name: universe.name,
        description: universe.description,
        content_count: universe.content_count,
        is_anonymous: universe.owner_id.starts_with("anon-"),
        is_template: universe.is_template,
        requires_login: universe.requires_login,
        visibility: universe.visibility,
        parent_key: universe.parent_key,
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

// GET /api/v1/themes/available — optional auth; returns theme tier for the caller
pub async fn get_available_themes(headers: HeaderMap) -> Json<AvailableThemes> {
    let is_real_user = extract_optional_claims(&headers)
        .map(|c| c.tier != "anon" && !c.sub.starts_with("anon-"))
        .unwrap_or(false);

    if is_real_user {
        Json(AvailableThemes {
            palettes: ["", "scholarly", "scholarly-dark", "relic", "relic-light"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            variants: PREMIUM_VARIANTS.iter().map(|s| s.to_string()).collect(),
            custom: Some(true),
        })
    } else {
        Json(AvailableThemes {
            palettes: ["scholarly", "scholarly-dark", "relic", "relic-light"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            variants: vec![],
            custom: None,
        })
    }
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

/// Try to extract a user ID from the Authorization header or session cookie without hard-failing.
fn extract_optional_user_id(headers: &HeaderMap, _state: &AppState) -> Option<String> {
    extract_optional_claims(headers).map(|c| c.sub)
}

/// Try to decode full JWT claims from Authorization header or session cookie without hard-failing.
fn extract_optional_claims(headers: &HeaderMap) -> Option<crate::auth::Claims> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| crate::auth::extract_session_cookie(headers))?;

    let secret = crate::auth::jwt_secret();
    crate::auth::decode_claims(&token, &secret).ok()
}

// GET /api/v1/universes/quilomboaraucaria/stats — public stats endpoint (CO-41)
pub async fn quilombo_stats(
    State(state): State<AppState>,
) -> Result<Json<crate::models::QuilomboStats>, AppError> {
    let storage = lock_storage(&state)?;
    let stats = storage.quilombo_stats();
    Ok(Json(stats))
}

// GET /api/v1/universes/:slug/config — public: returns presentation config
pub async fn get_universe_config(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<UniverseFormConfig>, AppError> {
    let storage = lock_storage(&state)?;
    let config = storage
        .get_universe_form_config(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
    Ok(Json(config))
}

// PUT /api/v1/universes/:slug/config — owner only: update presentation config
pub async fn update_universe_config(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
    Json(body): Json<UpdateUniverseFormConfig>,
) -> Result<Json<UniverseFormConfig>, AppError> {
    // Verify universe exists and caller is the owner.
    {
        let storage = lock_storage(&state)?;
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
        if universe.owner_id != user_id.0 {
            return Err(AppError::Forbidden(
                "Only the owner can update universe config".into(),
            ));
        }
    }

    // Validate layout value.
    if let Some(ref layout) = body.layout {
        let valid = ["board", "table", "timeline", "calendar", "dashboard"];
        if !valid.contains(&layout.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Invalid layout '{layout}'. Must be one of: {}",
                valid.join(", ")
            )));
        }
    }

    // Validate theme_preset value against the caller's tier.
    if let Some(ref theme) = body.theme_preset {
        let is_anon = user_id.0.starts_with("anon-");
        if FREE_PALETTES.contains(&theme.as_str()) {
            // Free palette — always allowed.
        } else if PREMIUM_PALETTES.contains(&theme.as_str()) {
            if is_anon {
                return Err(AppError::Forbidden(format!(
                    "Theme '{theme}' requires a logged-in account"
                )));
            }
        } else {
            return Err(AppError::BadRequest(format!(
                "Invalid theme_preset '{theme}'. Must be one of: {}",
                FREE_PALETTES
                    .iter()
                    .chain(PREMIUM_PALETTES.iter())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }

    let config = lock_storage(&state)?
        .update_universe_form_config(&slug, body)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(config))
}

// ---------------------------------------------------------------------------
// Theme CSS endpoint (CO-30)
// ---------------------------------------------------------------------------

/// Compute a stable ETag from the active theme preset name + serialized custom tokens.
fn config_etag(theme_preset: &str, custom_tokens: Option<&serde_json::Value>) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    theme_preset.hash(&mut hasher);
    if let Some(tokens) = custom_tokens {
        tokens.to_string().hash(&mut hasher);
    }
    format!("\"{}\"", hasher.finish())
}

/// GET /api/v1/themes/:preset.css
///
/// Returns the CSS for a built-in preset by name, independent of any universe.
/// Used by the SPA when `co_user_palette` is set so the user's preferred theme
/// wins regardless of the active board's stored `theme_preset`.
///
/// Unknown preset → 404. Cached aggressively since the preset definitions are
/// compiled in and don't change at runtime.
pub async fn get_preset_theme_css(
    Path(preset_name): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Strip optional `.css` suffix so the route matches both `/themes/modern`
    // and `/themes/modern.css`.
    let name = preset_name
        .strip_suffix(".css")
        .unwrap_or(&preset_name)
        .to_string();

    let preset = match crate::theme_engine::ThemePreset::by_name(&name) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain")],
                format!("Unknown theme preset '{name}'"),
            )
                .into_response();
        }
    };

    let etag = config_etag(&name, None);
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && inm == etag
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let css = crate::theme_engine::generate_css(&preset, None);

    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("public, max-age=300, must-revalidate"),
            ),
            (
                header::ETAG,
                axum::http::HeaderValue::from_str(&etag)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("\"0\"")),
            ),
        ],
        css,
    )
        .into_response()
}

/// GET /api/v1/universes/:slug/theme.css
///
/// Returns a generated CSS stylesheet with all design tokens for the universe's
/// active theme preset, merged with any custom token overrides set by the owner.
/// Responds with `Cache-Control: no-cache` and an ETag derived from the config.
pub async fn get_universe_theme_css(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let config = {
        let Ok(storage) = lock_storage(&state) else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain")],
                "Storage error".to_string(),
            )
                .into_response();
        };
        match storage.get_universe_form_config(&slug) {
            Some(c) => c,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    [(header::CONTENT_TYPE, "text/plain")],
                    format!("Universe '{}' not found", slug),
                )
                    .into_response();
            }
        }
    };

    let etag = config_etag(&config.theme_preset, config.custom_tokens.as_ref());

    // Honour If-None-Match conditional request.
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && inm == etag
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let preset = crate::theme_engine::ThemePreset::by_name(&config.theme_preset)
        .unwrap_or_else(|| crate::theme_engine::ThemePreset::by_name("modern").unwrap());

    let css = crate::theme_engine::generate_css(&preset, config.custom_tokens.as_ref());

    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-cache"),
            ),
            (
                header::ETAG,
                axum::http::HeaderValue::from_str(&etag)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("\"0\"")),
            ),
        ],
        css,
    )
        .into_response()
}

pub fn router() -> Router<AppState> {
    // Public routes (no auth layer)
    let public_routes = Router::new()
        // CO-41: specific literal route must come before /{slug} wildcard
        .route("/quilomboaraucaria/stats", get(quilombo_stats))
        // CO-49: universe search (no auth required)
        .route("/search", get(search_universes))
        .route("/{slug}", get(get_universe_info))
        .route("/{slug}/config", get(get_universe_config))
        .route("/{slug}/theme.css", get(get_universe_theme_css))
        .route("/{slug}/projects", get(list_universe_projects))
        .route("/{slug}/clone", post(clone_universe))
        // CO-95 preview: owner-controlled duplication. Auth resolved inline
        // (accepts JWT or API token) since this isn't behind require_auth.
        .route("/{slug}/duplicate", post(duplicate_universe))
        // CO-72: doc-gen last error is readable by the owner (auth resolved inline)
        .route(
            "/{slug}/jobs/doc-gen/last-error",
            get(get_doc_gen_last_error),
        );

    // Protected routes (auth required)
    let protected_routes = Router::new()
        .route("/", get(list_universes).post(create_universe))
        .route("/{key}/members", get(list_members).post(add_member))
        .route("/{key}/members/{user_id}", delete(remove_member))
        .route("/{slug}", put(update_universe))
        .route("/{slug}/claim", post(claim_universe))
        .route("/{slug}/config", put(update_universe_config))
        // CO-49: subscription routes
        .route(
            "/{slug}/subscribe",
            post(subscribe_universe).delete(unsubscribe_universe),
        )
        .route("/{slug}/subscribers", get(list_subscribers))
        // CO-72: doc-gen job submission (owner only, auth via require_auth layer)
        .route("/{slug}/jobs/doc-gen", post(submit_doc_gen_job))
        .layer(axum::middleware::from_fn(crate::auth::require_auth));

    Router::new().merge(public_routes).merge(protected_routes)
}

// ---------------------------------------------------------------------------
// CO-72: Doc-generator job submission and status
// ---------------------------------------------------------------------------

/// Request body for `POST /api/v1/universes/:slug/jobs/doc-gen`.
#[derive(Debug, serde::Deserialize)]
pub struct DocGenRequest {
    /// One of: scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc.
    pub format: String,
    /// Relative or absolute path to the source directory (e.g. `src/main/scala`).
    pub source_dir: String,
    /// Entry type tag for generated entries (e.g. `doc.scala`). Defaults to
    /// the adapter's built-in output type when empty.
    #[serde(default)]
    pub output_type: String,
}

/// POST /api/v1/universes/:slug/jobs/doc-gen — submit a doc-gen job (owner only).
pub async fn submit_doc_gen_job(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
    Json(body): Json<DocGenRequest>,
) -> Result<impl IntoResponse, AppError> {
    use std::str::FromStr as _;

    // Validate format early.
    let doc_format = crate::doc_gen::DocFormat::from_str(&body.format).map_err(|_| {
        AppError::BadRequest(format!(
            "Unknown doc format '{}'. Supported: scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc",
            body.format
        ))
    })?;

    if body.source_dir.trim().is_empty() {
        return Err(AppError::BadRequest("source_dir cannot be empty".into()));
    }

    let storage = lock_storage(&state)?;
    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

    if universe.owner_id != user_id.0 {
        return Err(AppError::Forbidden(
            "Only the owner can submit doc-gen jobs".into(),
        ));
    }

    let output_type = if body.output_type.is_empty() {
        format!("doc.{}", doc_format.as_str())
    } else {
        body.output_type.clone()
    };

    let payload = crate::job_queue::DocGenPayload {
        format: body.format,
        source_dir: body.source_dir,
        output_type,
        limits: crate::doc_gen::ResourceLimits::default(),
    };

    let job_id = crate::job_queue::enqueue_doc_gen(storage.conn(), &slug, &payload)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "job_id": job_id })),
    ))
}

/// Last doc-gen error info returned by the status endpoint.
#[derive(Debug, serde::Serialize)]
pub struct DocGenErrorInfo {
    pub universe_key: String,
    pub error: Option<String>,
    pub error_at: Option<String>,
}

/// GET /api/v1/universes/:slug/jobs/doc-gen/last-error — last failure (owner only, auth inline).
pub async fn get_doc_gen_last_error(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<DocGenErrorInfo>, AppError> {
    let caller_id = extract_optional_user_id(&headers, &state)
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".into()))?;

    let storage = lock_storage(&state)?;
    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

    if universe.owner_id != caller_id {
        return Err(AppError::Forbidden(
            "Only the owner can view doc-gen errors".into(),
        ));
    }

    let (error, error_at): (Option<String>, Option<String>) = storage
        .conn()
        .query_row(
            "SELECT doc_gen_error, doc_gen_error_at FROM universes WHERE key = ?1",
            rusqlite::params![slug],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(DocGenErrorInfo {
        universe_key: slug,
        error,
        error_at,
    }))
}

/// Standalone router for the `/api/v1/themes` namespace (no auth layer).
pub fn themes_router() -> Router<AppState> {
    Router::new()
        .route("/available", get(get_available_themes))
        // Direct preset CSS — used by the SPA's user-level palette override so
        // we don't depend on any universe's stored theme_preset.
        // Route is `/{preset}` (without `.css`) because Axum's matchit doesn't
        // accept literal suffixes on dynamic segments. The handler still
        // tolerates a `.css` suffix on the preset name.
        .route("/{preset}", get(get_preset_theme_css))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::tempdir;

    use crate::models::UpdateUniverseFormConfig;
    use crate::storage::Storage;

    fn make_storage() -> (Storage, tempfile::TempDir) {
        // SAFETY: single-threaded test environment.
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path());
        (storage, dir)
    }

    /// After migration v14, a universe gets scholarly-light theme and board layout by default.
    #[test]
    fn test_universe_form_config_defaults() {
        let (storage, _dir) = make_storage();
        let config = storage
            .get_universe_form_config("default")
            .expect("default universe must exist");
        assert_eq!(config.theme_preset, "scholarly-light");
        assert_eq!(config.layout, "board");
        assert!(config.font_headline.is_none());
        assert!(config.font_body.is_none());
        assert!(config.custom_tokens.is_none());
    }

    /// Updating theme_preset changes only that field; layout is preserved.
    #[test]
    fn test_update_form_config_theme() {
        let (mut storage, _dir) = make_storage();
        let updated = storage
            .update_universe_form_config(
                "default",
                UpdateUniverseFormConfig {
                    theme_preset: Some("relic-dark".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.theme_preset, "relic-dark");
        assert_eq!(updated.layout, "board"); // unchanged

        // Persisted correctly.
        let persisted = storage.get_universe_form_config("default").unwrap();
        assert_eq!(persisted.theme_preset, "relic-dark");
    }

    /// Cloning a universe copies its form config exactly.
    #[test]
    fn test_clone_universe_inherits_form_config() {
        let (mut storage, _dir) = make_storage();

        // Give the default universe a custom theme + layout.
        storage
            .update_universe_form_config(
                "default",
                UpdateUniverseFormConfig {
                    theme_preset: Some("scholarly-dark".to_string()),
                    layout: Some("calendar".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        // Make default public so it can be cloned.
        storage
            .conn()
            .execute(
                "UPDATE universes SET is_public = 1 WHERE key = 'default'",
                params![],
            )
            .unwrap();

        storage
            .clone_universe("default", "clone1", "Clone 1", "", "usr_test")
            .unwrap();

        let clone_config = storage
            .get_universe_form_config("clone1")
            .expect("clone must have form config");
        assert_eq!(clone_config.theme_preset, "scholarly-dark");
        assert_eq!(clone_config.layout, "calendar");
    }

    /// Changing form config does not affect entries in the same universe.
    #[test]
    fn test_form_config_change_does_not_affect_entries() {
        let (mut storage, _dir) = make_storage();

        // Create a project entry so entries table is non-empty.
        let universe_root = storage.universe_root("default");
        let entry = crate::entry_index::make_entry(
            "projects/TEST/_project.md",
            serde_json::json!({
                "type": "project",
                "key": "TEST",
                "title": "Test",
                "status": "active",
                "next_id": 1,
                "archived": false,
                "tags": []
            }),
            "Test project",
        );
        co::entry::write_entry(&universe_root, &entry).unwrap();
        crate::entry_index::EntryIndex::new(storage.conn())
            .upsert("default", &entry)
            .unwrap();

        // Change theme.
        storage
            .update_universe_form_config(
                "default",
                UpdateUniverseFormConfig {
                    theme_preset: Some("relic".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        // Entry still present and unmodified.
        let index = crate::entry_index::EntryIndex::new(storage.conn());
        let count = index.count("default", Some("project"));
        assert!(
            count > 0,
            "project entries must still be present after config change"
        );

        // Config changed.
        let config = storage.get_universe_form_config("default").unwrap();
        assert_eq!(config.theme_preset, "relic");
    }

    /// `.universo.yaml` is written when form config is updated.
    #[test]
    fn test_universo_yaml_written_on_update() {
        let (mut storage, _dir) = make_storage();

        storage
            .update_universe_form_config(
                "default",
                UpdateUniverseFormConfig {
                    theme_preset: Some("relic-light".to_string()),
                    layout: Some("table".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        let yaml_path = storage.universe_root("default").join(".universo.yaml");
        assert!(yaml_path.exists(), ".universo.yaml must be written");
        let contents = std::fs::read_to_string(yaml_path).unwrap();
        assert!(contents.contains("relic-light"));
        assert!(contents.contains("table"));
    }

    // --- CO-25: theme gating ---

    /// Anonymous user (no auth header) sees 4 free palettes, no variants, no custom editor.
    #[tokio::test]
    async fn test_themes_available_anonymous() {
        let headers = axum::http::HeaderMap::new();
        let axum::Json(themes) = super::get_available_themes(headers).await;

        assert_eq!(
            themes.palettes,
            vec!["scholarly", "scholarly-dark", "relic", "relic-light"]
        );
        assert!(themes.variants.is_empty());
        assert!(themes.custom.is_none());
    }

    /// Real logged-in user sees Modern + 4 free palettes + 8 variants + custom editor.
    #[tokio::test]
    async fn test_themes_available_logged_in() {
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let (token, _) =
            crate::auth::sign_jwt("usr_real", "user@example.com", "player", "test-secret").unwrap();

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let axum::Json(themes) = super::get_available_themes(headers).await;

        assert_eq!(
            themes.palettes,
            vec!["", "scholarly", "scholarly-dark", "relic", "relic-light"]
        );
        assert_eq!(themes.variants.len(), 8);
        assert_eq!(themes.custom, Some(true));
    }

    /// Anon-tier user (cookie JWT with tier="anon") sees only free palettes.
    #[tokio::test]
    async fn test_themes_available_anon_cookie() {
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let (token, _) = crate::auth::sign_jwt("anon-abc123", "", "anon", "test-secret").unwrap();

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            format!("session={token}").parse().unwrap(),
        );
        let axum::Json(themes) = super::get_available_themes(headers).await;

        assert_eq!(
            themes.palettes,
            vec!["scholarly", "scholarly-dark", "relic", "relic-light"]
        );
        assert!(themes.variants.is_empty());
    }

    /// A premium theme (scholarly, relic) set by an owner persists even if the user logs out —
    /// the storage layer always returns the stored preset regardless of auth.
    #[test]
    fn test_premium_theme_persists_after_owner_sets_it() {
        let (mut storage, _dir) = make_storage();

        // Owner sets a premium theme while logged in.
        storage
            .update_universe_form_config(
                "default",
                UpdateUniverseFormConfig {
                    theme_preset: Some("relic".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        // Reading config back (as if a new, unauthenticated visitor renders the universe)
        // must still return the premium theme — gating only applies to the switcher UI.
        let config = storage.get_universe_form_config("default").unwrap();
        assert_eq!(config.theme_preset, "relic");
    }

    // --- CO-30: theme.css endpoint ---

    /// Build a minimal in-process router for the universe API (no port binding).
    fn make_universe_router(
        storage: Storage,
        dir: &std::path::Path,
    ) -> (axum::Router, tempfile::TempDir) {
        use crate::config::WebConfig;
        use crate::experiment::ExperimentStore;
        use crate::server::{AppState, AppStateInner, build_router};
        use std::sync::{Arc, Mutex};

        let config = WebConfig {
            port: 0,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
        };
        let experiment = ExperimentStore::new(dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
        let state: AppState = Arc::new(AppStateInner {
            storage: Mutex::new(storage),
            experiment: Mutex::new(experiment),
            config,
            auth_store: Mutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            doc_rooms: crate::ws::new_room_manager(),
        });
        let router = build_router(state, None);
        let tmp = tempdir().unwrap(); // keep alive
        (router, tmp)
    }

    async fn body_bytes(response: axum::http::Response<axum::body::Body>) -> String {
        use http_body_util::BodyExt;
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// GET /api/v1/universes/default/theme.css returns 200 with :root block.
    #[tokio::test]
    async fn test_theme_css_returns_ok() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let (storage, dir) = make_storage();
        let (router, _tmp) = make_universe_router(storage, dir.path());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/default/theme.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let ct = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("text/css"), "Content-Type must be text/css");
        let body = body_bytes(response).await;
        assert!(body.contains(":root {"), "CSS must contain :root block");
        assert!(body.contains("--bg:"), "CSS must contain --bg token");
        assert!(
            body.contains("--accent:"),
            "CSS must contain --accent token"
        );
    }

    /// All required tokens are present in the generated CSS.
    #[tokio::test]
    async fn test_theme_css_all_required_tokens() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let (storage, dir) = make_storage();
        let (router, _tmp) = make_universe_router(storage, dir.path());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/default/theme.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_bytes(response).await;

        for token in crate::theme_engine::tests::REQUIRED_TOKENS {
            assert!(
                body.contains(*token),
                "theme.css must contain token '{token}'"
            );
        }
    }

    /// Changing the theme changes the CSS output.
    #[tokio::test]
    async fn test_theme_css_changes_when_theme_changes() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let (mut storage, dir) = make_storage();

        // Set theme to scholarly-dark
        storage
            .update_universe_form_config(
                "default",
                UpdateUniverseFormConfig {
                    theme_preset: Some("scholarly-dark".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let (router, _tmp) = make_universe_router(storage, dir.path());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/default/theme.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_bytes(response).await;

        // scholarly-dark --bg is #1c1610
        assert!(
            body.contains("#1c1610"),
            "scholarly-dark --bg must be #1c1610"
        );
        // Must NOT have scholarly-light --bg
        assert!(
            !body.contains("#FFF9ED"),
            "scholarly-dark must not contain scholarly-light --bg"
        );
    }

    /// GET /theme.css for a missing universe returns 404.
    #[tokio::test]
    async fn test_theme_css_404_for_missing_universe() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let (storage, dir) = make_storage();
        let (router, _tmp) = make_universe_router(storage, dir.path());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/no-such-universe/theme.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    /// ETag is present and the same ETag triggers 304 Not Modified.
    #[tokio::test]
    async fn test_theme_css_etag_304() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let (storage, dir) = make_storage();
        let (router, _tmp) = make_universe_router(storage, dir.path());

        // First request: capture ETag.
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/default/theme.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let etag = response
            .headers()
            .get(axum::http::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();

        // Second request with If-None-Match: expect 304.
        let response2 = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/default/theme.css")
                    .header(axum::http::header::IF_NONE_MATCH, &etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response2.status(), axum::http::StatusCode::NOT_MODIFIED);
    }

    // --- CO-49: deterministic access check ---

    /// Helper: set visibility on a universe.
    fn set_visibility(storage: &Storage, key: &str, visibility: &str) {
        storage.conn()
            .execute(
                "UPDATE universes SET visibility = ?1, is_public = ?2, is_template = ?3 WHERE key = ?4",
                rusqlite::params![
                    visibility,
                    if visibility == "public-subscribable" || visibility == "public" { 1i64 } else { 0i64 },
                    if visibility == "template" { 1i64 } else { 0i64 },
                    key
                ],
            )
            .unwrap();
    }

    /// 1. Template universe → READ for everyone (anonymous).
    #[test]
    fn test_access_template_anonymous() {
        let (mut storage, _dir) = make_storage();
        set_visibility(&storage, "default", "template");
        let access = storage.check_universe_access(None, "default");
        assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
    }

    /// 1. Template universe → READ for logged-in user too.
    #[test]
    fn test_access_template_logged_in() {
        let (mut storage, _dir) = make_storage();
        set_visibility(&storage, "default", "template");
        let access = storage.check_universe_access(Some("some-user"), "default");
        assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
    }

    /// 2. Owner → READ+WRITE regardless of visibility.
    #[test]
    fn test_access_owner_readwrite() {
        let (mut storage, _dir) = make_storage();
        // "default" universe is owned by "system"; create one owned by test-owner.
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "my-uni".into(),
                    name: "My Universe".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        let access = storage.check_universe_access(Some("owner-1"), "my-uni");
        assert_eq!(access, crate::models::UniverseAccess::ReadWrite);
    }

    /// 3. Member with editor role → READ+WRITE.
    #[test]
    fn test_access_editor_member_readwrite() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "collab".into(),
                    name: "Collab".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        storage
            .add_universe_member("collab", "editor-1", "editor")
            .unwrap();
        let access = storage.check_universe_access(Some("editor-1"), "collab");
        assert_eq!(access, crate::models::UniverseAccess::ReadWrite);
    }

    /// 4. Member with viewer role → READ only.
    #[test]
    fn test_access_viewer_member_readonly() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "readonly-uni".into(),
                    name: "Read Only".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        storage
            .add_universe_member("readonly-uni", "viewer-1", "viewer")
            .unwrap();
        let access = storage.check_universe_access(Some("viewer-1"), "readonly-uni");
        assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
    }

    /// 5. Subscribed user → READ only.
    #[test]
    fn test_access_subscribed_readonly() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "pub-uni".into(),
                    name: "Public".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        set_visibility(&storage, "pub-uni", "public-subscribable");
        storage.subscribe_universe("sub-user", "pub-uni").unwrap();
        let access = storage.check_universe_access(Some("sub-user"), "pub-uni");
        assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
    }

    /// 6. Public-subscribable universe → MetadataOnly for non-subscribed anonymous.
    #[test]
    fn test_access_public_subscribable_anonymous_metadata_only() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "disco".into(),
                    name: "Discoverable".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        set_visibility(&storage, "disco", "public-subscribable");
        // Anonymous (no user_id)
        let access = storage.check_universe_access(None, "disco");
        assert_eq!(access, crate::models::UniverseAccess::MetadataOnly);
    }

    /// 6. Public-subscribable → MetadataOnly for non-subscribed logged-in user.
    #[test]
    fn test_access_public_subscribable_logged_in_not_subscribed() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "disco2".into(),
                    name: "Discoverable2".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        set_visibility(&storage, "disco2", "public-subscribable");
        let access = storage.check_universe_access(Some("other-user"), "disco2");
        assert_eq!(access, crate::models::UniverseAccess::MetadataOnly);
    }

    /// 7. Private universe → Denied for non-owner.
    #[test]
    fn test_access_private_denied_to_non_owner() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "secret".into(),
                    name: "Secret".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        let access = storage.check_universe_access(Some("attacker"), "secret");
        assert_eq!(access, crate::models::UniverseAccess::Denied);
    }

    /// 7. Private universe → Denied for anonymous user.
    #[test]
    fn test_access_private_denied_anonymous() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "secret2".into(),
                    name: "Secret2".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        let access = storage.check_universe_access(None, "secret2");
        assert_eq!(access, crate::models::UniverseAccess::Denied);
    }

    /// Non-existent universe → Denied.
    #[test]
    fn test_access_nonexistent_denied() {
        let (storage, _dir) = make_storage();
        let access = storage.check_universe_access(None, "does-not-exist");
        assert_eq!(access, crate::models::UniverseAccess::Denied);
    }

    /// Subscribe/unsubscribe flow: subscriptions table is correctly updated.
    #[test]
    fn test_subscribe_unsubscribe_flow() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "pub3".into(),
                    name: "Public3".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        set_visibility(&storage, "pub3", "public-subscribable");

        // Not subscribed yet.
        assert!(!storage.is_subscribed("user-a", "pub3"));

        // Subscribe.
        storage.subscribe_universe("user-a", "pub3").unwrap();
        assert!(storage.is_subscribed("user-a", "pub3"));

        // Appears in user's universe list.
        let universes = storage.list_universes_for_user("user-a");
        assert!(
            universes.iter().any(|u| u.key == "pub3"),
            "subscribed universe must appear in user list"
        );

        // Unsubscribe.
        storage.unsubscribe_universe("user-a", "pub3").unwrap();
        assert!(!storage.is_subscribed("user-a", "pub3"));

        // No longer in user's universe list.
        let universes_after = storage.list_universes_for_user("user-a");
        assert!(
            !universes_after.iter().any(|u| u.key == "pub3"),
            "unsubscribed universe must not appear in user list"
        );
    }

    /// Cannot subscribe to a private universe.
    #[test]
    fn test_cannot_subscribe_to_private_universe() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "private-u".into(),
                    name: "Private".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();
        let result = storage.subscribe_universe("user-b", "private-u");
        assert!(
            result.is_err(),
            "subscribing to a private universe must fail"
        );
    }

    /// Search returns only public-subscribable universes matching the query.
    #[test]
    fn test_search_public_universes() {
        let (mut storage, _dir) = make_storage();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "co-dev".into(),
                    name: "CO Development".into(),
                    description: "The main dev board".into(),
                },
                "owner-1",
            )
            .unwrap();
        set_visibility(&storage, "co-dev", "public-subscribable");

        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "private-proj".into(),
                    name: "Private Project".into(),
                    description: String::new(),
                },
                "owner-1",
            )
            .unwrap();

        let results = storage.search_public_universes("dev");
        assert!(
            results.iter().any(|u| u.key == "co-dev"),
            "co-dev must appear in search results"
        );
        assert!(
            !results.iter().any(|u| u.key == "private-proj"),
            "private universe must not appear in search results"
        );
    }

    // --- CO-66: 409 on duplicate universe key ---

    /// POST /api/v1/universes with an existing key returns 409 Conflict.
    #[tokio::test]
    async fn test_create_universe_duplicate_key_returns_409() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let (mut storage, dir) = make_storage();

        // Pre-create the universe directly in storage.
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "dupe-uni".into(),
                    name: "Dupe Universe".into(),
                    description: String::new(),
                },
                "usr_owner",
            )
            .unwrap();

        let (router, _tmp) = make_universe_router(storage, dir.path());

        let (token, _) =
            crate::auth::sign_jwt("usr_owner", "owner@example.com", "player", "test-secret")
                .unwrap();

        let payload = serde_json::json!({
            "key": "dupe-uni",
            "name": "Another Universe",
            "description": ""
        });
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
        let body = body_bytes(response).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["error"], "conflict");
    }
}
