//! CO-156: `reference` content type — metadata cards for PDF/image/video/audio/web assets.
//!
//! A reference card is a `.md` file with `type: reference` in its frontmatter.
//! It carries metadata about a bound binary asset (sibling file) or external URL.
//!
//! Routes (nested under `/api/v1/universes/{u}`):
//!   GET    /references                — list cards (filter: medium, seed_status, q=fts)
//!   POST   /references                — create a new card
//!   GET    /references/orphan-blobs   — assets with no card
//!   GET    /references/broken-cards   — cards whose `file:` doesn't resolve
//!   GET    /references/{*path}        — read one card
//!   PUT    /references/{*path}        — update card
//!   DELETE /references/{*path}        — delete card (blob unaffected)

use std::path::Path as FsPath;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Minimal hex encoder (avoids adding a dep; mirrors the one in asset_routes).
fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const ALPH: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(ALPH[(b >> 4) as usize] as char);
        out.push(ALPH[(b & 0xf) as usize] as char);
    }
    out
}

use crate::auth::resolve_user_id;
use crate::entry_index::{EntryIndex, make_entry};
use crate::error::AppError;
use crate::server::AppState;
use crate::telemetry::{CrudEvent, emit_crud_event, extract_session_id};

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

/// A row from `references_meta` joined with the entry title.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceCard {
    pub universe_key: String,
    pub entry_path: String,
    pub edition_id: String,
    pub work_id: String,
    pub primary_layer: Option<i64>,
    pub file: Option<String>,
    pub blob_sha256: Option<String>,
    pub url: Option<String>,
    pub medium: String,
    pub mime: Option<String>,
    pub size_bytes: Option<i64>,
    pub language: Option<String>,
    pub seed_status: String,
    pub indexed_at: String,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListRefsQuery {
    pub medium: Option<String>,
    pub seed_status: Option<String>,
    /// Filter by conceptual work identity (returns all editions of that work).
    pub work_id: Option<String>,
    /// Filter by minimum source-chain layer (0=phenomenon, 1=transcription, …).
    pub primary_layer: Option<i64>,
    /// Full-text search across title, body, transcription.
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRefBody {
    pub path: String,
    pub frontmatter: serde_json::Value,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRefBody {
    pub frontmatter: Option<serde_json::Value>,
    pub body: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OrphanBlob {
    pub sha256: String,
    pub mime: String,
    pub size_bytes: i64,
    pub filename: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BrokenCard {
    pub entry_path: String,
    pub file: String,
    pub expected_path: String,
}

// ---------------------------------------------------------------------------
// Inner sync helpers — pub(crate) for use by entry_routes and vault_routes
// ---------------------------------------------------------------------------

/// Called after every entry write. Upserts `references_meta` iff `entry_type == "reference"`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn maybe_sync_reference_meta(
    conn: &Connection,
    universe_key: &str,
    entry_path: &str,
    entry_type: &str,
    frontmatter: &serde_json::Value,
    body: &str,
    title: Option<&str>,
    universe_root: &std::path::Path,
) {
    if entry_type != "reference" {
        return;
    }
    upsert_reference_meta(
        conn,
        universe_key,
        entry_path,
        frontmatter,
        body,
        title,
        universe_root,
    );
}

/// Upsert `references_meta` + `reference_cards_fts` for a reference card.
///
/// If the frontmatter carries an `editions:` array, one row per edition is
/// written (replacing any previous set for this entry_path). Otherwise a
/// single row with `edition_id = "default"` is written.
///
/// Duplicate sha256 detection: if an edition's blob_sha256 already exists for
/// the same `work_id` under a *different* entry_path, the duplicate edition
/// row is skipped (the existing row already represents that artifact).
fn upsert_reference_meta(
    conn: &Connection,
    universe_key: &str,
    entry_path: &str,
    frontmatter: &serde_json::Value,
    body: &str,
    title: Option<&str>,
    universe_root: &std::path::Path,
) {
    let work_id = frontmatter
        .get("work_id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| work_id_from_path(entry_path));

    let primary_layer: Option<i64> = frontmatter
        .get("primary_source_chain")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|item| item.get("layer").and_then(|l| l.as_i64()))
                .min()
        });

    let top_medium = frontmatter
        .get("medium")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let top_mime = frontmatter
        .get("mime")
        .and_then(|v| v.as_str())
        .map(String::from);
    let top_size_bytes = frontmatter.get("size_bytes").and_then(|v| v.as_i64());
    let top_language = frontmatter
        .get("language")
        .and_then(|v| v.as_str())
        .map(String::from);
    let transcription = frontmatter
        .get("transcription")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Delete all existing edition rows for this (universe_key, entry_path).
    // This cleanly handles edition additions, removals, and renames on update.
    conn.execute(
        "DELETE FROM references_meta WHERE universe_key = ?1 AND entry_path = ?2",
        params![universe_key, entry_path],
    )
    .ok();

    if let Some(serde_json::Value::Array(editions)) = frontmatter.get("editions") {
        // Multi-edition card: one row per edition entry.
        for edition in editions {
            let edition_id = edition
                .get("edition_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let file = edition
                .get("file")
                .and_then(|v| if v.is_null() { None } else { v.as_str() })
                .map(String::from);
            let url = edition
                .get("url")
                .and_then(|v| if v.is_null() { None } else { v.as_str() })
                .map(String::from);
            let language = edition
                .get("language")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| top_language.clone());
            let mime = edition
                .get("mime")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| top_mime.clone());
            let size_bytes = edition
                .get("size_bytes")
                .or_else(|| edition.get("pages"))
                .and_then(|v| v.as_i64())
                .or(top_size_bytes);
            let seed_status_raw = edition
                .get("seed_status")
                .and_then(|v| v.as_str())
                .unwrap_or("stub")
                .to_string();

            // sha256: prefer explicit field, then compute from disk.
            let blob_sha256 = edition
                .get("sha256")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| compute_blob_sha256(entry_path, file.as_deref(), universe_root));

            // Duplicate detection: skip if same sha256 already exists for
            // this work_id under a different entry_path.
            if let Some(ref sha) = blob_sha256 {
                let dup: Option<String> = conn
                    .query_row(
                        "SELECT entry_path FROM references_meta \
                         WHERE universe_key = ?1 AND work_id = ?2 \
                           AND blob_sha256 = ?3 AND entry_path != ?4 LIMIT 1",
                        params![universe_key, work_id, sha, entry_path],
                        |r| r.get(0),
                    )
                    .ok();
                if dup.is_some() {
                    continue;
                }
            }

            let seed_status = if file.is_some() && blob_sha256.is_none() {
                "stub".to_string()
            } else {
                seed_status_raw
            };

            conn.execute(
                "INSERT OR REPLACE INTO references_meta \
                 (universe_key, entry_path, edition_id, work_id, primary_layer, \
                  file, blob_sha256, url, medium, mime, size_bytes, language, \
                  seed_status, indexed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))",
                params![
                    universe_key,
                    entry_path,
                    edition_id,
                    work_id,
                    primary_layer,
                    file,
                    blob_sha256,
                    url,
                    top_medium,
                    mime,
                    size_bytes,
                    language,
                    seed_status,
                ],
            )
            .ok();
        }
    } else {
        // Single-edition card: one row with edition_id = "default".
        let file = frontmatter
            .get("file")
            .and_then(|v| v.as_str())
            .map(String::from);
        let url = frontmatter
            .get("url")
            .and_then(|v| v.as_str())
            .map(String::from);
        let seed_status_raw = frontmatter
            .get("seed_status")
            .and_then(|v| v.as_str())
            .unwrap_or("stub")
            .to_string();

        let blob_sha256 = compute_blob_sha256(entry_path, file.as_deref(), universe_root);

        // Duplicate detection.
        let is_dup = blob_sha256.as_deref().is_some_and(|sha| {
            conn.query_row(
                "SELECT 1 FROM references_meta \
                 WHERE universe_key = ?1 AND work_id = ?2 \
                   AND blob_sha256 = ?3 AND entry_path != ?4 LIMIT 1",
                params![universe_key, work_id, sha, entry_path],
                |_| Ok(true),
            )
            .unwrap_or(false)
        });

        if !is_dup {
            let seed_status = if file.is_some() && blob_sha256.is_none() {
                "stub".to_string()
            } else {
                seed_status_raw
            };

            conn.execute(
                "INSERT OR REPLACE INTO references_meta \
                 (universe_key, entry_path, edition_id, work_id, primary_layer, \
                  file, blob_sha256, url, medium, mime, size_bytes, language, \
                  seed_status, indexed_at) \
                 VALUES (?1, ?2, 'default', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'))",
                params![
                    universe_key,
                    entry_path,
                    work_id,
                    primary_layer,
                    file,
                    blob_sha256,
                    url,
                    top_medium,
                    top_mime,
                    top_size_bytes,
                    top_language,
                    seed_status,
                ],
            )
            .ok();
        }
    }

    // FTS: one row per card (not per edition) — delete + reinsert.
    conn.execute(
        "DELETE FROM reference_cards_fts WHERE universe_key = ?1 AND entry_path = ?2",
        params![universe_key, entry_path],
    )
    .ok();
    conn.execute(
        "INSERT INTO reference_cards_fts (universe_key, entry_path, title, body, transcription) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            universe_key,
            entry_path,
            title.unwrap_or(""),
            body,
            transcription,
        ],
    )
    .ok();
}

/// Remove `references_meta` + `reference_cards_fts` for an entry (idempotent — safe even if no row).
pub(crate) fn remove_reference_meta(conn: &Connection, universe_key: &str, entry_path: &str) {
    conn.execute(
        "DELETE FROM references_meta WHERE universe_key = ?1 AND entry_path = ?2",
        params![universe_key, entry_path],
    )
    .ok();
    conn.execute(
        "DELETE FROM reference_cards_fts WHERE universe_key = ?1 AND entry_path = ?2",
        params![universe_key, entry_path],
    )
    .ok();
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

fn require_writer(
    state: &AppState,
    headers: &HeaderMap,
    universe_key: &str,
) -> Result<String, AppError> {
    let user_id = resolve_user_id(state, headers)
        .ok_or_else(|| AppError::Unauthorized("Login required".into()))?;
    let storage = lock_storage(state)?;
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

fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReferenceCard> {
    Ok(ReferenceCard {
        universe_key: row.get(0)?,
        entry_path: row.get(1)?,
        edition_id: row.get(2)?,
        work_id: row.get(3)?,
        primary_layer: row.get(4)?,
        file: row.get(5)?,
        blob_sha256: row.get(6)?,
        url: row.get(7)?,
        medium: row.get(8)?,
        mime: row.get(9)?,
        size_bytes: row.get(10)?,
        language: row.get(11)?,
        seed_status: row.get(12)?,
        indexed_at: row.get(13)?,
        title: row.get(14)?,
    })
}

/// Derive a work_id slug from an entry path: take the filename stem.
/// e.g. "refs/GNDicLex.md" → "GNDicLex"
fn work_id_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Compute sha256 of a sibling asset file given the card's entry_path and file name.
fn compute_blob_sha256(
    entry_path: &str,
    file: Option<&str>,
    universe_root: &std::path::Path,
) -> Option<String> {
    let f = file?;
    let entry_dir = std::path::Path::new(entry_path).parent()?;
    let file_path = universe_root.join(entry_dir).join(f);
    let bytes = std::fs::read(&file_path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(hex_encode(hasher.finalize()))
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
    let conn = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    const REFS_SELECT: &str = "SELECT rm.universe_key, rm.entry_path, rm.edition_id, rm.work_id, rm.primary_layer, \
                rm.file, rm.blob_sha256, rm.url, rm.medium, rm.mime, rm.size_bytes, \
                rm.language, rm.seed_status, rm.indexed_at, e.title \
         FROM references_meta rm \
         LEFT JOIN entries e \
           ON e.universe_key = rm.universe_key AND e.path = rm.entry_path";

    let cards = if let Some(ref fts_query) = q.q {
        // FTS path: join with reference_cards_fts
        let fts_sql = format!(
            "{REFS_SELECT} \
             JOIN reference_cards_fts fts \
               ON fts.universe_key = rm.universe_key AND fts.entry_path = rm.entry_path \
             WHERE fts.universe_key = ?1 AND reference_cards_fts MATCH ?2 \
             ORDER BY rm.entry_path, rm.edition_id LIMIT 200"
        );
        let mut stmt = guard
            .prepare(&fts_sql)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        stmt.query_map(params![universe_key, fts_query], row_to_card)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        // Structured filter path
        let mut sql = format!("{REFS_SELECT} WHERE rm.universe_key = ?1");
        let mut bind_vals: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(universe_key.clone())];

        if let Some(ref m) = q.medium {
            bind_vals.push(Box::new(m.clone()));
            sql.push_str(&format!(" AND rm.medium = ?{}", bind_vals.len()));
        }
        if let Some(ref s) = q.seed_status {
            bind_vals.push(Box::new(s.clone()));
            sql.push_str(&format!(" AND rm.seed_status = ?{}", bind_vals.len()));
        }
        if let Some(ref w) = q.work_id {
            bind_vals.push(Box::new(w.clone()));
            sql.push_str(&format!(" AND rm.work_id = ?{}", bind_vals.len()));
        }
        if let Some(pl) = q.primary_layer {
            bind_vals.push(Box::new(pl));
            sql.push_str(&format!(" AND rm.primary_layer = ?{}", bind_vals.len()));
        }
        sql.push_str(" ORDER BY rm.entry_path, rm.edition_id LIMIT 500");

        let params_refs: Vec<&dyn rusqlite::types::ToSql> = bind_vals
            .iter()
            .map(|b| b.as_ref() as &dyn rusqlite::types::ToSql)
            .collect();
        let mut stmt = guard
            .prepare(&sql)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        stmt.query_map(params_refs.as_slice(), row_to_card)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect()
    };

    Ok(Json(cards))
}

/// GET /api/v1/universes/{u}/references/orphan-blobs
///
/// List assets that have no corresponding reference card in `references_meta`.
pub async fn orphan_blobs(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
) -> Result<Json<Vec<OrphanBlob>>, AppError> {
    let conn = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    let mut stmt = guard
        .prepare(
            "SELECT a.sha256, a.mime, a.size_bytes, a.filename \
             FROM assets a \
             WHERE NOT EXISTS ( \
               SELECT 1 FROM references_meta rm \
               WHERE rm.universe_key = ?1 AND rm.blob_sha256 = a.sha256 \
             ) \
             ORDER BY a.sha256 LIMIT 500",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let blobs: Vec<OrphanBlob> = stmt
        .query_map(params![universe_key], |row| {
            Ok(OrphanBlob {
                sha256: row.get(0)?,
                mime: row.get(1)?,
                size_bytes: row.get(2)?,
                filename: row.get(3)?,
            })
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
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
    let (conn, universe_root) = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        (
            storage.universe_conn(&universe_key),
            storage.universe_root(&universe_key),
        )
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    let mut stmt = guard
        .prepare(
            "SELECT entry_path, file FROM references_meta \
             WHERE universe_key = ?1 AND file IS NOT NULL",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![universe_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let broken: Vec<BrokenCard> = rows
        .into_iter()
        .filter_map(|(entry_path, file)| {
            let entry_dir = FsPath::new(&entry_path).parent()?;
            let file_path = universe_root.join(entry_dir).join(&file);
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
    let conn = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    // Returns the canonical edition (or the first one if no canonical_edition set).
    let card = guard
        .query_row(
            "SELECT rm.universe_key, rm.entry_path, rm.edition_id, rm.work_id, rm.primary_layer, \
                    rm.file, rm.blob_sha256, rm.url, rm.medium, rm.mime, rm.size_bytes, \
                    rm.language, rm.seed_status, rm.indexed_at, e.title \
             FROM references_meta rm \
             LEFT JOIN entries e \
               ON e.universe_key = rm.universe_key AND e.path = rm.entry_path \
             WHERE rm.universe_key = ?1 AND rm.entry_path = ?2 \
             ORDER BY CASE rm.edition_id WHEN 'default' THEN 0 ELSE 1 END, rm.edition_id \
             LIMIT 1",
            params![universe_key, path],
            row_to_card,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::NotFound(format!("Reference card '{path}' not found"))
            }
            other => AppError::Internal(other.to_string()),
        })?;

    Ok(Json(card))
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
    // Force entry_type = reference
    if let Some(obj) = fm.as_object_mut() {
        obj.insert(
            "type".to_string(),
            serde_json::Value::String("reference".to_string()),
        );
    }

    let (universe_root, universe_conn) = {
        let storage = lock_storage(&state)?;
        (
            storage.universe_root(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };

    let entry = make_entry(&body.path, fm.clone(), &body.body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    let title = fm.get("title").and_then(|v| v.as_str());
    {
        let guard = universe_conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&guard);
        index
            .upsert(&universe_key, &entry)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        upsert_reference_meta(
            &guard,
            &universe_key,
            &body.path,
            &fm,
            &body.body,
            title,
            &universe_root,
        );
    }

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
    lock_storage(&state)?.increment_universe_content_count(&universe_key);

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
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        (
            storage.universe_root(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };

    // Read existing entry
    let existing = {
        let guard = universe_conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&guard);
        index
            .get(&universe_key, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .ok_or_else(|| AppError::NotFound(format!("Reference card '{path}' not found")))?
    };

    let new_fm = body.frontmatter.unwrap_or(existing.frontmatter.clone());
    let new_body = body.body.unwrap_or(existing.body.clone());

    let entry = make_entry(&path, new_fm.clone(), &new_body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    let title = new_fm.get("title").and_then(|v| v.as_str());
    {
        let guard = universe_conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&guard);
        index
            .upsert(&universe_key, &entry)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        upsert_reference_meta(
            &guard,
            &universe_key,
            &path,
            &new_fm,
            &new_body,
            title,
            &universe_root,
        );
    }

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
    let conn = {
        let storage = lock_storage(&state)?;
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let card = guard
        .query_row(
            "SELECT rm.universe_key, rm.entry_path, rm.edition_id, rm.work_id, rm.primary_layer, \
                    rm.file, rm.blob_sha256, rm.url, rm.medium, rm.mime, rm.size_bytes, \
                    rm.language, rm.seed_status, rm.indexed_at, e.title \
             FROM references_meta rm \
             LEFT JOIN entries e \
               ON e.universe_key = rm.universe_key AND e.path = rm.entry_path \
             WHERE rm.universe_key = ?1 AND rm.entry_path = ?2 \
             ORDER BY CASE rm.edition_id WHEN 'default' THEN 0 ELSE 1 END, rm.edition_id \
             LIMIT 1",
            params![universe_key, path],
            row_to_card,
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(card))
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
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        (
            storage.universe_root(&universe_key),
            storage.universe_conn(&universe_key),
        )
    };

    co::delete_entry(&universe_root, &path).map_err(|e| AppError::Internal(e.to_string()))?;

    {
        let guard = universe_conn
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = EntryIndex::new(&guard);
        index
            .remove(&universe_key, &path)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        remove_reference_meta(&guard, &universe_key, &path);
    }

    lock_storage(&state)?.decrement_universe_content_count(&universe_key, 1);

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
    let conn = {
        let storage = lock_storage(&state)?;
        storage
            .get_universe(&universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
        storage.universe_conn(&universe_key)
    };
    let guard = conn
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    let mut stmt = guard
        .prepare(
            "SELECT DISTINCT work_id FROM references_meta \
             WHERE universe_key = ?1 AND work_id != '' \
             ORDER BY work_id",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let works: Vec<String> = stmt
        .query_map(params![universe_key], |row| row.get(0))
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();
        conn.execute_batch(crate::universe_pool::UNIVERSE_SCHEMA_FOR_TEST)
            .unwrap();
        crate::universe_pool::run_universe_migrations_for_test(&conn);
        conn
    }

    #[test]
    fn test_upsert_reference_meta_basic() {
        let conn = open_test_db();
        let fm = json!({
            "type": "reference",
            "medium": "pdf",
            "mime": "application/pdf",
            "seed_status": "stub",
        });
        upsert_reference_meta(
            &conn,
            "mbya",
            "refs/dooley.md",
            &fm,
            "some body text",
            Some("Dooley 2006"),
            std::path::Path::new("/nonexistent"),
        );

        let row: (String, String, String) = conn
            .query_row(
                "SELECT medium, mime, seed_status FROM references_meta \
                 WHERE universe_key = 'mbya' AND entry_path = 'refs/dooley.md'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, "pdf");
        assert_eq!(row.1, "application/pdf");
        assert_eq!(row.2, "stub");
    }

    #[test]
    fn test_upsert_enforces_stub_when_file_absent() {
        let conn = open_test_db();
        let fm = json!({
            "type": "reference",
            "medium": "pdf",
            "file": "missing.pdf",
            "seed_status": "reviewed",   // claimed reviewed, but file doesn't exist
        });
        upsert_reference_meta(
            &conn,
            "mbya",
            "refs/missing.md",
            &fm,
            "",
            None,
            std::path::Path::new("/nonexistent"),
        );
        let status: String = conn
            .query_row(
                "SELECT seed_status FROM references_meta WHERE entry_path = 'refs/missing.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "stub",
            "seed_status should be enforced to stub when blob absent"
        );
    }

    #[test]
    fn test_remove_reference_meta_is_idempotent() {
        let conn = open_test_db();
        // Remove from an empty table — must not panic or error.
        remove_reference_meta(&conn, "mbya", "refs/nonexistent.md");

        // Insert then remove.
        let fm = json!({ "type": "reference", "medium": "pdf", "seed_status": "stub" });
        upsert_reference_meta(
            &conn,
            "mbya",
            "refs/a.md",
            &fm,
            "",
            None,
            std::path::Path::new("/x"),
        );
        remove_reference_meta(&conn, "mbya", "refs/a.md");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM references_meta WHERE entry_path = 'refs/a.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_upsert_is_idempotent() {
        let conn = open_test_db();
        let fm = json!({ "type": "reference", "medium": "video", "seed_status": "stub" });
        // Two identical upserts must leave exactly one row.
        upsert_reference_meta(
            &conn,
            "u",
            "r/a.md",
            &fm,
            "",
            None,
            std::path::Path::new("/x"),
        );
        upsert_reference_meta(
            &conn,
            "u",
            "r/a.md",
            &fm,
            "",
            None,
            std::path::Path::new("/x"),
        );
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM references_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_references_tables_exist_after_migration() {
        let conn = open_test_db();
        let has_meta: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='references_meta'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(has_meta, "references_meta table should exist");

        let has_fts: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='reference_cards_fts'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(has_fts, "reference_cards_fts table should exist");
    }

    #[test]
    fn test_schema_version_reaches_8() {
        let conn = open_test_db();
        let v: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap_or(0);
        assert!(
            v >= 8,
            "schema_version should be at least 8 (CO-158), got {v}"
        );
    }

    // --- CO-158 tests ---

    #[test]
    fn test_three_editions_round_trip() {
        let conn = open_test_db();
        let fm = json!({
            "type": "reference",
            "work_id": "ayvu-rapyta-cadogan-1959",
            "medium": "pdf",
            "seed_status": "stub",
            "primary_source_chain": [
                {"layer": 0, "role": "phenomenon"},
                {"layer": 1, "role": "transcription"},
                {"layer": 2, "role": "publication"},
            ],
            "editions": [
                {"edition_id": "usp-1959",    "seed_status": "native-confirmed"},
                {"edition_id": "ucsa-1992",   "seed_status": "reviewed"},
                {"edition_id": "ocr-2010",    "seed_status": "stub", "url": "https://example.com"},
            ],
        });

        upsert_reference_meta(
            &conn,
            "mbya",
            "refs/ayvu-rapyta.md",
            &fm,
            "body text",
            Some("Ayvu Rapyta"),
            std::path::Path::new("/nonexistent"),
        );

        // All 3 editions must be present.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM references_meta \
                 WHERE universe_key = 'mbya' AND entry_path = 'refs/ayvu-rapyta.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3, "expected 3 edition rows in references_meta");

        // All share the same work_id.
        let work_id_count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT work_id) FROM references_meta \
                 WHERE entry_path = 'refs/ayvu-rapyta.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            work_id_count, 1,
            "all editions should share the same work_id"
        );

        let work_id: String = conn
            .query_row(
                "SELECT work_id FROM references_meta WHERE entry_path = 'refs/ayvu-rapyta.md' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(work_id, "ayvu-rapyta-cadogan-1959");

        // primary_layer = min layer in primary_source_chain = 0.
        let layer: Option<i64> = conn
            .query_row(
                "SELECT primary_layer FROM references_meta \
                 WHERE entry_path = 'refs/ayvu-rapyta.md' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            layer,
            Some(0),
            "primary_layer should be min chain layer = 0"
        );
    }

    #[test]
    fn test_work_id_derived_from_path_when_absent() {
        let conn = open_test_db();
        let fm = json!({
            "type": "reference",
            "medium": "pdf",
            "seed_status": "stub",
        });
        upsert_reference_meta(
            &conn,
            "u",
            "refs/GNDicLex.md",
            &fm,
            "",
            None,
            std::path::Path::new("/nonexistent"),
        );

        let work_id: String = conn
            .query_row(
                "SELECT work_id FROM references_meta WHERE entry_path = 'refs/GNDicLex.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(work_id, "GNDicLex", "work_id should be the filename stem");
    }

    #[test]
    fn test_sha256_duplicate_skips_second_edition() {
        let conn = open_test_db();

        // Card A: explicit sha256 in editions (simulates computed from disk).
        let fm_a = json!({
            "type": "reference",
            "work_id": "shared-work",
            "medium": "pdf",
            "editions": [
                {"edition_id": "first", "sha256": "deadbeef01", "seed_status": "stub"},
            ],
        });
        upsert_reference_meta(
            &conn,
            "u",
            "refs/a.md",
            &fm_a,
            "",
            None,
            std::path::Path::new("/nonexistent"),
        );

        // Card B: same sha256, same work_id, different entry_path.
        let fm_b = json!({
            "type": "reference",
            "work_id": "shared-work",
            "medium": "pdf",
            "editions": [
                {"edition_id": "dup", "sha256": "deadbeef01", "seed_status": "stub"},
            ],
        });
        upsert_reference_meta(
            &conn,
            "u",
            "refs/b.md",
            &fm_b,
            "",
            None,
            std::path::Path::new("/nonexistent"),
        );

        // Only one row with that sha256 should exist.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM references_meta WHERE blob_sha256 = 'deadbeef01'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "duplicate sha256 must not create a second edition row"
        );
    }

    #[test]
    fn test_work_id_filter_returns_all_editions() {
        let conn = open_test_db();
        let fm = json!({
            "type": "reference",
            "work_id": "my-work",
            "medium": "pdf",
            "editions": [
                {"edition_id": "ed1", "seed_status": "stub"},
                {"edition_id": "ed2", "seed_status": "reviewed"},
            ],
        });
        upsert_reference_meta(
            &conn,
            "u",
            "refs/card.md",
            &fm,
            "",
            None,
            std::path::Path::new("/nonexistent"),
        );

        let edition_ids: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT edition_id FROM references_meta \
                     WHERE work_id = 'my-work' ORDER BY edition_id",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(edition_ids, ["ed1", "ed2"]);
    }

    #[test]
    fn test_primary_layer_null_when_no_chain() {
        let conn = open_test_db();
        let fm = json!({
            "type": "reference",
            "medium": "web",
            "seed_status": "stub",
        });
        upsert_reference_meta(
            &conn,
            "u",
            "refs/nochain.md",
            &fm,
            "",
            None,
            std::path::Path::new("/nonexistent"),
        );

        let layer: Option<i64> = conn
            .query_row(
                "SELECT primary_layer FROM references_meta WHERE entry_path = 'refs/nochain.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            layer, None,
            "primary_layer should be null when no primary_source_chain"
        );
    }

    #[test]
    fn test_editions_replaced_on_update() {
        let conn = open_test_db();

        // Initial write: 2 editions.
        let fm1 = json!({
            "type": "reference",
            "work_id": "w",
            "medium": "pdf",
            "editions": [
                {"edition_id": "a", "seed_status": "stub"},
                {"edition_id": "b", "seed_status": "stub"},
            ],
        });
        upsert_reference_meta(
            &conn,
            "u",
            "refs/r.md",
            &fm1,
            "",
            None,
            std::path::Path::new("/x"),
        );

        // Update: remove edition "a", add edition "c".
        let fm2 = json!({
            "type": "reference",
            "work_id": "w",
            "medium": "pdf",
            "editions": [
                {"edition_id": "b", "seed_status": "reviewed"},
                {"edition_id": "c", "seed_status": "stub"},
            ],
        });
        upsert_reference_meta(
            &conn,
            "u",
            "refs/r.md",
            &fm2,
            "",
            None,
            std::path::Path::new("/x"),
        );

        let edition_ids: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT edition_id FROM references_meta \
                     WHERE entry_path = 'refs/r.md' ORDER BY edition_id",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(
            edition_ids,
            ["b", "c"],
            "removed edition 'a' must be gone; 'c' must appear"
        );
    }
}
