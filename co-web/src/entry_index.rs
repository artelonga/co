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
use serde::{Deserialize, Serialize};
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
    /// CO-164: cosine similarity score ∈ [0, 1] — only present in semantic search results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _score: Option<f32>,
}

/// One row from the per-universe `entry_events` append-only log.
///
/// Maps 1:1 to the future `co.v1.EntryEvent` protobuf — see
/// `co::public/transaction-log.md` for the lakehouse trajectory
/// (Kafka, Iceberg, Pinot, Flink). The body itself is intentionally
/// omitted from this struct so listing history doesn't load N body
/// blobs into memory; fetch the body via `current_body_hash` +
/// `put_blob`/`get_blob`, or include it explicitly in a future
/// dedicated reader if needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryEventRow {
    pub seq: i64,
    pub ts_micros: i64,
    pub op: String,
    pub path: String,
    pub body_hash: Option<String>,
    pub prev_body_hash: Option<String>,
    pub frontmatter_json: Option<String>,
    pub author_id: Option<String>,
    pub request_id: Option<String>,
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

    /// Look up the current `body_hash` for an entry without loading the
    /// body. Used by the event-log writer to record `prev_body_hash`
    /// on an update. Returns None if the entry doesn't exist yet.
    pub fn current_body_hash(&self, universe_key: &str, path: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT body_hash FROM entries WHERE universe_key = ?1 AND path = ?2",
                params![universe_key, path],
                |r| r.get::<_, String>(0),
            )
            .ok()
    }

    /// Append a row to `entry_events` — the per-universe append-only
    /// transaction log. See `co::public/transaction-log.md` for the
    /// design + the trajectory toward Kafka/Iceberg/Pinot/Flink.
    ///
    /// `op` must be `"put"` or `"delete"`. For deletes, `body`,
    /// `body_hash`, and `frontmatter_json` describe the entry as it
    /// was just before deletion (so replay can reconstruct it).
    ///
    /// `request_id` is the idempotency key; when present, the unique
    /// index on `entry_events.request_id` rejects a retried write.
    #[allow(clippy::too_many_arguments)]
    pub fn log_event(
        &self,
        path: &str,
        op: &str,
        body_hash: Option<&str>,
        prev_body_hash: Option<&str>,
        body: Option<&str>,
        frontmatter_json: Option<&str>,
        author_id: Option<&str>,
        request_id: Option<&str>,
    ) -> anyhow::Result<i64> {
        let ts_micros = chrono::Utc::now().timestamp_micros();
        self.conn.execute(
            "INSERT INTO entry_events
                (ts_micros, op, path, body_hash, prev_body_hash,
                 body, frontmatter_json, author_id, request_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                ts_micros,
                op,
                path,
                body_hash,
                prev_body_hash,
                body,
                frontmatter_json,
                author_id,
                request_id,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Read the event log for one entry (path), newest first.
    pub fn list_events_for_path(
        &self,
        path: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<EntryEventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ts_micros, op, path, body_hash, prev_body_hash,
                    frontmatter_json, author_id, request_id
             FROM entry_events
             WHERE path = ?1
             ORDER BY ts_micros DESC, seq DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![path, limit as i64], |row| {
                Ok(EntryEventRow {
                    seq: row.get(0)?,
                    ts_micros: row.get(1)?,
                    op: row.get(2)?,
                    path: row.get(3)?,
                    body_hash: row.get(4)?,
                    prev_body_hash: row.get(5)?,
                    frontmatter_json: row.get(6)?,
                    author_id: row.get(7)?,
                    request_id: row.get(8)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
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

        let body_lines = entry.body.lines().count() as i64;
        let body_words = entry.body.split_whitespace().count() as i64;
        let body_chars = entry.body.chars().count() as i64;

        self.conn.execute(
            "INSERT INTO entries \
               (path, universe_key, entry_type, title, frontmatter_json, payload, \
                body, body_hash, body_lines, body_words, body_chars, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(universe_key, path) DO UPDATE SET
               entry_type = excluded.entry_type,
               title = excluded.title,
               frontmatter_json = excluded.frontmatter_json,
               payload = excluded.payload,
               body = excluded.body,
               body_hash = excluded.body_hash,
               body_lines = excluded.body_lines,
               body_words = excluded.body_words,
               body_chars = excluded.body_chars,
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
                body_lines,
                body_words,
                body_chars,
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
    ///
    /// `limit` caps the result set. Pass `None` to use the default (5 000).
    /// Hard cap is 50 000.
    pub fn query(
        &self,
        universe_key: &str,
        entry_type: &str,
        filters: &JsonValue,
    ) -> anyhow::Result<Vec<EntryRow>> {
        self.query_with_limit(universe_key, entry_type, filters, None)
    }

    pub fn query_with_limit(
        &self,
        universe_key: &str,
        entry_type: &str,
        filters: &JsonValue,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntryRow>> {
        let effective_limit = limit.unwrap_or(5_000).min(50_000);
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
             FROM entries WHERE {}{} LIMIT {}",
            where_clause, order_clause, effective_limit
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

    /// Upsert semantic date rows for an entry into `entry_dates`.
    ///
    /// Scans the manifest for `Date` fields with a declared `semantic`, reads
    /// the corresponding value from `entry.frontmatter`, normalises it to UTC
    /// ISO-8601, and replaces any existing rows for this entry.
    ///
    /// If no manifest is provided or no matching content type is found, this is
    /// a no-op (dates simply won't be indexed for that entry).
    pub fn upsert_dates(
        &self,
        universe_key: &str,
        entry: &co::entry::Entry,
        manifest: Option<&co::manifest::Manifest>,
    ) -> anyhow::Result<()> {
        // Clear stale rows for this entry regardless.
        self.conn.execute(
            "DELETE FROM entry_dates WHERE universe_key = ?1 AND entry_path = ?2",
            params![universe_key, entry.path],
        )?;

        let manifest = match manifest {
            Some(m) => m,
            None => return Ok(()),
        };
        let ct = manifest
            .content_types
            .iter()
            .find(|ct| ct.name == entry.entry_type);
        let ct = match ct {
            Some(ct) => ct,
            None => return Ok(()),
        };

        for (field_name, field_def) in &ct.schema {
            if !matches!(field_def.field_type, co::manifest::FieldType::Date) {
                continue;
            }
            let semantic = match &field_def.semantic {
                Some(s) => s.as_str(),
                None => continue,
            };
            let raw = match entry.frontmatter.get(field_name).and_then(|v| v.as_str()) {
                Some(s) if !s.is_empty() => s,
                _ => continue,
            };
            let utc_val = match normalize_date_to_utc(raw) {
                Some(v) => v,
                None => continue,
            };
            self.conn.execute(
                "INSERT OR REPLACE INTO entry_dates (universe_key, entry_path, semantic, value)
                 VALUES (?1, ?2, ?3, ?4)",
                params![universe_key, entry.path, semantic, utc_val],
            )?;
        }
        Ok(())
    }

    /// Remove all semantic date rows for an entry from `entry_dates`.
    pub fn remove_dates(&self, universe_key: &str, path: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM entry_dates WHERE universe_key = ?1 AND entry_path = ?2",
            params![universe_key, path],
        )?;
        Ok(())
    }

    /// Query entries by date semantic and optional UTC date range.
    ///
    /// `semantic` must be one of the `DateSemantic::as_str()` values (e.g. `"event_at"`).
    /// `from` / `to` are inclusive ISO-8601 bounds (date or datetime).
    /// Results are ordered by `entry_dates.value` ascending (chronological).
    pub fn query_by_date(
        &self,
        universe_key: &str,
        semantic: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> anyhow::Result<Vec<EntryRow>> {
        let mut conditions = vec![
            "e.universe_key = ?1".to_string(),
            "ed.semantic = ?2".to_string(),
        ];
        let mut param_strings: Vec<String> = vec![universe_key.to_string(), semantic.to_string()];

        if let Some(f) = from {
            let utc = normalize_date_to_utc(f).unwrap_or_else(|| f.to_string());
            conditions.push(format!("ed.value >= ?{}", param_strings.len() + 1));
            param_strings.push(utc);
        }
        if let Some(t) = to {
            let utc = normalize_date_to_utc(t).unwrap_or_else(|| t.to_string());
            conditions.push(format!("ed.value <= ?{}", param_strings.len() + 1));
            param_strings.push(utc);
        }

        let where_clause = conditions.join(" AND ");
        let sql = format!(
            "SELECT e.path, e.universe_key, e.entry_type, e.title, \
                    e.frontmatter_json, e.body, e.body_hash, e.created_at, e.updated_at \
             FROM entries e \
             JOIN entry_dates ed \
               ON ed.universe_key = e.universe_key AND ed.entry_path = e.path \
             WHERE {where_clause} \
             ORDER BY ed.value ASC \
             LIMIT 5000"
        );

        let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_strings
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_refs.as_slice(), row_to_entry_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Execute a parameterized SQL query and return matching [`EntryRow`]s.
    ///
    /// Used by the CO-74 query DSL to run compiled graph queries.
    /// `params` must align with the `?N` placeholders in `sql`.
    pub fn query_raw(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> anyhow::Result<Vec<EntryRow>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params, row_to_entry_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Return entries ranked by event frequency (write count) in `entry_events`,
    /// falling back to `updated_at DESC` for entries with no recorded events.
    /// Excludes `index.md` and task/project entries under `projects/`.
    /// Used by `GET /api/v1/universes/{slug}/entries/popular`.
    pub fn popular(&self, universe_key: &str, limit: usize) -> anyhow::Result<Vec<EntryRow>> {
        let cap = (limit.min(50)) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT e.path, e.universe_key, e.entry_type, e.title, \
             e.frontmatter_json, e.body, e.body_hash, e.created_at, e.updated_at \
             FROM entries e \
             LEFT JOIN ( \
               SELECT path, COUNT(*) AS cnt \
               FROM entry_events WHERE op = 'put' \
               GROUP BY path \
             ) ev ON e.path = ev.path \
             WHERE e.universe_key = ?1 \
               AND e.path != 'index.md' \
               AND e.path NOT LIKE 'projects/%' \
             ORDER BY COALESCE(ev.cnt, 0) DESC, e.updated_at DESC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![universe_key, cap], row_to_entry_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
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
        _score: None,
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

/// Normalise a raw date string to a UTC ISO-8601 datetime string.
///
/// Accepts full RFC3339 datetimes and `YYYY-MM-DD` date-only strings.
/// Returns `None` if the input cannot be parsed.
pub fn normalize_date_to_utc(s: &str) -> Option<String> {
    if let Ok(dt) = s.parse::<chrono::DateTime<chrono::Utc>>() {
        return Some(dt.to_rfc3339());
    }
    if let Ok(dt) = format!("{s}T00:00:00Z").parse::<chrono::DateTime<chrono::Utc>>() {
        return Some(dt.to_rfc3339());
    }
    None
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

// ---------------------------------------------------------------------------
// Tests — CO-73 temporal model
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use co::manifest::{ContentType, DateSemantic, FieldDef, FieldType, Manifest, Presentation};
    use rusqlite::Connection;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn open_mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::universe_pool::UNIVERSE_SCHEMA_FOR_TEST)
            .unwrap();
        conn
    }

    fn manifest_with_event_at() -> Manifest {
        let mut schema = BTreeMap::new();
        schema.insert(
            "event_at".to_string(),
            FieldDef {
                field_type: FieldType::Date,
                required: false,
                values: vec![],
                semantic: Some(DateSemantic::EventAt),
                target: None,
            },
        );
        schema.insert(
            "title".to_string(),
            FieldDef {
                field_type: FieldType::String,
                required: false,
                values: vec![],
                semantic: None,
                target: None,
            },
        );
        Manifest {
            schema_version: 1,
            name: "Test".to_string(),
            content_types: vec![ContentType {
                name: "evento".to_string(),
                schema,
                presentation: Presentation::default(),
                indexes: vec![],
            }],
            doc_generators: vec![],
            relationships: vec![],
            views: vec![],
            properties_per_type: std::collections::BTreeMap::new(),
        }
    }

    fn insert_entry(conn: &Connection, universe_key: &str, path: &str, entry_type: &str, fm: &str) {
        conn.execute(
            "INSERT INTO entries (path, universe_key, entry_type, title, frontmatter_json, payload, body, body_hash)
             VALUES (?1, ?2, ?3, '', ?4, ?4, '', '')",
            rusqlite::params![path, universe_key, entry_type, fm],
        )
        .unwrap();
    }

    #[test]
    fn test_normalize_date_to_utc_datetime() {
        let out = normalize_date_to_utc("2026-05-01T10:00:00Z").unwrap();
        assert!(out.contains("2026-05-01"), "got: {out}");
    }

    #[test]
    fn test_normalize_date_to_utc_date_only() {
        let out = normalize_date_to_utc("2026-05-01").unwrap();
        assert!(out.starts_with("2026-05-01"), "got: {out}");
    }

    #[test]
    fn test_normalize_date_to_utc_invalid() {
        assert!(normalize_date_to_utc("not-a-date").is_none());
    }

    #[test]
    fn test_upsert_dates_inserts_semantic_row() {
        let conn = open_mem_db();
        let idx = EntryIndex::new(&conn);
        let manifest = manifest_with_event_at();

        insert_entry(
            &conn,
            "uni",
            "events/1.md",
            "evento",
            r#"{"type":"evento","event_at":"2026-06-01"}"#,
        );
        let entry = make_entry(
            "events/1.md",
            json!({"type": "evento", "event_at": "2026-06-01"}),
            "",
        );

        idx.upsert_dates("uni", &entry, Some(&manifest)).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entry_dates WHERE universe_key='uni' AND entry_path='events/1.md' AND semantic='event_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "should have one entry_dates row");
    }

    #[test]
    fn test_upsert_dates_replaces_on_update() {
        let conn = open_mem_db();
        let idx = EntryIndex::new(&conn);
        let manifest = manifest_with_event_at();

        let entry_v1 = make_entry(
            "events/1.md",
            json!({"type":"evento","event_at":"2026-06-01"}),
            "",
        );
        idx.upsert_dates("uni", &entry_v1, Some(&manifest)).unwrap();

        let entry_v2 = make_entry(
            "events/1.md",
            json!({"type":"evento","event_at":"2026-07-15"}),
            "",
        );
        idx.upsert_dates("uni", &entry_v2, Some(&manifest)).unwrap();

        let val: String = conn
            .query_row(
                "SELECT value FROM entry_dates WHERE universe_key='uni' AND entry_path='events/1.md' AND semantic='event_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(val.contains("2026-07-15"), "value should be updated: {val}");
    }

    #[test]
    fn test_remove_dates_clears_rows() {
        let conn = open_mem_db();
        let idx = EntryIndex::new(&conn);
        let manifest = manifest_with_event_at();

        let entry = make_entry(
            "events/1.md",
            json!({"type":"evento","event_at":"2026-06-01"}),
            "",
        );
        idx.upsert_dates("uni", &entry, Some(&manifest)).unwrap();
        idx.remove_dates("uni", "events/1.md").unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entry_dates WHERE universe_key='uni'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_query_by_date_returns_matching_entries() {
        let conn = open_mem_db();
        let idx = EntryIndex::new(&conn);
        let manifest = manifest_with_event_at();

        // Insert entries
        for (path, date) in [
            ("events/1.md", "2026-06-10"),
            ("events/2.md", "2026-06-20"),
            ("events/3.md", "2026-07-05"),
        ] {
            let fm = json!({"type":"evento","event_at": date});
            insert_entry(&conn, "uni", path, "evento", &fm.to_string());
            let entry = make_entry(path, fm, "");
            idx.upsert_dates("uni", &entry, Some(&manifest)).unwrap();
        }

        let results = idx
            .query_by_date("uni", "event_at", Some("2026-06-01"), Some("2026-06-30"))
            .unwrap();
        assert_eq!(results.len(), 2, "should return June events only");
        assert!(results.iter().all(|r| r.entry_type == "evento"));
    }

    #[test]
    fn test_query_by_date_no_range_returns_all_with_semantic() {
        let conn = open_mem_db();
        let idx = EntryIndex::new(&conn);
        let manifest = manifest_with_event_at();

        for (path, date) in [("events/1.md", "2026-01-01"), ("events/2.md", "2026-12-31")] {
            let fm = json!({"type":"evento","event_at": date});
            insert_entry(&conn, "uni", path, "evento", &fm.to_string());
            idx.upsert_dates("uni", &make_entry(path, fm, ""), Some(&manifest))
                .unwrap();
        }

        let results = idx.query_by_date("uni", "event_at", None, None).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_upsert_dates_no_manifest_is_noop() {
        let conn = open_mem_db();
        let idx = EntryIndex::new(&conn);
        let entry = make_entry("x.md", json!({"type":"evento","event_at":"2026-01-01"}), "");
        idx.upsert_dates("uni", &entry, None).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entry_dates", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "no manifest = no rows indexed");
    }
}
