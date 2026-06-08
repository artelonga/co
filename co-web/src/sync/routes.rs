//! CO-385: Sync conflict resolution routes.
//!
//! GET  /api/v1/me/sync/conflicts?universe=...
//! POST /api/v1/sync/conflicts/{id}/resolve
//!
//! Both routes require authentication (JWT or session cookie).

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::auth::UserId;
use crate::eda::event::{Event, Visibility};
use crate::server::AppState;

use super::conflict_detector::{Conflict, ConflictKind, EntryRevision};
use super::conflict_resolver::{Action, apply_action};

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Protected routes — all require authentication.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/sync/conflicts/{id}/resolve",
            post(resolve_conflict_handler),
        )
        .route("/me/sync/conflicts", get(list_conflicts_handler))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::require_auth,
        ))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ListConflictsQuery {
    universe: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveRequest {
    action: String,
    /// When `true`, apply this action to all remaining unresolved conflicts of
    /// the same `ConflictKind` in the same universe.
    #[serde(default)]
    apply_to_all_matching: bool,
}

#[derive(Debug, Serialize)]
struct ConflictRow {
    id: String,
    universe_key: String,
    path: String,
    local_body_hash: String,
    remote_body_hash: String,
    kind: String,
    detected_at: String,
    resolved_at: Option<String>,
    resolution_action: Option<String>,
    /// Default action for this universe (CO-383: yggdrasil defaults to Replace).
    suggested_action: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/me/sync/conflicts?universe=...
///
/// Lists all unresolved conflicts for the authenticated user.
/// Optional `universe` filter narrows to a specific universe.
async fn list_conflicts_handler(
    State(state): State<AppState>,
    UserId(user_id): UserId,
    Query(q): Query<ListConflictsQuery>,
) -> impl IntoResponse {
    let storage = state.core.storage.lock();

    let rows: Vec<ConflictRow> = {
        let sql = if q.universe.is_some() {
            "SELECT id, universe_key, path, local_body_hash, remote_body_hash,
                    kind, detected_at, resolved_at, resolution_action
             FROM sync_conflicts
             WHERE resolved_at IS NULL
               AND universe_key = ?1
             ORDER BY detected_at DESC
             LIMIT 200"
        } else {
            "SELECT id, universe_key, path, local_body_hash, remote_body_hash,
                    kind, detected_at, resolved_at, resolution_action
             FROM sync_conflicts
             WHERE resolved_at IS NULL
             ORDER BY detected_at DESC
             LIMIT 200"
        };

        let mut stmt = match storage.conn().prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                warn!(user_id = %user_id, "list_conflicts: prepare: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "db_error"})),
                )
                    .into_response();
            }
        };

        let map_row = |row: &rusqlite::Row<'_>| {
            let universe_key: String = row.get(1)?;
            let kind_str: String = row.get(5)?;
            Ok(ConflictRow {
                id: row.get(0)?,
                universe_key: universe_key.clone(),
                path: row.get(2)?,
                local_body_hash: row.get(3)?,
                remote_body_hash: row.get(4)?,
                kind: kind_str,
                detected_at: row.get(6)?,
                resolved_at: row.get(7)?,
                resolution_action: row.get(8)?,
                suggested_action: suggested_action_for_universe(&universe_key),
            })
        };

        let result = if let Some(ref universe) = q.universe {
            stmt.query_map(rusqlite::params![universe], map_row)
        } else {
            stmt.query_map(rusqlite::params![], map_row)
        };

        match result {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                warn!(user_id = %user_id, "list_conflicts: query: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "db_error"})),
                )
                    .into_response();
            }
        }
    };

    Json(serde_json::json!({ "conflicts": rows, "total": rows.len() })).into_response()
}

/// POST /api/v1/sync/conflicts/{id}/resolve
///
/// Applies the requested action to the conflict, optionally propagating to all
/// remaining unresolved conflicts of the same shape (`apply_to_all_matching`).
async fn resolve_conflict_handler(
    State(state): State<AppState>,
    UserId(user_id): UserId,
    Path(conflict_id): Path<String>,
    Json(body): Json<ResolveRequest>,
) -> impl IntoResponse {
    let action: Action = match body.action.parse() {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("unknown action: {e}")})),
            )
                .into_response();
        }
    };

    // Load conflict from DB.
    let conflict = {
        let st = state.core.storage.lock();
        load_conflict_from_db(st.conn(), &conflict_id)
    };

    let conflict = match conflict {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "conflict_not_found"})),
            )
                .into_response();
        }
    };

    // Collect sibling conflicts if bulk-apply requested.
    let siblings: Vec<Conflict> = if body.apply_to_all_matching {
        let st = state.core.storage.lock();
        load_sibling_conflicts(st.conn(), &conflict)
    } else {
        vec![]
    };

    // Apply action to the primary conflict.
    let result = apply_action(
        &conflict,
        action.clone(),
        Some(&user_id),
        &state.core.storage,
        &state.core.eda_bus,
    );

    match result {
        Err(e) => {
            warn!(conflict_id = %conflict_id, "resolve_conflict: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response()
        }
        Ok(primary_result) => {
            // Bulk apply to siblings if requested.
            let mut bulk_count = 0usize;
            for sibling in &siblings {
                if apply_action(
                    sibling,
                    action.clone(),
                    Some(&user_id),
                    &state.core.storage,
                    &state.core.eda_bus,
                )
                .is_ok()
                {
                    bulk_count += 1;
                }
            }

            if bulk_count > 0 {
                state.core.eda_bus.publish(Event::new(
                    "sync.conflict_resolved_bulk",
                    Some(conflict.universe_key.clone()),
                    Some(user_id.clone()),
                    serde_json::json!({
                        "count": bulk_count,
                        "action": action.to_string(),
                        "universe": &conflict.universe_key,
                    }),
                    Visibility::UniverseOwner,
                ));
            }

            Json(serde_json::json!({
                "conflict_id": &primary_result.conflict_id,
                "action": primary_result.action.to_string(),
                "path": &primary_result.path,
                "event_type": &primary_result.event_type,
                "bulk_resolved": bulk_count,
            }))
            .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

fn load_conflict_from_db(conn: &rusqlite::Connection, id: &str) -> Option<Conflict> {
    conn.query_row(
        "SELECT id, universe_key, path, local_body_hash, remote_body_hash,
                common_ancestor_hash, kind, detected_at
         FROM sync_conflicts WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            let kind_str: String = row.get(6)?;
            let kind: ConflictKind = kind_str.parse().unwrap_or(ConflictKind::BothModified);
            Ok(Conflict {
                id: row.get(0)?,
                universe_key: row.get(1)?,
                path: row.get(2)?,
                local: EntryRevision {
                    path: row.get(2)?,
                    body_hash: row.get(3)?,
                    body: None,
                    updated_at: None,
                    source: "local".into(),
                },
                remote: EntryRevision {
                    path: row.get(2)?,
                    body_hash: row.get(4)?,
                    body: None,
                    updated_at: None,
                    source: "remote".into(),
                },
                common_ancestor: {
                    let hash: Option<String> = row.get(5)?;
                    hash.map(|h| EntryRevision {
                        path: row.get(2).unwrap_or_default(),
                        body_hash: h,
                        body: None,
                        updated_at: None,
                        source: "ancestor".into(),
                    })
                },
                kind,
                detected_at: row.get(7)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

fn load_sibling_conflicts(conn: &rusqlite::Connection, primary: &Conflict) -> Vec<Conflict> {
    let mut stmt = match conn.prepare(
        "SELECT id, universe_key, path, local_body_hash, remote_body_hash,
                common_ancestor_hash, kind, detected_at
         FROM sync_conflicts
         WHERE resolved_at IS NULL
           AND universe_key = ?1
           AND kind = ?2
           AND id != ?3
         ORDER BY detected_at
         LIMIT 100",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map(
        rusqlite::params![&primary.universe_key, primary.kind.to_string(), &primary.id],
        |row| {
            let kind_str: String = row.get(6)?;
            let kind: ConflictKind = kind_str.parse().unwrap_or(ConflictKind::BothModified);
            Ok(Conflict {
                id: row.get(0)?,
                universe_key: row.get(1)?,
                path: row.get(2)?,
                local: EntryRevision {
                    path: row.get(2)?,
                    body_hash: row.get(3)?,
                    body: None,
                    updated_at: None,
                    source: "local".into(),
                },
                remote: EntryRevision {
                    path: row.get(2)?,
                    body_hash: row.get(4)?,
                    body: None,
                    updated_at: None,
                    source: "remote".into(),
                },
                common_ancestor: {
                    let hash: Option<String> = row.get(5)?;
                    hash.map(|h| EntryRevision {
                        path: row.get(2).unwrap_or_default(),
                        body_hash: h,
                        body: None,
                        updated_at: None,
                        source: "ancestor".into(),
                    })
                },
                kind,
                detected_at: row.get(7)?,
            })
        },
    )
    .ok()
    .map(|iter| iter.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// CO-383 integration: yggdrasil universe defaults to `replace` (remote wins).
fn suggested_action_for_universe(universe_key: &str) -> String {
    if universe_key == "yggdrasil" {
        "replace".into()
    } else {
        "update".into()
    }
}

// ---------------------------------------------------------------------------
// Helpers for bridge hash-skip and conflict persistence
// ---------------------------------------------------------------------------

/// Persist a conflict to `sync_conflicts`. Called from the bridge handler
/// when a federated entry event has a differing `body_hash`.
pub fn persist_conflict(
    conn: &rusqlite::Connection,
    conflict: &Conflict,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO sync_conflicts
         (id, universe_key, path, local_body_hash, remote_body_hash,
          common_ancestor_hash, kind, detected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            &conflict.id,
            &conflict.universe_key,
            &conflict.path,
            &conflict.local.body_hash,
            &conflict.remote.body_hash,
            conflict.common_ancestor.as_ref().map(|a| &a.body_hash),
            conflict.kind.to_string(),
            &conflict.detected_at,
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_action_yggdrasil_is_replace() {
        assert_eq!(suggested_action_for_universe("yggdrasil"), "replace");
    }

    #[test]
    fn suggested_action_other_is_update() {
        assert_eq!(suggested_action_for_universe("my-universe"), "update");
    }

    #[test]
    fn resolve_request_deserializes() {
        let json = r#"{"action":"update","apply_to_all_matching":true}"#;
        let req: ResolveRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.action, "update");
        assert!(req.apply_to_all_matching);
    }

    #[test]
    fn resolve_request_default_apply_to_all() {
        let json = r#"{"action":"replace"}"#;
        let req: ResolveRequest = serde_json::from_str(json).unwrap();
        assert!(!req.apply_to_all_matching);
    }
}
