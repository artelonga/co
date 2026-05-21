//! CO-146 — Phase 1 of CO-145.
//!
//! Content-addressable binary asset upload + GET endpoint.
//!
//! Storage layout (per universe):
//! ```text
//! data/universes/<aa>/<bb>/<key>/
//!   data.db                      (assets table — see universe_pool.rs)
//!   blobs/<aa>/<bb>/<sha256>     (raw bytes; plaintext in Phase 1)
//! ```
//!
//! Phase 1 is plaintext to unblock the 506 MB quilomboaraucaria upload.
//! CO-148 (Phase 3) wraps every blob in ChaCha20-Poly1305 with a per-universe
//! DEK, and adds `nonce` + `cipher_size` columns to the `assets` table.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::auth::resolve_user_id;
use crate::error::AppError;
use crate::server::AppState;

/// Phase 1 hard cap. Larger files require chunked-AEAD streaming (deferred
/// to CO-149 + CO-148 Phase 6). The cap also keeps memory bounded since we
/// hold the whole body in RAM during sha256 + write.
pub const MAX_ASSET_BYTES: usize = 50 * 1024 * 1024;

#[derive(Serialize)]
pub struct AssetUploadResponse {
    pub sha256: String,
    pub mime: String,
    pub size: i64,
    pub url: String,
}

#[derive(Deserialize)]
pub struct UploadQuery {
    /// Optional original filename. Stored only for display; sha256 is
    /// the content identity.
    pub filename: Option<String>,
}

#[derive(Serialize)]
pub struct AssetMeta {
    pub sha256: String,
    pub mime: String,
    pub size: i64,
    pub filename: Option<String>,
    pub created_at_ns: i64,
    pub created_by: Option<String>,
    pub refcount: i64,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Build the on-disk path for a blob: `<universe-dir>/blobs/<aa>/<bb>/<sha256>`.
fn blob_path(universe_dir: &std::path::Path, sha256: &str) -> PathBuf {
    let aa = &sha256[0..2];
    let bb = &sha256[2..4];
    universe_dir.join("blobs").join(aa).join(bb).join(sha256)
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or_else(|| {
        // 2262-04-11 overflow guard; keep seconds * 1e9 as a fallback.
        chrono::Utc::now().timestamp() * 1_000_000_000
    })
}

/// Map a MIME type to a CO-242 asset entry type.
pub fn mime_to_asset_entry_type(mime: &str) -> &'static str {
    if mime == "application/pdf" {
        "asset.pdf"
    } else if mime.starts_with("image/") {
        "asset.image"
    } else if mime.starts_with("video/") {
        "asset.video"
    } else if mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/javascript"
                | "application/typescript"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
        )
    {
        "asset.code"
    } else {
        "asset.binary"
    }
}

/// Sniff a mime from the bytes (best-effort) or fall back to the supplied
/// `Content-Type` header / `application/octet-stream`.
pub fn detect_mime(bytes: &[u8], content_type: Option<&str>) -> String {
    // Cheap manual sniffing of the most common formats — we don't pull in
    // a new dep for Phase 1. The `Content-Type` header takes precedence
    // because the client knows best for cases like `text/markdown` and
    // `application/json` that share leading bytes with random text.
    if let Some(ct) = content_type
        && !ct.is_empty()
        && ct != "application/octet-stream"
    {
        // Trim parameters like "; charset=utf-8".
        return ct.split(';').next().unwrap_or(ct).trim().to_string();
    }
    let prefix = bytes.iter().take(16).copied().collect::<Vec<_>>();
    if prefix.starts_with(b"\xFF\xD8\xFF") {
        return "image/jpeg".into();
    }
    if prefix.starts_with(b"\x89PNG\r\n\x1A\n") {
        return "image/png".into();
    }
    if prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a") {
        return "image/gif".into();
    }
    if prefix.starts_with(b"RIFF") && prefix.len() >= 12 && &prefix[8..12] == b"WEBP" {
        return "image/webp".into();
    }
    if prefix.starts_with(b"%PDF-") {
        return "application/pdf".into();
    }
    if prefix.starts_with(b"\x00\x00\x00") && prefix.len() >= 8 && &prefix[4..8] == b"ftyp" {
        return "video/mp4".into();
    }
    "application/octet-stream".into()
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// Returns Ok(()) if the caller may upload to this universe.
/// Phase 1 rule: must be logged in AND own the universe (or be a member).
fn require_writer(
    state: &AppState,
    headers: &HeaderMap,
    universe_key: &str,
) -> Result<String, AppError> {
    let user_id = resolve_user_id(state, headers)
        .ok_or_else(|| AppError::Unauthorized("Login required".into()))?;
    let storage = state.core.storage.lock();
    let universe = storage
        .get_universe(universe_key)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
    if universe.owner_id == user_id {
        return Ok(user_id);
    }
    // Member check (universe_members table).
    let is_member: bool = storage
        .conn()
        .query_row(
            "SELECT 1 FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
            rusqlite::params![universe_key, &user_id],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if is_member {
        Ok(user_id)
    } else {
        Err(AppError::Forbidden(
            "Not authorized to upload to this universe".into(),
        ))
    }
}

// check_reader removed — universe read visibility is enforced by
// universe_visibility_gate middleware applied in server::build_router (CO-161).

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/universes/:slug/assets`
///
/// Body: raw bytes. Response: { sha256, mime, size, url }.
/// Idempotent: same bytes → same sha256 → single on-disk blob (existing row reused).
pub async fn upload_asset(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AssetUploadResponse>, AppError> {
    if body.len() > MAX_ASSET_BYTES {
        return Err(AppError::BadRequest(format!(
            "Asset too large ({} > {} bytes); CO-149 Phase 4 will lift this with streaming",
            body.len(),
            MAX_ASSET_BYTES
        )));
    }

    let user_id = require_writer(&state, &headers, &universe_key)?;

    // sha256 of the bytes — content identity.
    let mut hasher = Sha256::new();
    hasher.update(&body);
    let sha256 = hex::encode_lower(hasher.finalize());

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let mime = detect_mime(&body, content_type);

    let (universe_dir, conn) = {
        let storage = state.core.storage.lock();
        (
            storage.universe_pool.universe_dir(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };

    let blob_path = blob_path(&universe_dir, &sha256);

    // Idempotency: if the row exists we trust the on-disk blob too. If the
    // row is missing but the file is present (rare; partial write) we'll
    // overwrite both atomically below.
    let existing: Option<(String, i64)> = {
        let guard = conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        guard
            .query_row(
                "SELECT mime, size_bytes FROM assets WHERE sha256 = ?1",
                rusqlite::params![&sha256],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .ok()
    };

    if let Some((existing_mime, existing_size)) = existing
        && blob_path.exists()
    {
        return Ok(Json(AssetUploadResponse {
            sha256: sha256.clone(),
            mime: existing_mime,
            size: existing_size,
            url: format!("/api/v1/universes/{universe_key}/assets/{sha256}"),
        }));
    }

    if let Some(parent) = blob_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Internal(format!("create blob dir: {e}")))?;
    }

    // CO-148: encrypt at rest. Bytes on disk are ChaCha20-Poly1305 ciphertext;
    // sha256 is still of the plaintext (content identity is content, not envelope).
    let (ciphertext, nonce) = crate::asset_crypto::encrypt_blob(&body, &universe_key, &sha256)
        .map_err(|e| AppError::Internal(format!("encrypt blob: {e}")))?;

    // Atomic write: write to temp then rename (POSIX atomic on the same FS).
    let tmp_path = blob_path.with_extension("tmp");
    std::fs::write(&tmp_path, &ciphertext)
        .map_err(|e| AppError::Internal(format!("write blob: {e}")))?;
    std::fs::rename(&tmp_path, &blob_path)
        .map_err(|e| AppError::Internal(format!("rename blob: {e}")))?;

    let rel_blob_path = format!("blobs/{}/{}/{}", &sha256[0..2], &sha256[2..4], &sha256);
    let size = body.len() as i64;
    let cipher_size = ciphertext.len() as i64;
    let now = now_ns();

    let entry_path = format!("attachments/{sha256}");
    let entry_type = mime_to_asset_entry_type(&mime);
    let now_iso = chrono::Utc::now().to_rfc3339();
    let title = q
        .filename
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| &sha256[..16]);
    let fm_json = serde_json::json!({
        "type": entry_type,
        "mime": mime,
        "asset_sha256": sha256,
        "size_bytes": size,
        "filename": q.filename.as_deref().unwrap_or(""),
    });
    let fm_json_str = fm_json.to_string();

    {
        let guard = conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        // CO-242: wrap asset row + entries row in one transaction.
        guard
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| AppError::Internal(format!("begin asset tx: {e}")))?;
        let tx_result: rusqlite::Result<()> = (|| {
            guard.execute(
                "INSERT OR REPLACE INTO assets \
                 (sha256, blob_path, mime, size_bytes, filename, created_at_ns, created_by, \
                  refcount, nonce, cipher_size, encrypted) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, \
                  COALESCE((SELECT refcount FROM assets WHERE sha256 = ?1), 0), \
                  ?8, ?9, 1)",
                rusqlite::params![
                    &sha256,
                    &rel_blob_path,
                    &mime,
                    size,
                    q.filename.as_deref(),
                    now,
                    &user_id,
                    &nonce[..],
                    cipher_size,
                ],
            )?;
            // CO-242: create (or update) the matching entries row so this asset
            // appears in GET /entries and in the unified content count.
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
                    &entry_path,
                    &universe_key,
                    entry_type,
                    title,
                    &fm_json_str,
                    &now_iso,
                ],
            )?;
            guard
                .execute(
                    "INSERT INTO entries_fts (universe_key, path, title, body) \
                     VALUES (?1, ?2, ?3, '')",
                    rusqlite::params![&universe_key, &entry_path, title],
                )
                .ok();
            Ok(())
        })();
        match tx_result {
            Ok(()) => guard
                .execute_batch("COMMIT")
                .map_err(|e| AppError::Internal(format!("commit asset tx: {e}")))?,
            Err(e) => {
                let _ = guard.execute_batch("ROLLBACK");
                return Err(AppError::Internal(format!("insert asset + entry row: {e}")));
            }
        }
        // CO-242: refresh content_count so the universe page count includes assets.
        let actual_count: Option<i64> = guard
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                rusqlite::params![&universe_key],
                |row| row.get(0),
            )
            .ok();
        if let Some(n) = actual_count {
            let meta = state.core.storage.lock();
            let _ = meta.conn().execute(
                "UPDATE universes SET content_count = ?1 WHERE key = ?2",
                rusqlite::params![n, &universe_key],
            );
        }
    }

    // CO-156: emit asset.upload telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "asset.upload",
            universe: universe_key.clone(),
            list: None,
            key: Some(sha256.clone()),
            actor: Some(user_id.clone()),
            session_id: crate::telemetry::extract_session_id(&headers),
            extra: Some(serde_json::json!({ "mime": mime, "size_bytes": size })),
        },
    );

    // CO-220: emit AssetUploaded through the event bus so reference-card
    // workers can react without coupling asset_routes to reference_routes.
    state
        .core
        .event_bus
        .publish(crate::events::DomainEvent::AssetUploaded {
            universe_key: universe_key.clone(),
            sha256: sha256.clone(),
            mime: mime.clone(),
            size_bytes: size,
            user_id,
            filename: q.filename.clone(),
        });

    Ok(Json(AssetUploadResponse {
        sha256: sha256.clone(),
        mime,
        size,
        url: format!("/api/v1/universes/{universe_key}/assets/{sha256}"),
    }))
}

/// `GET /api/v1/universes/:slug/assets/:sha256`
///
/// Returns raw bytes with `Content-Type` from the index, immutable cache,
/// and an ETag equal to the sha256. Honors `If-None-Match` for 304s.
pub async fn get_asset(
    State(state): State<AppState>,
    Path((universe_key, sha256)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest("Malformed sha256".into()));
    }

    // Visibility gate enforced by universe_visibility_gate middleware (CO-161).

    let etag = format!("\"{sha256}\"");

    let (universe_dir, conn) = {
        let storage = state.core.storage.lock();
        (
            storage.universe_pool.universe_dir(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };

    // CO-148: row now carries nonce + encrypted flag; nonce is empty for
    // legacy plaintext rows (encrypted=0).
    type Row = (String, String, i64, Option<Vec<u8>>, i64);
    let row: Option<Row> = {
        let guard = conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        guard
            .query_row(
                "SELECT blob_path, mime, size_bytes, nonce, COALESCE(encrypted, 0) \
                 FROM assets WHERE sha256 = ?1",
                rusqlite::params![&sha256],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<Vec<u8>>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .ok()
    };

    let (blob_rel_path, mime, _size, nonce_blob, encrypted) =
        row.ok_or_else(|| AppError::NotFound("Asset".into()))?;

    // CO-145 fix: 304 short-circuit must run AFTER row existence check, not
    // before. Earlier ordering let any valid 64-char hex sha return 304
    // even when the row didn't exist, which broke client-side idempotency
    // probes that GET-with-If-None-Match to test "do you have this blob".
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && inm == etag
    {
        return Ok(Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, &etag)
            .header(
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            )
            .body(axum::body::Body::empty())
            .unwrap());
    }

    let blob_path = universe_dir.join(&blob_rel_path);
    let raw = std::fs::read(&blob_path)
        .map_err(|e| AppError::NotFound(format!("Asset bytes missing: {e}")))?;

    // Phase 1 plaintext rows (encrypted=0) pass through unchanged. Phase 3
    // ciphertext rows decrypt with the per-universe DEK + stored nonce.
    let plaintext = if encrypted == 1 {
        let nonce_vec =
            nonce_blob.ok_or_else(|| AppError::Internal("Encrypted asset missing nonce".into()))?;
        if nonce_vec.len() != 12 {
            return Err(AppError::Internal("Asset nonce wrong length".into()));
        }
        let mut nonce_arr = [0u8; 12];
        nonce_arr.copy_from_slice(&nonce_vec);
        crate::asset_crypto::decrypt_blob(&raw, &nonce_arr, &universe_key, &sha256)
            .map_err(|e| AppError::Internal(format!("decrypt blob: {e}")))?
    } else {
        raw
    };

    // CO-149: HTTP Range support. Parse `Range: bytes=N-M` (or open-ended)
    // and reply 206 with `Content-Range`. Full decrypt is fine here because
    // ChaCha20-Poly1305 verifies over the whole stream — any chunked-AEAD
    // path would change Phase 3's correctness story.
    if let Some(range_hdr) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        let total = plaintext.len();
        if let Some((start, end)) = parse_range(range_hdr, total) {
            if start > end || end >= total {
                return Ok(Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .body(axum::body::Body::empty())
                    .unwrap());
            }
            let slice = plaintext[start..=end].to_vec();
            return Ok(Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, &mime)
                .header(header::CONTENT_LENGTH, slice.len())
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{total}"),
                )
                .header(header::ETAG, etag)
                .header(
                    header::CACHE_CONTROL,
                    "private, max-age=31536000, immutable",
                )
                .header(header::ACCEPT_RANGES, "bytes")
                .body(axum::body::Body::from(slice))
                .unwrap());
        }
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &mime)
        .header(header::CONTENT_LENGTH, plaintext.len())
        .header(header::ETAG, etag)
        .header(
            header::CACHE_CONTROL,
            "private, max-age=31536000, immutable",
        )
        .header(header::ACCEPT_RANGES, "bytes")
        .body(axum::body::Body::from(plaintext))
        .unwrap())
}

/// Parse a single byte-range expression like `bytes=0-1023`, `bytes=10-`,
/// or `bytes=-500`. Returns `(start, end)` inclusive, or `None` if the
/// header is malformed or specifies multiple ranges (we only support one
/// in Phase 4).
fn parse_range(header: &str, total: usize) -> Option<(usize, usize)> {
    let header = header.trim();
    let spec = header.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (s, e) = spec.split_once('-')?;
    let s = s.trim();
    let e = e.trim();
    match (s, e) {
        ("", "") => None,
        ("", suffix) => {
            // bytes=-N → last N bytes
            let n: usize = suffix.parse().ok()?;
            if n == 0 || total == 0 {
                return None;
            }
            let n = n.min(total);
            Some((total - n, total - 1))
        }
        (start, "") => {
            // bytes=N- → from N to end
            let s: usize = start.parse().ok()?;
            if total == 0 {
                return None;
            }
            Some((s, total - 1))
        }
        (start, end) => {
            let s: usize = start.parse().ok()?;
            let e: usize = end.parse().ok()?;
            Some((s, e))
        }
    }
}

/// `DELETE /api/v1/universes/:slug/assets/:sha256`
///
/// Removes the asset row and unlinks the blob if `refcount == 0`.
/// Returns 409 if the asset is still referenced.
pub async fn delete_asset(
    State(state): State<AppState>,
    Path((universe_key, sha256)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest("Malformed sha256".into()));
    }
    require_writer(&state, &headers, &universe_key)?;

    let (universe_dir, conn) = {
        let storage = state.core.storage.lock();
        (
            storage.universe_pool.universe_dir(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };

    let row: Option<(String, i64)> = {
        let guard = conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        guard
            .query_row(
                "SELECT blob_path, refcount FROM assets WHERE sha256 = ?1",
                rusqlite::params![&sha256],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .ok()
    };

    let (blob_rel_path, refcount) = row.ok_or_else(|| AppError::NotFound("Asset".into()))?;
    if refcount > 0 {
        return Err(AppError::Conflict(format!(
            "Asset still referenced ({refcount} entries); refcount must be 0 to delete"
        )));
    }

    {
        let guard = conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        guard
            .execute(
                "DELETE FROM assets WHERE sha256 = ?1",
                rusqlite::params![&sha256],
            )
            .map_err(|e| AppError::Internal(format!("delete asset row: {e}")))?;
    }

    let blob_path = universe_dir.join(&blob_rel_path);
    let _ = std::fs::remove_file(&blob_path); // best-effort

    // CO-156: emit asset.delete telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "asset.delete",
            universe: universe_key.clone(),
            list: None,
            key: Some(sha256.clone()),
            actor: require_writer(&state, &headers, &universe_key).ok(),
            session_id: crate::telemetry::extract_session_id(&headers),
            extra: None,
        },
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Optional filters for the asset list endpoint.
#[derive(Deserialize)]
pub struct ListAssetsQuery {
    /// MIME prefix filter, e.g. `image/` returns only image assets.
    pub mime: Option<String>,
    /// Filename substring search (case-insensitive contains).
    pub search: Option<String>,
    /// Tag filter — only assets carrying this tag.
    pub tag: Option<String>,
}

#[derive(Serialize)]
pub struct AssetListResponse {
    pub assets: Vec<AssetWithTags>,
    pub total: usize,
}

/// Asset row plus its tag list — what the asset browser UI consumes.
#[derive(Serialize)]
pub struct AssetWithTags {
    #[serde(flatten)]
    pub meta: AssetMeta,
    pub tags: Vec<String>,
}

/// `GET /api/v1/universes/:slug/assets`
///
/// List all assets in the universe. Supports optional `?mime=image/` prefix
/// filter and `?search=filename` substring search.
/// Auth: same read rules as `GET /assets/:sha256`.
pub async fn list_assets(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
    Query(q): Query<ListAssetsQuery>,
) -> Result<Json<AssetListResponse>, AppError> {
    // Visibility gate enforced by universe_visibility_gate middleware (CO-161).

    let conn = {
        let storage = state.core.storage.lock();
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    let mut stmt = guard
        .prepare(
            "SELECT sha256, mime, size_bytes, filename, created_at_ns, created_by, refcount \
             FROM assets \
             ORDER BY created_at_ns DESC",
        )
        .map_err(|e| AppError::Internal(format!("list assets prepare: {e}")))?;

    let metas: Vec<AssetMeta> = stmt
        .query_map([], |row| {
            Ok(AssetMeta {
                sha256: row.get(0)?,
                mime: row.get(1)?,
                size: row.get(2)?,
                filename: row.get(3)?,
                created_at_ns: row.get(4)?,
                created_by: row.get(5)?,
                refcount: row.get(6)?,
            })
        })
        .map_err(|e| AppError::Internal(format!("list assets query: {e}")))?
        .filter_map(|r| r.ok())
        .filter(|a| q.mime.as_deref().is_none_or(|m| a.mime.starts_with(m)))
        .filter(|a| {
            q.search.as_deref().is_none_or(|s| {
                a.filename
                    .as_deref()
                    .is_some_and(|f| f.to_lowercase().contains(&s.to_lowercase()))
                    || a.sha256.starts_with(s)
            })
        })
        .collect();

    // Per-asset tag fetch in one pass.
    let mut tag_stmt = guard
        .prepare("SELECT tag FROM asset_tags WHERE sha256 = ?1 ORDER BY tag")
        .map_err(|e| AppError::Internal(format!("tag fetch prepare: {e}")))?;

    let mut assets: Vec<AssetWithTags> = Vec::with_capacity(metas.len());
    for meta in metas {
        let tags: Vec<String> = tag_stmt
            .query_map(rusqlite::params![&meta.sha256], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        if let Some(want) = q.tag.as_deref()
            && !tags.iter().any(|t| t == want)
        {
            continue;
        }
        assets.push(AssetWithTags { meta, tags });
    }
    drop(tag_stmt);

    let total = assets.len();
    Ok(Json(AssetListResponse { assets, total }))
}

#[derive(Serialize)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

/// `GET /api/v1/universes/:slug/assets/tags` — tag counts across all assets.
pub async fn list_tags(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
) -> Result<Json<Vec<TagCount>>, AppError> {
    // Visibility gate enforced by universe_visibility_gate middleware (CO-161).
    let conn = {
        let storage = state.core.storage.lock();
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let mut stmt = guard
        .prepare(
            "SELECT tag, COUNT(*) FROM asset_tags GROUP BY tag ORDER BY COUNT(*) DESC, tag ASC",
        )
        .map_err(|e| AppError::Internal(format!("tag list prepare: {e}")))?;
    let tags: Vec<TagCount> = stmt
        .query_map([], |row| {
            Ok(TagCount {
                tag: row.get(0)?,
                count: row.get(1)?,
            })
        })
        .map_err(|e| AppError::Internal(format!("tag list query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(Json(tags))
}

#[derive(Deserialize)]
pub struct TagAddRequest {
    pub tags: Vec<String>,
}

/// `POST /api/v1/universes/:slug/assets/:sha256/tags` — add one or more tags.
pub async fn add_tags(
    State(state): State<AppState>,
    Path((universe_key, sha256)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<TagAddRequest>,
) -> Result<StatusCode, AppError> {
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest("Malformed sha256".into()));
    }
    require_writer(&state, &headers, &universe_key)?;

    let conn = {
        let storage = state.core.storage.lock();
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    // Asset must exist (FK would catch it, but a clear 404 is friendlier).
    let exists: bool = guard
        .query_row(
            "SELECT 1 FROM assets WHERE sha256 = ?1",
            rusqlite::params![&sha256],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !exists {
        return Err(AppError::NotFound("Asset".into()));
    }

    for tag in &req.tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            continue;
        }
        guard
            .execute(
                "INSERT OR IGNORE INTO asset_tags (sha256, tag) VALUES (?1, ?2)",
                rusqlite::params![&sha256, trimmed],
            )
            .map_err(|e| AppError::Internal(format!("insert tag: {e}")))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/universes/:slug/assets/:sha256/tags/:tag` — remove one tag.
pub async fn remove_tag(
    State(state): State<AppState>,
    Path((universe_key, sha256, tag)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest("Malformed sha256".into()));
    }
    require_writer(&state, &headers, &universe_key)?;

    let conn = {
        let storage = state.core.storage.lock();
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    guard
        .execute(
            "DELETE FROM asset_tags WHERE sha256 = ?1 AND tag = ?2",
            rusqlite::params![&sha256, &tag],
        )
        .map_err(|e| AppError::Internal(format!("delete tag: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn asset_router() -> Router<AppState> {
    use axum::extract::DefaultBodyLimit;
    use axum::routing::{delete as del, post};
    // CO-145 / CO-146: asset uploads carry binary bodies up to 50 MB —
    // axum's 2 MB default body limit (and the 1 MB cap applied to other
    // routers in server.rs) would 413 every full-resolution image. Raise
    // the limit on this router specifically; MAX_ASSET_BYTES still gates
    // the handler so the cap stays consistent with what tests verify.
    Router::new()
        .route("/{slug}/assets", get(list_assets).post(upload_asset))
        .route("/{slug}/assets/tags", get(list_tags))
        .route(
            "/{slug}/assets/{sha256}",
            get(get_asset).delete(delete_asset),
        )
        .route("/{slug}/assets/{sha256}/tags", post(add_tags))
        .route("/{slug}/assets/{sha256}/tags/{tag}", del(remove_tag))
        // Raw blob: serve any file from the universe directory by relative path.
        // Used by the PDF viewer when a reference card has `file:` but no blob_sha256.
        // Visibility gate already applied (universe_content_api layer).
        .route("/{slug}/blob/{*path}", get(get_raw_blob))
        .layer(DefaultBodyLimit::max(MAX_ASSET_BYTES))
}

// ---------------------------------------------------------------------------
// Raw blob endpoint
// ---------------------------------------------------------------------------

/// `GET /api/v1/universes/:slug/blob/*path`
///
/// Serve an arbitrary file from the universe directory. Intended for binary
/// siblings of reference cards (PDFs, images) that were never uploaded as
/// content-addressed assets (no `blob_sha256`). Visibility gate is applied
/// by the outer `universe_content_api` layer.
pub async fn get_raw_blob(
    axum::extract::State(state): axum::extract::State<crate::server::AppState>,
    axum::extract::Path((slug, path)): axum::extract::Path<(String, String)>,
) -> Result<axum::response::Response, AppError> {
    let universe_root = {
        let storage = state.core.storage.lock();
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_root(&slug)
    };

    // Reject obvious path traversal attempts before canonicalization.
    if path.contains("..") {
        return Err(AppError::Forbidden("Path traversal rejected".into()));
    }

    let file_path = universe_root.join(&path);
    // Canonicalize to resolve symlinks, then verify we're still inside universe_root.
    let canonical = file_path
        .canonicalize()
        .map_err(|_| AppError::NotFound(format!("File '{path}' not found")))?;
    if !canonical.starts_with(&universe_root) {
        return Err(AppError::Forbidden("Path escapes universe root".into()));
    }

    let bytes =
        std::fs::read(&canonical).map_err(|e| AppError::NotFound(format!("Read '{path}': {e}")))?;

    // Determine MIME from extension, defaulting to octet-stream.
    let ext = canonical.extension().and_then(|s| s.to_str()).unwrap_or("");
    let mime = match ext {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };

    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(mime)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("application/octet-stream")),
    );
    // 1-hour cache (not immutable — file content can change without a new sha256 key)
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, max-age=3600"),
    );
    Ok((StatusCode::OK, headers, bytes).into_response())
}

// ---------------------------------------------------------------------------
// Hex helper (avoids new dep)
// ---------------------------------------------------------------------------

mod hex {
    pub fn encode_lower(bytes: impl AsRef<[u8]>) -> String {
        const ALPH: &[u8; 16] = b"0123456789abcdef";
        let bytes = bytes.as_ref();
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(ALPH[(b >> 4) as usize] as char);
            out.push(ALPH[(b & 0xf) as usize] as char);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let bytes = [0x12u8, 0x34, 0xab, 0xcd, 0xef];
        assert_eq!(hex::encode_lower(bytes), "1234abcdef");
    }

    #[test]
    fn detect_mime_png() {
        let png = b"\x89PNG\r\n\x1A\n\x00\x00\x00";
        assert_eq!(detect_mime(png, None), "image/png");
    }

    #[test]
    fn detect_mime_jpeg() {
        let jpg = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
        assert_eq!(detect_mime(jpg, None), "image/jpeg");
    }

    #[test]
    fn detect_mime_header_wins() {
        let bytes = b"hello world";
        assert_eq!(detect_mime(bytes, Some("text/markdown")), "text/markdown");
        assert_eq!(
            detect_mime(bytes, Some("text/markdown; charset=utf-8")),
            "text/markdown"
        );
    }

    #[test]
    fn detect_mime_octet_default() {
        assert_eq!(
            detect_mime(b"random binary", None),
            "application/octet-stream"
        );
    }

    #[test]
    fn blob_path_shards_correctly() {
        let dir = std::path::Path::new("/data/u/68/b5/foo");
        let p = blob_path(
            dir,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        );
        assert_eq!(
            p,
            std::path::Path::new(
                "/data/u/68/b5/foo/blobs/ab/cd/abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
            )
        );
    }
}
