//! CO-156: `reference` content type — metadata cards for PDF/image/video/audio/web assets.
//!
//! A reference card is a `.md` file with `type: reference` in its frontmatter.
//! It carries metadata about a bound binary asset (sibling file) or external URL.
//!
//! CO-432: layered per the CO-390 template — handlers here hold HTTP concerns
//! only; data access lives in `crate::repository::ReferenceRepository`, pure
//! rules in `crate::service::ReferenceService`, transport types in
//! `crate::dto::references`.
//!
//! Routes (nested under `/api/v1/universes/{u}`):
//!   GET    /references                — list cards (filter: medium, seed_status, q=fts)
//!   POST   /references                — create a new card
//!   GET    /references/orphan-blobs   — assets with no card
//!   GET    /references/broken-cards   — cards whose `file:` doesn't resolve
//!   GET    /references/{*path}        — read one card
//!   PUT    /references/{*path}        — update card
//!   DELETE /references/{*path}        — delete card (blob unaffected)

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use rusqlite::params;

use crate::auth::resolve_user_id;
use crate::entry_index::make_entry;
use crate::error::AppError;
use crate::mapper::ReferenceMapper;
use crate::repository::{ReferenceRepository, SqliteReferenceRepository};
use crate::server::AppState;
use crate::service::ReferenceService;
use crate::telemetry::{CrudEvent, emit_crud_event, extract_session_id};

// DTOs live in the transport layer (CO-432); re-exported under their
// pre-432 paths for call-site compatibility.
pub use crate::dto::references::{
    BrokenCard, CreateRefBody, ListRefsQuery, OrphanBlob, ReferenceCard, UpdateRefBody,
};
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.core.storage.lock()
}

fn require_writer(
    state: &AppState,
    headers: &HeaderMap,
    universe_key: &str,
) -> Result<String, AppError> {
    let user_id = resolve_user_id(state, headers)
        .ok_or_else(|| AppError::Unauthorized("Login required".into()))?;
    let storage = lock_storage(state);
    let universe = storage
        .get_universe(universe_key)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
    if universe.owner_id == user_id {
        return Ok(user_id);
    }
    let is_member: bool = storage
        .conn()
        .query_row(
            "SELECT 1 FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
            params![universe_key, &user_id],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if is_member {
        Ok(user_id)
    } else {
        Err(AppError::Forbidden(
            "Not authorized to write to this universe".into(),
        ))
    }
}

/// The references repository for a universe, without holding the storage lock.
fn reference_repo(state: &AppState, universe_key: &str) -> SqliteReferenceRepository {
    let conn = {
        let storage = lock_storage(state);
        storage.universe_conn(universe_key)
    };
    SqliteReferenceRepository::new(conn)
}

fn require_universe(state: &AppState, universe_key: &str) -> Result<(), AppError> {
    lock_storage(state)
        .get_universe(universe_key)
        .map(|_| ())
        .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/{u}/references
///
/// List reference cards with optional filters.
/// `?medium=pdf` — filter by medium (pdf, image, video, audio, web, citation).
/// `?seed_status=reviewed` — filter by seed status.
/// `?q=<text>` — full-text search across title, body, transcription.
pub async fn list_references(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
    Query(q): Query<ListRefsQuery>,
) -> Result<Json<Vec<ReferenceCard>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let repo = reference_repo(&state, &universe_key);
    let filter = ReferenceMapper::query_to_filter(q);
    let cards = repo
        .list_cards(&universe_key, &filter)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .into_iter()
        .map(ReferenceMapper::domain_to_card)
        .collect();
    Ok(Json(cards))
}

/// GET /api/v1/universes/{u}/references/orphan-blobs
///
/// List assets that have no corresponding reference card in `references_meta`.
pub async fn orphan_blobs(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
) -> Result<Json<Vec<OrphanBlob>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    require_universe(&state, &universe_key)?;
    let repo = reference_repo(&state, &universe_key);
    let blobs = repo
        .orphan_blobs(&universe_key)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .into_iter()
        .map(ReferenceMapper::orphan_to_dto)
        .collect();
    Ok(Json(blobs))
}

/// GET /api/v1/universes/{u}/references/broken-cards
///
/// List reference cards whose `file:` field doesn't resolve to an existing file on disk.
pub async fn broken_cards(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
) -> Result<Json<Vec<BrokenCard>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let universe_root = {
        let storage = lock_storage(&state);
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        storage.universe_root(&universe_key)
    };
    let repo = reference_repo(&state, &universe_key);
    let rows = repo
        .card_files(&universe_key)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let broken: Vec<BrokenCard> = rows
        .into_iter()
        .filter_map(|(entry_path, file)| {
            let rel = ReferenceService::expected_blob_path(&entry_path, &file)?;
            let file_path = universe_root.join(rel);
            let expected = file_path.to_string_lossy().into_owned();
            if !file_path.exists() {
                Some(BrokenCard {
                    entry_path,
                    file,
                    expected_path: expected,
                })
            } else {
                None
            }
        })
        .collect();

    Ok(Json(broken))
}

/// GET /api/v1/universes/{u}/references/{*path}
///
/// Read a single reference card by its entry path.
pub async fn get_reference(
    State(state): State<AppState>,
    Path((universe_key, path)): Path<(String, String)>,
) -> Result<Json<ReferenceCard>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    require_universe(&state, &universe_key)?;
    let repo = reference_repo(&state, &universe_key);
    // Returns the canonical edition (or the first one if no canonical_edition set).
    let card = repo
        .get_card(&universe_key, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Reference card '{path}' not found")))?;
    Ok(Json(ReferenceMapper::domain_to_card(card)))
}

/// POST /api/v1/universes/{u}/references
///
/// Create a new reference card entry. If `file:` is set in frontmatter, the
/// sibling blob must already exist (uploaded via `/assets`); sha256 is resolved
/// from disk and stored in `references_meta.blob_sha256`.
pub async fn create_reference(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateRefBody>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = require_writer(&state, &headers, &universe_key)?;

    let mut fm = body.frontmatter.clone();
    ReferenceService::force_reference_type(&mut fm);

    let (universe_root, universe_conn) = {
        let storage = lock_storage(&state);
        (
            storage.universe_root(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };

    let entry = make_entry(&body.path, fm.clone(), &body.body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    let title = fm.get("title").and_then(|v| v.as_str());
    SqliteReferenceRepository::new(universe_conn)
        .upsert_card(
            &universe_key,
            &entry,
            &fm,
            &body.body,
            title,
            &universe_root,
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let session_id = extract_session_id(&headers);
    emit_crud_event(
        &state,
        CrudEvent {
            kind: "entry.upsert",
            universe: universe_key.clone(),
            list: Some("reference".to_string()),
            key: Some(body.path.clone()),
            actor: Some(user_id),
            session_id,
            extra: None,
        },
    );

    // Update content count
    lock_storage(&state).increment_universe_content_count(&universe_key);

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "path": body.path,
            "universe_key": universe_key,
            "entry_type": "reference",
        })),
    )
        .into_response())
}

/// PUT /api/v1/universes/{u}/references/{*path}
///
/// Update an existing reference card.
pub async fn update_reference(
    State(state): State<AppState>,
    Path((universe_key, path)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpdateRefBody>,
) -> Result<Json<ReferenceCard>, AppError> {
    let user_id = require_writer(&state, &headers, &universe_key)?;

    let (universe_root, universe_conn) = {
        let storage = lock_storage(&state);
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        (
            storage.universe_root(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };
    let repo = SqliteReferenceRepository::new(universe_conn.clone());

    // Read existing entry through the entry repository.
    let existing = {
        use crate::repository::EntryRepository;
        crate::repository::SqliteEntryRepository::new(universe_conn)
            .find(&universe_key, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .ok_or_else(|| AppError::NotFound(format!("Reference card '{path}' not found")))?
    };

    let (new_fm, new_body) = ReferenceService::merged_update(
        &existing.frontmatter,
        &existing.body,
        body.frontmatter,
        body.body,
    );

    let entry = make_entry(&path, new_fm.clone(), &new_body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    let title = new_fm.get("title").and_then(|v| v.as_str());
    repo.upsert_card(
        &universe_key,
        &entry,
        &new_fm,
        &new_body,
        title,
        &universe_root,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let session_id = extract_session_id(&headers);
    emit_crud_event(
        &state,
        CrudEvent {
            kind: "entry.upsert",
            universe: universe_key.clone(),
            list: Some("reference".to_string()),
            key: Some(path.clone()),
            actor: Some(user_id),
            session_id,
            extra: None,
        },
    );

    // Return the canonical (first) edition of the updated card.
    let card = repo
        .get_card(&universe_key, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| {
            AppError::Internal(format!("reference card '{path}' missing after update"))
        })?;

    Ok(Json(ReferenceMapper::domain_to_card(card)))
}

/// DELETE /api/v1/universes/{u}/references/{*path}
///
/// Delete a reference card. The bound blob on disk is NOT deleted.
pub async fn delete_reference(
    State(state): State<AppState>,
    Path((universe_key, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let user_id = require_writer(&state, &headers, &universe_key)?;

    let (universe_root, universe_conn) = {
        let storage = lock_storage(&state);
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        (
            storage.universe_root(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };

    co::delete_entry(&universe_root, &path).map_err(|e| AppError::Internal(e.to_string()))?;

    SqliteReferenceRepository::new(universe_conn)
        .delete_card(&universe_key, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    lock_storage(&state).decrement_universe_content_count(&universe_key, 1);

    let session_id = extract_session_id(&headers);
    emit_crud_event(
        &state,
        CrudEvent {
            kind: "entry.delete",
            universe: universe_key.clone(),
            list: Some("reference".to_string()),
            key: Some(path),
            actor: Some(user_id),
            session_id,
            extra: None,
        },
    );

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/universes/{u}/references/works
///
/// Return the distinct `work_id` values present in the universe.
pub async fn list_works(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
) -> Result<Json<Vec<String>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    require_universe(&state, &universe_key)?;
    let repo = reference_repo(&state, &universe_key);
    let works = repo
        .list_works(&universe_key)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(works))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn reference_router() -> Router<AppState> {
    Router::new()
        // Specific paths must come before the wildcard {*path} route.
        .route("/{u}/references/orphan-blobs", get(orphan_blobs))
        .route("/{u}/references/broken-cards", get(broken_cards))
        .route("/{u}/references/works", get(list_works))
        .route(
            "/{u}/references",
            get(list_references).post(create_reference),
        )
        .route(
            "/{u}/references/{*path}",
            get(get_reference)
                .put(update_reference)
                .delete(delete_reference),
        )
}
