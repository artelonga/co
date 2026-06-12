//! Relation extraction + backfill — frontmatter `ref`/`ref_list` fields,
//! body wikilinks (CO-363), provenance edges (CO-418), and the backfill
//! passes that promote existing content to typed edges.
//!
//! Split out of `index.rs` (CO-432); re-exported there so the
//! `crate::relation_index::*` paths are unchanged.

use std::sync::Arc;

use rusqlite::{Connection, params};

use chrono::Utc;

use super::index::{RelationIndex, parse_co_uri};
use crate::universe_pool::UniversePool;

// ---------------------------------------------------------------------------
// Relation extraction from frontmatter
// ---------------------------------------------------------------------------

/// Extract `(relation_type, to_path, to_universe, link_text)` 4-tuples from an
/// entry's frontmatter JSON using the manifest schema for `entry_type`.
///
/// Only fields declared as `ref` or `ref_list` in the manifest contribute rows.
/// Wikilink notation is stripped via `co::wikilink::resolve_ref_value`.
/// `co://` URIs (CO-153) and `key::path` syntax (CO-363) are split into
/// `(to_path, Some(to_universe))`.
/// Plain paths produce `to_universe = None` (same universe).
/// `link_text` is always `None` for frontmatter fields.
pub fn extract_relations(
    manifest: &co::manifest::Manifest,
    entry_type: &str,
    frontmatter: &serde_json::Value,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    use co::manifest::FieldType;

    let ct = match manifest
        .content_types
        .iter()
        .find(|ct| ct.name == entry_type)
    {
        Some(ct) => ct,
        None => return vec![],
    };

    let mut relations = Vec::new();

    for (field_name, field_def) in &ct.schema {
        match field_def.field_type {
            FieldType::Ref => {
                if let Some(val) = frontmatter.get(field_name).and_then(|v| v.as_str()) {
                    let resolved = co::wikilink::resolve_ref_value(val);
                    if let Some(cr) = parse_co_uri(resolved)
                        && !cr.path.is_empty()
                    {
                        relations.push((field_name.clone(), cr.path, cr.universe, None));
                    }
                }
            }
            FieldType::RefList => {
                if let Some(arr) = frontmatter.get(field_name).and_then(|v| v.as_array()) {
                    for item in arr {
                        if let Some(val) = item.as_str() {
                            let resolved = co::wikilink::resolve_ref_value(val);
                            if let Some(cr) = parse_co_uri(resolved)
                                && !cr.path.is_empty()
                            {
                                relations.push((field_name.clone(), cr.path, cr.universe, None));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    relations
}

/// CO-363: Extract all body wikilinks as relation edges.
///
/// Handles four forms:
/// - `[[key::path]]` / `[[key::path|label]]` → cross-universe wikilink, `to_universe = Some(key)`
/// - `[[path]]` / `[[path|label]]` (no `::`) → same-universe wikilink, `to_universe = None`
/// - `[[../sibling/path]]` → deprecated relative path; emits a warning log and
///   returns `relation_type = "wikilink_relative_deprecated"`, `to_universe = None`
///
/// Returns `(relation_type, to_path, to_universe, link_text)` 4-tuples.
pub fn extract_body_wikilinks(body: &str) -> Vec<(String, String, Option<String>, Option<String>)> {
    use co::wikilink::extract_wikilinks_with_labels;

    extract_wikilinks_with_labels(body)
        .into_iter()
        .filter_map(|(target, label)| {
            if target.is_empty() {
                return None;
            }

            // Deprecated relative-path wikilinks: [[../something]]
            if target.starts_with("../") || target.starts_with("./") {
                tracing::warn!(
                    target = %target,
                    "CO-363: deprecated relative-path wikilink — \
                     migrate to [[key::path]] canonical form"
                );
                return Some((
                    "wikilink_relative_deprecated".to_string(),
                    target,
                    None,
                    None,
                ));
            }

            // Cross-universe: [[key::path]] or [[key::path|label]]
            if let Some((key, path)) = target.split_once("::") {
                let key = key.trim();
                let path = path.trim();
                if key.is_empty() || path.is_empty() {
                    return None;
                }
                return Some((
                    "wikilink".to_string(),
                    path.to_string(),
                    Some(key.to_string()),
                    label,
                ));
            }

            // Same-universe: [[path]] or [[path|label]]
            Some(("wikilink".to_string(), target, None, label))
        })
        .collect()
}

/// CO-418: Extract provenance/traceback relations from frontmatter.
///
/// Two manifest-independent typed relations carry the render-review-publish
/// traceback so the UI can surface "Origem" + "Pedido por":
/// - `origin` ← the `source` field (e.g. `github:owner/repo@sha`, from CO-417).
///   The target is an external ref, not a local entry, so `to_universe` is set
///   to a sentinel `"@source"` to mark it as a non-entry external target.
/// - `requested_by` ← the `requested_by` field (the task that asked for the
///   publish, e.g. `CO-418`). `to_universe` is the sentinel `"@task"`.
///
/// These derive from frontmatter, so they survive every `replace_all` and are
/// idempotent: re-saving the same entry reproduces exactly the same edges.
///
/// Returns `(relation_type, to_path, to_universe, link_text)` 4-tuples.
pub fn extract_provenance_relations(
    frontmatter: &serde_json::Value,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let mut out = Vec::new();
    if let Some(source) = frontmatter.get("source").and_then(|v| v.as_str()) {
        let source = source.trim();
        if !source.is_empty() {
            out.push((
                "origin".to_string(),
                source.to_string(),
                Some("@source".to_string()),
                None,
            ));
        }
    }
    if let Some(req) = frontmatter.get("requested_by").and_then(|v| v.as_str()) {
        let req = req.trim();
        if !req.is_empty() {
            out.push((
                "requested_by".to_string(),
                req.to_string(),
                Some("@task".to_string()),
                None,
            ));
        }
    }
    out
}

/// Sync relations for a single entry: extract from frontmatter + body, replace in DB.
///
/// Combines manifest-declared frontmatter refs (if manifest is provided) with
/// body wikilinks (CO-363) and provenance/traceback edges (CO-418) into one
/// `replace_all` call. Silently skips manifest extraction if no manifest is
/// given, but always processes body wikilinks + provenance.
///
/// Returns the number of relations upserted (0 = all cleared).
pub fn sync_entry_relations(
    conn: &Connection,
    universe_key: &str,
    path: &str,
    entry_type: &str,
    frontmatter: &serde_json::Value,
    body: &str,
    manifest: Option<&co::manifest::Manifest>,
) -> anyhow::Result<usize> {
    let mut relations = if let Some(m) = manifest {
        extract_relations(m, entry_type, frontmatter)
    } else {
        vec![]
    };
    relations.extend(extract_body_wikilinks(body));
    relations.extend(extract_provenance_relations(frontmatter));
    let count = relations.len();
    RelationIndex::new(conn).replace_all(universe_key, path, &relations)?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Backfill
// ---------------------------------------------------------------------------

/// Backfill relations for all entries of manifest-declared ref/ref_list types.
///
/// Runs synchronously on `conn`.  Returns the number of entries processed.
/// Called after a manifest update to promote existing wikilinks to typed FKs.
pub fn backfill_for_manifest(
    conn: &Connection,
    universe_key: &str,
    manifest: &co::manifest::Manifest,
) -> anyhow::Result<usize> {
    use co::manifest::FieldType;

    let affected_types: Vec<&str> = manifest
        .content_types
        .iter()
        .filter(|ct| {
            ct.schema
                .values()
                .any(|def| matches!(def.field_type, FieldType::Ref | FieldType::RefList))
        })
        .map(|ct| ct.name.as_str())
        .collect();

    if affected_types.is_empty() {
        return Ok(0);
    }

    let mut total = 0usize;

    for type_name in &affected_types {
        let mut stmt = conn.prepare(
            "SELECT path, frontmatter_json, body FROM entries \
             WHERE universe_key = ?1 AND entry_type = ?2",
        )?;
        let entries: Vec<(String, String, String)> = stmt
            .query_map(params![universe_key, type_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (path, fm_json, body) in entries {
            if let Ok(fm) = serde_json::from_str::<serde_json::Value>(&fm_json) {
                let mut relations = extract_relations(manifest, type_name, &fm);
                relations.extend(extract_body_wikilinks(&body));
                RelationIndex::new(conn).replace_all(universe_key, &path, &relations)?;
                total += 1;
            }
        }
    }

    Ok(total)
}

/// CO-363: Backfill body wikilink relations for ALL entries in a universe.
///
/// Uses INSERT OR REPLACE scoped to `relation_type IN ('wikilink',
/// 'wikilink_relative_deprecated')` to avoid disturbing manifest-based FK rows
/// (which have field-name relation types). Idempotent — safe to re-run.
/// Returns the number of entries processed.
pub fn backfill_body_wikilinks(conn: &Connection, universe_key: &str) -> anyhow::Result<usize> {
    let entries: Vec<(String, String)> = {
        let mut stmt = conn.prepare("SELECT path, body FROM entries WHERE universe_key = ?1")?;
        stmt.query_map(params![universe_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect()
    };

    let now = Utc::now().to_rfc3339();
    let mut total = 0usize;

    for (path, body) in &entries {
        // Remove existing wikilink rows for this entry before re-inserting,
        // so stale links (e.g. after an edit) are cleaned up.
        conn.execute(
            "DELETE FROM entry_relations \
             WHERE universe_key = ?1 AND from_path = ?2 \
               AND relation_type IN ('wikilink', 'wikilink_relative_deprecated')",
            params![universe_key, path],
        )?;

        let wikilinks = extract_body_wikilinks(body);
        for (rel_type, to_path, to_universe, link_text) in &wikilinks {
            conn.execute(
                "INSERT OR REPLACE INTO entry_relations \
                 (universe_key, from_path, to_path, relation_type, created_at, to_universe, link_text) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![universe_key, path, to_path, rel_type, now, to_universe, link_text],
            )?;
        }
        total += 1;
    }

    Ok(total)
}

/// Spawn a background thread to backfill relations after a manifest update.
///
/// Fire-and-forget: failures are logged but do not propagate to the caller.
pub fn backfill_relations_background(
    universe_pool: Arc<UniversePool>,
    universe_key: String,
    manifest: co::manifest::Manifest,
) {
    std::thread::spawn(move || {
        let conn_arc = universe_pool.get_or_open(&universe_key);
        match conn_arc.lock() {
            Ok(conn) => match backfill_for_manifest(&conn, &universe_key, &manifest) {
                Ok(n) => tracing::info!(
                    universe = %universe_key,
                    entries = n,
                    "CO-74: relation backfill complete"
                ),
                Err(e) => tracing::warn!(
                    universe = %universe_key,
                    error = %e,
                    "CO-74: relation backfill failed"
                ),
            },
            Err(e) => tracing::warn!(
                universe = %universe_key,
                error = ?e,
                "CO-74: relation backfill: failed to lock universe DB"
            ),
        }
    });
}
