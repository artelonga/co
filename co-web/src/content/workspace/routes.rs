//! CO-352: Workspace ("Sala") state persistence.
//!
//! GET  /api/v1/universes/{key}/workspaces/{slug}/state
//!      — returns the caller's saved layout, or an empty canvas if none exists
//! PUT  /api/v1/universes/{key}/workspaces/{slug}/state
//!      — upsert layout (auth required)
//! POST /api/v1/universes/{key}/workspaces/{slug}/state/share
//!      — generate a share token for the caller's state (auth required)
//! GET  /api/v1/workspace-states/{token}
//!      — resolve a share token → state (public, no auth)

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::Utc;
use serde::Deserialize;

use crate::auth::extractors::AuthedUser;
use crate::error::AppError;
use crate::server::AppState;
use crate::storage::WorkspaceState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct UpsertStateBody {
    /// Serialised layout, persisted opaquely (the canvas owns its schema):
    /// `{cells, notes, folders, universes, edges, view}`. CO-400 universe nodes
    /// (`universes:[{kind:"universe", key, x, y}]`) round-trip through here with
    /// no server-side schema change — they are just JSON inside `layout_json`.
    pub layout_json: String,
    #[serde(default)]
    pub is_public: bool,
}

fn err_json(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn empty_layout_json() -> String {
    r#"{"nodes":[],"edges":[],"view":{"tx":0,"ty":0,"scale":1}}"#.to_string()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/{key}/workspaces/{slug}/state
///
/// Returns the caller's saved workspace state. If the caller is not
/// authenticated (or has no saved state for this workspace) returns an
/// empty-canvas placeholder so the frontend can start fresh without a 404.
async fn get_workspace_state(
    State(state): State<AppState>,
    Path((universe_key, workspace_slug)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let caller_id = extract_optional_user_id(&headers);

    let Some(user_id) = caller_id else {
        return Ok(Json(empty_state(&universe_key, &workspace_slug)).into_response());
    };

    let storage = state.core.storage.lock();
    match storage.workspace_state_for_user(&universe_key, &workspace_slug, &user_id)? {
        Some(ws) => Ok(Json(ws).into_response()),
        None => Ok(Json(empty_state(&universe_key, &workspace_slug)).into_response()),
    }
}

/// GET /api/v1/universes/{key}/workspaces/{slug}/state/public
///
/// Returns the most-recently-updated public state for this workspace
/// (the one with is_public=1). Read-only, no auth required.
async fn get_public_workspace_state(
    State(state): State<AppState>,
    Path((universe_key, workspace_slug)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let storage = state.core.storage.lock();
    match storage.public_workspace_state(&universe_key, &workspace_slug)? {
        Some(ws) => Ok(Json(ws).into_response()),
        None => Ok(err_json(
            StatusCode::NOT_FOUND,
            "no public workspace state found",
        )),
    }
}

/// PUT /api/v1/universes/{key}/workspaces/{slug}/state
///
/// Upsert the caller's workspace layout. The UNIQUE constraint on
/// (universe_key, workspace_slug, user_id) ensures one state per
/// user per workspace.
async fn upsert_workspace_state(
    State(state): State<AppState>,
    Path((universe_key, workspace_slug)): Path<(String, String)>,
    user: AuthedUser,
    Json(body): Json<UpsertStateBody>,
) -> Result<Response, AppError> {
    if serde_json::from_str::<serde_json::Value>(&body.layout_json).is_err() {
        return Ok(err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "layout_json must be valid JSON",
        ));
    }

    let now = Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();

    {
        let storage = state.core.storage.lock();
        storage.upsert_workspace_state(
            &id,
            &universe_key,
            &workspace_slug,
            &user.user_id,
            &body.layout_json,
            body.is_public,
            &now,
        )?;
    }

    let ws = fetch_state_by_user(&state, &universe_key, &workspace_slug, &user.user_id)?
        .ok_or_else(|| AppError::Internal("workspace state missing after upsert".into()))?;
    Ok((StatusCode::OK, Json(ws)).into_response())
}

/// POST /api/v1/universes/{key}/workspaces/{slug}/state/share
///
/// Generates a random share token for the caller's existing workspace state.
/// The caller must have previously PUT their state. Returns the token so
/// the client can build a shareable URL: `/sala/{token}`.
async fn share_workspace_state(
    State(state): State<AppState>,
    Path((universe_key, workspace_slug)): Path<(String, String)>,
    user: AuthedUser,
) -> Result<Response, AppError> {
    let token = uuid::Uuid::new_v4().as_simple().to_string();
    let now = Utc::now().to_rfc3339();

    let rows = {
        let storage = state.core.storage.lock();
        storage.set_workspace_share_token(
            &token,
            &now,
            &universe_key,
            &workspace_slug,
            &user.user_id,
        )?
    };

    if rows == 0 {
        return Ok(err_json(
            StatusCode::NOT_FOUND,
            "workspace state not found — PUT your layout first",
        ));
    }

    Ok(Json(serde_json::json!({ "share_token": token })).into_response())
}

/// GET /api/v1/workspace-states/{token}
///
/// Resolve a share token to a workspace state. Public — no auth required.
async fn get_by_share_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Response, AppError> {
    let storage = state.core.storage.lock();
    match storage.workspace_state_by_share_token(&token)? {
        Some(ws) => Ok(Json(ws).into_response()),
        None => Ok(err_json(StatusCode::NOT_FOUND, "share token not found")),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn fetch_state_by_user(
    state: &AppState,
    universe_key: &str,
    workspace_slug: &str,
    user_id: &str,
) -> Result<Option<WorkspaceState>, AppError> {
    let storage = state.core.storage.lock();
    Ok(storage.workspace_state_for_user(universe_key, workspace_slug, user_id)?)
}

fn empty_state(universe_key: &str, workspace_slug: &str) -> serde_json::Value {
    serde_json::json!({
        "id": null,
        "universe_key": universe_key,
        "workspace_slug": workspace_slug,
        "user_id": null,
        "layout_json": empty_layout_json(),
        "is_public": false,
        "share_token": null,
        "created_at": null,
        "updated_at": null,
    })
}

// ---------------------------------------------------------------------------
// Routers
// ---------------------------------------------------------------------------

/// Routes mounted at `/api/v1/universes` — no auth middleware applied.
/// GET handlers extract the caller's identity themselves.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/{key}/workspaces/{slug}/state", get(get_workspace_state))
        .route(
            "/{key}/workspaces/{slug}/state/public",
            get(get_public_workspace_state),
        )
}

/// Routes mounted at `/api/v1/universes` — `require_auth` applied at the call site.
pub fn authed_router() -> Router<AppState> {
    Router::new()
        .route(
            "/{key}/workspaces/{slug}/state",
            put(upsert_workspace_state),
        )
        .route(
            "/{key}/workspaces/{slug}/state/share",
            post(share_workspace_state),
        )
}

/// Routes mounted at `/api/v1` — share-token resolution, public.
pub fn share_router() -> Router<AppState> {
    Router::new().route("/workspace-states/{token}", get(get_by_share_token))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::workspace_states::row_to_state;
    use rusqlite::{Connection, params};

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workspace_states (
               id            TEXT PRIMARY KEY,
               universe_key  TEXT NOT NULL,
               workspace_slug TEXT NOT NULL,
               user_id       TEXT NOT NULL,
               layout_json   TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[],\"view\":{\"tx\":0,\"ty\":0,\"scale\":1}}',
               is_public     INTEGER NOT NULL DEFAULT 0,
               share_token   TEXT,
               created_at    TEXT NOT NULL,
               updated_at    TEXT NOT NULL,
               UNIQUE (universe_key, workspace_slug, user_id)
             );
             CREATE INDEX IF NOT EXISTS idx_workspace_states_user
               ON workspace_states (user_id);
             CREATE INDEX IF NOT EXISTS idx_workspace_states_token
               ON workspace_states (share_token) WHERE share_token IS NOT NULL;",
        )
        .unwrap();
        conn
    }

    fn insert_state(
        conn: &Connection,
        id: &str,
        universe_key: &str,
        workspace_slug: &str,
        user_id: &str,
        is_public: i64,
        share_token: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO workspace_states \
             (id, universe_key, workspace_slug, user_id, layout_json, is_public, \
              share_token, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, '{\"nodes\":[]}', ?5, ?6, '2026-01-01', '2026-01-01')",
            params![
                id,
                universe_key,
                workspace_slug,
                user_id,
                is_public,
                share_token
            ],
        )
        .unwrap();
    }

    #[test]
    fn test_row_to_state_maps_correctly() {
        let conn = setup_db();
        insert_state(&conn, "id-1", "mbya", "default", "user-1", 0, None);

        let ws: WorkspaceState = conn
            .query_row(
                "SELECT id, universe_key, workspace_slug, user_id, layout_json, \
                        is_public, share_token, created_at, updated_at \
                 FROM workspace_states WHERE id = 'id-1'",
                [],
                row_to_state,
            )
            .unwrap();

        assert_eq!(ws.universe_key, "mbya");
        assert_eq!(ws.workspace_slug, "default");
        assert_eq!(ws.user_id, "user-1");
        assert!(!ws.is_public);
        assert!(ws.share_token.is_none());
    }

    #[test]
    fn test_public_state_query() {
        let conn = setup_db();
        insert_state(&conn, "id-1", "mbya", "default", "user-1", 0, None);
        insert_state(&conn, "id-2", "mbya", "default", "user-2", 1, None);

        let ws: WorkspaceState = conn
            .query_row(
                "SELECT id, universe_key, workspace_slug, user_id, layout_json, \
                        is_public, share_token, created_at, updated_at \
                 FROM workspace_states \
                 WHERE universe_key = 'mbya' AND workspace_slug = 'default' AND is_public = 1 \
                 ORDER BY updated_at DESC LIMIT 1",
                [],
                row_to_state,
            )
            .unwrap();

        assert_eq!(ws.user_id, "user-2");
        assert!(ws.is_public);
    }

    #[test]
    fn test_share_token_lookup() {
        let conn = setup_db();
        insert_state(
            &conn,
            "id-1",
            "mbya",
            "default",
            "user-1",
            0,
            Some("abc123"),
        );

        let ws: WorkspaceState = conn
            .query_row(
                "SELECT id, universe_key, workspace_slug, user_id, layout_json, \
                        is_public, share_token, created_at, updated_at \
                 FROM workspace_states WHERE share_token = 'abc123'",
                [],
                row_to_state,
            )
            .unwrap();

        assert_eq!(ws.id, "id-1");
        assert_eq!(ws.share_token.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_upsert_conflict_updates_layout() {
        let conn = setup_db();
        insert_state(&conn, "id-1", "mbya", "default", "user-1", 0, None);

        conn.execute(
            "INSERT INTO workspace_states \
             (id, universe_key, workspace_slug, user_id, layout_json, is_public, created_at, updated_at) \
             VALUES ('id-2', 'mbya', 'default', 'user-1', '{\"nodes\":[{\"id\":\"x\"}]}', 0, '2026-01-02', '2026-01-02') \
             ON CONFLICT (universe_key, workspace_slug, user_id) DO UPDATE SET \
               layout_json = excluded.layout_json, \
               updated_at  = excluded.updated_at",
            [],
        )
        .unwrap();

        let layout: String = conn
            .query_row(
                "SELECT layout_json FROM workspace_states \
                 WHERE universe_key = 'mbya' AND workspace_slug = 'default' AND user_id = 'user-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        assert!(layout.contains("\"x\""));
    }

    /// CO-400: a layout carrying a universe node must survive the upsert →
    /// read trip byte-for-byte. The state API stores `layout_json` opaquely, so
    /// universe nodes (a frontend-only schema addition) need no server change —
    /// this test guards against a future "validate the layout shape" regression
    /// that would reject the new `universes` array.
    #[test]
    fn test_universe_node_round_trips() {
        let conn = setup_db();
        let layout = r#"{"cells":{},"notes":[],"folders":[],"universes":[{"id":"u1","kind":"universe","key":"mse","name":"MSE","x":3,"y":-2}],"edges":[],"view":{"tx":0,"ty":0,"scale":1}}"#;

        conn.execute(
            "INSERT INTO workspace_states \
             (id, universe_key, workspace_slug, user_id, layout_json, is_public, created_at, updated_at) \
             VALUES ('id-1', 'miguel', 'default', 'user-1', ?1, 0, '2026-01-01', '2026-01-01')",
            params![layout],
        )
        .unwrap();

        let stored: String = conn
            .query_row(
                "SELECT layout_json FROM workspace_states WHERE id = 'id-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        // Verbatim preservation, plus a structural check on the universe node.
        assert_eq!(stored, layout);
        let v: serde_json::Value = serde_json::from_str(&stored).unwrap();
        let node = &v["universes"][0];
        assert_eq!(node["kind"], "universe");
        assert_eq!(node["key"], "mse");
        assert_eq!(node["x"], 3);
        assert_eq!(node["y"], -2);
    }

    #[test]
    fn test_empty_layout_json_is_valid_json() {
        let s = empty_layout_json();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v["nodes"].is_array());
        assert!(v["edges"].is_array());
        assert!(v["view"].is_object());
    }
}
