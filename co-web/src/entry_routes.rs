//! Entry API — `/api/v1/universes/:slug/entries`
//!
//! Every entity in a CO universe is an Entry: a markdown file with YAML frontmatter.
//! These routes provide CRUD over the entry index, with content-type negotiation
//! between JSON (`application/json`) and protobuf (`application/x-protobuf`).

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::entry_index::{EntryRow, TagCount, TreeNode, make_entry};
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Manifest validation helpers
// ---------------------------------------------------------------------------

/// Load `_universe.yaml` from disk without caching.
fn load_manifest(universe_root: &std::path::Path) -> Option<co::manifest::Manifest> {
    let bytes = std::fs::read(universe_root.join(co::manifest::MANIFEST_FILENAME)).ok()?;
    co::manifest::parse(&bytes).ok().map(|r| r.manifest)
}

/// CO-79: Load manifest from the L1 cache; fall back to disk on miss and insert into cache.
///
/// Uses singleflight coalescing via `ManifestCache::get_or_fill` so that N
/// concurrent misses for the same slug result in exactly one disk read.
async fn load_manifest_cached(
    state: &AppState,
    slug: &str,
    universe_root: &std::path::Path,
) -> Option<std::sync::Arc<co::manifest::Manifest>> {
    let root = universe_root.to_path_buf();
    state
        .cache
        .manifest
        .get_or_fill(slug.to_string(), || async move { load_manifest(&root) })
        .await
}

/// Validate `frontmatter` against the manifest's content type for `entry_type`.
///
/// Returns `Ok(())` when:
/// - `manifest` is `None` (no `_universe.yaml`).
/// - The manifest has no schema for `entry_type`.
/// - The payload passes all schema checks.
///
/// Returns `Err(AppError::UnprocessableEntity)` with a field-path message
/// when validation fails.
fn validate_against_manifest(
    manifest: Option<&co::manifest::Manifest>,
    entry_type: &str,
    frontmatter: &JsonValue,
) -> Result<(), AppError> {
    let manifest = match manifest {
        Some(m) => m,
        None => return Ok(()),
    };
    let ct = match manifest
        .content_types
        .iter()
        .find(|ct| ct.name == entry_type)
    {
        Some(ct) => ct,
        None => return Ok(()),
    };
    co::payload::validate_payload(ct, frontmatter)
        .map_err(|e| AppError::UnprocessableEntity(format!("payload validation failed: {e}")))
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EntryListQuery {
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    /// Extra frontmatter filter — JSON encoded, e.g. `{"project":"MP"}`
    pub filter: Option<String>,
    /// Full-text search query
    pub q: Option<String>,
    /// CO-73: date semantic to filter by (e.g. `event_at`, `due_at`)
    pub date_semantic: Option<String>,
    /// CO-73: inclusive ISO-8601 start of date range
    pub from: Option<String>,
    /// CO-73: inclusive ISO-8601 end of date range
    pub to: Option<String>,
    /// Max entries to return. Defaults to 5000; capped at 50000.
    pub limit: Option<usize>,
    /// 1.62.0 (Phase 7): rewind view — when set to a `states/...md` path,
    /// the result is filtered to only entries whose path appears in that
    /// state's manifest. Bodies served are still current (this is path-
    /// only rewind for v1; full-fidelity rewind requires content-addressed
    /// blob storage and is out of scope here). Use `?as_of=states/...md`.
    pub as_of: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEntryBody {
    pub path: String,
    pub frontmatter: JsonValue,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEntryBody {
    pub frontmatter: Option<JsonValue>,
    pub body: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EntryListResponse {
    pub entries: Vec<EntryRow>,
    pub total: usize,
}

/// CO-74: query DSL parameters for `GET /:slug/query`.
#[derive(Debug, Deserialize)]
pub struct QueryDslParams {
    pub q: String,
}

/// CO-74: entry detail with outbound relations for board relation-aware views.
#[derive(Debug, Serialize)]
pub struct EntryWithRelations {
    #[serde(flatten)]
    pub entry: EntryRow,
    /// Outbound FK relations declared in manifest (relation_type → to_path).
    pub relations: Vec<crate::relation_index::RelationRow>,
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

fn accept_protobuf(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("application/x-protobuf"))
        .unwrap_or(false)
}

// check_reader_for_entries removed — universe read visibility is enforced by
// universe_visibility_gate middleware applied in server::build_router (CO-161).

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/:slug/entries — list entries
pub async fn list_entries(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<EntryListQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let uc = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = crate::entry_index::EntryIndex::new(&uc_guard);

    let entries = if let Some(ref fts_query) = q.q {
        index
            .search(&slug, fts_query)
            .map_err(|e| AppError::Internal(e.to_string()))?
    } else if let Some(ref semantic) = q.date_semantic {
        // CO-73: date-semantic range query
        index
            .query_by_date(&slug, semantic, q.from.as_deref(), q.to.as_deref())
            .map_err(|e| AppError::Internal(e.to_string()))?
    } else {
        let entry_type = q.entry_type.as_deref().unwrap_or("");
        let limit = q.limit;
        if entry_type.is_empty() {
            // list all
            index
                .query_with_limit(&slug, "", &serde_json::json!({}), limit)
                .or_else(|_| Ok::<Vec<EntryRow>, anyhow::Error>(vec![]))
                .unwrap_or_default()
        } else {
            let filter = q
                .filter
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::json!({}));
            index
                .query_with_limit(&slug, entry_type, &filter, limit)
                .map_err(|e| AppError::Internal(e.to_string()))?
        }
    };

    // 1.62.0 Phase 7 + 1.74.0 Phase 8 step 4: rewind view via
    // `?as_of=states/...md`. Filters the result to paths in the manifest
    // AND, when the manifest carries a per-entry `body_hash` (3-column
    // form, post-1.74.0 captures), substitutes the entry body with the
    // historical bytes from the CAS blob store. Legacy 2-column manifest
    // lines fall through to current bodies (path-fidelity only).
    let entries = if let Some(as_of) = q.as_of.as_deref() {
        if !as_of.starts_with("states/") {
            return Err(AppError::BadRequest(format!(
                "as_of must be a path under states/ (got '{as_of}')"
            )));
        }
        let state_row = index
            .get(&slug, as_of)
            .map_err(|e| AppError::Internal(format!("get state: {e}")))?
            .ok_or_else(|| {
                AppError::BadRequest(format!("state '{as_of}' not found in '{slug}'"))
            })?;
        if state_row.entry_type != "state" {
            return Err(AppError::BadRequest(format!(
                "'{as_of}' is type '{}', not 'state'",
                state_row.entry_type
            )));
        }
        let manifest = crate::state_routes::parse_state_manifest_full(&state_row.body);
        let body_hash_by_path: std::collections::HashMap<&str, &str> = manifest
            .iter()
            .filter_map(|(path, _combined, body_hash)| {
                body_hash.as_deref().map(|h| (path.as_str(), h))
            })
            .collect();
        let allowed_paths: std::collections::HashSet<&str> =
            manifest.iter().map(|(p, _, _)| p.as_str()).collect();

        let storage_for_blobs = lock_storage(&state)?;
        entries
            .into_iter()
            .filter(|e| allowed_paths.contains(e.path.as_str()))
            .map(|mut e| {
                // If the manifest carries a body_hash for this path AND we
                // have the bytes in the CAS blob store, swap the current
                // body for the historical one. Otherwise leave as-is
                // (path-fidelity fallback).
                if let Some(h) = body_hash_by_path.get(e.path.as_str())
                    && let Some(bytes) = storage_for_blobs.get_blob(h)
                    && let Ok(historical) = String::from_utf8(bytes)
                {
                    e.body = historical;
                }
                e
            })
            .collect()
    } else {
        entries
    };

    if accept_protobuf(&headers) {
        // Encode as protobuf EntryList
        use prost::Message;
        let proto_entries: Vec<co::proto::entry::Entry> =
            entries.iter().map(entry_row_to_proto).collect();
        let list = co::proto::entry::EntryList {
            entries: proto_entries,
            total: entries.len() as u64,
        };
        let mut buf = Vec::new();
        list.encode(&mut buf)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok((
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/x-protobuf")],
            buf,
        )
            .into_response())
    } else {
        let total = entries.len();
        Ok(Json(EntryListResponse { entries, total }).into_response())
    }
}

/// GET /api/v1/universes/:slug/entries/tags — aggregate tags
pub async fn list_entry_tags(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<TagCount>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let uc = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = crate::entry_index::EntryIndex::new(&uc_guard);
    let tags = index
        .tags(&slug)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(tags))
}

/// GET /api/v1/universes/:slug/entries/tree — hierarchical tree
pub async fn entry_tree(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<TreeQuery>,
) -> Result<Json<Vec<TreeNode>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let uc = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = crate::entry_index::EntryIndex::new(&uc_guard);
    let entry_type = q.entry_type.as_deref().unwrap_or("page");
    let tree = index
        .tree(&slug, entry_type)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(tree))
}

/// Query params for single-entry GET: `?excerpt=true` returns frontmatter + 200-char excerpt only.
#[derive(Debug, Deserialize)]
pub struct GetEntryQuery {
    pub excerpt: Option<bool>,
}

/// Frontmatter + first-200-char excerpt — returned when `?excerpt=true`.
/// Used by board view to render task cards without fetching full bodies.
#[derive(Debug, Serialize)]
pub struct EntryExcerpt {
    pub frontmatter: JsonValue,
    pub excerpt: String,
}

/// GET /api/v1/universes/:slug/entries/*path — read single entry
pub async fn get_entry(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    Query(q): Query<GetEntryQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let uc = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = crate::entry_index::EntryIndex::new(&uc_guard);
    let entry = index
        .get(&slug, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{}' not found", path)))?;

    // CO-150: ?excerpt=true — fast path for board cards; returns frontmatter + 200-char excerpt only.
    if q.excerpt.unwrap_or(false) {
        let excerpt: String = entry.body.chars().take(200).collect();
        return Ok(Json(EntryExcerpt {
            frontmatter: entry.frontmatter,
            excerpt,
        })
        .into_response());
    }

    if accept_protobuf(&headers) {
        use prost::Message;
        let proto = entry_row_to_proto(&entry);
        let mut buf = Vec::new();
        proto
            .encode(&mut buf)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok((
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/x-protobuf")],
            buf,
        )
            .into_response())
    } else {
        // CO-74: include outbound FK relations in entry detail for board relation-aware views.
        let relations = {
            crate::relation_index::RelationIndex::new(&uc_guard)
                .outbound(&slug, &path)
                .unwrap_or_default()
        };
        Ok(Json(EntryWithRelations { entry, relations }).into_response())
    }
}

/// POST /api/v1/universes/:slug/entries — create entry
pub async fn create_entry(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateEntryBody>,
) -> Result<impl IntoResponse, AppError> {
    if body.path.is_empty() {
        return Err(AppError::BadRequest("Entry path cannot be empty".into()));
    }

    let universe_root = {
        let storage = lock_storage(&state)?;
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

        // CO-80: quota check — anonymous usage gate or tier-based storage quota.
        if let Some((uid, tier)) = crate::rate_limit::extract_auth_identity(&headers) {
            crate::rate_limit::check_storage_quota(&storage, &uid, tier, &headers)?;
        } else if universe.owner_id.starts_with("anon-") && universe.content_count >= 100 {
            return Err(AppError::UsageLimitExceeded {
                current: universe.content_count,
            });
        }

        storage.universe_root(&slug)
    };

    // CO-79: load manifest once from L1 cache (singleflight stampede protection).
    let manifest_arc = load_manifest_cached(&state, &slug, &universe_root).await;

    // CO-71: validate frontmatter against manifest schema before writing.
    let entry_type = body
        .frontmatter
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    validate_against_manifest(manifest_arc.as_deref(), entry_type, &body.frontmatter)?;

    // Write .md file
    let entry = make_entry(&body.path, body.frontmatter.clone(), &body.body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    // If the manifest file itself was written, invalidate the L1 cache.
    if body.path == co::manifest::MANIFEST_FILENAME {
        state.cache.invalidate_universe(&slug);
    }

    // Index into universe data.db
    let relation_count = {
        let uc = {
            let storage = lock_storage(&state)?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        index
            .upsert(&slug, &entry)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-73: index semantic date fields (reuse cached manifest)
        index
            .upsert_dates(&slug, &entry, manifest_arc.as_deref())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-74: extract and store typed FK relations from manifest-declared ref/ref_list fields
        let rc = if let Some(ref m) = manifest_arc {
            crate::relation_index::sync_entry_relations(
                &uc_guard,
                &slug,
                &body.path,
                &entry.entry_type,
                &body.frontmatter,
                m,
            )
            .unwrap_or(0)
        } else {
            0
        };
        // CO-156: sync references_meta shadow table for reference cards
        crate::reference_routes::maybe_sync_reference_meta(
            &uc_guard,
            &slug,
            &body.path,
            &entry.entry_type,
            &body.frontmatter,
            &body.body,
            body.frontmatter.get("title").and_then(|v| v.as_str()),
            &universe_root,
        );
        rc
    };
    // CO-79: invalidate query cache entries for this universe after a write.
    state.cache.query.invalidate_prefix(&format!("{slug}:"));

    // CO-156: emit entry.upsert telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "entry.upsert",
            universe: slug.clone(),
            list: Some(entry.entry_type.clone()),
            key: Some(body.path.clone()),
            actor: crate::auth::resolve_user_id(&state, &headers),
            session_id: crate::telemetry::extract_session_id(&headers),
            extra: None,
        },
    );

    // CO-156: emit relation.create if relations were written
    if relation_count > 0 {
        crate::telemetry::emit_crud_event(
            &state,
            crate::telemetry::CrudEvent {
                kind: "relation.create",
                universe: slug.clone(),
                list: Some(entry.entry_type.clone()),
                key: Some(body.path.clone()),
                actor: crate::auth::resolve_user_id(&state, &headers),
                session_id: crate::telemetry::extract_session_id(&headers),
                extra: Some(serde_json::json!({ "count": relation_count })),
            },
        );
    }

    // Update universe content_count
    let mut storage = lock_storage(&state)?;
    storage.increment_universe_content_count(&slug);

    // CO-45: log mutation on UAT before body values are moved into the response
    if state.config.is_uat() {
        let after_val = serde_json::json!({
            "path": body.path,
            "frontmatter": body.frontmatter,
            "body": body.body,
        });
        let target = format!("{}:{}", slug, body.path);
        let _ = storage.log_uat_mutation(
            "entry.create",
            &target,
            None,
            Some(&after_val.to_string()),
            None,
            None,
        );
    }

    Ok((
        StatusCode::CREATED,
        Json(EntryRow {
            path: body.path,
            universe_key: slug,
            entry_type: entry.entry_type,
            title: entry
                .frontmatter
                .get("title")
                .and_then(|v| v.as_str())
                .map(String::from),
            frontmatter: body.frontmatter,
            body: body.body,
            body_hash: entry.body_hash,
            created_at: Some(entry.stat.created.to_rfc3339()),
            updated_at: Some(entry.stat.modified.to_rfc3339()),
        }),
    )
        .into_response())
}

/// PUT /api/v1/universes/:slug/entries/*path — update entry
pub async fn update_entry(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    Json(body): Json<UpdateEntryBody>,
) -> Result<Json<EntryRow>, AppError> {
    let universe_root = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
        storage.universe_root(&slug)
    };

    // Read existing entry from universe data.db
    let existing = {
        let uc = {
            let storage = lock_storage(&state)?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        index
            .get(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .ok_or_else(|| AppError::NotFound(format!("Entry '{}' not found", path)))?
    };

    let new_fm = body.frontmatter.unwrap_or(existing.frontmatter.clone());
    let new_body = body.body.unwrap_or(existing.body.clone());

    // CO-79: load manifest once from L1 cache (singleflight stampede protection).
    let manifest_arc = load_manifest_cached(&state, &slug, &universe_root).await;

    // CO-71: validate merged frontmatter against manifest schema before writing.
    let entry_type_for_validation = new_fm
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    validate_against_manifest(manifest_arc.as_deref(), entry_type_for_validation, &new_fm)?;

    let entry = make_entry(&path, new_fm.clone(), &new_body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    // If the manifest file itself was written, invalidate the L1 cache.
    if path == co::manifest::MANIFEST_FILENAME {
        state.cache.invalidate_universe(&slug);
    }

    {
        let uc = {
            let storage = lock_storage(&state)?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        index
            .upsert(&slug, &entry)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-73: index semantic date fields (reuse cached manifest)
        index
            .upsert_dates(&slug, &entry, manifest_arc.as_deref())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-74: re-sync typed FK relations
        let _ = if let Some(ref m) = manifest_arc {
            crate::relation_index::sync_entry_relations(
                &uc_guard,
                &slug,
                &path,
                &entry.entry_type,
                &new_fm,
                m,
            )
        } else {
            Ok(0)
        };
        // CO-156: sync references_meta for reference cards
        crate::reference_routes::maybe_sync_reference_meta(
            &uc_guard,
            &slug,
            &path,
            &entry.entry_type,
            &new_fm,
            &new_body,
            new_fm.get("title").and_then(|v| v.as_str()),
            &universe_root,
        );
    }
    // CO-79: invalidate query cache entries for this universe after a write.
    state.cache.query.invalidate_prefix(&format!("{slug}:"));

    // CO-156: emit entry.upsert telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "entry.upsert",
            universe: slug.clone(),
            list: Some(entry.entry_type.clone()),
            key: Some(path.clone()),
            actor: None,
            session_id: None,
            extra: None,
        },
    );

    let storage = lock_storage(&state)?;
    // CO-45: log mutation on UAT
    if state.config.is_uat() {
        let before_val = serde_json::to_string(&existing).unwrap_or_default();
        let after_val = serde_json::json!({ "frontmatter": new_fm, "body": new_body }).to_string();
        let target = format!("{}:{}", slug, path);
        let _ = storage.log_uat_mutation(
            "entry.update",
            &target,
            Some(&before_val),
            Some(&after_val),
            None,
            None,
        );
    }

    Ok(Json(EntryRow {
        path,
        universe_key: slug,
        entry_type: entry.entry_type,
        title: new_fm
            .get("title")
            .and_then(|v| v.as_str())
            .map(String::from),
        frontmatter: new_fm,
        body: new_body,
        body_hash: entry.body_hash,
        created_at: existing.created_at,
        updated_at: Some(entry.stat.modified.to_rfc3339()),
    }))
}

/// GET /api/v1/universes/:slug/manifest — return the universe manifest
///
/// Returns the parsed `_universe.yaml` manifest as JSON.  Falls back to
/// the built-in default manifest (task board) when no `_universe.yaml` is
/// present in the universe directory.
pub async fn get_manifest(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<co::manifest::Manifest>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let universe_root = {
        let storage = lock_storage(&state)?;
        storage.universe_root(&slug)
    };
    // CO-79: serve from L1 manifest cache (singleflight on miss).
    let slug_clone = slug.clone();
    let manifest = load_manifest_cached(&state, &slug, &universe_root)
        .await
        .map(|arc| arc.as_ref().clone())
        .unwrap_or_else(|| co::manifest::default_manifest(&slug_clone));
    Ok(Json(manifest))
}

/// DELETE /api/v1/universes/:slug/entries/*path — delete entry
pub async fn delete_entry(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let universe_root = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
        storage.universe_root(&slug)
    };

    // CO-45: capture before_value on UAT before deletion
    let before_val = if state.config.is_uat() {
        let uc = {
            let storage = lock_storage(&state)?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        index
            .get(&slug, &path)
            .ok()
            .flatten()
            .and_then(|e| serde_json::to_string(&e).ok())
    } else {
        None
    };

    co::delete_entry(&universe_root, &path).map_err(|e| AppError::Internal(e.to_string()))?;

    {
        let uc = {
            let storage = lock_storage(&state)?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        index
            .remove(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-73: remove semantic date rows
        index
            .remove_dates(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-74: remove outbound FK relations
        let _ = crate::relation_index::RelationIndex::new(&uc_guard).delete_for_entry(&slug, &path);
        // CO-156: remove references_meta + references_fts (idempotent)
        crate::reference_routes::remove_reference_meta(&uc_guard, &slug, &path);
    }
    let mut storage = lock_storage(&state)?;
    storage.decrement_universe_content_count(&slug, 1);

    // CO-45: log mutation on UAT
    if state.config.is_uat() {
        let target = format!("{}:{}", slug, path);
        let _ = storage.log_uat_mutation(
            "entry.delete",
            &target,
            before_val.as_deref(),
            None,
            None,
            None,
        );
    }

    // CO-156: emit relation.delete + entry.delete telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "relation.delete",
            universe: slug.clone(),
            list: None,
            key: Some(path.clone()),
            actor: None,
            session_id: None,
            extra: None,
        },
    );
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "entry.delete",
            universe: slug.clone(),
            list: None,
            key: Some(path),
            actor: None,
            session_id: None,
            extra: None,
        },
    );

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/universes/:slug/query — execute a CO-74 query DSL expression.
///
/// Accepts a `?q=<dsl>` query parameter and returns matching entries.
/// The DSL is compiled to parameterized SQLite and executed against the
/// per-universe `data.db`.  Results are capped at 1 000 rows.
///
/// # DSL examples
///
/// ```text
/// FROM evento WHERE attendees INCLUDES "yuri"
/// FROM tarefa WHERE status = "todo" LIMIT 50
/// FROM evento WHERE attendees INCLUDES "yuri" AND status = "confirmed"
/// ```
pub async fn query_handler(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<QueryDslParams>,
) -> Result<Json<EntryListResponse>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let uc = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    let dsl_query = crate::query_dsl::parse(&params.q)
        .map_err(|e| AppError::BadRequest(format!("query parse error: {e}")))?;

    let (sql, sql_params) = crate::query_dsl::compile(&dsl_query, &slug);

    let params_refs: Vec<&dyn rusqlite::types::ToSql> = sql_params
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();

    let index = crate::entry_index::EntryIndex::new(&uc_guard);
    let entries = index
        .query_raw(&sql, params_refs.as_slice())
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let total = entries.len();
    Ok(Json(EntryListResponse { entries, total }))
}

// ---------------------------------------------------------------------------
// CO-154: References API
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct ReferencesQuery {
    source: Option<String>,
    url_contains: Option<String>,
    q: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReferencesResponse {
    references: Vec<crate::reference_index::ReferenceRow>,
    total: usize,
}

/// GET /api/v1/universes/:slug/references
///
/// Query `references_index` with optional filters:
/// - `?source=<substring>` — filter by source name
/// - `?url_contains=<substring>` — filter by URL
/// - `?q=<fts-query>` — full-text search over source + excerpt_body
pub(crate) async fn list_references(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<ReferencesQuery>,
) -> Result<Json<ReferencesResponse>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let uc = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&slug)
    };
    let conn = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let references = crate::reference_index::ReferenceIndex::new(&conn)
        .query(
            &slug,
            params.source.as_deref(),
            params.url_contains.as_deref(),
            params.q.as_deref(),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let total = references.len();
    Ok(Json(ReferencesResponse { references, total }))
}

/// GET /api/v1/universes/:slug/references/orphan-wikilinks
///
/// Return all `[[wikilink]]` targets found inside `## Referência:` excerpts
/// that do not have a corresponding entry in this universe — the candidate-entry
/// backlog.
pub(crate) async fn list_orphan_wikilinks(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<String>>, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    let uc = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&slug)
    };
    let conn = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let orphans = crate::reference_index::ReferenceIndex::new(&conn)
        .orphan_wikilinks(&slug)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(orphans))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        // Specific paths must come before wildcard
        .route("/{slug}/manifest", get(get_manifest))
        // CO-74: query DSL endpoint
        .route("/{slug}/query", get(query_handler))
        .route("/{slug}/entries/tags", get(list_entry_tags))
        .route("/{slug}/entries/tree", get(entry_tree))
        .route("/{slug}/entries", get(list_entries).post(create_entry))
        .route(
            "/{slug}/entries/{*path}",
            get(get_entry).put(update_entry).delete(delete_entry),
        )
        // CO-154: citations endpoints (per-citation index, distinct from
        // CO-156's /references which lists reference cards). Renamed from
        // `/references` to `/citations` to disambiguate at the route layer.
        .route(
            "/{slug}/citations/orphan-wikilinks",
            get(list_orphan_wikilinks),
        )
        .route("/{slug}/citations", get(list_references))
}

// ---------------------------------------------------------------------------
// Protobuf helpers
// ---------------------------------------------------------------------------

fn entry_row_to_proto(row: &EntryRow) -> co::proto::entry::Entry {
    use co::proto::entry::{Entry, FileStat, Value};

    let frontmatter: std::collections::HashMap<String, Value> = row
        .frontmatter
        .as_object()
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| (k.clone(), json_value_to_proto(v)))
                .collect()
        })
        .unwrap_or_default();

    let title = row
        .frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .or(row.title.as_deref())
        .unwrap_or("")
        .to_string();

    Entry {
        path: row.path.clone(),
        entry_type: row.entry_type.clone(),
        title,
        frontmatter,
        body: row.body.clone(),
        body_hash: row.body_hash.clone(),
        stat: Some(FileStat {
            created_ms: 0,
            modified_ms: 0,
            size: 0,
        }),
    }
}

fn json_value_to_proto(val: &JsonValue) -> co::proto::entry::Value {
    use co::proto::entry::{ListValue, Value, value::Kind};
    let kind = match val {
        JsonValue::String(s) => Some(Kind::StringValue(s.clone())),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(Kind::IntValue(i))
            } else {
                Some(Kind::FloatValue(n.as_f64().unwrap_or(0.0)))
            }
        }
        JsonValue::Bool(b) => Some(Kind::BoolValue(*b)),
        JsonValue::Array(arr) => Some(Kind::ListValue(ListValue {
            values: arr.iter().map(json_value_to_proto).collect(),
        })),
        _ => Some(Kind::StringValue(val.to_string())),
    };
    Value { kind }
}
