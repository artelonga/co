//! CO-153: cross-universe relation query endpoints.
//!
//! GET /api/v1/universes/:slug/relations/inbound?path=<path>
//!   Returns all inbound relations pointing to `<path>` within universe `<slug>`,
//!   including cross-universe edges stored in other universes' `entry_relations`.
//!
//! GET /api/v1/universes/:slug/relations/outbound?path=<path>
//!   Returns all outbound relations originating from `<path>` in `<slug>`.
//!
//! CO-432: layered per the CO-390 template — handlers hold HTTP concerns only;
//! data access lives in `crate::repository::RelationRepository`, the ordering
//! rule in `crate::service::RelationService`, transport types in
//! `crate::dto::relations`.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use std::sync::Arc;

use crate::error::AppError;
use crate::mapper::RelationMapper;
use crate::repository::{RelationRepository, SqliteRelationRepository};
use crate::server::AppState;
use crate::service::RelationService;
use crate::universe_pool::UniversePool;

// DTOs live in the transport layer (CO-432); re-exported under their
// pre-432 paths for call-site compatibility.
pub use crate::dto::relations::{InboundRelation, OutboundRelation, RelationQuery};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.core.storage.lock()
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
        let storage = lock_storage(&state);
        let keys = storage.all_universe_keys();
        let pool = Arc::clone(&storage.universe_pool);
        (keys, pool)
    };

    let path = q.path.clone();
    let mut result: Vec<InboundRelation> = Vec::new();

    // 1. Same-universe inbound: rows in slug's own DB where to_universe IS NULL
    //    (standard inbound — to_path = <path>, no cross-universe column)
    let own_repo = SqliteRelationRepository::new(pool.get_or_open(&slug));
    result.extend(
        own_repo
            .inbound(&slug, &path)
            .unwrap_or_default()
            .iter()
            .map(RelationMapper::domain_to_inbound),
    );

    // 2. Cross-universe inbound: scan every other universe's DB for rows where
    //    to_universe = <slug> AND to_path = <path>.
    for key in &all_keys {
        if key == &slug {
            continue;
        }
        let repo = SqliteRelationRepository::new(pool.get_or_open(key));
        result.extend(
            repo.inbound_from_other(&slug, &path)
                .unwrap_or_default()
                .iter()
                .map(RelationMapper::domain_to_inbound),
        );
    }

    RelationService::sort_inbound(&mut result);

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
        let storage = lock_storage(&state);
        storage.universe_conn(&slug)
    };
    let repo = SqliteRelationRepository::new(uc);

    let result: Vec<OutboundRelation> = repo
        .outbound(&slug, &q.path)
        .unwrap_or_default()
        .iter()
        .map(RelationMapper::domain_to_outbound)
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
