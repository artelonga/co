//! `references_meta` + `reference_cards_fts` sync — the write-side projection
//! of reference cards (CO-156/CO-158).
//!
//! These helpers take a raw `&Connection` because they run inside the same
//! lock scope as the entry write that triggered them (entry, vault, and
//! reference write paths). Handler-facing access goes through
//! `crate::repository::ReferenceRepository`, which wraps them.

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::service::ReferenceService;

// Minimal hex encoder (avoids adding a dep; mirrors the one in asset routes).
pub(crate) fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const ALPH: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(ALPH[(b >> 4) as usize] as char);
        out.push(ALPH[(b & 0xf) as usize] as char);
    }
    out
}

/// Called after every entry write. Upserts `references_meta` iff `entry_type == "reference"`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn maybe_sync_reference_meta(
    conn: &Connection,
    universe_key: &str,
    entry_path: &str,
    entry_type: &str,
    frontmatter: &serde_json::Value, // FREEFORM: reference frontmatter is an open schema
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
pub(crate) fn upsert_reference_meta(
    conn: &Connection,
    universe_key: &str,
    entry_path: &str,
    frontmatter: &serde_json::Value, // FREEFORM: reference frontmatter is an open schema
    body: &str,
    title: Option<&str>,
    universe_root: &std::path::Path,
) {
    let work_id = frontmatter
        .get("work_id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| ReferenceService::work_id_from_path(entry_path));

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

            let seed_status = ReferenceService::edition_seed_status(
                file.is_some(),
                blob_sha256.is_some(),
                &seed_status_raw,
            );

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
            let seed_status = ReferenceService::edition_seed_status(
                file.is_some(),
                blob_sha256.is_some(),
                &seed_status_raw,
            );

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

/// Compute sha256 of a sibling asset file given the card's entry_path and file name.
pub(crate) fn compute_blob_sha256(
    entry_path: &str,
    file: Option<&str>,
    universe_root: &std::path::Path,
) -> Option<String> {
    let rel = ReferenceService::expected_blob_path(entry_path, file?)?;
    let bytes = std::fs::read(universe_root.join(rel)).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(hex_encode(hasher.finalize()))
}

#[cfg(test)]
mod tests;
