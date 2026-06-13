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

use crate::entry_index::{EntryRow, make_entry};
use crate::error::AppError;
use crate::server::AppState;
// CO-390 spike: thin controller delegates business rules to EntryService.
use crate::service::EntryService;

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
        .index
        .cache
        .manifest
        .get_or_fill(slug.to_string(), || async move { load_manifest(&root) })
        .await
}

/// CO-390 spike: thin wrapper — delegates to `EntryService::validate_entry_type`.
///
/// Kept as a local function so all call sites in this file stay unchanged,
/// but the business rule is now tested in isolation via the service layer.
fn validate_against_manifest(
    manifest: Option<&co::manifest::Manifest>,
    entry_type: &str,
    frontmatter: &JsonValue,
) -> Result<(), AppError> {
    EntryService::validate_entry_type(manifest, entry_type, frontmatter)
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
    /// CO-164: semantic similarity query text.
    pub semantic: Option<String>,
    /// CO-164: number of top-K results to return for semantic/similar queries (default 10).
    pub k: Option<usize>,
    /// CO-264: filter entries by path prefix (e.g. `public/` returns all `public/*` entries).
    pub path_prefix: Option<String>,
}

/// CO-164: query parameters for the `/similar` endpoint.
#[derive(Debug, Deserialize)]
pub struct SimilarQuery {
    /// Vault-relative path of the entry to find similar entries for.
    pub path: String,
    pub k: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEntryBody {
    pub path: String,
    pub frontmatter: JsonValue, // FREEFORM: per-type schema defined by universe manifest
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEntryBody {
    pub frontmatter: Option<JsonValue>, // FREEFORM: partial patch on open per-type schema
    pub body: Option<String>,
    /// CO-128: optimistic-concurrency token. The `body_hash` the client last
    /// observed for this entry. When present and the stored entry has since
    /// diverged, the write is rejected with `409 Conflict` and a
    /// `ConflictPayload { local, remote, base }` so the SPA can open the
    /// Apple-style conflict-resolution modal. Absent → last-write-wins
    /// (backward compatible: draft autosave and other callers are unaffected).
    pub base_hash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EntryListResponse {
    pub entries: Vec<EntryRow>,
    pub total: usize,
}

/// Typed response for `GET /:slug/entries/history`.
#[derive(Debug, Serialize)]
pub struct EntryHistoryResponse {
    pub path: String,
    pub events: Vec<crate::entry_index::EntryEventRow>,
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

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.core.storage.lock()
}

/// The entry repository over a universe's connection (CO-432) — handlers go
/// through this instead of constructing `EntryIndex` on a raw guard.
fn entry_repo(state: &AppState, slug: &str) -> crate::repository::SqliteEntryRepository {
    let conn = {
        let storage = lock_storage(state);
        storage.universe_conn(slug)
    };
    crate::repository::SqliteEntryRepository::new(conn)
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
// `public/` convention — anon visibility filter (2.7.20)
//
// A universe can opt in to the `public/` convention: anon visitors
// only see entries whose path starts with `public/`. Authenticated
// callers see everything they had access to before. The current
// allowlist is below; future iteration generalizes this to a per-
// universe flag + recursive subuniverse mapping.
//
// CO-268: public-subscribable universes bypass this filter — the
// visibility gate middleware (CO-161) already controls universe-level
// access, so adding a per-path restriction on top is wrong.
// ---------------------------------------------------------------------------

const PUBLIC_CONVENTION_UNIVERSES: &[&str] = &["co"];

fn is_public_convention(slug: &str) -> bool {
    PUBLIC_CONVENTION_UNIVERSES.contains(&slug)
}

fn caller_is_anon(state: &AppState, headers: &HeaderMap) -> bool {
    crate::auth::resolve_user_id(state, headers).is_none()
}

/// CO-54: field-level merge for a `PUT` frontmatter patch (Scenario 1).
///
/// Shallow-merges `patch` over `existing` so a client only overwrites the
/// fields it actually sends — two clients editing *different* fields therefore
/// merge instead of clobbering each other. Semantics:
///
/// * a key present in `patch` overwrites the same key in `existing`
///   (same-field edits resolve last-write-wins — the later PUT lands second);
/// * a key set to explicit JSON `null` in `patch` is *removed* from the result;
/// * keys absent from `patch` are preserved unchanged.
///
/// If either side isn't a JSON object the patch wins wholesale (nothing
/// meaningful to merge).
fn merge_frontmatter(existing: &JsonValue, patch: &JsonValue) -> JsonValue {
    match (existing, patch) {
        (JsonValue::Object(base), JsonValue::Object(over)) => {
            let mut merged = base.clone();
            for (k, v) in over {
                if v.is_null() {
                    merged.remove(k);
                } else {
                    merged.insert(k.clone(), v.clone());
                }
            }
            JsonValue::Object(merged)
        }
        _ => patch.clone(),
    }
}

fn is_public_path(path: &str) -> bool {
    path.starts_with("public/") || path == "public"
}

/// CO-390 spike: thin wrapper — delegates to `EntryService::apply_public_convention_filter`.
fn filter_public_for_anon(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
    universe_is_pub_sub: bool,
    entries: Vec<EntryRow>,
) -> Vec<EntryRow> {
    let is_anon = caller_is_anon(state, headers);
    EntryService::apply_public_convention_filter(entries, is_anon, slug, universe_is_pub_sub)
}

fn is_public_for_anon(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
    universe_is_pub_sub: bool,
    path: &str,
) -> bool {
    // CO-268: public-subscribable universes expose all entries to anon callers.
    if !is_public_convention(slug) || !caller_is_anon(state, headers) || universe_is_pub_sub {
        return true;
    }
    is_public_path(path)
}

/// CO-390 spike: thin wrapper — delegates to `EntryService::apply_published_filter`.
fn filter_published_for_anon(
    is_anon: bool,
    anon_published_only: bool,
    entries: Vec<EntryRow>,
) -> Vec<EntryRow> {
    EntryService::apply_published_filter(entries, is_anon, anon_published_only)
}

/// CO-390 spike: thin wrapper — delegates to `EntryService::apply_review_status_filter`.
fn filter_review_status(entries: Vec<EntryRow>, is_owner: bool, viewer_key: &str) -> Vec<EntryRow> {
    EntryService::apply_review_status_filter(entries, is_owner, viewer_key)
}

/// CO-354: single-entry version of [`filter_review_status`].
fn is_review_visible(frontmatter: &serde_json::Value, is_owner: bool, viewer_key: &str) -> bool {
    if is_owner {
        return true;
    }
    let status = frontmatter
        .get("review_status")
        .and_then(|v| v.as_str())
        .unwrap_or("published");
    if status == "published" {
        return true;
    }
    frontmatter
        .get("submitted_by")
        .and_then(|v| v.as_str())
        .map(|s| s == viewer_key)
        .unwrap_or(false)
}

/// CO-330: single-entry version — returns false when the entry is not visible
/// to anonymous callers because the universe requires `published: true`.
fn is_published_for_anon(
    is_anon: bool,
    anon_published_only: bool,
    frontmatter: &serde_json::Value,
) -> bool {
    if !is_anon || !anon_published_only {
        return true;
    }
    frontmatter
        .get("published")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// `Cache-Control` for anon GETs of stable public seed content.
///
/// Returns Some(`public, max-age=60`) when the caller is anon AND the
/// entry is either in the template universe (welcome / onboarding
/// pages) or under `co::public/*` (transparency cluster). Both
/// surfaces are reseeded from `include_str!` constants — content only
/// changes on deploy. 60s strikes a balance: SPA refreshes hit the
/// browser cache (no 429), but a new deploy is visible within a minute.
///
/// Authenticated callers get None — they may be editing.
fn entry_cache_control(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
    path: &str,
) -> Option<axum::http::HeaderValue> {
    if !caller_is_anon(state, headers) {
        return None;
    }
    let is_template_seed = slug == "template";
    let is_co_public = slug == "co" && is_public_path(path);
    if is_template_seed || is_co_public {
        Some(axum::http::HeaderValue::from_static(
            "public, max-age=60, must-revalidate",
        ))
    } else {
        None
    }
}

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
    // CO-268: also look up the universe's visibility so filter_public_for_anon
    // can bypass the public/ path restriction for public-subscribable universes.
    // CO-330: also fetch anon_published_only for the published-only filter.
    // CO-290: go through the Storage trait instead of direct connection access.
    let universe = state.core.storage_trait.get_universe(&slug);
    let universe_is_pub_sub = universe
        .as_ref()
        .map(|u| u.visibility == "public-subscribable")
        .unwrap_or(false);
    let anon_published_only = universe
        .as_ref()
        .map(|u| u.anon_published_only)
        .unwrap_or(false);

    // CO-266: limit is applied in memory after filter_public_for_anon so that
    // `total` reflects the full visible count and `items` is the paginated slice.
    // Queries that carry their own internal cap (FTS=100, date=5000, semantic=k)
    // are left unchanged; only the user-limit-aware paths pass None here.
    let entries = if let Some(ref sem_query) = q.semantic {
        // CO-164: semantic similarity search (optionally combined with FTS for hybrid).
        // Uses EmbeddingIndex alongside EntryIndex — still needs the raw connection.
        let k = q.k.unwrap_or(10).min(200);
        let uc = state.core.storage_trait.universe_conn(&slug);
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        semantic_search_entries(&state, &slug, &uc_guard, sem_query, k, q.q.as_deref())
            .map_err(|e| AppError::Internal(e.to_string()))?
    } else if let Some(ref prefix) = q.path_prefix {
        // CO-264: folder-prefix filter — fetch all matching; limit applied below.
        state
            .core
            .storage_trait
            .list_entries_by_prefix(&slug, prefix, None)
            .map_err(|e| AppError::Internal(e.to_string()))?
    } else if let Some(ref fts_query) = q.q {
        state
            .core
            .storage_trait
            .search_entries(&slug, fts_query)
            .map_err(|e| AppError::Internal(e.to_string()))?
    } else if let Some(ref semantic) = q.date_semantic {
        // CO-73: date-semantic range query
        state
            .core
            .storage_trait
            .list_entries_by_date(&slug, semantic, q.from.as_deref(), q.to.as_deref())
            .map_err(|e| AppError::Internal(e.to_string()))?
    } else {
        let entry_type = q.entry_type.as_deref().unwrap_or("");
        if entry_type.is_empty() {
            // list all — fetch all; limit applied below.
            state
                .core
                .storage_trait
                .list_entries(&slug, "", &serde_json::json!({}), None)
                .unwrap_or_default()
        } else {
            let filter = q
                .filter
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::json!({}));
            state
                .core
                .storage_trait
                .list_entries(&slug, entry_type, &filter, None)
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
        let state_row = state
            .core
            .storage_trait
            .get_entry(&slug, as_of)
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

        entries
            .into_iter()
            .filter(|e| allowed_paths.contains(e.path.as_str()))
            .map(|mut e| {
                // If the manifest carries a body_hash for this path AND we
                // have the bytes in the CAS blob store, swap the current
                // body for the historical one. Otherwise leave as-is
                // (path-fidelity fallback).
                if let Some(h) = body_hash_by_path.get(e.path.as_str())
                    && let Some(bytes) = state.core.storage_trait.get_blob(h)
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

    // 2.7.20: anon visibility filter — in universes that adopt the
    // `public/` folder convention, anon visitors only see entries
    // under `public/`. Owners/members see everything they always did.
    // The convention is opt-in per universe via the static list below;
    // CO-268 generalizes: public-subscribable universes bypass the filter.
    let entries = filter_public_for_anon(&state, &headers, &slug, universe_is_pub_sub, entries);

    // CO-330: published-only filter — when the universe has anon_published_only=1,
    // anonymous callers only see entries with frontmatter.published == true.
    let is_anon = caller_is_anon(&state, &headers);
    let entries = filter_published_for_anon(is_anon, anon_published_only, entries);

    // CO-354: review-pipeline filter — draft/reviewed entries are hidden from
    // everyone except the universe owner and the original submitter.
    let viewer_key = crate::review_routes::submitter_key(&state, &headers);
    let is_owner = universe
        .as_ref()
        .map(|u| u.owner_id == viewer_key)
        .unwrap_or(false);
    let entries = filter_review_status(entries, is_owner, &viewer_key);

    // CO-266: total = full visible count; entries = paginated slice.
    let effective_limit = q.limit.unwrap_or(5_000).min(50_000);
    let total = entries.len();
    let entries: Vec<EntryRow> = entries.into_iter().take(effective_limit).collect();

    if accept_protobuf(&headers) {
        // Encode as protobuf EntryList
        use prost::Message;
        let proto_entries: Vec<co::proto::entry::Entry> =
            entries.iter().map(entry_row_to_proto).collect();
        let list = co::proto::entry::EntryList {
            entries: proto_entries,
            total: total as u64,
        };
        let mut buf = Vec::new();
        list.encode(&mut buf)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let mut resp = (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/x-protobuf")],
            buf,
        )
            .into_response();
        resp.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
        Ok(resp)
    } else {
        let mut resp = Json(EntryListResponse { entries, total }).into_response();
        resp.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
        Ok(resp)
    }
}

/// GET /api/v1/universes/:slug/entries/popular — top-N entries by event frequency
#[derive(Debug, Deserialize)]
pub struct PopularQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct PopularEntry {
    pub path: String,
    pub title: String,
    pub entry_type: String,
    pub updated_at: Option<String>,
}

pub async fn popular_entries(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<PopularQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = q.limit.unwrap_or(10).min(50);
    let repo = entry_repo(&state, &slug);
    let rows = repo
        .popular(&slug, limit)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let items: Vec<PopularEntry> = rows
        .into_iter()
        .map(|e| PopularEntry {
            path: e.path.clone(),
            title: e.title.clone().unwrap_or_else(|| e.path.clone()),
            entry_type: e.entry_type.clone(),
            updated_at: e.updated_at.clone(),
        })
        .collect();
    let mut resp = Json(items).into_response();
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(resp)
}

/// GET /api/v1/universes/:slug/entries/tags — aggregate tags
pub async fn list_entry_tags(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    // CO-290: go through the Storage trait.
    let tags = state
        .core
        .storage_trait
        .list_entry_tags(&slug)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut resp = Json(tags).into_response();
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(resp)
}

/// GET /api/v1/universes/:slug/entries/tree — hierarchical tree
pub async fn entry_tree(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<TreeQuery>,
) -> Result<impl IntoResponse, AppError> {
    // Visibility gate is enforced by universe_visibility_gate middleware (CO-161).
    // CO-290: go through the Storage trait.
    let entry_type = q.entry_type.as_deref().unwrap_or("page");
    let tree = state
        .core
        .storage_trait
        .entry_tree(&slug, entry_type)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut resp = Json(tree).into_response();
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(resp)
}

/// Query params for single-entry GET: `?excerpt=true` returns frontmatter + 200-char excerpt only.
#[derive(Debug, Deserialize)]
pub struct GetEntryQuery {
    pub excerpt: Option<bool>,
    /// CO-75: reconstruct the entry as it was at this RFC3339 instant, replaying
    /// the CO-54 version history. Returns a [`ReconstructedEntry`] view.
    pub as_of: Option<String>,
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
    // CO-268: also look up the universe's visibility so is_public_for_anon
    // can bypass the public/ path restriction for public-subscribable universes.
    // CO-330: also fetch anon_published_only for the published-only filter.
    // CO-290: go through the Storage trait instead of direct connection access.
    let universe = state.core.storage_trait.get_universe(&slug);
    let universe_is_pub_sub = universe
        .as_ref()
        .map(|u| u.visibility == "public-subscribable")
        .unwrap_or(false);
    let anon_published_only = universe
        .as_ref()
        .map(|u| u.anon_published_only)
        .unwrap_or(false);

    let entry = state
        .core
        .storage_trait
        .get_entry(&slug, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{}' not found", path)))?;

    // 2.7.20: anon visibility filter — universes that adopt the
    // `public/` convention only expose entries under `public/` to
    // anon visitors. 404 mimics the not-found shape so we don't
    // leak the existence of private paths.
    // CO-268: public-subscribable universes bypass this per-path filter.
    if !is_public_for_anon(&state, &headers, &slug, universe_is_pub_sub, &path) {
        return Err(AppError::NotFound(format!("Entry '{}' not found", path)));
    }

    // CO-330: published-only filter — anon callers cannot read unpublished entries.
    let is_anon = caller_is_anon(&state, &headers);
    if !is_published_for_anon(is_anon, anon_published_only, &entry.frontmatter) {
        return Err(AppError::NotFound(format!("Entry '{}' not found", path)));
    }

    // CO-354: review-pipeline filter — a draft/reviewed entry is visible only to
    // the universe owner and its submitter; everyone else gets a 404.
    let viewer_key = crate::review_routes::submitter_key(&state, &headers);
    let is_owner = universe
        .as_ref()
        .map(|u| u.owner_id == viewer_key)
        .unwrap_or(false);
    if !is_review_visible(&entry.frontmatter, is_owner, &viewer_key) {
        return Err(AppError::NotFound(format!("Entry '{}' not found", path)));
    }

    // CO-75: ?as_of=<RFC3339> — reconstruct the entry as it was at a past
    // instant by replaying the CO-54 version history. Visibility is already
    // enforced above, so historical reads honour the same access rules.
    if let Some(ref as_of_raw) = q.as_of {
        let as_of = chrono::DateTime::parse_from_rfc3339(as_of_raw)
            .map_err(|_| {
                AppError::BadRequest(format!("Invalid 'as_of' RFC3339 timestamp: '{as_of_raw}'"))
            })?
            .with_timezone(&chrono::Utc);
        let versions = {
            let storage = lock_storage(&state);
            storage
                .list_entry_versions(&slug, &path, 1000)
                .map_err(|e| AppError::Internal(e.to_string()))?
        };
        let state_at = crate::content::versioning::reconstruct::reconstruct_at(
            &versions,
            &entry.frontmatter,
            &entry.body,
            as_of,
        );
        let mut resp = Json(serde_json::json!({
            "path": path,
            "as_of": as_of_raw,
            "version": state_at.version,
            "source_timestamp": state_at.source_timestamp,
            "is_current": state_at.is_current,
            "frontmatter": state_at.frontmatter,
            "body": state_at.body,
        }))
        .into_response();
        resp.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
        return Ok(resp);
    }

    // CO-150: ?excerpt=true — fast path for board cards; returns frontmatter + 200-char excerpt only.
    if q.excerpt.unwrap_or(false) {
        let excerpt: String = entry.body.chars().take(200).collect();
        return Ok(Json(EntryExcerpt {
            frontmatter: entry.frontmatter,
            excerpt,
        })
        .into_response());
    }

    // 2.7.21: cache header for public seed content. Anon GETs of the
    // template universe and `co::public/*` are stable enough to cache
    // 60s in the browser — covers the SPA's per-page-load entry
    // fanout without hitting the rate limiter on every refresh.
    let cache_header = entry_cache_control(&state, &headers, &slug, &path);

    if accept_protobuf(&headers) {
        use prost::Message;
        let proto = entry_row_to_proto(&entry);
        let mut buf = Vec::new();
        proto
            .encode(&mut buf)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let mut resp = (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/x-protobuf")],
            buf,
        )
            .into_response();
        if let Some(cc) = cache_header {
            resp.headers_mut()
                .insert(axum::http::header::CACHE_CONTROL, cc);
        }
        Ok(resp)
    } else {
        // CO-74: include outbound FK relations in entry detail for board relation-aware views.
        let relations = {
            use crate::repository::RelationRepository;
            let repo = crate::repository::SqliteRelationRepository::new(
                state.core.storage_trait.universe_conn(&slug),
            );
            repo.outbound(&slug, &path)
                .unwrap_or_default()
                .iter()
                .map(crate::mapper::RelationMapper::domain_to_row)
                .collect()
        };
        let mut resp = Json(EntryWithRelations { entry, relations }).into_response();
        if let Some(cc) = cache_header {
            resp.headers_mut()
                .insert(axum::http::header::CACHE_CONTROL, cc);
        }
        Ok(resp)
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
        let storage = lock_storage(&state);
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

        // CO-383: reject writes on event-bus-backed universes (e.g. yggdrasil notes).
        // CO-390 spike: delegated to EntryService::check_not_event_bus.
        EntryService::check_not_event_bus(
            universe.source_kind.as_deref(),
            universe.source_url.clone(),
        )?;

        // CO-80: quota check — anonymous usage gate or tier-based storage quota.
        // CO-390 spike: anon quota rule delegated to EntryService::check_anon_quota.
        if let Some((uid, tier)) = crate::rate_limit::extract_auth_identity(&headers) {
            crate::rate_limit::check_storage_quota(&storage, &uid, tier, &headers)?;
        } else if universe.owner_id.starts_with("anon-") {
            EntryService::check_anon_quota(universe.content_count)?;
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
        state.index.cache.invalidate_universe(&slug);
    }

    // Index into universe data.db — entries row + dates + relations +
    // references_meta in one lock scope (CO-432: via the entry repository).
    let relation_count = entry_repo(&state, &slug)
        .index_entry_create(
            &slug,
            &entry,
            &body.frontmatter,
            &body.body,
            body.frontmatter.get("title").and_then(|v| v.as_str()),
            manifest_arc.as_deref(),
            &universe_root,
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    // CO-220: route entry write through the event bus so embedding + other
    // workers subscribe without coupling to entry_routes directly.
    state
        .core
        .event_bus
        .publish(crate::events::DomainEvent::EntryWritten {
            universe_key: slug.clone(),
            path: body.path.clone(),
            body: body.body.clone(),
            body_hash: entry.body_hash.clone(),
        });

    // CO-380: also publish to EDA bus for universal observability.
    state.core.eda_bus.publish(crate::eda::Event::new(
        "entry.created",
        Some(slug.clone()),
        crate::auth::resolve_user_id(&state, &headers),
        serde_json::json!({ "path": body.path, "entry_type": entry.entry_type }),
        crate::eda::Visibility::UniverseMembers,
    ));

    // CO-367: non-blocking KB ingest — fires async, never blocks the write response.
    crate::kb_routes::fire_kb_ingest(
        &state,
        &slug,
        &body.path,
        &entry.body_hash,
        &entry.entry_type,
        &body.body,
        &body.frontmatter,
        &chrono::Utc::now().to_rfc3339(),
    );

    // CO-79: invalidate query cache entries for this universe after a write.
    state
        .index
        .cache
        .query
        .invalidate_prefix(&format!("{slug}:"));

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
    let mut storage = lock_storage(&state);
    storage.increment_universe_content_count(&slug);

    // CO-45: log mutation on UAT before body values are moved into the response
    if state.core.config.is_uat() {
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
            _score: None,
        }),
    )
        .into_response())
}

/// PUT /api/v1/universes/:slug/entries/*path — update entry
pub async fn update_entry(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpdateEntryBody>,
) -> Result<axum::response::Response, AppError> {
    let universe_root = {
        let storage = lock_storage(&state);
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
        // CO-383: reject writes on event-bus-backed universes.
        // CO-390 spike: delegated to EntryService::check_not_event_bus.
        EntryService::check_not_event_bus(
            universe.source_kind.as_deref(),
            universe.source_url.clone(),
        )?;
        storage.universe_root(&slug)
    };

    // Read existing entry from universe data.db
    let existing = entry_repo(&state, &slug)
        .get(&slug, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{}' not found", path)))?;

    // CO-54: field-level merge — PUT is a partial patch. A frontmatter field is
    // only overwritten when the caller actually sends it, so two clients editing
    // *different* fields merge instead of clobbering each other (Scenario 1).
    let new_fm = match body.frontmatter {
        Some(patch) => merge_frontmatter(&existing.frontmatter, &patch),
        None => existing.frontmatter.clone(),
    };
    let new_body = body.body.unwrap_or_else(|| existing.body.clone());

    // CO-54: idempotency — re-applying identical content is a no-op. No version
    // bump, no disk write, no event storm (Scenario 3: co auto setting
    // status:done when already done converges silently).
    if new_body == existing.body && new_fm == existing.frontmatter {
        return Ok(Json(existing).into_response());
    }

    // CO-128: optimistic-concurrency check. When the client sent the
    // `base_hash` its edit was based on and the stored entry has since
    // diverged, reject with `409 Conflict` carrying both versions so the SPA
    // can render the conflict-resolution modal (Ignore / Replace / Keep both).
    if let Some(ref base) = body.base_hash
        && base != &existing.body_hash
    {
        let local_entry = make_entry(&path, new_fm.clone(), &new_body);
        return Ok(conflict_response(
            &slug,
            &path,
            &new_body,
            &local_entry.body_hash,
            &existing,
            base,
        ));
    }

    // CO-79: load manifest once from L1 cache (singleflight stampede protection).
    let manifest_arc = load_manifest_cached(&state, &slug, &universe_root).await;

    // CO-71: validate merged frontmatter against manifest schema before writing.
    let entry_type_for_validation = new_fm
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    validate_against_manifest(manifest_arc.as_deref(), entry_type_for_validation, &new_fm)?;

    // CO-54: snapshot the *previous* version before overwriting it on disk.
    // Storing prior content durably first means a crash mid-write can never lose
    // committed data, and backs GET …/entries/versions for manual recovery.
    {
        let actor = crate::auth::resolve_user_id(&state, &headers);
        let prev_fm_json = serde_json::to_string(&existing.frontmatter).unwrap_or_default();
        let storage = lock_storage(&state);
        if let Err(e) = storage.save_entry_version(
            &slug,
            &path,
            &existing.body,
            &prev_fm_json,
            actor.as_deref(),
        ) {
            tracing::warn!("entry_versions snapshot failed for {slug}/{path}: {e}");
        }
    }

    let entry = make_entry(&path, new_fm.clone(), &new_body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    // If the manifest file itself was written, invalidate the L1 cache.
    if path == co::manifest::MANIFEST_FILENAME {
        state.index.cache.invalidate_universe(&slug);
    }

    // Re-index — prev-hash capture, entries row, event log, dates, relations,
    // references_meta in one lock scope (CO-432: via the entry repository).
    entry_repo(&state, &slug)
        .index_entry_update(
            &slug,
            &path,
            &entry,
            &new_fm,
            &new_body,
            manifest_arc.as_deref(),
            &universe_root,
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    // CO-220: route entry update through the event bus.
    state
        .core
        .event_bus
        .publish(crate::events::DomainEvent::EntryWritten {
            universe_key: slug.clone(),
            path: path.clone(),
            body: new_body.clone(),
            body_hash: entry.body_hash.clone(),
        });

    // CO-380: also publish to EDA bus.
    let old_status = existing
        .frontmatter
        .get("status")
        .and_then(|v| v.as_str())
        .map(String::from);
    let new_status = new_fm
        .get("status")
        .and_then(|v| v.as_str())
        .map(String::from);
    state.core.eda_bus.publish(crate::eda::Event::new(
        "entry.updated",
        Some(slug.clone()),
        None,
        serde_json::json!({
            "path": path,
            "entry_type": entry.entry_type,
            "status": new_status,
        }),
        crate::eda::Visibility::UniverseMembers,
    ));
    // CO-398: publish task.status_changed when the status field transitions.
    if old_status != new_status
        && let Some(ref to) = new_status
    {
        let title = new_fm
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        state.core.eda_bus.publish(crate::eda::Event::new(
            "task.status_changed",
            Some(slug.clone()),
            None,
            serde_json::json!({
                "path": path,
                "title": title,
                "from": old_status,
                "to": to,
                "trigger": "manual",
            }),
            crate::eda::Visibility::UniverseMembers,
        ));
    }

    // CO-367: non-blocking KB ingest — fires async, never blocks the write response.
    crate::kb_routes::fire_kb_ingest(
        &state,
        &slug,
        &path,
        &entry.body_hash,
        &entry.entry_type,
        &new_body,
        &new_fm,
        &chrono::Utc::now().to_rfc3339(),
    );

    // CO-79: invalidate query cache entries for this universe after a write.
    state
        .index
        .cache
        .query
        .invalidate_prefix(&format!("{slug}:"));

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

    let storage = lock_storage(&state);
    // CO-45: log mutation on UAT
    if state.core.config.is_uat() {
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
        _score: None,
    })
    .into_response())
}

/// CO-128: maximum body size (per side) embedded in a conflict payload.
/// Larger bodies are truncated with a `truncated` flag so the modal shows a
/// summary instead of choking the diff renderer (spec risk: diff perf).
const CONFLICT_BODY_CAP: usize = 100 * 1024;

/// Truncate a body to [`CONFLICT_BODY_CAP`], returning `(text, was_truncated)`.
fn cap_conflict_body(body: &str) -> (String, bool) {
    if body.len() <= CONFLICT_BODY_CAP {
        (body.to_string(), false)
    } else {
        // Truncate on a char boundary at or below the cap.
        let mut end = CONFLICT_BODY_CAP;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        (body[..end].to_string(), true)
    }
}

/// CO-128: build the `409 Conflict` response carrying both divergent versions.
///
/// Shape (`ConflictPayload`):
/// ```json
/// {
///   "error": "conflict",
///   "conflict": {
///     "universe_key": "...", "path": "...", "kind": "both_modified",
///     "local":  { "body": "...", "body_hash": "...", "truncated": false },
///     "remote": { "body": "...", "body_hash": "...", "truncated": false },
///     "base":   { "body_hash": "..." }
///   }
/// }
/// ```
fn conflict_response(
    universe_key: &str,
    path: &str,
    local_body: &str,
    local_hash: &str,
    remote: &EntryRow,
    base_hash: &str,
) -> axum::response::Response {
    let (local_text, local_trunc) = cap_conflict_body(local_body);
    let (remote_text, remote_trunc) = cap_conflict_body(&remote.body);
    let payload = serde_json::json!({
        "error": "conflict",
        "conflict": {
            "universe_key": universe_key,
            "path": path,
            "kind": "both_modified",
            "local":  { "body": local_text,  "body_hash": local_hash,        "truncated": local_trunc },
            "remote": { "body": remote_text, "body_hash": remote.body_hash,  "truncated": remote_trunc },
            "base":   { "body_hash": base_hash },
        }
    });
    (StatusCode::CONFLICT, Json(payload)).into_response()
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
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
        // CO-383: reject writes on event-bus-backed universes.
        // CO-390 spike: delegated to EntryService::check_not_event_bus.
        EntryService::check_not_event_bus(
            universe.source_kind.as_deref(),
            universe.source_url.clone(),
        )?;
        storage.universe_root(&slug)
    };

    // CO-45: capture before_value on UAT before deletion
    let before_val = if state.core.config.is_uat() {
        entry_repo(&state, &slug)
            .get(&slug, &path)
            .ok()
            .flatten()
            .and_then(|e| serde_json::to_string(&e).ok())
    } else {
        None
    };

    co::delete_entry(&universe_root, &path).map_err(|e| AppError::Internal(e.to_string()))?;

    // Remove every projection — entries row, dates, relations, references_meta,
    // embedding — in one lock scope (CO-432: via the entry repository).
    entry_repo(&state, &slug)
        .unindex_entry(&slug, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    // CO-220: route entry delete through the event bus.
    state
        .core
        .event_bus
        .publish(crate::events::DomainEvent::EntryDeleted {
            universe_key: slug.clone(),
            path: path.clone(),
        });
    // CO-380: also publish to EDA bus.
    state.core.eda_bus.publish(crate::eda::Event::new(
        "entry.deleted",
        Some(slug.clone()),
        None,
        serde_json::json!({ "path": path }),
        crate::eda::Visibility::UniverseMembers,
    ));
    let mut storage = lock_storage(&state);
    storage.decrement_universe_content_count(&slug, 1);

    // CO-45: log mutation on UAT
    if state.core.config.is_uat() {
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
    let dsl_query = crate::query_dsl::parse(&params.q)
        .map_err(|e| AppError::BadRequest(format!("query parse error: {e}")))?;

    // CO-325: expand category names (e.g. "music" → ["song", "album"]).
    let categories = crate::query_dsl::default_type_categories();
    let dsl_query = crate::query_dsl::resolve(dsl_query, &categories);

    let (sql, sql_params) = crate::query_dsl::compile(&dsl_query, &slug);

    let params_refs: Vec<&dyn rusqlite::types::ToSql> = sql_params
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();

    let entries = entry_repo(&state, &slug)
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
    use crate::repository::ReferenceRepository;
    let conn = {
        let storage = lock_storage(&state);
        storage.universe_conn(&slug)
    };
    let references = crate::repository::SqliteReferenceRepository::new(conn)
        .query_refs(
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
    use crate::repository::ReferenceRepository;
    let conn = {
        let storage = lock_storage(&state);
        storage.universe_conn(&slug)
    };
    let orphans = crate::repository::SqliteReferenceRepository::new(conn)
        .orphan_wikilinks(&slug)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(orphans))
}

// ---------------------------------------------------------------------------
// CO-164: Semantic search helpers + `/similar` endpoint
// ---------------------------------------------------------------------------

/// Run semantic (and optionally hybrid) search, returning top-K `EntryRow`s with `_score`.
///
/// When `fts_query` is also provided, results are merged using the harmonic mean of the
/// normalised FTS rank and cosine similarity score (hybrid search).
pub(crate) fn semantic_search_entries(
    state: &AppState,
    universe_key: &str,
    conn: &rusqlite::Connection,
    sem_query: &str,
    k: usize,
    fts_query: Option<&str>,
) -> anyhow::Result<Vec<EntryRow>> {
    let query_embedding = match state.index.embeddings.embed_one(sem_query) {
        Some(e) => e,
        None => return Ok(vec![]), // model unavailable
    };

    let emb_idx = crate::embedding_index::EmbeddingIndex::new(conn);
    let scored_paths = emb_idx.search_similar(universe_key, &query_embedding, k, None)?;

    let fts_rank: std::collections::HashMap<String, f32> = if let Some(fts_q) = fts_query {
        let fts_results = crate::entry_index::EntryIndex::new(conn).search(universe_key, fts_q)?;
        let n = fts_results.len().max(1) as f32;
        fts_results
            .iter()
            .enumerate()
            .map(|(i, e)| (e.path.clone(), 1.0 - (i as f32 / n)))
            .collect()
    } else {
        std::collections::HashMap::new()
    };

    // Merge: if fts_query present use harmonic mean, else pure cosine score.
    let mut merged: Vec<(String, f32)> = scored_paths
        .into_iter()
        .map(|(path, cos)| {
            let score = if let Some(&fts) = fts_rank.get(&path) {
                if fts > 0.0 && cos > 0.0 {
                    2.0 * fts * cos / (fts + cos)
                } else {
                    fts.max(cos)
                }
            } else {
                cos
            };
            (path, score)
        })
        .collect();

    // Include FTS-only hits not in semantic results when hybrid
    if fts_query.is_some() {
        for (path, fts) in &fts_rank {
            if !merged.iter().any(|(p, _)| p == path) {
                merged.push((path.clone(), *fts * 0.5)); // penalise pure-FTS hits
            }
        }
    }

    merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    merged.truncate(k);

    // Fetch full EntryRow for each path, inject score.
    let entry_idx = crate::entry_index::EntryIndex::new(conn);
    let mut results = Vec::with_capacity(merged.len());
    for (path, score) in merged {
        if let Ok(Some(mut row)) = entry_idx.get(universe_key, &path) {
            row._score = Some(score);
            results.push(row);
        }
    }
    Ok(results)
}

/// GET /api/v1/universes/:slug/entries/similar?path=<vault-path>&k=10
///
/// Returns entries most similar to the given one (excluding itself).
/// Uses `?path=` query param rather than a URL segment to avoid axum catch-all
/// routing conflicts.
pub async fn similar_entries(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<SimilarQuery>,
) -> Result<Json<EntryListResponse>, AppError> {
    let k = q.k.unwrap_or(10).min(200);
    let path = q.path;
    let uc = {
        let storage = lock_storage(&state);
        storage.universe_conn(&slug)
    };
    // EmbeddingIndex needs the raw connection; scope its lock so the entry
    // repository can take the same connection afterwards (CO-432).
    let scored_paths = {
        let conn = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let emb_idx = crate::embedding_index::EmbeddingIndex::new(&conn);
        let query_embedding = match emb_idx.get_embedding(&slug, &path)? {
            Some(e) => e,
            None => {
                return Ok(Json(EntryListResponse {
                    entries: vec![],
                    total: 0,
                }));
            }
        };
        emb_idx.search_similar(&slug, &query_embedding, k, Some(&path))?
    };

    let repo = crate::repository::SqliteEntryRepository::new(uc);
    let mut results = Vec::with_capacity(scored_paths.len());
    for (p, score) in scored_paths {
        if let Ok(Some(mut row)) = repo.get(&slug, &p) {
            row._score = Some(score);
            results.push(row);
        }
    }
    let total = results.len();
    Ok(Json(EntryListResponse {
        entries: results,
        total,
    }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// `GET /api/v1/universes/{slug}/entries/history?path=<entry-path>&limit=<N>`
///
/// Returns the per-entry transaction log, newest first. Each row maps to
/// the future `co.v1.EntryEvent` protobuf (see
/// `co::public/transaction-log.md` for the lakehouse trajectory).
///
/// Public — any caller who can read the entry can read its history.
/// Future iteration may add audit-tier filtering (hide author_id from
/// anon callers).
#[derive(Debug, serde::Deserialize)]
pub struct EntryHistoryQuery {
    pub path: String,
    pub limit: Option<usize>,
}

pub async fn entry_history_handler(
    State(state): State<AppState>,
    axum::extract::Path(slug): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<EntryHistoryQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = q.limit.unwrap_or(100).min(1000);
    let events = entry_repo(&state, &slug)
        .list_events_for_path(&q.path, limit)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let total = events.len();
    Ok(Json(EntryHistoryResponse {
        path: q.path,
        events,
        total,
    }))
}

// ---------------------------------------------------------------------------
// CO-54: entry version history (audit trail / manual recovery)
// ---------------------------------------------------------------------------

/// Typed response for `GET /:slug/entries/versions`.
#[derive(Debug, Serialize)]
pub struct EntryVersionsResponse {
    pub path: String,
    pub versions: Vec<crate::storage::entry_versions::EntryVersionRow>,
    pub total: usize,
}

/// Query params for the versions endpoint. Mirrors `EntryHistoryQuery` — the
/// entry path is a query arg (not a path segment) so it doesn't collide with
/// the greedy `entries/{*path}` wildcard route.
#[derive(Debug, serde::Deserialize)]
pub struct EntryVersionsQuery {
    pub path: String,
    pub limit: Option<usize>,
}

/// GET /api/v1/universes/:slug/entries/versions?path=…
///
/// Returns the pre-overwrite snapshot history for an entry, newest first. Each
/// version carries `actor`, `timestamp`, `hash`, and the full prior content so a
/// caller can manually recover any earlier revision.
pub async fn entry_versions_handler(
    State(state): State<AppState>,
    axum::extract::Path(slug): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<EntryVersionsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = q.limit.unwrap_or(50).min(1000);
    let versions = {
        let storage = lock_storage(&state);
        storage
            .list_entry_versions(&slug, &q.path, limit)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    let total = versions.len();
    Ok(Json(EntryVersionsResponse {
        path: q.path,
        versions,
        total,
    }))
}

// ---------------------------------------------------------------------------
// CO-75: version reconstruction — per-entry diff + universe auto-changelog
// ---------------------------------------------------------------------------

/// Query params for the per-entry diff endpoint. `path` is a query arg (not a
/// path segment) to avoid the greedy `entries/{*path}` wildcard, matching the
/// `history`/`versions` convention.
#[derive(Debug, serde::Deserialize)]
pub struct EntryDiffQuery {
    pub path: String,
    /// RFC3339 lower bound of the interval.
    pub from: String,
    /// RFC3339 upper bound of the interval.
    pub to: String,
}

/// GET /api/v1/universes/:slug/entries/diff?path=…&from=<T1>&to=<T2>
///
/// Reconstructs the entry at `from` and at `to` from the CO-54 version history
/// and returns the op-level diff (changed frontmatter fields + body change)
/// over that interval.
pub async fn entry_diff_handler(
    State(state): State<AppState>,
    axum::extract::Path(slug): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<EntryDiffQuery>,
) -> Result<impl IntoResponse, AppError> {
    let from = parse_rfc3339(&q.from, "from")?;
    let to = parse_rfc3339(&q.to, "to")?;

    // Current live state (may be absent if the entry was deleted/never existed).
    let current = state
        .core
        .storage_trait
        .get_entry(&slug, &q.path)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let (cur_fm, cur_body) = match &current {
        Some(e) => (e.frontmatter.clone(), e.body.clone()),
        None => (serde_json::json!({}), String::new()),
    };

    let versions = {
        let storage = lock_storage(&state);
        storage
            .list_entry_versions(&slug, &q.path, 1000)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };

    use crate::content::versioning::reconstruct;
    let before = reconstruct::reconstruct_at(&versions, &cur_fm, &cur_body, from);
    let after = reconstruct::reconstruct_at(&versions, &cur_fm, &cur_body, to);
    let diff = reconstruct::diff_states(&q.path, &q.from, &q.to, &before, &after);
    Ok(Json(diff))
}

/// Query params for the universe changelog endpoint.
#[derive(Debug, serde::Deserialize)]
pub struct ChangelogQuery {
    /// RFC3339 lower bound (inclusive). Defaults to the beginning of time.
    pub since: Option<String>,
    /// RFC3339 upper bound (inclusive). Defaults to now.
    pub until: Option<String>,
    /// Max ops to aggregate. Defaults to 10000; capped at 100000.
    pub limit: Option<usize>,
}

/// GET /api/v1/universes/:slug/changelog?since=<T>&until=<T>
///
/// Aggregates the `entry_events` op log over `[since, until]` into a
/// Keep-a-Changelog document, classifying each op (Added/Changed/Removed) and
/// rendering its line via the content type's manifest `changelog_summary`
/// template. Reconstructs without any manual maintenance.
pub async fn changelog_handler(
    State(state): State<AppState>,
    axum::extract::Path(slug): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<ChangelogQuery>,
) -> Result<impl IntoResponse, AppError> {
    let since_micros = match &q.since {
        Some(s) => parse_rfc3339(s, "since")?.timestamp_micros(),
        None => i64::MIN,
    };
    let until_micros = match &q.until {
        Some(s) => parse_rfc3339(s, "until")?.timestamp_micros(),
        None => i64::MAX,
    };
    let limit = q.limit.unwrap_or(10_000).min(100_000) as i64;

    // Resolve the universe (404 if missing) + its connection + root for the manifest.
    let (conn, universe_root) = {
        let storage = lock_storage(&state);
        if storage.get_universe(&slug).is_none() {
            return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
        }
        (storage.universe_conn(&slug), storage.universe_root(&slug))
    };

    use crate::content::versioning::reconstruct::ChangeEvent;
    let events: Vec<ChangeEvent> = {
        let guard = conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let mut stmt = guard
            .prepare(
                "SELECT path, op, ts_micros, prev_body_hash, frontmatter_json \
                 FROM entry_events \
                 WHERE ts_micros >= ?1 AND ts_micros <= ?2 \
                 ORDER BY ts_micros ASC, seq ASC \
                 LIMIT ?3",
            )
            .map_err(|e| AppError::Internal(format!("changelog prepare: {e}")))?;
        stmt.query_map(
            rusqlite::params![since_micros, until_micros, limit],
            |row| {
                Ok(ChangeEvent {
                    path: row.get(0)?,
                    op: row.get(1)?,
                    ts_micros: row.get(2)?,
                    prev_body_hash: row.get(3)?,
                    frontmatter_json: row.get(4)?,
                })
            },
        )
        .map_err(|e| AppError::Internal(format!("changelog query: {e}")))?
        .filter_map(|r| r.ok())
        .collect()
    };

    let manifest = load_manifest_cached(&state, &slug, &universe_root).await;
    let changelog = crate::content::versioning::reconstruct::build_changelog(
        &events,
        manifest.as_deref(),
        q.since.clone(),
        q.until.clone(),
    );
    Ok(Json(changelog))
}

/// Parse an RFC3339 timestamp query param, mapping failure to a 400.
fn parse_rfc3339(raw: &str, field: &str) -> Result<chrono::DateTime<chrono::Utc>, AppError> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|_| AppError::BadRequest(format!("Invalid '{field}' RFC3339 timestamp: '{raw}'")))
}

// ---------------------------------------------------------------------------
// CO-272: dev-tasks — entries-as-tasks for the kanban view
// ---------------------------------------------------------------------------

/// Query params for `GET /{slug}/dev-tasks`.
#[derive(Debug, Deserialize)]
pub struct DevTasksQuery {
    /// Optional status filter (e.g. `?status=done`).
    pub status: Option<String>,
    /// Max tasks to return. Defaults to 500; capped at 5 000.
    pub limit: Option<usize>,
}

/// A single entry mapped to the task shape expected by the kanban.
#[derive(Debug, Serialize)]
pub struct DevTask {
    /// Ticket key synthesized from path: `work/co/CO-261.md` → `CO-261`.
    pub key: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub task_type: String,
    pub description: String,
    pub path: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DevTasksResponse {
    pub tasks: Vec<DevTask>,
    pub total: usize,
}

/// Synthesize a ticket key from a vault-relative path.
/// `work/co/CO-261.md` → `CO-261`; `work/artelonga/AL-50.md` → `AL-50`.
fn key_from_path(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_string()
}

/// GET /api/v1/universes/:slug/dev-tasks
///
/// Returns entries from `work/` that have entry_type in ('user-story', 'task',
/// 'epic'), mapped to a flat task shape so the kanban can render them without
/// knowing about the entry model.  Anonymous callers can reach this endpoint on
/// public-subscribable universes (visibility gate is already applied by the
/// universe_visibility_gate middleware).
pub async fn list_dev_tasks(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<DevTasksQuery>,
) -> Result<impl IntoResponse, AppError> {
    let repo = entry_repo(&state, &slug);
    let limit = q.limit.unwrap_or(500).min(5_000);
    // CO-346: search both `work/` (vault-pushed files) and `public/` (boot-time
    // seed via CO-262) so the kanban board is never empty regardless of which
    // path convention was used to populate the universe.
    let mut all_entries = repo
        .query_by_path_prefix(&slug, "work/", Some(limit))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let public_entries = repo
        .query_by_path_prefix(&slug, "public/", Some(limit))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    all_entries.extend(public_entries);

    const DEV_TASK_TYPES: &[&str] = &["user-story", "task", "epic"];

    let tasks: Vec<DevTask> = all_entries
        .into_iter()
        .filter(|e| DEV_TASK_TYPES.contains(&e.entry_type.as_str()))
        .filter_map(|e| {
            let status = e
                .frontmatter
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("todo")
                .to_string();

            if q.status.as_deref().is_some_and(|f| f != status) {
                return None;
            }

            Some(DevTask {
                key: key_from_path(&e.path),
                title: e.title.clone().unwrap_or_else(|| key_from_path(&e.path)),
                status,
                priority: e
                    .frontmatter
                    .get("priority")
                    .and_then(|v| v.as_str())
                    .unwrap_or("medium")
                    .to_string(),
                task_type: e
                    .frontmatter
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&e.entry_type)
                    .to_string(),
                description: e.body,
                path: e.path,
                created_at: e.created_at,
                updated_at: e.updated_at,
            })
        })
        .collect();

    let total = tasks.len();
    let mut resp = Json(DevTasksResponse { tasks, total }).into_response();
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(resp)
}

pub fn router() -> Router<AppState> {
    Router::new()
        // Specific paths must come before wildcard
        .route("/{slug}/manifest", get(get_manifest))
        // CO-74: query DSL endpoint
        .route("/{slug}/query", get(query_handler))
        .route("/{slug}/entries/popular", get(popular_entries))
        .route("/{slug}/entries/tags", get(list_entry_tags))
        .route("/{slug}/entries/tree", get(entry_tree))
        // CO-272: entries-as-tasks for the kanban dogfooding loop
        .route("/{slug}/dev-tasks", get(list_dev_tasks))
        .route("/{slug}/entries", get(list_entries).post(create_entry))
        // CO-164: similar entries — uses ?path= query param to avoid catch-all conflict
        .route("/{slug}/entries/similar", get(similar_entries))
        // 2.7.25: per-entry transaction log — newest first. Uses ?path= to
        // avoid catching the `/entries/{*path}` wildcard.
        .route("/{slug}/entries/history", get(entry_history_handler))
        // CO-54: entry version history (pre-overwrite snapshots). Uses ?path= to
        // avoid catching the `/entries/{*path}` wildcard, same as /history.
        .route("/{slug}/entries/versions", get(entry_versions_handler))
        // CO-75: per-entry op-level diff between two instants. ?path= for the
        // same wildcard-avoidance reason as /history and /versions.
        .route("/{slug}/entries/diff", get(entry_diff_handler))
        // CO-75: auto-generated Keep-a-Changelog for the whole universe.
        .route("/{slug}/changelog", get(changelog_handler))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// CO-54: field-level merge — different fields combine; explicit null
    /// deletes; absent keys are preserved; same key overwrites.
    #[test]
    fn test_merge_frontmatter() {
        let existing = serde_json::json!({"type": "task", "title": "Orig", "desc": "d"});

        // Editing only `title` preserves `type` and `desc`.
        let merged = merge_frontmatter(&existing, &serde_json::json!({"title": "New"}));
        assert_eq!(merged["title"], "New");
        assert_eq!(merged["type"], "task");
        assert_eq!(merged["desc"], "d");

        // Explicit null removes the key.
        let merged = merge_frontmatter(&existing, &serde_json::json!({"desc": null}));
        assert!(merged.get("desc").is_none());
        assert_eq!(merged["title"], "Orig");

        // Non-object patch wins wholesale.
        let merged = merge_frontmatter(&existing, &serde_json::json!("scalar"));
        assert_eq!(merged, serde_json::json!("scalar"));
    }

    /// key_from_path strips directory prefix and .md extension.
    #[test]
    fn test_key_from_path() {
        assert_eq!(key_from_path("work/co/CO-261.md"), "CO-261");
        assert_eq!(key_from_path("work/artelonga/AL-50.md"), "AL-50");
        assert_eq!(key_from_path("work/qb/QB-23.md"), "QB-23");
        assert_eq!(key_from_path("CO-1.md"), "CO-1");
    }

    /// DevTasksResponse serializes with the expected shape.
    #[test]
    fn test_dev_tasks_response_serializes() {
        let resp = DevTasksResponse {
            tasks: vec![DevTask {
                key: "CO-272".to_string(),
                title: "Kanban shows dev tasks".to_string(),
                status: "done".to_string(),
                priority: "critical".to_string(),
                task_type: "feat".to_string(),
                description: "body text".to_string(),
                path: "work/co/CO-272.md".to_string(),
                created_at: None,
                updated_at: None,
            }],
            total: 1,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["total"], 1);
        assert_eq!(json["tasks"][0]["key"], "CO-272");
        assert_eq!(json["tasks"][0]["status"], "done");
    }

    /// EntryHistoryResponse serializes with the expected shape.
    #[test]
    fn test_entry_history_response_serializes() {
        let resp = EntryHistoryResponse {
            path: "projects/CO/1.md".to_string(),
            events: vec![],
            total: 0,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["path"], "projects/CO/1.md");
        assert_eq!(json["total"], 0);
        assert!(json["events"].is_array());
    }

    // CO-330: anon published-only filter tests
    fn make_entry_row(path: &str, published: Option<bool>) -> EntryRow {
        let fm = match published {
            Some(v) => serde_json::json!({ "published": v }),
            None => serde_json::json!({}),
        };
        EntryRow {
            path: path.to_string(),
            universe_key: "test".to_string(),
            entry_type: "page".to_string(),
            title: None,
            frontmatter: fm,
            body: String::new(),
            body_hash: String::new(),
            created_at: None,
            updated_at: None,
            _score: None,
        }
    }

    #[test]
    fn test_filter_published_for_anon_passthrough_when_disabled() {
        let entries = vec![
            make_entry_row("a.md", Some(true)),
            make_entry_row("b.md", Some(false)),
            make_entry_row("c.md", None),
        ];
        // anon_published_only=false → no filtering regardless of is_anon
        let result = filter_published_for_anon(true, false, entries.clone());
        assert_eq!(result.len(), 3);
        // authenticated caller → no filtering even when flag is set
        let result2 = filter_published_for_anon(false, true, entries);
        assert_eq!(result2.len(), 3);
    }

    #[test]
    fn test_filter_published_for_anon_filters_unpublished() {
        let entries = vec![
            make_entry_row("pub.md", Some(true)),
            make_entry_row("draft.md", Some(false)),
            make_entry_row("no-flag.md", None),
        ];
        let result = filter_published_for_anon(true, true, entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "pub.md");
    }

    #[test]
    fn test_is_published_for_anon() {
        let published = serde_json::json!({ "published": true });
        let draft = serde_json::json!({ "published": false });
        let empty = serde_json::json!({});
        // anon + flag=true: only published=true passes
        assert!(is_published_for_anon(true, true, &published));
        assert!(!is_published_for_anon(true, true, &draft));
        assert!(!is_published_for_anon(true, true, &empty));
        // authenticated: always passes
        assert!(is_published_for_anon(false, true, &draft));
        // flag disabled: always passes
        assert!(is_published_for_anon(true, false, &draft));
    }
}
