//! CO-74: typed FK relationship storage via `entry_relations` table.
//!
//! One row per directed edge: `from_path --[relation_type]--> to_path`.
//! Stored in the per-universe `data.db` alongside entries.
//!
//! The `relation_type` equals the manifest field name that declared the `ref`
//! or `ref_list` relationship (e.g. `"attendees"`, `"assignee"`).
//!
//! CO-153: adds `to_universe` for cross-universe references stored as
//! `co://<universe>/<path>` URIs in frontmatter.
//!
//! CO-363: adds `link_text` for wikilink alias labels; extends frontmatter
//! parser to handle `key::path` syntax; adds body wikilink extraction for all
//! four forms (cross-universe, same-universe, with-label, deprecated-relative).

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// co:// URI resolver (CO-153) + key::path (CO-363)
// ---------------------------------------------------------------------------

/// Resolved components of a `co://` cross-universe reference, a `key::path`
/// canonical cross-universe reference, or a plain path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossRef {
    /// Target universe key — `None` means same universe as the from-side.
    pub universe: Option<String>,
    /// Entry path within the target universe.
    pub path: String,
}

/// Parse a frontmatter ref value into a `CrossRef`.
///
/// Accepted forms:
/// - `co://<universe>/<path>` → `CrossRef { universe: Some("universe"), path }`
/// - `<key>::<path>` (CO-363) → `CrossRef { universe: Some("key"), path }`
/// - Anything else → `CrossRef { universe: None, path: s }` (same universe)
///
/// Returns `None` only when a `co://` URI is present but malformed (no slash
/// after the universe component).
pub fn parse_co_uri(s: &str) -> Option<CrossRef> {
    if let Some(rest) = s.strip_prefix("co://") {
        let (u, p) = rest.split_once('/')?;
        Some(CrossRef {
            universe: Some(u.into()),
            path: p.into(),
        })
    } else if let Some((key, path)) = s.split_once("::") {
        let key = key.trim();
        let path = path.trim();
        if key.is_empty() || path.is_empty() {
            Some(CrossRef {
                universe: None,
                path: s.into(),
            })
        } else {
            Some(CrossRef {
                universe: Some(key.into()),
                path: path.into(),
            })
        }
    } else {
        Some(CrossRef {
            universe: None,
            path: s.into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

/// A single row from the `entry_relations` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationRow {
    pub universe_key: String,
    pub from_path: String,
    pub to_path: String,
    pub relation_type: String,
    pub created_at: String,
    /// CO-153: target universe key — `None` means same universe as `universe_key`.
    pub to_universe: Option<String>,
    /// CO-363: optional alias label from `[[target|label]]` wikilinks.
    pub link_text: Option<String>,
}

// ---------------------------------------------------------------------------
// CRUD operations
// ---------------------------------------------------------------------------

/// CRUD operations on `entry_relations`.
pub struct RelationIndex<'a> {
    conn: &'a Connection,
}

impl<'a> RelationIndex<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Replace all outbound relations for `from_path` atomically.
    ///
    /// Each element is `(relation_type, to_path, to_universe, link_text)` where
    /// `to_universe = None` means same-universe (back-compat with CO-74) and
    /// `link_text = None` means no alias label.
    /// Calling with an empty slice removes all existing relations. Idempotent.
    pub fn replace_all(
        &self,
        universe_key: &str,
        from_path: &str,
        relations: &[(String, String, Option<String>, Option<String>)],
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM entry_relations WHERE universe_key = ?1 AND from_path = ?2",
            params![universe_key, from_path],
        )?;
        if relations.is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        for (relation_type, to_path, to_universe, link_text) in relations {
            self.conn.execute(
                "INSERT OR REPLACE INTO entry_relations \
                 (universe_key, from_path, to_path, relation_type, created_at, to_universe, link_text) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    universe_key,
                    from_path,
                    to_path,
                    relation_type,
                    now,
                    to_universe,
                    link_text
                ],
            )?;
        }
        Ok(())
    }

    /// Remove all outbound relations for `from_path`.
    pub fn delete_for_entry(&self, universe_key: &str, from_path: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM entry_relations WHERE universe_key = ?1 AND from_path = ?2",
            params![universe_key, from_path],
        )?;
        Ok(())
    }

    /// All relations originating from `from_path` (outbound edges).
    pub fn outbound(
        &self,
        universe_key: &str,
        from_path: &str,
    ) -> anyhow::Result<Vec<RelationRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT universe_key, from_path, to_path, relation_type, created_at, to_universe, link_text \
             FROM entry_relations WHERE universe_key = ?1 AND from_path = ?2 \
             ORDER BY relation_type, to_path",
        )?;
        let rows = stmt.query_map(params![universe_key, from_path], row_to_relation)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// All same-universe relations pointing to `to_path` (inbound edges).
    ///
    /// For cross-universe inbound, use `cross_universe_inbound` instead.
    pub fn inbound(&self, universe_key: &str, to_path: &str) -> anyhow::Result<Vec<RelationRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT universe_key, from_path, to_path, relation_type, created_at, to_universe, link_text \
             FROM entry_relations WHERE universe_key = ?1 AND to_path = ?2 \
             ORDER BY relation_type, from_path",
        )?;
        let rows = stmt.query_map(params![universe_key, to_path], row_to_relation)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// CO-153: rows in this DB that point INTO `target_universe` at `target_path`.
    ///
    /// Called once per universe during a cross-universe inbound scan.
    pub fn inbound_from_other(
        &self,
        target_universe: &str,
        target_path: &str,
    ) -> anyhow::Result<Vec<RelationRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT universe_key, from_path, to_path, relation_type, created_at, to_universe, link_text \
             FROM entry_relations \
             WHERE to_universe = ?1 AND to_path = ?2 \
             ORDER BY relation_type, from_path",
        )?;
        let rows = stmt.query_map(params![target_universe, target_path], row_to_relation)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

fn row_to_relation(row: &rusqlite::Row<'_>) -> rusqlite::Result<RelationRow> {
    Ok(RelationRow {
        universe_key: row.get(0)?,
        from_path: row.get(1)?,
        to_path: row.get(2)?,
        relation_type: row.get(3)?,
        created_at: row.get(4)?,
        to_universe: row.get(5)?,
        link_text: row.get(6)?,
    })
}

// Extraction + backfill live in the sibling `extract` module (CO-432);
// re-exported so pre-432 `crate::relation_index::*` paths keep working.
pub use super::extract::*;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
