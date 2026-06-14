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
use crate::entry_index::{TagCount, make_entry};
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
    /// Raw token — `Some` only at creation time; `None` on all subsequent reads.
    pub token: Option<String>,
    pub token_hash: String,
    pub token_prefix: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// CO-448: least-privilege capability set (`recurso:ação`), resolved from
    /// the issuance request (bundles already expanded). `None` ⇒ no declared
    /// scope → inherit the owner's tier (pre-CO-448 all-or-nothing behavior).
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
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

/// Listing entry (path + stat + body hash).
#[derive(Debug, Serialize)]
pub struct VaultFileInfo {
    pub path: String,
    pub stat: VaultStat,
    /// CO-438: SHA-256 of the entry body (frontmatter excluded), the same value
    /// stored in the `body_hash` column. Lets bulk-sync clients (`co source
    /// add`) skip re-PUTting entries whose body is byte-identical, so a re-sync
    /// only touches what changed and a rate-limited import can resume.
    pub body_hash: String,
}

/// Full file content response.
#[derive(Debug, Serialize)]
pub struct VaultFile {
    pub path: String,
    pub content: String,
    pub frontmatter: JsonValue, // FREEFORM: vault note frontmatter is user-defined YAML with arbitrary keys
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
    /// CO-448: optional least-privilege scope. A list of capability strings
    /// (`recurso:ação`) and/or bundle names (`read`/`write`/`admin`/`agent`),
    /// resolved + expanded at issuance. Absent/empty ⇒ NULL scopes (the token
    /// inherits the owner's tier, pre-CO-448 behavior).
    #[serde(default)]
    pub scopes: Vec<String>,
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
    /// CO-448: the resolved capability set the token carries (bundles already
    /// expanded). `None` ⇒ no declared scope (inherits the owner's tier).
    pub scopes: Option<Vec<String>>,
}

/// Token info for listing (token value omitted).
#[derive(Debug, Serialize)]
pub struct TokenInfo {
    pub id: String,
    pub name: String,
    pub token_prefix: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// CO-448: the resolved capability set (auditable). `None` ⇒ inherits tier.
    pub scopes: Option<Vec<String>>,
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
    let storage = state.core.storage.lock();
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
            let storage = state.core.storage.lock();
            storage
                .get_user_by_id(&user_id)
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

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.core.storage.lock()
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

/// CO-245: plaintext code/data files that are stored verbatim (no markdown
/// frontmatter wrapper) and indexed as `asset.code` entries with their
/// detected MIME type. This allows inline editing in the browser while
/// keeping the file valid for interpreters and linters.
fn is_code_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".py")
        || lower.ends_with(".rs")
        || lower.ends_with(".ts")
        || lower.ends_with(".js")
        || lower.ends_with(".mjs")
        || lower.ends_with(".sh")
        || lower.ends_with(".bash")
        || lower.ends_with(".sql")
        || lower.ends_with(".go")
        || lower.ends_with(".r")
        || lower.ends_with(".rb")
        || lower.ends_with(".csv")
        || lower.ends_with(".tsv")
        || lower.ends_with(".html")
        || lower.ends_with(".htm")
        || lower.ends_with(".css")
        || lower.ends_with(".scss")
        || lower.ends_with(".xml")
        || lower.ends_with(".txt")
        || lower.ends_with(".cpp")
        || lower.ends_with(".c")
        || lower.ends_with(".h")
        || lower.ends_with(".java")
        || lower.ends_with(".kt")
        || lower.ends_with(".php")
}

/// CO-245: return the MIME type for a known plaintext code file extension.
fn code_file_mime(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".py") {
        "text/x-python"
    } else if lower.ends_with(".rs") {
        "text/x-rust"
    } else if lower.ends_with(".ts") {
        "application/typescript"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "application/javascript"
    } else if lower.ends_with(".sh") || lower.ends_with(".bash") {
        "text/x-shellscript"
    } else if lower.ends_with(".sql") {
        "text/x-sql"
    } else if lower.ends_with(".go") {
        "text/x-go"
    } else if lower.ends_with(".r") {
        "text/x-r"
    } else if lower.ends_with(".rb") {
        "text/x-ruby"
    } else if lower.ends_with(".csv") {
        "text/csv"
    } else if lower.ends_with(".tsv") {
        "text/tab-separated-values"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html"
    } else if lower.ends_with(".css") || lower.ends_with(".scss") {
        "text/css"
    } else if lower.ends_with(".xml") {
        "text/xml"
    } else if lower.ends_with(".cpp") || lower.ends_with(".c") || lower.ends_with(".h") {
        "text/x-c"
    } else if lower.ends_with(".java") || lower.ends_with(".kt") {
        "text/x-java"
    } else if lower.ends_with(".php") {
        "text/x-php"
    } else {
        "text/plain"
    }
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

/// CO-242: handle a binary vault PUT — write blob + create asset + entries rows atomically.
async fn put_vault_binary(
    state: &AppState,
    universe_key: &str,
    path: &str,
    content_type: Option<&str>,
    bytes: axum::body::Bytes,
) -> Result<axum::response::Response, AppError> {
    use sha2::{Digest, Sha256};

    if bytes.len() > crate::asset_routes::MAX_ASSET_BYTES {
        return Err(AppError::BadRequest(format!(
            "Asset too large ({} > {} bytes)",
            bytes.len(),
            crate::asset_routes::MAX_ASSET_BYTES
        )));
    }

    let mime = crate::asset_routes::detect_mime(&bytes, content_type);
    let entry_type = crate::asset_routes::mime_to_asset_entry_type(&mime);

    let sha256 = {
        let mut h = Sha256::new();
        h.update(&bytes);
        let digest = h.finalize();
        let mut out = String::with_capacity(64);
        for b in digest.iter() {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
        }
        out
    };

    let filename = path.split('/').next_back().unwrap_or("").to_string();
    let now_ns = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000);
    let now_iso = chrono::Utc::now().to_rfc3339();

    let (universe_dir, conn) = {
        let storage = lock_storage(state);
        storage
            .get_universe(universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        (
            storage.universe_pool.universe_dir(universe_key),
            storage.universe_conn(universe_key),
        )
    };

    let blob_dir = universe_dir
        .join("blobs")
        .join(&sha256[0..2])
        .join(&sha256[2..4]);
    std::fs::create_dir_all(&blob_dir)
        .map_err(|e| AppError::Internal(format!("create blob dir: {e}")))?;
    let blob_path_full = blob_dir.join(&sha256);

    let (ciphertext, nonce) = crate::asset_crypto::encrypt_blob(&bytes, universe_key, &sha256)
        .map_err(|e| AppError::Internal(format!("encrypt blob: {e}")))?;

    let tmp = blob_path_full.with_extension("tmp");
    std::fs::write(&tmp, &ciphertext)
        .map_err(|e| AppError::Internal(format!("write blob: {e}")))?;
    std::fs::rename(&tmp, &blob_path_full)
        .map_err(|e| AppError::Internal(format!("rename blob: {e}")))?;

    let rel_blob_path = format!("blobs/{}/{}/{}", &sha256[0..2], &sha256[2..4], &sha256);
    let size = bytes.len() as i64;
    let cipher_size = ciphertext.len() as i64;

    let fm = serde_json::json!({
        "type": entry_type,
        "mime": mime,
        "asset_sha256": sha256,
        "size_bytes": size,
        "filename": filename,
    });
    let fm_json_str = fm.to_string();
    let title = if filename.is_empty() {
        &sha256[..16]
    } else {
        &filename
    };

    {
        let guard = conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        guard
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| AppError::Internal(format!("begin binary vault tx: {e}")))?;
        let tx: rusqlite::Result<()> = (|| {
            guard.execute(
                "INSERT OR REPLACE INTO assets \
                 (sha256, blob_path, mime, size_bytes, filename, created_at_ns, created_by, \
                  refcount, nonce, cipher_size, encrypted) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, \
                  COALESCE((SELECT refcount FROM assets WHERE sha256 = ?1), 0), \
                  ?7, ?8, 1)",
                rusqlite::params![
                    &sha256,
                    &rel_blob_path,
                    &mime,
                    size,
                    &filename,
                    now_ns,
                    &nonce[..],
                    cipher_size,
                ],
            )?;
            guard.execute(
                "INSERT INTO entries \
                   (path, universe_key, entry_type, title, \
                    frontmatter_json, payload, body, body_hash, \
                    body_lines, body_words, body_chars, \
                    created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, '', '', 0, 0, 0, ?6, ?6) \
                 ON CONFLICT(universe_key, path) DO UPDATE SET \
                   entry_type = excluded.entry_type, \
                   title = excluded.title, \
                   frontmatter_json = excluded.frontmatter_json, \
                   payload = excluded.payload, \
                   updated_at = excluded.updated_at",
                rusqlite::params![
                    path,
                    universe_key,
                    entry_type,
                    title,
                    &fm_json_str,
                    &now_iso,
                ],
            )?;
            guard
                .execute(
                    "INSERT INTO entries_fts (universe_key, path, title, body) VALUES (?1, ?2, ?3, '')",
                    rusqlite::params![universe_key, path, title],
                )
                .ok();
            Ok(())
        })();
        match tx {
            Ok(()) => guard
                .execute_batch("COMMIT")
                .map_err(|e| AppError::Internal(format!("commit binary vault tx: {e}")))?,
            Err(e) => {
                let _ = guard.execute_batch("ROLLBACK");
                return Err(AppError::Internal(format!("binary vault insert: {e}")));
            }
        }
        let actual_count: Option<i64> = guard
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                rusqlite::params![universe_key],
                |r| r.get(0),
            )
            .ok();
        if let Some(n) = actual_count {
            let meta = lock_storage(state);
            let _ = meta.set_universe_content_count(universe_key, n);
        }
    }

    let stat = make_stat(Some(&now_iso), Some(&now_iso), size as usize);
    use axum::http::StatusCode;
    Ok((
        StatusCode::CREATED,
        Json(VaultFile {
            path: path.to_string(),
            content: String::new(),
            frontmatter: fm,
            tags: vec![],
            stat,
        }),
    )
        .into_response())
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
        let storage = lock_storage(state);
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
        let storage = lock_storage(state);
        storage.universe_conn(universe_key)
    };
    let repo = crate::repository::SqliteEntryRepository::new(uc);
    repo.upsert_synced(universe_key, &entry)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let row = repo
        .get(universe_key, path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::Internal("raw entry vanished after upsert".into()))?;
    // 1.71.0 (Phase 8 step 2): same CAS dual-write for raw YAML files.
    {
        let meta = state.core.storage.lock();
        if let Err(e) = meta.put_blob(body.as_bytes()) {
            tracing::warn!("put_blob failed for {universe_key}/{path}: {e}");
        }
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
        let storage = lock_storage(state);
        storage
            .get_universe(universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        storage.universe_root(universe_key)
    };

    let entry = make_entry(path, frontmatter.clone(), body);
    co::entry::write_entry(&universe_root, &entry)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // CO-79: load manifest from L1 cache (sync fast-path); fall back to disk on miss.
    let manifest_arc = state.index.cache.manifest.get(universe_key).or_else(|| {
        let bytes = std::fs::read(universe_root.join(co::manifest::MANIFEST_FILENAME)).ok()?;
        let m = co::manifest::parse(&bytes).ok().map(|r| r.manifest)?;
        state
            .index
            .cache
            .manifest
            .insert(universe_key.to_string(), m);
        state.index.cache.manifest.get(universe_key)
    });

    let now = Utc::now().to_rfc3339();
    // CO-432: combined index write via the entry repository — entries upsert +
    // event log (CO-95 tx), dates, relations, references_meta in one lock scope.
    let uc = {
        let storage = lock_storage(state);
        storage.universe_conn(universe_key)
    };
    let actual_count = crate::repository::SqliteEntryRepository::new(uc)
        .index_vault_write(
            universe_key,
            &entry,
            &frontmatter,
            body,
            manifest_arc.as_deref(),
            &universe_root,
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // 1.71.0 (Phase 8 step 2): dual-write the body to the CAS blob
    // store. The entry's `body_hash` column is already sha256 of the
    // body, which is the same key `put_blob` uses — so the entry
    // doubles as a reference into `blobs` with zero schema change.
    // Phase 8 step 4 will read historical bytes via this on pin
    // rewind. Errors are logged but non-fatal — the vault write is
    // already durable in the on-disk file + entries index.
    {
        let meta = state.core.storage.lock();
        if let Err(e) = meta.put_blob(body.as_bytes()) {
            tracing::warn!("put_blob failed for {universe_key}/{path}: {e}");
        }
    }

    // 1.67.0: refresh content_count from the actual entry-index row
    // count after every write. The pre-1.67 increment-by-one approach
    // overcounted on updates and undercounted on writes that bypassed
    // put_vault_file (states/branches/proposals/merges all go through
    // write_vault_entry directly). SELECT COUNT(*) is cheap on the
    // per-universe SQLite (indexed by universe_key) and idempotent.
    if let Some(n) = actual_count {
        let meta = state.core.storage.lock();
        let _ = meta.set_universe_content_count(universe_key, n);
    }

    // CO-380: publish vault.write to EDA bus for observability.
    state.core.eda_bus.publish(crate::eda::Event::new(
        "vault.write",
        Some(universe_key.to_string()),
        None,
        serde_json::json!({ "path": path, "entry_type": entry.entry_type }),
        crate::eda::Visibility::UniverseOwner,
    ));

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
        _score: None,
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
        let storage = lock_storage(&state);
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let mut stmt = uc_guard.prepare(
        "SELECT path, created_at, updated_at, LENGTH(body), body_hash \
         FROM entries WHERE universe_key = ?1 ORDER BY path",
    )?;
    let files: Vec<VaultFileInfo> = stmt
        .query_map(rusqlite::params![slug], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .map(|(path, created, updated, size, body_hash)| VaultFileInfo {
            stat: make_stat(created.as_deref(), updated.as_deref(), size as usize),
            path,
            body_hash,
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

    // CO-88b: record content-pipeline telemetry when the caller announces a
    // layer combo via X-Co-* headers (the co-pipeline UAT runner does).
    if let Some(metrics) = crate::pipeline::PipelineMetrics::from_headers(&headers, 0) {
        crate::pipeline::emit_pipeline_event(&state, &slug, &path, "get", &metrics, &headers);
    }

    let uc = {
        let storage = lock_storage(&state);
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let row = crate::repository::SqliteEntryRepository::new(uc)
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

/// Returns true when the Content-Type indicates binary (non-text) content that
/// should be stored as a CO-242 asset entry rather than a markdown vault note.
fn is_binary_vault_content(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else { return false };
    let base = ct.split(';').next().unwrap_or(ct).trim();
    !base.is_empty()
        && !base.starts_with("text/")
        && !matches!(
            base,
            "application/json"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
                | "application/javascript"
                | "application/typescript"
                | "application/xml"
        )
}

/// PUT /api/v1/universes/:slug/vault/*path
///
/// For text content (markdown, YAML, JSON, …) behaves as before: write the
/// vault note and update the entries index.
///
/// CO-242: for binary content (PDF, image, video, …) — detected via
/// Content-Type — write a blob asset + a unified entries row in one
/// transaction, mirroring what POST /assets does but addressed by path.
pub async fn put_vault_file(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, AppError> {
    vault_auth(&state, &headers)?;

    // CO-88b: record content-pipeline telemetry when the caller announces a
    // layer combo via X-Co-* headers (the co-pipeline UAT runner does).
    if let Some(metrics) = crate::pipeline::PipelineMetrics::from_headers(&headers, body.len()) {
        crate::pipeline::emit_pipeline_event(&state, &slug, &path, "put", &metrics, &headers);
    }

    // CO-242: route binary uploads through the asset + entries writer.
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    if is_binary_vault_content(ct) {
        return put_vault_binary(&state, &slug, &path, ct, body).await;
    }

    let body = String::from_utf8(body.to_vec()).map_err(|_| {
        AppError::BadRequest(
            "Vault PUT: body is not valid UTF-8; set Content-Type for binary uploads".into(),
        )
    })?;

    // CO-245: write plaintext code files (.py, .rs, .ts, etc.) verbatim —
    // no markdown frontmatter wrapper — and index them as `asset.code` entries
    // so the browser can offer inline editing with syntax highlighting.
    if is_code_file(&path) {
        write_raw_vault_file(&state, &slug, &path, &body)?;
        let filename = path.split('/').next_back().unwrap_or("").to_string();
        let mime = code_file_mime(&path);
        let size_bytes = body.len() as i64;
        let fm = serde_json::json!({
            "type": "asset.code",
            "mime": mime,
            "filename": filename,
            "size_bytes": size_bytes,
        });
        let entry = make_entry(&path, fm.clone(), &body);
        // Scope the mutex guard so it is dropped before the first .await below.
        let row = {
            let uc = {
                let storage = lock_storage(&state);
                storage
                    .get_universe(&slug)
                    .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
                storage.universe_conn(&slug)
            };
            let repo = crate::repository::SqliteEntryRepository::new(uc);
            repo.upsert_synced(&slug, &entry)
                .map_err(|e| AppError::Internal(e.to_string()))?;
            repo.get(&slug, &path)
                .map_err(|e| AppError::Internal(e.to_string()))?
                .ok_or_else(|| AppError::Internal("code entry vanished after upsert".into()))?
        };
        crate::telemetry::emit_crud_event(
            &state,
            crate::telemetry::CrudEvent {
                kind: "entry.upsert",
                universe: slug.clone(),
                list: Some("asset.code".to_string()),
                key: Some(path.clone()),
                actor: crate::auth::resolve_user_id(&state, &headers),
                session_id: crate::telemetry::extract_session_id(&headers),
                extra: None,
            },
        );
        state
            .index
            .cache
            .query
            .invalidate_prefix(&format!("{slug}:"));
        crate::sync_ws::emit_rest_upsert(&state, &slug, &path, &body).await;
        let stat = make_stat(
            row.created_at.as_deref(),
            row.updated_at.as_deref(),
            body.len(),
        );
        let tags = extract_tags(&fm);
        return Ok((
            StatusCode::CREATED,
            Json(VaultFile {
                path,
                content: body,
                frontmatter: fm,
                tags,
                stat,
            }),
        )
            .into_response());
    }

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
    state
        .index
        .cache
        .query
        .invalidate_prefix(&format!("{slug}:"));

    // CO-71 + CO-74: when `_universe.yaml` is updated, apply manifest indexes and
    // backfill typed FK relations for affected content types — both in background.
    if path == co::manifest::MANIFEST_FILENAME {
        // CO-79: invalidate manifest cache so the next read picks up the new schema.
        state.index.cache.invalidate_universe(&slug);
        let db_path = Some(state.core.storage.lock().data_dir.join("co.db"));
        let universe_root = Some(lock_storage(&state).universe_root(&slug));
        let universe_pool = Some(state.core.storage.lock().universe_pool.clone());
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
            let storage = lock_storage(&state);
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
            storage.universe_conn(&slug)
        };
        match crate::repository::SqliteEntryRepository::new(uc)
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
            let storage = lock_storage(&state);
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
            storage.universe_conn(&slug)
        };
        let row = crate::repository::SqliteEntryRepository::new(uc)
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
            let storage = lock_storage(&state);
            storage
                .get_universe(&slug)
                .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
            storage.universe_conn(&slug)
        };
        let exists = crate::repository::SqliteEntryRepository::new(uc)
            .get(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .is_some();
        let universe_root = {
            let storage = lock_storage(&state);
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

    // Remove from index — entries remove + delete log (CO-95 tx), relations,
    // references_meta in one lock scope (CO-432: via the entry repository).
    {
        let uc = {
            let storage = lock_storage(&state);
            storage.universe_conn(&slug)
        };
        crate::repository::SqliteEntryRepository::new(uc)
            .unindex_vault_entry(&slug, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    lock_storage(&state).decrement_universe_content_count(&slug, 1);

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
        let storage = lock_storage(&state);
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let rows = crate::repository::SqliteEntryRepository::new(uc)
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
        let storage = lock_storage(&state);
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_conn(&slug)
    };
    let tags = crate::repository::SqliteEntryRepository::new(uc)
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
        let storage = lock_storage(&state);
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
        let storage = lock_storage(&state);
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
    let storage = lock_storage(&state);
    // CO-448: an explicit (non-empty) scope mints a least-privilege token; an
    // empty list keeps the legacy NULL-scope (inherit-tier) behavior.
    let tok = if req.scopes.is_empty() {
        storage
            .create_api_token(&user_id.0, &req.name)
            .map_err(|e| AppError::Internal(e.to_string()))?
    } else {
        let resolved =
            crate::auth::capabilities::resolve_scopes(&req.scopes).map_err(|invalid| {
                AppError::BadRequest(format!("unknown capability/bundle: {}", invalid.join(", ")))
            })?;
        storage
            .create_api_token_with_scopes(&user_id.0, &req.name, &resolved)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };

    Ok((
        StatusCode::CREATED,
        Json(CreateTokenResponse {
            id: tok.id,
            name: tok.name,
            token: tok.token.unwrap_or_default(),
            expires_at: tok.expires_at,
            scopes: tok.scopes,
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
    let storage = lock_storage(&state);
    let tokens = storage
        .list_api_tokens(&user_id.0)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let infos: Vec<TokenInfo> = tokens
        .into_iter()
        .map(|t| TokenInfo {
            id: t.id,
            name: t.name,
            token_prefix: t.token_prefix,
            created_at: t.created_at,
            expires_at: t.expires_at,
            last_used_at: t.last_used_at,
            scopes: t.scopes,
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
    let storage = lock_storage(&state);
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
    // CO-242: vault PUT accepts binary blobs up to 50 MB (same cap as asset
    // uploads). Override the global 1 MB DefaultBodyLimit for this router.
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
        .layer(axum::extract::DefaultBodyLimit::max(
            crate::asset_routes::MAX_ASSET_BYTES,
        ))
}

/// API token management routes (nested under /v1/auth, JWT-protected).
pub fn token_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/token", post(create_api_token))
        .route("/tokens", get(list_api_tokens))
        .route("/tokens/{id}", delete(revoke_api_token))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::require_auth,
        ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
