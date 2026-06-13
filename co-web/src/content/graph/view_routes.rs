//! CO-345: Publishable saved graph views.
//!
//! POST   /api/v1/me/graph-views          — create (auth required)
//! GET    /api/v1/me/graph-views          — list owner's views (auth required)
//! GET    /api/v1/graph-views/{slug}      — fetch (respects visibility)
//! PATCH  /api/v1/graph-views/{slug}      — update (owner only)
//! DELETE /api/v1/graph-views/{slug}      — delete (owner only)
//!
//! Visibility rules:
//!   private  — owner only
//!   unlisted — anyone with the slug
//!   public   — open

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
#[cfg(test)]
use rusqlite::params;
use serde::Deserialize;

use crate::auth::extractors::AuthedUser;
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

// CO-433: `GraphView` + its row mapper moved to the storage layer (it lives
// with the typed `graph_views` queries); re-exported here for the API types.
pub use crate::storage::graph_views::GraphView;

#[derive(Debug, Deserialize)]
pub struct CreateGraphViewBody {
    /// URL-safe slug; must be unique. Auto-derived from `name` when absent.
    pub slug: Option<String>,
    pub name: String,
    pub universe_filter: Vec<String>,
    pub type_filter: Option<Vec<String>>,
    pub relation_filter: Option<String>,
    pub depth: Option<i64>,
    pub root: Option<String>,
    pub layout_seed: Option<i64>,
    #[serde(default = "default_visibility")]
    pub visibility: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateGraphViewBody {
    pub name: Option<String>,
    pub universe_filter: Option<Vec<String>>,
    pub type_filter: Option<Vec<String>>,
    pub relation_filter: Option<String>,
    pub depth: Option<i64>,
    pub root: Option<String>,
    pub layout_seed: Option<i64>,
    pub visibility: Option<String>,
}

fn default_visibility() -> String {
    "private".to_string()
}

fn err_json(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn validate_visibility(v: &str) -> bool {
    matches!(v, "public" | "unlisted" | "private")
}

/// Derive a URL-safe slug from a human name.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn create_graph_view(
    State(state): State<AppState>,
    user: AuthedUser,
    Json(body): Json<CreateGraphViewBody>,
) -> Result<Response, AppError> {
    if body.name.trim().is_empty() {
        return Ok(err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "name is required",
        ));
    }
    if !validate_visibility(&body.visibility) {
        return Ok(err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "visibility must be public, unlisted, or private",
        ));
    }
    if body.universe_filter.is_empty() {
        return Ok(err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "universe_filter must have at least one universe",
        ));
    }

    let slug = body
        .slug
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| slugify(&body.name));

    let universe_filter =
        serde_json::to_string(&body.universe_filter).unwrap_or_else(|_| "[]".to_string());
    let type_filter = body
        .type_filter
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()));
    let now = Utc::now().to_rfc3339();

    {
        let storage = state.core.storage.lock();
        let result = storage.insert_graph_view(
            &slug,
            &user.user_id,
            body.name.trim(),
            &universe_filter,
            type_filter.as_deref(),
            body.relation_filter.as_deref(),
            body.depth,
            body.root.as_deref(),
            body.layout_seed,
            &body.visibility,
            &now,
        );
        match result {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Ok(err_json(StatusCode::CONFLICT, "slug already taken"));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let view = fetch_view_by_slug(&state, &slug, Some(&user.user_id))?
        .ok_or_else(|| AppError::Internal("graph view disappeared after insert".into()))?;
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn list_my_graph_views(
    State(state): State<AppState>,
    user: AuthedUser,
) -> Result<Json<Vec<GraphView>>, AppError> {
    let views = {
        let storage = state.core.storage.lock();
        storage.list_graph_views_by_owner(&user.user_id)
    };
    Ok(Json(views))
}

async fn get_graph_view(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let caller_id = extract_optional_user_id(&headers);
    match fetch_view_by_slug(&state, &slug, caller_id.as_deref())? {
        Some(view) => Ok(Json(view).into_response()),
        None => Ok(err_json(StatusCode::NOT_FOUND, "graph view not found")),
    }
}

fn extract_optional_user_id(headers: &HeaderMap) -> Option<String> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| crate::auth::extract_session_cookie(headers))?;
    let secret = crate::auth::jwt_secret();
    crate::auth::decode_user_id(&token, &secret).ok()
}

async fn update_graph_view(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user: AuthedUser,
    Json(body): Json<UpdateGraphViewBody>,
) -> Result<Response, AppError> {
    if let Some(v) = &body.visibility
        && !validate_visibility(v)
    {
        return Ok(err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "visibility must be public, unlisted, or private",
        ));
    }

    let now = Utc::now().to_rfc3339();
    let rows_affected = {
        let storage = state.core.storage.lock();

        // Verify owner.
        let owner: Option<String> = storage.graph_view_owner(&slug);
        match owner {
            None => return Ok(err_json(StatusCode::NOT_FOUND, "graph view not found")),
            Some(ref owner_id) if owner_id != &user.user_id => {
                return Ok(err_json(StatusCode::FORBIDDEN, "not your graph view"));
            }
            _ => {}
        }

        let universe_filter = body
            .universe_filter
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()));
        let type_filter = body
            .type_filter
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()));

        storage.update_graph_view(
            &slug,
            body.name.as_deref(),
            universe_filter.as_deref(),
            type_filter.as_deref(),
            body.relation_filter.as_deref(),
            body.depth,
            body.root.as_deref(),
            body.layout_seed,
            body.visibility.as_deref(),
            &now,
        )?
    };

    if rows_affected == 0 {
        return Ok(err_json(StatusCode::NOT_FOUND, "graph view not found"));
    }

    match fetch_view_by_slug(&state, &slug, Some(&user.user_id))? {
        Some(view) => Ok(Json(view).into_response()),
        None => Ok(err_json(StatusCode::NOT_FOUND, "graph view not found")),
    }
}

async fn delete_graph_view(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user: AuthedUser,
) -> Result<Response, AppError> {
    let storage = state.core.storage.lock();

    let owner: Option<String> = storage.graph_view_owner(&slug);

    match owner {
        None => Ok(err_json(StatusCode::NOT_FOUND, "graph view not found")),
        Some(ref owner_id) if owner_id != &user.user_id => {
            Ok(err_json(StatusCode::FORBIDDEN, "not your graph view"))
        }
        _ => {
            storage.delete_graph_view(&slug)?;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fetch_view_by_slug(
    state: &AppState,
    slug: &str,
    caller_id: Option<&str>,
) -> Result<Option<GraphView>, AppError> {
    let storage = state.core.storage.lock();
    match storage.fetch_graph_view_by_slug(slug) {
        Err(e) => Err(AppError::from(e)),
        Ok(None) => Ok(None),
        Ok(Some(view)) => {
            let visible = match view.visibility.as_str() {
                "public" | "unlisted" => true,
                "private" => caller_id == Some(view.owner_id.as_str()),
                _ => false,
            };
            if visible { Ok(Some(view)) } else { Ok(None) }
        }
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Routes mounted at /api/v1/me (auth-gated at router level or via AuthedUser).
pub fn me_router() -> Router<AppState> {
    Router::new().route(
        "/graph-views",
        post(create_graph_view).get(list_my_graph_views),
    )
}

/// Routes mounted at /api/v1 (visibility-gated per-handler).
pub fn public_router() -> Router<AppState> {
    Router::new().route(
        "/graph-views/{slug}",
        get(get_graph_view)
            .patch(update_graph_view)
            .delete(delete_graph_view),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::graph_views::row_to_view;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE graph_views (
                slug             TEXT PRIMARY KEY,
                owner_id         TEXT NOT NULL,
                name             TEXT NOT NULL,
                universe_filter  TEXT NOT NULL,
                type_filter      TEXT,
                relation_filter  TEXT,
                depth            INTEGER,
                root             TEXT,
                layout_seed      INTEGER,
                visibility       TEXT NOT NULL DEFAULT 'private',
                created_at       TEXT NOT NULL,
                updated_at       TEXT NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    fn insert_view(conn: &Connection, slug: &str, owner_id: &str, visibility: &str) {
        conn.execute(
            "INSERT INTO graph_views \
             (slug, owner_id, name, universe_filter, visibility, created_at, updated_at) \
             VALUES (?1, ?2, 'Test View', '[\"mbya\"]', ?3, '2026-01-01', '2026-01-01')",
            params![slug, owner_id, visibility],
        )
        .unwrap();
    }

    #[test]
    fn test_slugify_basic() {
        assert_eq!(slugify("My Cool View"), "my-cool-view");
        assert_eq!(slugify("yoruba × mbya"), "yoruba-mbya");
        assert_eq!(slugify("  spaces  "), "spaces");
    }

    #[test]
    fn test_visibility_validation() {
        assert!(validate_visibility("public"));
        assert!(validate_visibility("unlisted"));
        assert!(validate_visibility("private"));
        assert!(!validate_visibility("hidden"));
        assert!(!validate_visibility(""));
    }

    #[test]
    fn test_row_to_view_maps_correctly() {
        let conn = setup_db();
        insert_view(&conn, "my-view", "user-1", "public");

        let view: GraphView = conn
            .query_row(
                "SELECT slug, owner_id, name, universe_filter, type_filter, relation_filter, \
                  depth, root, layout_seed, visibility, created_at, updated_at \
                 FROM graph_views WHERE slug = 'my-view'",
                [],
                row_to_view,
            )
            .unwrap();

        assert_eq!(view.slug, "my-view");
        assert_eq!(view.owner_id, "user-1");
        assert_eq!(view.visibility, "public");
        assert_eq!(view.universe_filter, "[\"mbya\"]");
        assert!(view.type_filter.is_none());
    }

    #[test]
    fn test_private_view_visible_to_owner_only() {
        let conn = setup_db();
        insert_view(&conn, "secret", "user-1", "private");

        let view = conn
            .query_row(
                "SELECT slug, owner_id, name, universe_filter, type_filter, relation_filter, \
                  depth, root, layout_seed, visibility, created_at, updated_at \
                 FROM graph_views WHERE slug = 'secret'",
                [],
                row_to_view,
            )
            .unwrap();

        // Owner can see it.
        let visible_to_owner = match view.visibility.as_str() {
            "private" => Some(view.owner_id.as_str()) == Some("user-1"),
            _ => true,
        };
        assert!(visible_to_owner);

        // Another user cannot.
        let visible_to_other = match view.visibility.as_str() {
            "private" => Some(view.owner_id.as_str()) == Some("user-2"),
            _ => true,
        };
        assert!(!visible_to_other);
    }

    #[test]
    fn test_public_view_visible_to_all() {
        let conn = setup_db();
        insert_view(&conn, "public-view", "user-1", "public");

        let view = conn
            .query_row(
                "SELECT slug, owner_id, name, universe_filter, type_filter, relation_filter, \
                  depth, root, layout_seed, visibility, created_at, updated_at \
                 FROM graph_views WHERE slug = 'public-view'",
                [],
                row_to_view,
            )
            .unwrap();

        let visible = matches!(view.visibility.as_str(), "public" | "unlisted");
        assert!(visible, "public view must be visible to all");
    }

    #[test]
    fn test_universe_filter_serializes_as_json_array() {
        let filters = vec!["mbya".to_string(), "yoruba".to_string()];
        let serialized = serde_json::to_string(&filters).unwrap();
        assert_eq!(serialized, "[\"mbya\",\"yoruba\"]");
        // Verify round-trip.
        let deserialized: Vec<String> = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, filters);
    }
}
