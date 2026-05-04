//! CO-153: cross-universe relation query endpoints.
//!
//! GET /api/v1/universes/:slug/relations/inbound?path=<path>
//!   Returns all inbound relations pointing to `<path>` within universe `<slug>`,
//!   including cross-universe edges stored in other universes' `entry_relations`.
//!
//! GET /api/v1/universes/:slug/relations/outbound?path=<path>
//!   Returns all outbound relations originating from `<path>` in `<slug>`.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::error::AppError;
use crate::server::AppState;
use crate::universe_pool::UniversePool;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RelationQuery {
    pub path: String,
}

/// A single inbound relation edge — who points to this entry and from where.
#[derive(Debug, Serialize)]
pub struct InboundRelation {
    pub from_universe: String,
    pub from_path: String,
    pub relation_type: String,
}

/// A single outbound relation edge — where this entry points.
#[derive(Debug, Serialize)]
pub struct OutboundRelation {
    /// `None` means same universe.
    pub to_universe: Option<String>,
    pub to_path: String,
    pub relation_type: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lock_storage(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::storage::Storage>, AppError> {
    state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("Storage lock failed".into()))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/:slug/relations/inbound?path=<path>
///
/// Returns inbound relations from:
///   1. The same universe (`to_universe IS NULL, to_path = <path>`)
///   2. Every other universe where `to_universe = <slug>` and `to_path = <path>`
pub async fn get_inbound_relations(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<RelationQuery>,
) -> Result<Json<Vec<InboundRelation>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let (all_keys, pool): (Vec<String>, Arc<UniversePool>) = {
        let storage = lock_storage(&state)?;
        let keys = storage.all_universe_keys();
        let pool = Arc::clone(&storage.universe_pool);
        (keys, pool)
    };

    let path = q.path.clone();
    let mut result: Vec<InboundRelation> = Vec::new();

    // 1. Same-universe inbound: rows in slug's own DB where to_universe IS NULL
    //    (standard inbound — to_path = <path>, no cross-universe column)
    {
        let uc = pool.get_or_open(&slug);
        let conn = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let rows = crate::relation_index::RelationIndex::new(&conn)
            .inbound(&slug, &path)
            .unwrap_or_default();
        for r in rows {
            result.push(InboundRelation {
                from_universe: r.universe_key,
                from_path: r.from_path,
                relation_type: r.relation_type,
            });
        }
    }

    // 2. Cross-universe inbound: scan every other universe's DB for rows where
    //    to_universe = <slug> AND to_path = <path>.
    for key in &all_keys {
        if key == &slug {
            continue;
        }
        let uc = pool.get_or_open(key);
        let conn = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let rows = crate::relation_index::RelationIndex::new(&conn)
            .inbound_from_other(&slug, &path)
            .unwrap_or_default();
        for r in rows {
            result.push(InboundRelation {
                from_universe: r.universe_key,
                from_path: r.from_path,
                relation_type: r.relation_type,
            });
        }
    }

    result.sort_by(|a, b| {
        a.relation_type
            .cmp(&b.relation_type)
            .then(a.from_universe.cmp(&b.from_universe))
            .then(a.from_path.cmp(&b.from_path))
    });

    Ok(Json(result))
}

/// GET /api/v1/universes/:slug/relations/outbound?path=<path>
///
/// Returns all outbound relations from `<path>` in `<slug>`, including
/// cross-universe edges where `to_universe IS NOT NULL`.
pub async fn get_outbound_relations(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<RelationQuery>,
) -> Result<Json<Vec<OutboundRelation>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let uc = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&slug)
    };
    let conn = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    let rows = crate::relation_index::RelationIndex::new(&conn)
        .outbound(&slug, &q.path)
        .unwrap_or_default();

    let result: Vec<OutboundRelation> = rows
        .into_iter()
        .map(|r| OutboundRelation {
            to_universe: r.to_universe,
            to_path: r.to_path,
            relation_type: r.relation_type,
        })
        .collect();

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{slug}/relations/inbound", get(get_inbound_relations))
        .route("/{slug}/relations/outbound", get(get_outbound_relations))
}
