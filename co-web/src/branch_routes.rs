//! Branches — named pointers to a `state` (Phase 2 of CO-native versioning).
//!
//! A branch is just an entry with `type=branch`, stored at
//! `branches/<name>.md`. Frontmatter carries:
//!   - `name`: the branch slug (e.g., "main", "feat/new-flow")
//!   - `head_state`: path to the state this branch currently points at
//!   - `default`: whether this is the default branch for the universe
//!   - `created_at`, `updated_at`, `author`
//!
//! Phase 2 surface is intentionally minimal:
//!   - `POST   /api/v1/universes/:slug/branches`        create a branch
//!   - `PUT    /api/v1/universes/:slug/branches/:name`  advance head
//!
//! Listing is just `GET /entries?type=branch` (existing endpoint).
//! Single fetch is `GET /entries/branches/{name}.md`.
//!
//! What's deliberately deferred:
//!   - "Active" branch semantics (which branch new writes flow into).
//!     For now, writes always go to the universe filesystem flat; branches
//!     are just bookmarks over the linear state history.
//!   - Branch deletion (would need DELETE /universes/:slug as a
//!     prerequisite, since deleting a branch is just deleting an entry).
//!   - Conflict-resolution between divergent branches (Phase 3 — merges).
//!
//! The shape works because `state` already gives us atomic captures.
//! A branch is just the name "I want to remember this point". Advance
//! it by `PUT`-ing a newer state path.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::auth::resolve_user_id;
use crate::error::AppError;
use crate::server::AppState;

/// Branch name — letters, digits, hyphen, underscore, slash.
/// Slash is allowed so workflows can use `feat/foo`-style names.
fn validate_branch_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::BadRequest(
            "Branch name must be 1-100 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/')
    {
        return Err(AppError::BadRequest(
            "Branch name may contain letters, digits, '-', '_', '/'".into(),
        ));
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Err(AppError::BadRequest(
            "Invalid '/' usage in branch name".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateBranchRequest {
    pub name: String,
    pub head_state: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Deserialize)]
pub struct AdvanceBranchRequest {
    pub head_state: String,
}

#[derive(Debug, Serialize)]
pub struct BranchResponse {
    pub name: String,
    pub path: String,
    pub head_state: String,
    pub default: bool,
    pub author: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{slug}/branches", post(create_branch))
        .route("/{slug}/branches/{name}", put(advance_branch))
}

/// `POST /api/v1/universes/:slug/branches` — create a named branch
/// pointing at a state.
pub async fn create_branch(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    validate_branch_name(&req.name)?;
    if !req.head_state.starts_with("states/") {
        return Err(AppError::BadRequest(
            "head_state must be a path under states/ (e.g., states/2026-...md)".into(),
        ));
    }

    let author = resolve_user_id(&state, &headers);
    let now = chrono::Utc::now().to_rfc3339();
    let branch_path = format!("branches/{}.md", req.name);

    // Verify the head_state actually exists in this universe.
    {
        let uc = {
            let storage = state.core.storage.lock();
            if storage.get_universe(&slug).is_none() {
                return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
            }
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        if index
            .get(&slug, &req.head_state)
            .map_err(|e| AppError::Internal(format!("get state: {e}")))?
            .is_none()
        {
            return Err(AppError::BadRequest(format!(
                "head_state '{}' not found in universe '{}'",
                req.head_state, slug
            )));
        }
    }

    let mut frontmatter = json!({
        "type": "branch",
        "title": format!("Branch {}", req.name),
        "name": req.name.clone(),
        "head_state": req.head_state.clone(),
        "default": req.default,
        "created_at": now.clone(),
        "updated_at": now.clone(),
    });
    if let Some(ref a) = author {
        frontmatter["author"] = JsonValue::String(a.clone());
    }

    let body = format!(
        "# Branch `{name}`\n\
        \n\
        - **head_state:** [{head}](/{slug}/{head})\n\
        - **default:** {default}\n",
        name = req.name,
        head = req.head_state,
        slug = slug,
        default = req.default,
    );

    crate::vault_routes::write_vault_entry(&state, &slug, &branch_path, frontmatter, &body)?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(BranchResponse {
            name: req.name,
            path: branch_path,
            head_state: req.head_state,
            default: req.default,
            author,
            created_at: now.clone(),
            updated_at: now,
        }),
    ))
}

/// `PUT /api/v1/universes/:slug/branches/:name` — advance the branch's
/// head to a newer state.
pub async fn advance_branch(
    State(state): State<AppState>,
    Path((slug, name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<AdvanceBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    validate_branch_name(&name)?;
    if !req.head_state.starts_with("states/") {
        return Err(AppError::BadRequest(
            "head_state must be a path under states/".into(),
        ));
    }

    let author = resolve_user_id(&state, &headers);
    let now = chrono::Utc::now().to_rfc3339();
    let branch_path = format!("branches/{name}.md");

    // Read existing branch entry (if any) to preserve created_at + default flag.
    let (existing_created_at, existing_default) = {
        let uc = {
            let storage = state.core.storage.lock();
            if storage.get_universe(&slug).is_none() {
                return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
            }
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);

        // Verify head_state exists.
        if index
            .get(&slug, &req.head_state)
            .map_err(|e| AppError::Internal(format!("get state: {e}")))?
            .is_none()
        {
            return Err(AppError::BadRequest(format!(
                "head_state '{}' not found in universe '{}'",
                req.head_state, slug
            )));
        }

        // Existing branch row.
        let existing = index
            .get(&slug, &branch_path)
            .map_err(|e| AppError::Internal(format!("get branch: {e}")))?;
        let (created_at, default) = match existing {
            Some(row) => {
                let fm = &row.frontmatter;
                let c = fm
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&now)
                    .to_string();
                let d = fm.get("default").and_then(|v| v.as_bool()).unwrap_or(false);
                (c, d)
            }
            None => {
                return Err(AppError::NotFound(format!(
                    "Branch '{name}' not found — create with POST /branches first"
                )));
            }
        };
        (created_at, default)
    };

    let mut frontmatter = json!({
        "type": "branch",
        "title": format!("Branch {name}"),
        "name": name.clone(),
        "head_state": req.head_state.clone(),
        "default": existing_default,
        "created_at": existing_created_at.clone(),
        "updated_at": now.clone(),
    });
    if let Some(ref a) = author {
        frontmatter["author"] = JsonValue::String(a.clone());
    }

    let body = format!(
        "# Branch `{name}`\n\
        \n\
        - **head_state:** [{head}](/{slug}/{head})\n\
        - **default:** {default}\n",
        name = name,
        head = req.head_state,
        slug = slug,
        default = existing_default,
    );

    crate::vault_routes::write_vault_entry(&state, &slug, &branch_path, frontmatter, &body)?;

    Ok((
        axum::http::StatusCode::OK,
        Json(BranchResponse {
            name,
            path: branch_path,
            head_state: req.head_state,
            default: existing_default,
            author,
            created_at: existing_created_at,
            updated_at: now,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_branch_name_accepts_common_shapes() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feat/new-flow").is_ok());
        assert!(validate_branch_name("v1_release").is_ok());
        assert!(validate_branch_name("a").is_ok());
    }

    #[test]
    fn test_validate_branch_name_rejects_bad_shapes() {
        assert!(validate_branch_name("").is_err());
        assert!(validate_branch_name("/main").is_err());
        assert!(validate_branch_name("main/").is_err());
        assert!(validate_branch_name("a//b").is_err());
        assert!(validate_branch_name("with space").is_err());
        assert!(validate_branch_name("a".repeat(101).as_str()).is_err());
    }
}
