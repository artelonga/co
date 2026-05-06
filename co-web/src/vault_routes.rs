//! Vault REST API — /api/v1/universes/:slug/vault
//!
//! Obsidian Local REST API compatible endpoints for file CRUD, search, and metadata.
//! Modelled on coddingtonbear/obsidian-local-rest-api so existing agents/tools work.
//!
//! Auth: Bearer JWT (same as board API) or long-lived API token (POST /api/v1/auth/token).
//! Rate limit: 60 requests/minute per API token.

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::auth::{Claims, jwt_secret};
use crate::entry_index::{EntryIndex, TagCount, make_entry};
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Rate limiter — 60 req/min per API token id
// ---------------------------------------------------------------------------

static VAULT_RATE_LIMITER: LazyLock<Mutex<HashMap<String, Vec<i64>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns `true` if the request is within the 60 req/min limit.
fn check_rate_limit(token_id: &str) -> bool {
    let Ok(mut limiter) = VAULT_RATE_LIMITER.lock() else {
        return true; // fail open on lock poisoning
    };
    let now = Utc::now().timestamp();
    let cutoff = now - 60;
    let entry = limiter.entry(token_id.to_string()).or_default();
    entry.retain(|&ts| ts >= cutoff);
    if entry.len() >= 60 {
        return false;
    }
    entry.push(now);
    true
}

// ---------------------------------------------------------------------------
// API Token model (also used by storage.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub token: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// File system stat (Obsidian-compatible — milliseconds since epoch).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultStat {
    pub ctime: i64,
    pub mtime: i64,
    pub size: i64,
}

/// Listing entry (path + stat only).
#[derive(Debug, Serialize)]
pub struct VaultFileInfo {
    pub path: String,
    pub stat: VaultStat,
}

/// Full file content response.
#[derive(Debug, Serialize)]
pub struct VaultFile {
    pub path: String,
    pub content: String,
    pub frontmatter: JsonValue,
    pub tags: Vec<String>,
    pub stat: VaultStat,
}

/// Search request body.
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
}

/// Single search match context.
#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub context: String,
    pub start: usize,
    pub end: usize,
}

/// Search result for one file.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub path: String,
    pub score: f64,
    pub matches: Vec<SearchMatch>,
}

/// DELETE query parameters.
#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    /// "soft" (default, move to .trash/) or "hard" (permanent)
    #[serde(default = "default_delete_mode")]
    pub mode: String,
}

fn default_delete_mode() -> String {
    "soft".to_string()
}

/// Clipper paste request — raw markdown body with frontmatter.
#[derive(Debug, Deserialize)]
pub struct ClipRequest {
    /// Raw markdown string (with or without frontmatter).
    pub content: String,
    /// Optional destination override (defaults to content/clips/).
    pub destination: Option<String>,
}

/// Clipper paste response.
#[derive(Debug, Serialize)]
pub struct ClipResponse {
    pub path: String,
    pub slug: String,
    pub url: String,
}

/// API token creation request.
#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    #[serde(default = "default_token_name")]
    pub name: String,
}

fn default_token_name() -> String {
    "API token".to_string()
}

/// API token creation response (includes full token value — only shown once).
#[derive(Debug, Serialize)]
pub struct CreateTokenResponse {
    pub id: String,
    pub name: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Token info for listing (token value omitted).
#[derive(Debug, Serialize)]
pub struct TokenInfo {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

/// Validate Bearer JWT or API token from Authorization header.
/// Returns `(user_id, Option<token_id>)`.
/// `token_id` is `Some` only for API tokens (these are rate-limited).
fn validate_vault_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(String, Option<String>), AppError> {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .or_else(|| crate::auth::extract_session_cookie(headers))
        .ok_or_else(|| {
            AppError::Unauthorized("Missing or malformed Authorization header".into())
        })?;

    let secret = jwt_secret();

    // Try JWT first
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
    let validation = Validation::new(Algorithm::HS256);
    if let Ok(data) = decode::<Claims>(
        &bearer,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    ) {
        return Ok((data.claims.sub, None));
    }

    // Try API token
    let storage = state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("Storage lock failed".into()))?;
    match storage.get_api_token_by_value(&bearer) {
        Ok(Some(tok)) => Ok((tok.user_id, Some(tok.id))),
        Ok(None) => Err(AppError::Unauthorized("Invalid or expired token".into())),
        Err(e) => Err(AppError::Internal(e.to_string())),
    }
}

/// Authenticate and rate-limit a vault request.
/// Returns the user_id on success.
///
/// 1.69.0: API tokens owned by admin-tier users skip the per-token rate
/// limit. The check still applies to non-admin tokens. Matches the
/// 1.45.0 single-tier model where every authenticated user is admin —
/// in practice, this means the limit is now effectively only for the
/// (currently unused) public anonymous-tier path. Bulk-pushes from
/// `co-sync` no longer hit the 60/min ceiling.
fn vault_auth(state: &AppState, headers: &HeaderMap) -> Result<String, AppError> {
    let (user_id, token_id) = validate_vault_auth(state, headers)?;
    if let Some(ref tid) = token_id {
        // Look up the token's owner tier; admin-tier owners bypass the limit.
        let is_admin = {
            let storage = state.storage.lock().ok();
            storage
                .as_ref()
                .and_then(|s| s.get_user_by_id(&user_id))
                .map(|u| u.tier == "admin")
                .unwrap_or(false)
        };
        if !is_admin && !check_rate_limit(tid) {
            return Err(AppError::TooManyRequests(
                "Rate limit exceeded (60 req/min)".into(),
            ));
        }
    }
    Ok(user_id)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn lock_storage(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::storage::Storage>, AppError> {
    state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("Storage lock failed".into()))
}

/// Build a VaultStat from optional ISO-8601 datetime strings and body size.
fn make_stat(created_at: Option<&str>, updated_at: Option<&str>, size: usize) -> VaultStat {
    let parse_ms = |s: &str| -> i64 {
        s.parse::<DateTime<Utc>>()
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0)
    };
    let ctime = created_at.map(parse_ms).unwrap_or(0);
    let mtime = updated_at.map(parse_ms).unwrap_or(ctime);
    VaultStat {
        ctime,
        mtime,
        size: size as i64,
    }
}

/// Extract tags from a frontmatter JSON value.
fn extract_tags(frontmatter: &JsonValue) -> Vec<String> {
    frontmatter
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Render frontmatter JSON + body back to a markdown string.
fn entry_to_content(frontmatter: &JsonValue, body: &str) -> String {
    co::entry::entry_to_markdown(frontmatter, body).unwrap_or_else(|_| body.to_string())
}

/// 1.50.0: paths whose vault PUT body is structured data, not markdown.
/// These files are written verbatim to disk (no `---\n{}\n---\n` wrapper)
/// and indexed with empty frontmatter + raw body. Critical for files like
/// `_universe.yaml` whose downstream parser doesn't expect a markdown
/// frontmatter prefix.
fn is_raw_data_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".toml")
        || lower.ends_with(".json")
}

/// Parse a raw markdown string into (frontmatter JSON, body).
fn parse_markdown_content(content: &str) -> (JsonValue, String) {
    match co::entry::split_frontmatter(content) {
        Ok((fm_str, body)) if !fm_str.is_empty() => {
            let fm: JsonValue = serde_yaml::from_str::<serde_yaml::Value>(&fm_str)
                .map(co::entry::yaml_to_json)
                .unwrap_or(JsonValue::Object(Default::default()));
            (fm, body)
        }
        Ok((_, body)) => (JsonValue::Object(Default::default()), body),
        Err(_) => (JsonValue::Object(Default::default()), content.to_string()),
    }
}

/// 1.50.0: write a structured-data file (yaml/toml/json) verbatim to the
/// universe's filesystem root, with no markdown frontmatter wrap. Used by
/// `put_vault_file` when the path matches `is_raw_data_file`. Caller is
/// responsible for index registration via `index_raw_vault_file`.
fn write_raw_vault_file(
    state: &AppState,
    universe_key: &str,
    path: &str,
    body: &str,
) -> Result<(), AppError> {
    let universe_root = {
        let storage = lock_storage(state)?;
        storage
            .get_universe(universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        storage.universe_root(universe_key)
    };
    let full_path = universe_root.join(path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::Internal(e.to_string()))?;
    }
    // Atomic: tmp + rename.
    let tmp_path = full_path.with_extension("co-tmp");
    std::fs::write(&tmp_path, body).map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::rename(&tmp_path, &full_path).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

/// 1.50.0: index-only entry-row update for a raw-data file. Mirrors the
/// portion of `write_vault_entry` that touches the per-universe SQLite,
/// without re-writing the file (which would re-add the frontmatter wrap).
fn index_raw_vault_file(
    state: &AppState,
    universe_key: &str,
    path: &str,
    body: &str,
) -> Result<crate::entry_index::EntryRow, AppError> {
    let entry = make_entry(path, JsonValue::Object(Default::default()), body);
    let uc = {
        let storage = lock_storage(state)?;
        storage.universe_conn(universe_key)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = EntryIndex::new(&uc_guard);
    index
        .upsert(universe_key, &entry)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let row = index
        .get(universe_key, path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::Internal("raw entry vanished after upsert".into()))?;
    // 1.71.0 (Phase 8 step 2): same CAS dual-write for raw YAML files.
    if let Ok(meta) = state.storage.lock()
        && let Err(e) = meta.put_blob(body.as_bytes())
    {
        tracing::warn!("put_blob failed for {universe_key}/{path}: {e}");
    }
    Ok(row)
}

/// Upsert an entry: write .md file + update SQLite index. Returns row data.
pub(crate) fn write_vault_entry(
    state: &AppState,
    universe_key: &str,
    path: &str,
    frontmatter: JsonValue,
    body: &str,
) -> Result<crate::entry_index::EntryRow, AppError> {
    let universe_root = {
        let storage = lock_storage(state)?;
        storage
            .get_universe(universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        storage.universe_root(universe_key)
    };

    let entry = make_entry(path, frontmatter.clone(), body);
    co::entry::write_entry(&universe_root, &entry)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // CO-79: load manifest from L1 cache (sync fast-path); fall back to disk on miss.
    let manifest_arc = state.cache.manifest.get(universe_key).or_else(|| {
        let bytes = std::fs::read(universe_root.join(co::manifest::MANIFEST_FILENAME)).ok()?;
        let m = co::manifest::parse(&bytes).ok().map(|r| r.manifest)?;
        state.cache.manifest.insert(universe_key.to_string(), m);
        state.cache.manifest.get(universe_key)
    });

    let now = Utc::now().to_rfc3339();
    {
        let uc = {
            let storage = lock_storage(state)?;
            storage.universe_conn(universe_key)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&uc_guard);
        index
            .upsert(universe_key, &entry)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-73: index semantic date fields (reuse cached manifest)
        index
            .upsert_dates(universe_key, &entry, manifest_arc.as_deref())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-74: extract and store typed FK relations
        if let Some(ref m) = manifest_arc {
            let _ = crate::relation_index::sync_entry_relations(
                &uc_guard,
                universe_key,
                path,
                &entry.entry_type,
                &frontmatter,
                m,
            );
        }
        // CO-156: sync references_meta for reference cards
        crate::reference_routes::maybe_sync_reference_meta(
            &uc_guard,
            universe_key,
            path,
            &entry.entry_type,
            &frontmatter,
            body,
            frontmatter.get("title").and_then(|v| v.as_str()),
            &universe_root,
        );
        // 1.71.0 (Phase 8 step 2): dual-write the body to the CAS blob
        // store. The entry's `body_hash` column is already sha256 of the
        // body, which is the same key `put_blob` uses — so the entry
        // doubles as a reference into `blobs` with zero schema change.
        // Phase 8 step 4 will read historical bytes via this on pin
        // rewind. Errors are logged but non-fatal — the vault write is
        // already durable in the on-disk file + entries index.
        if let Ok(meta) = state.storage.lock()
            && let Err(e) = meta.put_blob(body.as_bytes())
        {
            tracing::warn!("put_blob failed for {universe_key}/{path}: {e}");
        }

        // 1.67.0: refresh content_count from the actual entry-index row
        // count after every write. The pre-1.67 increment-by-one approach
        // overcounted on updates and undercounted on writes that bypassed
        // put_vault_file (states/branches/proposals/merges all go through
        // write_vault_entry directly). SELECT COUNT(*) is cheap on the
        // per-universe SQLite (indexed by universe_key) and idempotent.
        let actual_count: Option<i64> = uc_guard
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                rusqlite::params![universe_key],
                |row| row.get(0),
            )
            .ok();
        if let Some(n) = actual_count
            && let Ok(meta) = state.storage.lock()
        {
            let _ = meta.conn().execute(
                "UPDATE universes SET content_count = ?1 WHERE key = ?2",
                rusqlite::params![n, universe_key],
            );
        }
    }

    Ok(crate::entry_index::EntryRow {
        path: path.to_string(),
        universe_key: universe_key.to_string(),
        entry_type: entry.entry_type.clone(),
        title: frontmatter
            .get("title")
            .and_then(|v| v.as_str())
            .map(String::from),
        frontmatter,
        body: body.to_string(),
        body_hash: entry.body_hash.clone(),
        created_at: Some(now.clone()),
        updated_at: Some(now),
    })
}

// ---------------------------------------------------------------------------
// Handlers — File CRUD
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/:slug/vault/
/// List all vault files with stat.
pub async fn list_vault_files(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<VaultFileInfo>>, AppError> {
    vault_auth(&state, &headers)?;

    let uc = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let mut stmt = uc_guard.prepare(
        "SELECT path, created_at, updated_at, LENGTH(body) \
         FROM entries WHERE universe_key = ?1 ORDER BY path",
    )?;
    let files: Vec<VaultFileInfo> = stmt
        .query_map(rusqlite::params![slug], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .map(|(path, created, updated, size)| VaultFileInfo {
            stat: make_stat(created.as_deref(), updated.as_deref(), size as usize),
            path,
        })
        .collect();

    Ok(Json(files))
}

/// GET /api/v1/universes/:slug/vault/*path
/// Read a single vault file.
pub async fn get_vault_file(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<VaultFile>, AppError> {
    vault_auth(&state, &headers)?;

    let uc = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = EntryIndex::new(&uc_guard);
    let row = index
        .get(&slug, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("File '{path}' not found")))?;

    // CO-37: inject Obsidian Tasks checkbox into body for task entries.
    let export_body =
        crate::obsidian_tasks::inject_task_checkbox(&row.entry_type, &row.frontmatter, &row.body);
    let content = entry_to_content(&row.frontmatter, &export_body);
    let stat = make_stat(
        row.created_at.as_deref(),
        row.updated_at.as_deref(),
        row.body.len(),
    );
    let tags = extract_tags(&row.frontmatter);

    Ok(Json(VaultFile {
        path,
        content,
        frontmatter: row.frontmatter,
        tags,
        stat,
    }))
}

/// PUT /api/v1/universes/:slug/vault/*path
/// Create or overwrite a file. Body is raw markdown text.
pub async fn put_vault_file(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, AppError> {
    vault_auth(&state, &headers)?;

    // 1.50.0: write structured-data files (.yaml/.yml/.toml/.json) verbatim
    // to disk so downstream parsers (e.g., manifest reader on
    // `_universe.yaml`) don't choke on a synthetic markdown frontmatter
    // wrapper. Markdown writes still go through the existing path.
    let (frontmatter, body_text) = if is_raw_data_file(&path) {
        write_raw_vault_file(&state, &slug, &path, &body)?;
        (JsonValue::Object(Default::default()), body.clone())
    } else {
        parse_markdown_content(&body)
    };
    // CO-37: parse Obsidian Tasks checkbox from body and set frontmatter status
    // (frontmatter status is canonical and is not overwritten if already present).
    let (frontmatter, body_text) = if is_raw_data_file(&path) {
        (frontmatter, body_text)
    } else {
        crate::obsidian_tasks::apply_obsidian_tasks(frontmatter, &body_text)
    };
    // For raw-data files we already wrote to disk above; only `make_entry`-style
    // markdown content needs the full vault writer path (which adds the
    // frontmatter wrapper). Index-only update via write_vault_entry would
    // re-write the file with the wrapper — undoing our raw write — so we
    // do an index-only upsert for raw files.
    let row = if is_raw_data_file(&path) {
        index_raw_vault_file(&state, &slug, &path, &body)?
    } else {
        write_vault_entry(&state, &slug, &path, frontmatter, &body_text)?
    };

    // CO-156: emit entry.upsert telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "entry.upsert",
            universe: slug.clone(),
            list: Some(row.entry_type.clone()),
            key: Some(path.clone()),
            actor: crate::auth::resolve_user_id(&state, &headers),
            session_id: crate::telemetry::extract_session_id(&headers),
            extra: None,
        },
    );

    // CO-79: invalidate query cache entries for this universe after any vault write.
    state.cache.query.invalidate_prefix(&format!("{slug}:"));

    // CO-71 + CO-74: when `_universe.yaml` is updated, apply manifest indexes and
    // backfill typed FK relations for affected content types — both in background.
    if path == co::manifest::MANIFEST_FILENAME {
        // CO-79: invalidate manifest cache so the next read picks up the new schema.
        state.cache.invalidate_universe(&slug);
        let db_path = state.storage.lock().ok().map(|s| s.data_dir.join("co.db"));
        let universe_root = lock_storage(&state).ok().map(|s| s.universe_root(&slug));
        let universe_pool = state.storage.lock().ok().map(|s| s.universe_pool.clone());
        if let (Some(db_path), Some(universe_root)) = (db_path, universe_root) {
            let manifest_path = universe_root.join(co::manifest::MANIFEST_FILENAME);
            if let Ok(bytes) = std::fs::read(&manifest_path)
                && let Ok(parsed) = co::manifest::parse(&bytes)
            {
                crate::index_manager::apply_manifest_indexes_background(
                    db_path,
                    slug.clone(),
                    parsed.manifest.content_types.clone(),
                );
                // CO-74: backfill relations for all ref/ref_list fields in the new manifest
                if let Some(pool) = universe_pool {
                    crate::relation_index::backfill_relations_background(
                        pool,
                        slug.clone(),
                        parsed.manifest,
                    );
                }
            }
        }
    }

    // 1.67.0: content_count is now refreshed inside write_vault_entry
    // (SELECT COUNT(*)). The legacy increment-by-one call here was both
    // redundant (already covered) and incorrect (overcounted on updates).

    let stat = make_stat(
        row.created_at.as_deref(),
        row.updated_at.as_deref(),
        row.body.len(),
    );
    let tags = extract_tags(&row.frontmatter);
    let content = entry_to_content(&row.frontmatter, &row.body);

    // CO-151 web→local: notify any connected watchers about this REST-side
    // write so they can apply the change to their local filesystems.
    crate::sync_ws::emit_rest_upsert(&state, &slug, &path, &content).await;

    Ok((
        StatusCode::CREATED,
        Json(VaultFile {
            path,
            content,
            frontmatter: row.frontmatter,
            tags,
            stat,
        }),
    )
        .into_response())
}

/// POST /api/v1/universes/:slug/vault/*path
/// Append text to a file (or create if missing).
pub async fn post_vault_file(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, AppError> {
    vault_auth(&state, &headers)?;

    let (existing_fm, existing_body) = {
        let uc = {
            let storage = lock_storage(&state)?;
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&uc_guard);
        match index
            .get(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?
        {
            Some(row) => (row.frontmatter, row.body),
            None => (JsonValue::Object(Default::default()), String::new()),
        }
    };

    let new_body = if existing_body.is_empty() {
        body
    } else {
        format!("{}\n{}", existing_body, body)
    };

    write_vault_entry(&state, &slug, &path, existing_fm, &new_body)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// PATCH /api/v1/universes/:slug/vault/*path
/// Targeted edit via headers:
///   Target-Type: heading | frontmatter | block
///   Target: ## Section | field_name | ^block-id
///   Operation: append | prepend | replace
pub async fn patch_vault_file(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, AppError> {
    vault_auth(&state, &headers)?;

    let target_type = headers
        .get("target-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("replace")
        .to_string();
    let target = headers
        .get("target")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let operation = headers
        .get("operation")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("replace")
        .to_string();

    let (existing_fm, existing_body) = {
        let uc = {
            let storage = lock_storage(&state)?;
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&uc_guard);
        let row = index
            .get(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .ok_or_else(|| AppError::NotFound(format!("File '{path}' not found")))?;
        (row.frontmatter, row.body)
    };

    let (new_fm, new_body) = match target_type.as_str() {
        "frontmatter" => {
            let new_fm = patch_frontmatter(existing_fm, &target, &body, &operation);
            (new_fm, existing_body)
        }
        "heading" => {
            let new_body = patch_heading(&existing_body, &target, &body, &operation);
            (existing_fm, new_body)
        }
        "block" => {
            let new_body = patch_block(&existing_body, &target, &body, &operation);
            (existing_fm, new_body)
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "Unknown Target-Type: {other}"
            )));
        }
    };

    write_vault_entry(&state, &slug, &path, new_fm, &new_body)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// PATCH helper — update a frontmatter field.
fn patch_frontmatter(mut fm: JsonValue, field: &str, value: &str, operation: &str) -> JsonValue {
    if let Some(obj) = fm.as_object_mut() {
        let parsed: JsonValue =
            serde_json::from_str(value).unwrap_or(JsonValue::String(value.to_string()));
        match operation {
            "append" => {
                if let Some(existing) = obj.get_mut(field) {
                    if let Some(arr) = existing.as_array_mut() {
                        arr.push(parsed);
                    } else {
                        let old = existing.clone();
                        *existing = JsonValue::Array(vec![old, parsed]);
                    }
                } else {
                    obj.insert(field.to_string(), JsonValue::Array(vec![parsed]));
                }
            }
            "prepend" => {
                if let Some(existing) = obj.get_mut(field) {
                    if let Some(arr) = existing.as_array_mut() {
                        arr.insert(0, parsed);
                    } else {
                        let old = existing.clone();
                        *existing = JsonValue::Array(vec![parsed, old]);
                    }
                } else {
                    obj.insert(field.to_string(), JsonValue::Array(vec![parsed]));
                }
            }
            _ => {
                // replace (default)
                obj.insert(field.to_string(), parsed);
            }
        }
    }
    fm
}

/// PATCH helper — replace/extend a markdown heading section.
fn patch_heading(body: &str, heading: &str, replacement: &str, operation: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let heading_trim = heading.trim();
    let Some(start_idx) = lines.iter().position(|l| l.trim() == heading_trim) else {
        // Heading not found — append it
        return format!("{}\n{}\n{}", body, heading_trim, replacement);
    };

    // Detect the heading level
    let level = heading_trim.chars().take_while(|&c| c == '#').count();

    // Find end of section: next heading at same or higher level (fewer #), or EOF
    let end_idx = lines[start_idx + 1..]
        .iter()
        .position(|l| {
            let l_level = l.chars().take_while(|&c| c == '#').count();
            l_level > 0 && l_level <= level
        })
        .map(|i| start_idx + 1 + i)
        .unwrap_or(lines.len());

    let mut result: Vec<String> = lines[..=start_idx].iter().map(|s| s.to_string()).collect();

    match operation {
        "append" => {
            for l in &lines[start_idx + 1..end_idx] {
                result.push(l.to_string());
            }
            result.push(replacement.to_string());
        }
        "prepend" => {
            result.push(replacement.to_string());
            for l in &lines[start_idx + 1..end_idx] {
                result.push(l.to_string());
            }
        }
        _ => {
            // replace section content
            result.push(replacement.to_string());
        }
    }

    for l in &lines[end_idx..] {
        result.push(l.to_string());
    }
    result.join("\n")
}

/// PATCH helper — replace/edit a block by its ^block-id marker.
fn patch_block(body: &str, block_id: &str, replacement: &str, operation: &str) -> String {
    let block_id_trim = block_id.trim().trim_start_matches('^');
    let marker = format!("^{block_id_trim}");
    let lines: Vec<&str> = body.lines().collect();

    let Some(idx) = lines.iter().position(|l| l.ends_with(&marker)) else {
        // Block not found — append with marker
        return format!("{}\n{} {}", body, replacement, marker);
    };

    let mut result: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    match operation {
        "append" => {
            result.insert(idx + 1, replacement.to_string());
        }
        "prepend" => {
            result.insert(idx, replacement.to_string());
        }
        _ => {
            // replace: swap the line, keeping the block-id marker
            result[idx] = format!("{} {}", replacement, marker);
        }
    }
    result.join("\n")
}

/// DELETE /api/v1/universes/:slug/vault/*path
/// Soft delete (to .trash/) by default; `?mode=hard` for permanent.
pub async fn delete_vault_file(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<DeleteQuery>,
) -> Result<impl IntoResponse, AppError> {
    vault_auth(&state, &headers)?;

    let (universe_root, entry_exists) = {
        let uc = {
            let storage = lock_storage(&state)?;
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&uc_guard);
        let exists = index
            .get(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .is_some();
        let universe_root = {
            let storage = lock_storage(&state)?;
            storage.universe_root(&slug)
        };
        (universe_root, exists)
    };

    if !entry_exists {
        return Err(AppError::NotFound(format!("File '{path}' not found")));
    }

    let full_path = universe_root.join(&path);

    if q.mode == "hard" {
        if full_path.exists() {
            std::fs::remove_file(&full_path).map_err(|e| AppError::Internal(e.to_string()))?;
        }
    } else {
        // Soft delete: move to .trash/
        let trash_path = universe_root.join(".trash").join(&path);
        if let Some(parent) = trash_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::Internal(e.to_string()))?;
        }
        if full_path.exists() {
            std::fs::rename(&full_path, &trash_path)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    // Remove from index
    {
        let uc = {
            let storage = lock_storage(&state)?;
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&uc_guard);
        index
            .remove(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // CO-74: remove outbound FK relations
        let _ = crate::relation_index::RelationIndex::new(&uc_guard).delete_for_entry(&slug, &path);
        // CO-156: remove references_meta (idempotent)
        crate::reference_routes::remove_reference_meta(&uc_guard, &slug, &path);
    }

    let _ = lock_storage(&state).map(|mut s| s.decrement_universe_content_count(&slug, 1));

    // CO-156: emit entry.delete telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "entry.delete",
            universe: slug.clone(),
            list: None,
            key: Some(path.clone()),
            actor: crate::auth::resolve_user_id(&state, &headers),
            session_id: crate::telemetry::extract_session_id(&headers),
            extra: None,
        },
    );

    // CO-151 web→local: tell connected watchers to remove the file locally.
    crate::sync_ws::emit_rest_delete(&state, &slug, &path).await;

    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Handlers — Search & Metadata
// ---------------------------------------------------------------------------

/// POST /api/v1/universes/:slug/vault/search
/// Fuzzy text search across all files.
pub async fn search_vault(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SearchRequest>,
) -> Result<Json<Vec<SearchResult>>, AppError> {
    vault_auth(&state, &headers)?;

    let uc = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = EntryIndex::new(&uc_guard);
    let rows = index
        .search(&slug, &req.query)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let results: Vec<SearchResult> = rows
        .into_iter()
        .map(|row| {
            let matches = extract_matches(&row.body, &req.query, 80);
            SearchResult {
                path: row.path,
                score: 1.0,
                matches,
            }
        })
        .collect();

    Ok(Json(results))
}

/// Extract context snippets for each occurrence of `query` in `text`.
fn extract_matches(text: &str, query: &str, context_len: usize) -> Vec<SearchMatch> {
    let q_lower = query.to_lowercase();
    let t_lower = text.to_lowercase();
    let mut matches = vec![];
    let mut pos = 0;

    while let Some(idx) = t_lower[pos..].find(&q_lower) {
        let abs = pos + idx;
        let start = abs.saturating_sub(context_len / 2);
        let end = (abs + query.len() + context_len / 2).min(text.len());

        // Snap to valid char boundaries
        let ctx_start = (0..=start)
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(0);
        let ctx_end = (end..=text.len())
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(text.len());

        matches.push(SearchMatch {
            context: text[ctx_start..ctx_end].to_string(),
            start: abs,
            end: abs + query.len(),
        });
        pos = abs + query.len().max(1);
        if pos >= text.len() {
            break;
        }
    }
    matches
}

/// GET /api/v1/universes/:slug/vault/tags
/// List all tags with occurrence counts.
pub async fn vault_tags(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<TagCount>>, AppError> {
    vault_auth(&state, &headers)?;

    let uc = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = EntryIndex::new(&uc_guard);
    let tags = index
        .tags(&slug)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(tags))
}

/// Vault tree node — directory or file.
#[derive(Debug, Serialize)]
pub struct VaultTreeNode {
    pub name: String,
    pub path: String,
    pub is_file: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stat: Option<VaultStat>,
    pub children: Vec<VaultTreeNode>,
}

/// GET /api/v1/universes/:slug/vault/tree
/// Return a nested directory tree of all vault files.
pub async fn vault_tree(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<VaultTreeNode>>, AppError> {
    vault_auth(&state, &headers)?;

    let uc = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let mut stmt = uc_guard.prepare(
        "SELECT path, created_at, updated_at, LENGTH(body) \
         FROM entries WHERE universe_key = ?1 ORDER BY path",
    )?;
    let files: Vec<(String, VaultStat)> = stmt
        .query_map(rusqlite::params![slug], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ),
            ))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .map(|(path, (c, u, s))| (path, make_stat(c.as_deref(), u.as_deref(), s as usize)))
        .collect();

    Ok(Json(build_tree(files)))
}

/// Build a nested tree from a flat list of (full_path, stat) pairs.
fn build_tree(files: Vec<(String, VaultStat)>) -> Vec<VaultTreeNode> {
    let mut root: BTreeMap<String, DirEntry> = BTreeMap::new();
    for (path, stat) in files {
        insert_path(&mut root, &path, "", stat);
    }
    root.into_values().map(DirEntry::into_node).collect()
}

enum DirEntry {
    File {
        name: String,
        full_path: String,
        stat: VaultStat,
    },
    Dir {
        name: String,
        full_path: String,
        children: BTreeMap<String, DirEntry>,
    },
}

impl DirEntry {
    fn into_node(self) -> VaultTreeNode {
        match self {
            DirEntry::File {
                name,
                full_path,
                stat,
            } => VaultTreeNode {
                name,
                path: full_path,
                is_file: true,
                stat: Some(stat),
                children: vec![],
            },
            DirEntry::Dir {
                name,
                full_path,
                children,
            } => VaultTreeNode {
                name,
                path: full_path,
                is_file: false,
                stat: None,
                children: children.into_values().map(DirEntry::into_node).collect(),
            },
        }
    }
}

/// Recursively insert a path into the tree. `prefix` is the path of the parent dir.
fn insert_path(
    map: &mut BTreeMap<String, DirEntry>,
    remaining: &str,
    prefix: &str,
    stat: VaultStat,
) {
    if let Some(slash) = remaining.find('/') {
        let dir_name = &remaining[..slash];
        let rest = &remaining[slash + 1..];
        let full_dir = if prefix.is_empty() {
            dir_name.to_string()
        } else {
            format!("{prefix}/{dir_name}")
        };
        let entry = map
            .entry(dir_name.to_string())
            .or_insert_with(|| DirEntry::Dir {
                name: dir_name.to_string(),
                full_path: full_dir.clone(),
                children: BTreeMap::new(),
            });
        if let DirEntry::Dir { children, .. } = entry {
            insert_path(children, rest, &full_dir, stat);
        }
    } else {
        // Leaf file
        let full_path = if prefix.is_empty() {
            remaining.to_string()
        } else {
            format!("{prefix}/{remaining}")
        };
        map.insert(
            remaining.to_string(),
            DirEntry::File {
                name: remaining.to_string(),
                full_path,
                stat,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Handler — Clipper
// ---------------------------------------------------------------------------

/// POST /api/v1/universes/:slug/vault/clip
/// Accept Obsidian Clipper-formatted markdown and store as a clip entry.
pub async fn vault_clip(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ClipRequest>,
) -> Result<impl IntoResponse, AppError> {
    vault_auth(&state, &headers)?;

    let (mut frontmatter, body) = parse_markdown_content(&req.content);

    let title = frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("clip")
        .to_string();
    let clip_slug = slugify(&title);
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let filename = format!("{timestamp}-{clip_slug}.md");

    let dest_dir = req
        .destination
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("content/clips");
    let path = format!("{dest_dir}/{filename}");

    // Map Clipper fields to CO conventions
    if let Some(obj) = frontmatter.as_object_mut() {
        obj.entry("type".to_string())
            .or_insert_with(|| JsonValue::String("clip".to_string()));
        obj.entry("created".to_string())
            .or_insert_with(|| JsonValue::String(Utc::now().to_rfc3339()));
        obj.entry("modified".to_string())
            .or_insert_with(|| JsonValue::String(Utc::now().to_rfc3339()));
    }

    {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
    }

    write_vault_entry(&state, &slug, &path, frontmatter, &body)?;
    // 1.67.0: count refreshed inside write_vault_entry — no redundant call.

    let url = format!("/api/v1/universes/{slug}/vault/{path}");
    Ok((
        StatusCode::CREATED,
        Json(ClipResponse {
            path,
            slug: clip_slug,
            url,
        }),
    )
        .into_response())
}

/// Convert a title to a URL-friendly slug.
pub fn slugify(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c.is_whitespace() || c == '-' || c == '_' {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ---------------------------------------------------------------------------
// Handlers — API token management
// ---------------------------------------------------------------------------

/// POST /api/v1/auth/token
/// Generate a long-lived API token (90 days) for plugin/agent use.
/// Requires JWT Bearer auth.
pub async fn create_api_token(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
    Json(req): Json<CreateTokenRequest>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state)?;
    let tok = storage
        .create_api_token(&user_id.0, &req.name)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(CreateTokenResponse {
            id: tok.id,
            name: tok.name,
            token: tok.token,
            expires_at: tok.expires_at,
        }),
    )
        .into_response())
}

/// GET /api/v1/auth/tokens
/// List API tokens for the authenticated user (values redacted).
pub async fn list_api_tokens(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
) -> Result<Json<Vec<TokenInfo>>, AppError> {
    let storage = lock_storage(&state)?;
    let tokens = storage
        .list_api_tokens(&user_id.0)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let infos: Vec<TokenInfo> = tokens
        .into_iter()
        .map(|t| TokenInfo {
            id: t.id,
            name: t.name,
            created_at: t.created_at,
            expires_at: t.expires_at,
            last_used_at: t.last_used_at,
        })
        .collect();

    Ok(Json(infos))
}

/// DELETE /api/v1/auth/tokens/:id
/// Revoke an API token.
pub async fn revoke_api_token(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state)?;
    let deleted = storage
        .delete_api_token(&id, &user_id.0)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if deleted {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(AppError::NotFound("Token not found".into()))
    }
}

// ---------------------------------------------------------------------------
// Router builders
// ---------------------------------------------------------------------------

/// Vault file-operation routes nested under /api/v1/universes.
/// Static routes (tags, tree, search, clip) must appear in the router BEFORE
/// the `*path` wildcard — Axum gives static segments priority over wildcards.
pub fn vault_router() -> Router<AppState> {
    Router::new()
        // Fixed sub-paths — registered first for static priority
        .route("/{slug}/vault/", get(list_vault_files))
        .route("/{slug}/vault/tags", get(vault_tags))
        .route("/{slug}/vault/tree", get(vault_tree))
        .route("/{slug}/vault/search", post(search_vault))
        .route("/{slug}/vault/clip", post(vault_clip))
        // Wildcard file CRUD
        .route(
            "/{slug}/vault/{*path}",
            get(get_vault_file)
                .put(put_vault_file)
                .post(post_vault_file)
                .patch(patch_vault_file)
                .delete(delete_vault_file),
        )
}

/// API token management routes (nested under /v1/auth, JWT-protected).
pub fn token_router() -> Router<AppState> {
    Router::new()
        .route("/token", post(create_api_token))
        .route("/tokens", get(list_api_tokens))
        .route("/tokens/{id}", delete(revoke_api_token))
        .layer(axum::middleware::from_fn(crate::auth::require_auth))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::auth::sign_jwt;
    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::models::CreateUniverse;
    use crate::server::{AppState, AppStateInner, build_router};
    use crate::storage::Storage;

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
            port: 3000,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: true,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "quilomboaraucaria".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
        }
    }

    /// Sign a test JWT with the CURRENT JWT_SECRET env var (or fallback).
    /// This avoids overriding the global env var and breaking concurrent tests
    /// (universe_routes and ws tests also mutate JWT_SECRET).
    fn test_bearer() -> String {
        let secret =
            std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".to_string());
        let (token, _) = sign_jwt("test-user", "test@example.com", "player", &secret).unwrap();
        format!("Bearer {token}")
    }

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        let config = test_config(dir);
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path)
                .expect("Failed to open test game storage"),
        );
        let state: AppState = Arc::new(AppStateInner {
            storage: Mutex::new(storage),
            experiment: Mutex::new(experiment),
            config,
            auth_store: Mutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            cache: crate::cache::CacheLayer::new(),
            rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
            wae: crate::wae::WaeEmitter::new(None, None),
        });
        build_router(state, None)
    }

    fn seed_universe(dir: &std::path::Path, slug: &str) {
        let mut storage = Storage::new(dir.to_str().unwrap());
        let _ = storage.create_universe(
            CreateUniverse {
                key: slug.to_string(),
                name: slug.to_string(),
                description: String::new(),
            },
            "test-user",
        );
    }

    async fn body_str(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    // -----------------------------------------------------------------------
    // CRUD cycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_vault_crud_cycle() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "test-vault");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        let content = "---\ntitle: Test Note\ntags: [rust, test]\ntype: note\n---\n\nHello vault!";

        // PUT — create file
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let json: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert_eq!(json["path"], "notes/hello.md");
        assert_eq!(json["frontmatter"]["title"], "Test Note");

        // GET — read file
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert_eq!(json["frontmatter"]["title"], "Test Note");
        assert!(
            json["tags"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("rust"))
        );

        // GET / — list files
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/test-vault/vault/")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let files: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert!(
            files
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["path"] == "notes/hello.md")
        );

        // POST — append
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from("Appended line."))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // GET — verify append
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert!(json["content"].as_str().unwrap().contains("Appended line."));

        // POST /search
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/test-vault/vault/search")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"query":"vault"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let results: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert!(!results.as_array().unwrap().is_empty());

        // DELETE — soft
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // GET after delete — 404
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // PATCH targeted edits
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_vault_patch_frontmatter() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "patch-test");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        let content = "---\ntitle: Patch Me\ntags: [old]\ntype: note\n---\n\nBody.";
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/universes/patch-test/vault/patch.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();

        // PATCH frontmatter — replace tags
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/universes/patch-test/vault/patch.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .header("target-type", "frontmatter")
                    .header("target", "tags")
                    .header("operation", "replace")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(r#"["new","updated"]"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/patch-test/vault/patch.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        let tags: Vec<&str> = json["frontmatter"]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(tags.contains(&"new"), "Expected 'new' in {tags:?}");
        assert!(tags.contains(&"updated"), "Expected 'updated' in {tags:?}");
        assert!(
            !tags.contains(&"old"),
            "Should not contain 'old' in {tags:?}"
        );
    }

    #[tokio::test]
    async fn test_vault_patch_heading() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "heading-test");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        let content = "---\ntitle: Doc\ntype: note\n---\n\n## Introduction\n\nOld intro text.\n\n## Other\n\nOther content.";
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/universes/heading-test/vault/doc.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/universes/heading-test/vault/doc.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .header("target-type", "heading")
                    .header("target", "## Introduction")
                    .header("operation", "replace")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from("New intro text."))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/heading-test/vault/doc.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        let c = json["content"].as_str().unwrap();
        assert!(c.contains("New intro text."), "Expected new text in: {c}");
        assert!(
            !c.contains("Old intro text."),
            "Should not contain old: {c}"
        );
        assert!(c.contains("## Other"), "Should keep other sections: {c}");
    }

    // -----------------------------------------------------------------------
    // Clipper format
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_vault_clipper_post() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "clipper-test");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        let clip_req = serde_json::json!({
            "content": "---\ntitle: \"Article Title\"\nsource: \"https://example.com/article\"\nauthor: \"Author Name\"\npublished: \"2026-04-06\"\ncreated: \"2026-04-06T13:00:00Z\"\ntags: [web-clip, topic]\n---\n\n# Article Title\n\nClipped content here..."
        });

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/clipper-test/vault/clip")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(clip_req.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let result: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert!(
            result["path"]
                .as_str()
                .unwrap()
                .starts_with("content/clips/"),
            "Path should be in content/clips/: {}",
            result["path"]
        );
        assert!(
            result["slug"].as_str().unwrap().contains("article-title"),
            "Slug should contain 'article-title': {}",
            result["slug"]
        );

        // Read back the clip and verify fields
        let path = result["path"].as_str().unwrap().to_string();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/universes/clipper-test/vault/{path}"))
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let file: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert_eq!(file["frontmatter"]["type"], "clip");
        assert_eq!(file["frontmatter"]["source"], "https://example.com/article");
        assert!(
            file["tags"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("web-clip"))
        );
    }

    // -----------------------------------------------------------------------
    // Auth — unauthenticated request rejected
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_vault_requires_auth() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "auth-test");
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/auth-test/vault/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // -----------------------------------------------------------------------
    // API token lifecycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_api_token_lifecycle() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        // Create token
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/token")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"obsidian-plugin"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        let token_val = created["token"].as_str().unwrap().to_string();
        let token_id = created["id"].as_str().unwrap().to_string();
        assert!(
            token_val.starts_with("co_"),
            "Token should have co_ prefix: {token_val}"
        );

        // List tokens
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/tokens")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let list: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert!(list.as_array().unwrap().iter().any(|t| t["id"] == token_id));

        // Use the API token to access vault
        seed_universe(dir.path(), "token-test");
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/token-test/vault/")
                    .header(header::AUTHORIZATION, format!("Bearer {token_val}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Revoke token
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/auth/tokens/{token_id}"))
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Revoked token should no longer work
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/token-test/vault/")
                    .header(header::AUTHORIZATION, format!("Bearer {token_val}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // -----------------------------------------------------------------------
    // Integration: Obsidian plugin mock → vault API → verify SQLite state
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_obsidian_plugin_mock_flow() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "obs-test");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        // Simulate plugin writing a note
        let note = "---\ntitle: Plugin Note\ntype: note\ntags: [from-plugin]\n---\n\nPlugin body.";
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/universes/obs-test/vault/notes/plugin-note.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(note))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "PUT should succeed");

        // Verify via vault listing — entry should appear
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/obs-test/vault/")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let files: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert!(
            files
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["path"] == "notes/plugin-note.md"),
            "Entry should appear in vault listing (SQLite indexed): {files}"
        );

        // Verify content preserved correctly
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/obs-test/vault/notes/plugin-note.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let file: serde_json::Value =
            serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
        assert_eq!(
            file["frontmatter"]["title"], "Plugin Note",
            "SQLite should have the correct title"
        );
        assert!(
            file["tags"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("from-plugin")),
            "Tags should be preserved in SQLite"
        );
    }

    // -----------------------------------------------------------------------
    // Rate limiter unit test (no network)
    // -----------------------------------------------------------------------

    #[test]
    fn test_rate_limiter_allows_up_to_60() {
        // Use a unique id per test run to avoid cross-test interference
        let id = format!("rate-test-{}", nanoid::nanoid!(12));
        for i in 0..60 {
            assert!(check_rate_limit(&id), "Request {i} should be within limit");
        }
        assert!(
            !check_rate_limit(&id),
            "61st request should be rate-limited"
        );
    }

    // -----------------------------------------------------------------------
    // Helper unit tests (no I/O)
    // -----------------------------------------------------------------------

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("Article Title"), "article-title");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("CO v1.0 Release"), "co-v10-release");
    }

    #[test]
    fn test_patch_frontmatter_replace() {
        let fm = serde_json::json!({"tags": ["old"], "title": "Test"});
        let result = patch_frontmatter(fm, "tags", r#"["new","updated"]"#, "replace");
        let tags = result["tags"].as_array().unwrap();
        assert!(tags.contains(&serde_json::json!("new")));
        assert!(!tags.contains(&serde_json::json!("old")));
    }

    #[test]
    fn test_patch_frontmatter_append() {
        let fm = serde_json::json!({"tags": ["existing"]});
        let result = patch_frontmatter(fm, "tags", r#""new-tag""#, "append");
        let tags = result["tags"].as_array().unwrap();
        assert!(tags.contains(&serde_json::json!("existing")));
        assert!(tags.contains(&serde_json::json!("new-tag")));
    }

    #[test]
    fn test_extract_matches() {
        let text = "Hello vault world, vault is great";
        let matches = extract_matches(text, "vault", 10);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_patch_heading_replace() {
        let body = "## Introduction\n\nOld text.\n\n## Other\n\nKeep this.";
        let result = patch_heading(body, "## Introduction", "New text.", "replace");
        assert!(result.contains("New text."), "Got: {result}");
        assert!(!result.contains("Old text."), "Got: {result}");
        assert!(result.contains("## Other"), "Got: {result}");
    }

    #[test]
    fn test_patch_block_replace() {
        let body = "Some paragraph with block ref. ^myblock\n\nOther content.";
        let result = patch_block(body, "^myblock", "Replaced paragraph.", "replace");
        assert!(
            result.contains("Replaced paragraph. ^myblock"),
            "Got: {result}"
        );
        assert!(!result.contains("Some paragraph"), "Got: {result}");
    }
}
