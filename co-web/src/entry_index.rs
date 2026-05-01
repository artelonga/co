//! EntryIndex — SQLite-backed index for CO entries.
//!
//! The SQLite `entries` table is a **materialized index** over `.md` files.
//! It is disposable: delete `co.db` and call `rebuild()` to regenerate.
//!
//! # Schema
//! ```sql
//! CREATE TABLE entries (
//!     path         TEXT NOT NULL,
//!     universe_key TEXT NOT NULL,
//!     entry_type   TEXT NOT NULL,
//!     title        TEXT,
//!     frontmatter_json TEXT NOT NULL,
//!     body         TEXT NOT NULL DEFAULT '',
//!     body_hash    TEXT NOT NULL,
//!     created_at   TEXT,
//!     updated_at   TEXT,
//!     PRIMARY KEY (universe_key, path)
//! );
//! ```

use std::path::Path;

use rusqlite::{Connection, params};
use serde_json::Value as JsonValue;

use chrono::Utc;
use co::entry::{Entry, FileStat, scan_entries};

/// Tag with occurrence count.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TagCount {
    pub tag: String,
    pub count: u64,
}

/// Tree node — an entry with its children (built from the `parent` field).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TreeNode {
    pub entry: EntryRow,
    pub children: Vec<TreeNode>,
}

/// A row from the entries table, ready for API serialization.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntryRow {
    pub path: String,
    pub universe_key: String,
    pub entry_type: String,
    pub title: Option<String>,
    pub frontmatter: JsonValue,
    pub body: String,
    pub body_hash: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// SQLite-backed index over Entry files.
///
/// All methods take `&Connection` — callers manage the connection lifecycle.
pub struct EntryIndex<'a> {
    conn: &'a Connection,
}

impl<'a> EntryIndex<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Scan all `.md` files under `universe_root`, parse them, and upsert into
    /// the `entries` table.  Returns the number of entries processed.
    pub fn rebuild(&self, universe_key: &str, universe_root: &Path) -> anyhow::Result<usize> {
        if !universe_root.exists() {
            return Ok(0);
        }
        let entries = scan_entries(universe_root)?;
        let count = entries.len();
        for entry in entries {
            self.upsert(universe_key, &entry)?;
        }
        Ok(count)
    }

    /// Upsert a single entry into the index.
    ///
    /// Both `frontmatter_json` and `payload` are written; `payload` mirrors
    /// `frontmatter_json` and is the column targeted by CO-71 expression indexes.
    pub fn upsert(&self, universe_key: &str, entry: &Entry) -> anyhow::Result<()> {
        let fm_json = serde_json::to_string(&entry.frontmatter)?;
        let title: Option<&str> = entry.frontmatter.get("title").and_then(|v| v.as_str());
        let created_at = entry
            .frontmatter
            .get("created")
            .and_then(|v| v.as_str())
            .map(String::from);
        let updated_at = entry
            .frontmatter
            .get("modified")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| created_at.clone());

        self.conn.execute(
            "INSERT INTO entries (path, universe_key, entry_type, title, frontmatter_json, payload, body, body_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(universe_key, path) DO UPDATE SET
               entry_type = excluded.entry_type,
               title = excluded.title,
               frontmatter_json = excluded.frontmatter_json,
               payload = excluded.payload,
               body = excluded.body,
               body_hash = excluded.body_hash,
               created_at = excluded.created_at,
               updated_at = excluded.updated_at",
            params![
                entry.path,
                universe_key,
                entry.entry_type,
                title,
                fm_json,
                entry.body,
                entry.body_hash,
                created_at,
                updated_at,
            ],
        )?;

        // Keep FTS table in sync
        self.conn
            .execute(
                "INSERT INTO entries_fts (universe_key, path, title, body)
             VALUES (?1, ?2, ?3, ?4)",
                params![universe_key, entry.path, title.unwrap_or(""), entry.body],
            )
            .ok(); // FTS upsert — ignore errors (may already exist)

        Ok(())
    }

    /// Remove a single entry from the index.
    pub fn remove(&self, universe_key: &str, path: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
            params![universe_key, path],
        )?;
        self.conn
            .execute(
                "DELETE FROM entries_fts WHERE universe_key = ?1 AND path = ?2",
                params![universe_key, path],
            )
            .ok();
        Ok(())
    }

    /// List entries by type with optional frontmatter filters.
    ///
    /// `filters` is a JSON object whose keys map to frontmatter field names.
    /// Supported operators:
    /// - `{"field": "value"}` — exact match
    /// - `{"field": {"$ne": "value"}}` — not equal
    /// - `{"$sort": {"field": 1}}` — ascending sort (`-1` = descending)
    pub fn query(
        &self,
        universe_key: &str,
        entry_type: &str,
        filters: &JsonValue,
    ) -> anyhow::Result<Vec<EntryRow>> {
        // Empty entry_type means "any type" — used by the unfiltered list_entries
        // endpoint. When non-empty, restrict by exact type.
        let mut conditions = vec!["universe_key = ?1".to_string()];
        let mut param_strings: Vec<String> = vec![universe_key.to_string()];
        if !entry_type.is_empty() {
            conditions.push(format!("entry_type = ?{}", param_strings.len() + 1));
            param_strings.push(entry_type.to_string());
        }
        let mut order_clause = String::new();

        if let Some(obj) = filters.as_object() {
            for (key, val) in obj {
                if key == "$sort" {
                    if let Some(sort_obj) = val.as_object()
                        && let Some((field, dir)) = sort_obj.iter().next()
                    {
                        let dir_sql = if dir.as_i64().unwrap_or(1) >= 0 {
                            "ASC"
                        } else {
                            "DESC"
                        };
                        order_clause = format!(
                            " ORDER BY CAST(json_extract(frontmatter_json, '$.{}') AS TEXT) {}",
                            field, dir_sql
                        );
                    }
                    continue;
                }
                let param_idx = param_strings.len() + 1;
                if let Some(ne_val) = val.as_object().and_then(|o| o.get("$ne")) {
                    let ne_str = value_to_sql_string(ne_val);
                    conditions.push(format!(
                        "json_extract(frontmatter_json, '$.{}') != ?{}",
                        key, param_idx
                    ));
                    param_strings.push(ne_str);
                } else {
                    let eq_str = value_to_sql_string(val);
                    conditions.push(format!(
                        "json_extract(frontmatter_json, '$.{}') = ?{}",
                        key, param_idx
                    ));
                    param_strings.push(eq_str);
                }
            }
        }

        let where_clause = conditions.join(" AND ");
        let sql = format!(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at \
             FROM entries WHERE {}{} LIMIT 500",
            where_clause, order_clause
        );

        let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_strings
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_refs.as_slice(), row_to_entry_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get a single entry by its vault-relative path.
    pub fn get(&self, universe_key: &str, path: &str) -> anyhow::Result<Option<EntryRow>> {
        let result = self.conn.query_row(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at \
             FROM entries WHERE universe_key = ?1 AND path = ?2",
            params![universe_key, path],
            row_to_entry_row,
        );
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Full-text search across title + body using FTS5.
    pub fn search(&self, universe_key: &str, query: &str) -> anyhow::Result<Vec<EntryRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.path, e.universe_key, e.entry_type, e.title, e.frontmatter_json, e.body, e.body_hash, e.created_at, e.updated_at \
             FROM entries e \
             JOIN entries_fts fts ON fts.path = e.path AND fts.universe_key = e.universe_key \
             WHERE fts.universe_key = ?1 AND entries_fts MATCH ?2 \
             ORDER BY rank LIMIT 100",
        )?;
        let rows = stmt.query_map(params![universe_key, query], row_to_entry_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Aggregate all `tags` arrays across entries, return with counts.
    pub fn tags(&self, universe_key: &str) -> anyhow::Result<Vec<TagCount>> {
        let mut stmt = self
            .conn
            .prepare("SELECT frontmatter_json FROM entries WHERE universe_key = ?1")?;
        let all_fm: Vec<String> = stmt
            .query_map(params![universe_key], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for fm_str in all_fm {
            if let Ok(fm) = serde_json::from_str::<JsonValue>(&fm_str)
                && let Some(tags) = fm.get("tags").and_then(|v| v.as_array())
            {
                for tag in tags {
                    if let Some(t) = tag.as_str()
                        && !t.is_empty()
                    {
                        *counts.entry(t.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut result: Vec<TagCount> = counts
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect();
        result.sort_by(|a, b| b.count.cmp(&a.count));
        Ok(result)
    }

    /// Build a hierarchy from the `parent` field in frontmatter.
    ///
    /// Entries with no `parent` (or parent=null) become root nodes.
    /// Entries whose `parent` matches another entry's `id` become children.
    pub fn tree(&self, universe_key: &str, entry_type: &str) -> anyhow::Result<Vec<TreeNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at \
             FROM entries WHERE universe_key = ?1 AND entry_type = ?2",
        )?;
        let all: Vec<EntryRow> = stmt
            .query_map(params![universe_key, entry_type], row_to_entry_row)?
            .filter_map(|r| r.ok())
            .collect();

        // Build id → index map
        let mut id_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (i, row) in all.iter().enumerate() {
            if let Some(id_val) = row.frontmatter.get("id") {
                let id_str = value_to_sql_string(id_val);
                id_map.insert(id_str, i);
            }
        }

        // Separate roots from children
        let nodes: Vec<TreeNode> = all
            .into_iter()
            .map(|e| TreeNode {
                entry: e,
                children: vec![],
            })
            .collect();

        // Collect (parent_idx, child_idx) pairs
        let relationships: Vec<(usize, usize)> = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| {
                let parent_val = node.entry.frontmatter.get("parent")?;
                if parent_val.is_null() {
                    return None;
                }
                let parent_str = value_to_sql_string(parent_val);
                let parent_idx = id_map.get(&parent_str)?;
                Some((*parent_idx, i))
            })
            .collect();

        // Mark which nodes are children
        let child_indices: std::collections::HashSet<usize> =
            relationships.iter().map(|(_, c)| *c).collect();

        // Attach children (clone to avoid borrow issues)
        let children_by_parent: std::collections::HashMap<usize, Vec<TreeNode>> = {
            let mut map: std::collections::HashMap<usize, Vec<TreeNode>> =
                std::collections::HashMap::new();
            for (p, c) in &relationships {
                map.entry(*p).or_default().push(nodes[*c].clone());
            }
            map
        };

        let mut roots: Vec<TreeNode> = nodes
            .into_iter()
            .enumerate()
            .filter_map(|(i, mut node)| {
                if child_indices.contains(&i) {
                    return None;
                }
                if let Some(children) = children_by_parent.get(&i) {
                    node.children = children.clone();
                }
                Some(node)
            })
            .collect();

        roots.sort_by(|a, b| a.entry.path.cmp(&b.entry.path));
        Ok(roots)
    }

    /// Count entries of a given type in a universe.
    pub fn count(&self, universe_key: &str, entry_type: Option<&str>) -> i64 {
        match entry_type {
            Some(et) => self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE universe_key = ?1 AND entry_type = ?2",
                    params![universe_key, et],
                    |row| row.get(0),
                )
                .unwrap_or(0),
            None => self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                    params![universe_key],
                    |row| row.get(0),
                )
                .unwrap_or(0),
        }
    }

    /// Return a typed view of an [`EntryRow`] using the manifest content type.
    ///
    /// Fields declared in the schema are coerced to typed Rust values
    /// (dates → `DateTime<Utc>`, enums → `String`, etc.).  Fields not in the
    /// schema are coerced by their JSON type.
    pub fn typed_view(
        &self,
        row: &EntryRow,
        manifest: &co::manifest::Manifest,
    ) -> co::payload::TypedEntry {
        let ct = manifest
            .content_types
            .iter()
            .find(|ct| ct.name == row.entry_type);

        match ct {
            Some(ct) => co::payload::typed_entry(
                &row.path,
                &row.universe_key,
                &row.entry_type,
                &row.frontmatter,
                &row.body,
                ct,
            ),
            None => {
                // No schema declared for this type — coerce without manifest.
                let default_ct = co::manifest::ContentType {
                    name: row.entry_type.clone(),
                    schema: std::collections::BTreeMap::new(),
                    presentation: co::manifest::Presentation::default(),
                    indexes: vec![],
                };
                co::payload::typed_entry(
                    &row.path,
                    &row.universe_key,
                    &row.entry_type,
                    &row.frontmatter,
                    &row.body,
                    &default_ct,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row_to_entry_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    let fm_str: String = row.get(4)?;
    let frontmatter: JsonValue =
        serde_json::from_str(&fm_str).unwrap_or(JsonValue::Object(Default::default()));
    Ok(EntryRow {
        path: row.get(0)?,
        universe_key: row.get(1)?,
        entry_type: row.get(2)?,
        title: row.get(3)?,
        frontmatter,
        body: row.get(5)?,
        body_hash: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn value_to_sql_string(val: &JsonValue) -> String {
    match val {
        JsonValue::String(s) => s.clone(),
        JsonValue::Number(n) => n.to_string(),
        JsonValue::Bool(b) => {
            if *b {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        JsonValue::Null => String::new(),
        other => other.to_string(),
    }
}

/// Build an Entry from frontmatter JSON object + body string (for creating new entries).
pub fn make_entry(path: &str, frontmatter: JsonValue, body: &str) -> co::entry::Entry {
    let entry_type = frontmatter
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let now = Utc::now();
    co::entry::Entry {
        path: path.to_string(),
        entry_type,
        frontmatter,
        body: body.to_string(),
        body_hash: co::entry::Entry::hash_body(body),
        stat: FileStat {
            created: now,
            modified: now,
            size: 0,
        },
    }
}
