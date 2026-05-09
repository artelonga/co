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
    /// CO-144: per-universe semver content version. Bumped by deterministic
    /// processes (e.g. alterar-pagina-na-web). Defaults to "0.0.0".
    #[serde(default)]
    pub content_version: String,
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

// 1.68.0: GET /api/v1/universes/public — list public-subscribable universes.
// No auth required. Used by the SPA hub for anonymous visitors so they see
// something at `/` instead of an empty sidebar.
pub async fn list_public_universes(
    State(state): State<AppState>,
) -> Result<Json<Vec<Universe>>, AppError> {
    let storage = lock_storage(&state)?;
    let mut stmt = storage
        .conn()
        .prepare(
            "SELECT key, name, description, owner_id, created_at, is_template, is_public, \
                COALESCE(content_count, 0), COALESCE(requires_login, 0), \
                COALESCE(visibility, 'private'), \
                COALESCE(parent_key, '') \
         FROM universes \
         WHERE COALESCE(hidden, 0) = 0 \
           AND ( \
                visibility = 'public-subscribable' \
             OR visibility = 'public-static' \
             OR is_template = 1 \
           ) \
         ORDER BY name ASC",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Universe {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                owner_id: row.get(3)?,
                created_at: row
                    .get::<_, String>(4)
                    .ok()
                    .and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(&s)
                            .ok()
                            .map(|d| d.with_timezone(&chrono::Utc))
                    })
                    .unwrap_or_else(chrono::Utc::now),
                is_template: row.get::<_, i64>(5).unwrap_or(0) != 0,
                is_public: row.get::<_, i64>(6).unwrap_or(0) != 0,
                content_count: row.get(7)?,
                requires_login: row.get::<_, i64>(8).unwrap_or(0) != 0,
                visibility: row.get(9)?,
                parent_key: {
                    let s: String = row.get(10).unwrap_or_default();
                    if s.is_empty() { None } else { Some(s) }
                },
            })
        })
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let universes: Vec<Universe> = rows.filter_map(|r| r.ok()).collect();
    Ok(Json(universes))
}

// POST /api/v1/universes — create a universe (caller becomes owner)
pub async fn create_universe(
    State(state): State<AppState>,
    user_id: UserId,
    headers: HeaderMap,
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
    let storage = lock_storage(&state)?;
    if storage.get_universe(&body.key).is_some() {
        return Err(AppError::Conflict(format!(
            "Universe key '{}' is already taken",
            body.key
        )));
    }

    // CO-80: check universe count quota for this user's tier.
    let tier = storage
        .get_user_by_id(&user_id.0)
        .map(|u| crate::rate_limit::Tier::parse(&u.tier))
        .unwrap_or(crate::rate_limit::Tier::User);
    crate::rate_limit::check_universe_quota(&storage, &user_id.0, tier, &headers)?;
    drop(storage);

    let mut storage = lock_storage(&state)?;
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
    // 1.45.0 model: every authenticated user is an admin and can edit any
    // universe they can see. The visibility gate (private vs subscribable
    // vs public) is the only access control that remains. A future `static`
    // flag will be the single read-only exception.
    let _caller_id = extract_optional_user_id(&headers, &state)
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".into()))?;

    let storage = lock_storage(&state)?;
    let _universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

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
        // 1.46.0: only `private` and `public-subscribable` are user-settable.
        // `template` is system-only; `requires_login` was collapsed into
        // `public-subscribable` (paired with `default_for_new_users` for
        // universes that should auto-attach to every new user).
        let is_public = match vis {
            "private" => 0,
            "public-subscribable" => 1,
            _ => {
                return Err(AppError::BadRequest(format!(
                    "Invalid visibility '{}'. Must be: private, public-subscribable",
                    vis
                )));
            }
        };
        let storage = lock_storage(&state)?;
        storage
            .conn()
            .execute(
                "UPDATE universes SET visibility = ?1, is_public = ?2, requires_login = 0 \
                 WHERE key = ?3",
                rusqlite::params![vis, is_public, slug],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    let storage = lock_storage(&state)?;
    let updated = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::Internal("Universe disappeared".into()))?;

    // CO-79: invalidate manifest + query caches for this universe.
    state.cache.invalidate_universe(&slug);

    Ok(Json(serde_json::json!({
        "key": updated.key,
        "name": updated.name,
        "description": updated.description,
        "visibility": updated.visibility,
    })))
}

/// DELETE /api/v1/universes/:slug — delete a universe entirely (1.50.0).
///
/// Removes the `universes` row, all `entries`/`entries_fts` rows for the
/// universe, all `universe_members`, and the on-disk universe directory.
/// 1.45.0 model: any authenticated user can delete any universe they can
/// see; the visibility middleware already gates discovery, and the
/// platform's single-tier permission model makes finer-grained checks
/// redundant. Refuses to delete `template` (the seed) for safety.
pub async fn delete_universe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _caller_id = extract_optional_user_id(&headers, &state)
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".into()))?;

    if slug == "template" {
        return Err(AppError::BadRequest(
            "Cannot delete the template universe".into(),
        ));
    }

    let storage = lock_storage(&state)?;
    if storage.get_universe(&slug).is_none() {
        return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
    }
    let universe_dir = storage.universe_root(&slug);

    // Cascade in the meta DB. Per-universe SQLite files in `universes/<slug>/`
    // are removed when we delete the directory below.
    storage
        .conn()
        .execute_batch(&format!(
            "DELETE FROM entries WHERE universe_key = '{slug}';
             DELETE FROM universe_members WHERE universe_key = '{slug}';
             DELETE FROM subscriptions WHERE universe_key = '{slug}';
             DELETE FROM universes WHERE key = '{slug}';"
        ))
        .map_err(|e| AppError::Internal(format!("delete cascade: {e}")))?;
    drop(storage);

    if universe_dir.exists() {
        let _ = std::fs::remove_dir_all(&universe_dir);
    }

    // Invalidate all caches keyed by this slug.
    state.cache.invalidate_universe(&slug);
    state.cache.query.invalidate_prefix(&format!("{slug}:"));

    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "deleted": slug,
        })),
    ))
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
            let cookie = crate::auth::build_session_cookie(
                &token,
                state.config.cookie_domain.as_deref(),
                2592000,
            );
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

// 1.60.0: PUT /api/v1/universes/:slug/subscribe/pin — pin a subscription
// to a specific state. Body: {"state": "states/...md"}. Auto-subscribes
// the user if they don't have a row yet. Validates that the named state
// exists in this universe.
#[derive(serde::Deserialize)]
pub struct PinSubscriptionRequest {
    pub state: String,
}

pub async fn pin_subscription(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
    Json(req): Json<PinSubscriptionRequest>,
) -> Result<StatusCode, AppError> {
    if !req.state.starts_with("states/") {
        return Err(AppError::BadRequest(
            "state must be a path under states/".into(),
        ));
    }
    // Verify the state exists in the universe before pinning to it.
    {
        let uc = {
            let storage = lock_storage(&state)?;
            if storage.get_universe(&slug).is_none() {
                return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
            }
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        let row = index
            .get(&slug, &req.state)
            .map_err(|e| AppError::Internal(format!("get state: {e}")))?
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "state '{}' not found in universe '{}'",
                    req.state, slug
                ))
            })?;
        if row.entry_type != "state" {
            return Err(AppError::BadRequest(format!(
                "'{}' is not a state entry",
                req.state
            )));
        }
    }
    lock_storage(&state)?
        .pin_subscription(&user_id.0, &slug, Some(&req.state))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// 1.60.0: DELETE /api/v1/universes/:slug/subscribe/pin — clear the pin
// (subscriber resumes following head). The subscription itself is left
// in place — this is just toggling the lock-to-version flag.
pub async fn unpin_subscription(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
) -> Result<StatusCode, AppError> {
    lock_storage(&state)?
        .pin_subscription(&user_id.0, &slug, None)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// 1.61.0: GET /api/v1/universes/:slug/subscription — current user's
// subscription status for this universe. Returns subscribed bool +
// optional pinned_state. Auth required (anonymous gets 401).
#[derive(serde::Serialize)]
pub struct SubscriptionStatus {
    pub subscribed: bool,
    pub pinned_state: Option<String>,
}

pub async fn get_my_subscription(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
) -> Result<Json<SubscriptionStatus>, AppError> {
    let storage = lock_storage(&state)?;
    let subscribed = storage.is_subscribed(&user_id.0, &slug);
    let pinned_state = storage.get_subscription_pin(&user_id.0, &slug);
    Ok(Json(SubscriptionStatus {
        subscribed,
        pinned_state,
    }))
}

// GET /api/v1/universes/:slug/subscribers — list subscribers (owner only)
pub async fn list_subscribers(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
) -> Result<Json<Vec<Subscription>>, AppError> {
    // 1.45.0 model: any authenticated user can list subscribers of any
    // universe. The `user_id` extractor still gates anonymous callers.
    let _ = user_id;
    let storage = lock_storage(&state)?;
    let _universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
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

    // CO-144: load content_version separately (defensive — column may not
    // exist on a partially-applied DB; default to "0.0.0").
    let content_version: String = storage
        .conn()
        .query_row(
            "SELECT COALESCE(content_version, '0.0.0') FROM universes WHERE key = ?1",
            rusqlite::params![&slug],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "0.0.0".to_string());

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
        content_version,
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

    // CO-79: invalidate manifest + query caches (theme preset may have changed).
    // Theme CSS cache is keyed by ETag — invalidates naturally when ETag changes.
    state.cache.invalidate_universe(&slug);

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
///
/// CO-79: short Cache-Control (60s, NOT immutable) + ETag for conditional
/// requests; generated CSS is served from an L1 in-process LRU cache.
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

    // Honour If-None-Match conditional request (client already has current CSS).
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && inm == etag
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    // CO-79: serve CSS from L1 cache; generate and insert on miss.
    let css = state
        .cache
        .theme_css
        .get(&etag)
        .map(|arc| arc.as_ref().clone())
        .unwrap_or_else(|| {
            let preset = crate::theme_engine::ThemePreset::by_name(&config.theme_preset)
                .unwrap_or_else(|| crate::theme_engine::ThemePreset::by_name("modern").unwrap());
            let generated =
                crate::theme_engine::generate_css(&preset, config.custom_tokens.as_ref());
            state
                .cache
                .theme_css
                .insert(etag.clone(), generated.clone());
            generated
        });

    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            // CO-79: 60s short cache, NOT immutable — no hashed filenames yet.
            (
                header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("public, max-age=60, must-revalidate"),
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
        // 1.68.0: public listing for the SPA hub (anonymous-visitor-friendly)
        .route("/public", get(list_public_universes))
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
        .route("/{slug}", put(update_universe).delete(delete_universe))
        .route("/{slug}/claim", post(claim_universe))
        .route("/{slug}/config", put(update_universe_config))
        // CO-49: subscription routes
        .route(
            "/{slug}/subscribe",
            post(subscribe_universe).delete(unsubscribe_universe),
        )
        // 1.60.0: pin a subscription to a specific state (Phase 6 storage)
        .route(
            "/{slug}/subscribe/pin",
            put(pin_subscription).delete(unpin_subscription),
        )
        // 1.61.0: read current user's subscription status (subscribed + pin)
        .route("/{slug}/subscription", get(get_my_subscription))
        .route("/{slug}/subscribers", get(list_subscribers))
        // CO-72: doc-gen job submission (owner only, auth via require_auth layer)
        .route("/{slug}/jobs/doc-gen", post(submit_doc_gen_job))
        // Bulk template apply + hub generation (owner-scoped, auth via require_auth)
        .route("/apply-template-all", post(apply_template_all))
        .layer(axum::middleware::from_fn(crate::auth::require_auth));

    Router::new().merge(public_routes).merge(protected_routes)
}

pub mod template;
use template::*;
pub use template::{themes_router, universe_actions_router};

#[cfg(test)]
mod tests;
