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

/// Load and parse `_universe.yaml` from `universe_root`.
/// Returns `None` if the file does not exist or fails to parse.
fn load_manifest(universe_root: &std::path::Path) -> Option<co::manifest::Manifest> {
    let path = universe_root.join(co::manifest::MANIFEST_FILENAME);
    let bytes = std::fs::read(&path).ok()?;
    co::manifest::parse(&bytes).ok().map(|r| r.manifest)
}

/// Validate `frontmatter` against the manifest's content type for `entry_type`.
///
/// Returns `Ok(())` when:
/// - No `_universe.yaml` is present (legacy universe).
/// - The manifest has no schema for `entry_type`.
/// - The payload passes all schema checks.
///
/// Returns `Err(AppError::UnprocessableEntity)` with a field-path message
/// when validation fails.
fn validate_against_manifest(
    universe_root: &std::path::Path,
    entry_type: &str,
    frontmatter: &JsonValue,
) -> Result<(), AppError> {
    let manifest = match load_manifest(universe_root) {
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
    let uc = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
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
    } else {
        let entry_type = q.entry_type.as_deref().unwrap_or("");
        if entry_type.is_empty() {
            // list all
            index
                .query(&slug, "", &serde_json::json!({}))
                .or_else(|_| {
                    // fallback: return all entries via raw query
                    Ok::<Vec<EntryRow>, anyhow::Error>(vec![])
                })
                .unwrap_or_default()
        } else {
            let filter = q
                .filter
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::json!({}));
            index
                .query(&slug, entry_type, &filter)
                .map_err(|e| AppError::Internal(e.to_string()))?
        }
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
    let uc = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
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
    let uc = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
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

/// GET /api/v1/universes/:slug/entries/*path — read single entry
pub async fn get_entry(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
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
        Ok(Json(entry).into_response())
    }
}

/// POST /api/v1/universes/:slug/entries — create entry
pub async fn create_entry(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<CreateEntryBody>,
) -> Result<impl IntoResponse, AppError> {
    if body.path.is_empty() {
        return Err(AppError::BadRequest("Entry path cannot be empty".into()));
    }

    let universe_root = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;
        storage.universe_root(&slug)
    };

    // CO-71: validate frontmatter against manifest schema before writing.
    let entry_type = body
        .frontmatter
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    validate_against_manifest(&universe_root, entry_type, &body.frontmatter)?;

    // Write .md file
    let entry = make_entry(&body.path, body.frontmatter.clone(), &body.body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    // Index into universe data.db
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

    // CO-71: validate merged frontmatter against manifest schema before writing.
    let entry_type_for_validation = new_fm
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    validate_against_manifest(&universe_root, entry_type_for_validation, &new_fm)?;

    let entry = make_entry(&path, new_fm.clone(), &new_body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

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
    }

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

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        // Specific paths must come before wildcard
        .route("/{slug}/entries/tags", get(list_entry_tags))
        .route("/{slug}/entries/tree", get(entry_tree))
        .route("/{slug}/entries", get(list_entries).post(create_entry))
        .route(
            "/{slug}/entries/{*path}",
            get(get_entry).put(update_entry).delete(delete_entry),
        )
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
