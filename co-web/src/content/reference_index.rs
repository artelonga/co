//! CO-154: References as a first-class content type.
//!
//! Extracts `{url, source, retrieved}` objects from entry frontmatter
//! `references[]` array into `references_index`, matches them to
//! `## Referência: <source>` body sections, and indexes excerpts in
//! `references_fts` for full-text search.

use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::universe_pool::UniversePool;

// ---------------------------------------------------------------------------
// Public row type (API-facing)
// ---------------------------------------------------------------------------

/// A single row from `references_index`, ready for API serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceRow {
    pub universe_key: String,
    /// Exposed as `path` in the JSON API (maps to `entry_path` DB column).
    #[serde(rename = "path")]
    pub entry_path: String,
    pub ref_index: i64,
    pub url: Option<String>,
    pub source: String,
    pub retrieved: Option<String>,
    pub excerpt_body: Option<String>,
}

// ---------------------------------------------------------------------------
// Internal candidate (pre-DB)
// ---------------------------------------------------------------------------

struct RefCandidate {
    index: usize,
    url: Option<String>,
    source: String,
    retrieved: Option<String>,
    excerpt_body: Option<String>,
}

// ---------------------------------------------------------------------------
// Extraction from frontmatter + body
// ---------------------------------------------------------------------------

/// Extract reference candidates from an entry's frontmatter and body.
///
/// Each item in `frontmatter.references[]` maps to one `RefCandidate`.
/// Body sections under `## Referência: <source>` headings (up to the next H2
/// or EOF) are matched to candidates by source name and stored as
/// `excerpt_body`.
fn extract_references(frontmatter: &serde_json::Value, body: &str) -> Vec<RefCandidate> {
    let arr = match frontmatter.get("references").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return vec![],
    };

    let mut candidates: Vec<RefCandidate> = arr
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let source = item.get("source").and_then(|v| v.as_str())?.to_string();
            Some(RefCandidate {
                index: i,
                url: item.get("url").and_then(|v| v.as_str()).map(String::from),
                source,
                retrieved: item
                    .get("retrieved")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                excerpt_body: None,
            })
        })
        .collect();

    // Walk body for ## Referência: sections and attach excerpt to matching candidate.
    let sections = extract_body_sections(body);
    for candidate in &mut candidates {
        if let Some(excerpt) = sections.get(&candidate.source) {
            candidate.excerpt_body = Some(excerpt.clone());
        } else if !sections.is_empty() {
            // Warn if body has Referência headings but this candidate has none —
            // likely a typo in the heading vs the frontmatter source name.
            tracing::warn!(
                source = %candidate.source,
                "CO-154: no matching ## Referência: heading for source"
            );
        }
    }

    candidates
}

/// Parse body for `## Referência: <source>` headings.
///
/// Returns a map from source name to section body (trimmed, exclusive of the
/// heading line itself, up to the next `## ` heading or EOF).
fn extract_body_sections(body: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut current_source: Option<String> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in body.lines() {
        if let Some(source) = parse_referencia_heading(line) {
            flush_section(&mut map, &mut current_source, &mut current_lines);
            current_source = Some(source);
        } else if line.starts_with("## ") {
            flush_section(&mut map, &mut current_source, &mut current_lines);
        } else if current_source.is_some() {
            current_lines.push(line);
        }
    }
    flush_section(&mut map, &mut current_source, &mut current_lines);

    map
}

fn flush_section(
    map: &mut HashMap<String, String>,
    current_source: &mut Option<String>,
    current_lines: &mut Vec<&str>,
) {
    if let Some(source) = current_source.take() {
        let text = current_lines.join("\n").trim().to_string();
        if !text.is_empty() {
            map.insert(source, text);
        }
        current_lines.clear();
    }
}

/// Parse a `## Referência: <source>` or `## Referencia: <source>` heading.
fn parse_referencia_heading(line: &str) -> Option<String> {
    let s = line.trim();
    for prefix in &["## Referência: ", "## Referencia: "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let source = rest.trim().to_string();
            if !source.is_empty() {
                return Some(source);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// ReferenceIndex — CRUD on references_index + references_fts
// ---------------------------------------------------------------------------

/// CRUD operations on `references_index` and `references_fts`.
pub struct ReferenceIndex<'a> {
    conn: &'a Connection,
}

impl<'a> ReferenceIndex<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Replace all reference rows for `entry_path` atomically.
    ///
    /// Deletes all existing rows for `(universe_key, entry_path)`, then
    /// inserts the new set. Calling with an empty `candidates` slice just
    /// removes all existing rows.  Idempotent.
    fn replace_all(
        &self,
        universe_key: &str,
        entry_path: &str,
        candidates: &[RefCandidate],
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM references_index WHERE universe_key = ?1 AND entry_path = ?2",
            params![universe_key, entry_path],
        )?;
        self.conn
            .execute(
                "DELETE FROM references_fts WHERE universe_key = ?1 AND entry_path = ?2",
                params![universe_key, entry_path],
            )
            .ok();

        for c in candidates {
            self.conn.execute(
                "INSERT OR REPLACE INTO references_index \
                 (universe_key, entry_path, ref_index, url, source, retrieved, excerpt_body) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    universe_key,
                    entry_path,
                    c.index as i64,
                    c.url,
                    c.source,
                    c.retrieved,
                    c.excerpt_body,
                ],
            )?;
            self.conn
                .execute(
                    "INSERT INTO references_fts \
                     (universe_key, entry_path, ref_index, source, excerpt_body) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        universe_key,
                        entry_path,
                        c.index as i64,
                        c.source,
                        c.excerpt_body.as_deref().unwrap_or(""),
                    ],
                )
                .ok();
        }
        Ok(())
    }

    /// Query references with optional filters.
    ///
    /// - `source_contains`: substring match on `source`
    /// - `url_contains`: substring match on `url`
    /// - `fts_query`: FTS5 MATCH query over `source` + `excerpt_body`
    pub fn query(
        &self,
        universe_key: &str,
        source_contains: Option<&str>,
        url_contains: Option<&str>,
        fts_query: Option<&str>,
    ) -> anyhow::Result<Vec<ReferenceRow>> {
        if let Some(q) = fts_query {
            return self.query_fts(universe_key, q);
        }

        let mut sql = String::from(
            "SELECT universe_key, entry_path, ref_index, url, source, retrieved, excerpt_body \
             FROM references_index WHERE universe_key = ?1",
        );
        let mut bind: Vec<String> = vec![universe_key.to_string()];

        if let Some(s) = source_contains {
            bind.push(format!("%{s}%"));
            sql.push_str(&format!(" AND source LIKE ?{}", bind.len()));
        }
        if let Some(u) = url_contains {
            bind.push(format!("%{u}%"));
            sql.push_str(&format!(" AND url LIKE ?{}", bind.len()));
        }
        sql.push_str(" ORDER BY entry_path, ref_index");

        let mut stmt = self.conn.prepare(&sql)?;
        let params = rusqlite::params_from_iter(bind.iter());
        let rows = stmt.query_map(params, row_to_ref)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn query_fts(&self, universe_key: &str, q: &str) -> anyhow::Result<Vec<ReferenceRow>> {
        let fts_q = q.replace('+', " ");
        let mut stmt = self.conn.prepare(
            "SELECT r.universe_key, r.entry_path, r.ref_index, r.url, r.source, \
                     r.retrieved, r.excerpt_body \
             FROM references_fts fts \
             JOIN references_index r \
               ON r.universe_key = fts.universe_key \
              AND r.entry_path   = fts.entry_path \
              AND r.ref_index    = fts.ref_index \
             WHERE fts.universe_key = ?1 AND references_fts MATCH ?2 \
             ORDER BY r.entry_path, r.ref_index",
        )?;
        let rows = stmt.query_map(params![universe_key, fts_q], row_to_ref)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Find wikilinks inside `## Referência:` excerpts that have no
    /// corresponding entry in the same universe.
    ///
    /// Returns a sorted list of orphaned `[[target]]` strings.
    pub fn orphan_wikilinks(&self, universe_key: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT excerpt_body FROM references_index \
             WHERE universe_key = ?1 AND excerpt_body IS NOT NULL",
        )?;
        let bodies: Vec<String> = stmt
            .query_map(params![universe_key], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut orphans: std::collections::BTreeSet<String> = Default::default();
        for body in &bodies {
            for link in extract_wikilinks(body) {
                if link.starts_with('/') {
                    continue;
                }
                let exists: bool = self
                    .conn
                    .query_row(
                        "SELECT 1 FROM entries WHERE universe_key = ?1 AND path = ?2",
                        params![universe_key, link],
                        |_| Ok(true),
                    )
                    .ok()
                    .unwrap_or(false);
                if !exists {
                    orphans.insert(link);
                }
            }
        }

        Ok(orphans.into_iter().collect())
    }
}

/// Extract all `[[target]]` or `[[target|alias]]` wikilink targets from text.
fn extract_wikilinks(text: &str) -> Vec<String> {
    let mut links = vec![];
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        if let Some(end) = rest.find("]]") {
            let inner = &rest[..end];
            let target = inner.split('|').next().unwrap_or(inner).trim();
            if !target.is_empty() {
                links.push(target.to_string());
            }
            rest = &rest[end + 2..];
        } else {
            break;
        }
    }
    links
}

fn row_to_ref(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReferenceRow> {
    Ok(ReferenceRow {
        universe_key: row.get(0)?,
        entry_path: row.get(1)?,
        ref_index: row.get(2)?,
        url: row.get(3)?,
        source: row.get(4)?,
        retrieved: row.get(5)?,
        excerpt_body: row.get(6)?,
    })
}

// ---------------------------------------------------------------------------
// Sync (called from entry write paths)
// ---------------------------------------------------------------------------

/// Sync references for a single entry: extract from frontmatter + body,
/// replace all rows in DB.
///
/// Called from entry write paths (create, update) and vault writes.
pub fn sync_entry_references(
    conn: &Connection,
    universe_key: &str,
    path: &str,
    frontmatter: &serde_json::Value,
    body: &str,
) -> anyhow::Result<()> {
    let candidates = extract_references(frontmatter, body);
    ReferenceIndex::new(conn).replace_all(universe_key, path, &candidates)
}

// ---------------------------------------------------------------------------
// Backfill (called from v7 migration)
// ---------------------------------------------------------------------------

/// One-shot backfill: walk all entries in `references_index` for a universe
/// and populate `references_index` + `references_fts` from their frontmatter +
/// body. Returns the number of entries processed.
///
/// Called from the v7 per-universe DB migration. Idempotent — re-running after
/// the initial backfill just overwrites existing rows.
pub fn backfill_references(conn: &Connection, universe_key: &str) -> anyhow::Result<usize> {
    let mut stmt =
        conn.prepare("SELECT path, frontmatter_json, body FROM entries WHERE universe_key = ?1")?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map(params![universe_key], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut count = 0usize;
    for (path, fm_json, body) in rows {
        if let Ok(fm) = serde_json::from_str::<serde_json::Value>(&fm_json) {
            let _ = sync_entry_references(conn, universe_key, &path, &fm, &body);
            count += 1;
        }
    }
    if count > 0 {
        tracing::info!(
            universe = %universe_key,
            entries = count,
            "CO-154: reference backfill complete"
        );
    }
    Ok(count)
}

/// Spawn a background thread to backfill references for a universe.
///
/// Fire-and-forget: failures are logged but do not propagate.
pub fn backfill_references_background(universe_pool: Arc<UniversePool>, universe_key: String) {
    std::thread::spawn(move || {
        let conn_arc = universe_pool.get_or_open(&universe_key);
        match conn_arc.lock() {
            Ok(conn) => match backfill_references(&conn, &universe_key) {
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    universe = %universe_key,
                    error = %e,
                    "CO-154: reference backfill failed"
                ),
            },
            Err(e) => tracing::warn!(
                universe = %universe_key,
                error = ?e,
                "CO-154: reference backfill: failed to lock universe DB"
            ),
        }
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                path             TEXT NOT NULL,
                universe_key     TEXT NOT NULL,
                entry_type       TEXT NOT NULL,
                title            TEXT,
                frontmatter_json TEXT NOT NULL DEFAULT '{}',
                payload          TEXT NOT NULL DEFAULT '{}',
                body             TEXT NOT NULL DEFAULT '',
                body_hash        TEXT NOT NULL DEFAULT '',
                created_at       TEXT,
                updated_at       TEXT,
                PRIMARY KEY (universe_key, path)
            );
            CREATE TABLE IF NOT EXISTS references_index (
                universe_key  TEXT NOT NULL,
                entry_path    TEXT NOT NULL,
                ref_index     INTEGER NOT NULL,
                url           TEXT,
                source        TEXT NOT NULL,
                retrieved     TEXT,
                excerpt_body  TEXT,
                PRIMARY KEY (universe_key, entry_path, ref_index)
            );
            CREATE INDEX IF NOT EXISTS idx_refs_source
                ON references_index(source);
            CREATE INDEX IF NOT EXISTS idx_refs_url
                ON references_index(url) WHERE url IS NOT NULL;
            CREATE VIRTUAL TABLE IF NOT EXISTS references_fts USING fts5(
                universe_key UNINDEXED,
                entry_path   UNINDEXED,
                ref_index    UNINDEXED,
                source,
                excerpt_body
            );",
        )
        .unwrap();
        conn
    }

    fn count_ref_rows(conn: &Connection, universe_key: &str, entry_path: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM references_index \
             WHERE universe_key = ?1 AND entry_path = ?2",
            params![universe_key, entry_path],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    // ---- extract_references ----

    #[test]
    fn test_extract_three_refs_no_body_sections() {
        let fm = json!({
            "references": [
                {"url": "https://a.com", "source": "Source A", "retrieved": "2026-01-01"},
                {"source": "Source B", "retrieved": "2026-02-01"},
                {"url": "https://c.com", "source": "Source C"},
            ]
        });
        let candidates = extract_references(&fm, "");
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].source, "Source A");
        assert_eq!(candidates[0].url.as_deref(), Some("https://a.com"));
        assert_eq!(candidates[1].source, "Source B");
        assert!(candidates[1].url.is_none());
        assert_eq!(candidates[2].source, "Source C");
        assert!(candidates[2].excerpt_body.is_none());
    }

    #[test]
    fn test_extract_refs_with_body_sections() {
        let fm = json!({
            "references": [
                {"source": "Source A"},
                {"source": "Source B"},
            ]
        });
        let body = "
Some intro text.

## Referência: Source A

Content for A line 1
Content for A line 2

## Referência: Source B

Content for B

## Other Section

Unrelated content.
";
        let candidates = extract_references(&fm, body);
        assert_eq!(candidates.len(), 2);
        assert!(
            candidates[0]
                .excerpt_body
                .as_deref()
                .unwrap_or("")
                .contains("Content for A")
        );
        assert!(
            candidates[1]
                .excerpt_body
                .as_deref()
                .unwrap_or("")
                .contains("Content for B")
        );
        // "Other Section" content must not leak into B's excerpt
        assert!(
            !candidates[1]
                .excerpt_body
                .as_deref()
                .unwrap_or("")
                .contains("Unrelated")
        );
    }

    #[test]
    fn test_no_references_returns_empty() {
        let fm = json!({"type": "term", "word": "foo"});
        let candidates = extract_references(&fm, "body text");
        assert!(candidates.is_empty());
    }

    // ---- extract_wikilinks ----

    #[test]
    fn test_extract_wikilinks_bare() {
        let links = extract_wikilinks("See [[terms/ogum.md]] and [[terms/iya.md]].");
        assert_eq!(links, vec!["terms/ogum.md", "terms/iya.md"]);
    }

    #[test]
    fn test_extract_wikilinks_with_alias() {
        let links = extract_wikilinks("See [[terms/ogum.md|Ogum]] and [[terms/iya.md|Ìyá]].");
        assert_eq!(links, vec!["terms/ogum.md", "terms/iya.md"]);
    }

    #[test]
    fn test_extract_wikilinks_empty() {
        let links = extract_wikilinks("No links here.");
        assert!(links.is_empty());
    }

    // ---- sync_entry_references ----

    #[test]
    fn test_round_trip_three_references() {
        let conn = setup_db();
        let fm = json!({
            "references": [
                {"url": "https://a.com", "source": "Source A", "retrieved": "2026-01-01"},
                {"source": "Source B"},
                {"url": "https://c.com", "source": "Source C"},
            ]
        });
        sync_entry_references(&conn, "u1", "terms/test.md", &fm, "").unwrap();
        assert_eq!(count_ref_rows(&conn, "u1", "terms/test.md"), 3);
    }

    #[test]
    fn test_sync_replaces_on_update() {
        let conn = setup_db();
        let fm1 = json!({"references": [{"source": "A"}, {"source": "B"}]});
        sync_entry_references(&conn, "u1", "terms/test.md", &fm1, "").unwrap();
        assert_eq!(count_ref_rows(&conn, "u1", "terms/test.md"), 2);

        let fm2 = json!({"references": [{"source": "C"}]});
        sync_entry_references(&conn, "u1", "terms/test.md", &fm2, "").unwrap();
        assert_eq!(count_ref_rows(&conn, "u1", "terms/test.md"), 1);

        let idx = ReferenceIndex::new(&conn);
        let rows = idx.query("u1", None, None, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "C");
    }

    #[test]
    fn test_sync_removes_on_empty_references() {
        let conn = setup_db();
        let fm = json!({"references": [{"source": "A"}]});
        sync_entry_references(&conn, "u1", "terms/test.md", &fm, "").unwrap();
        assert_eq!(count_ref_rows(&conn, "u1", "terms/test.md"), 1);

        let fm_empty = json!({"type": "term"});
        sync_entry_references(&conn, "u1", "terms/test.md", &fm_empty, "").unwrap();
        assert_eq!(count_ref_rows(&conn, "u1", "terms/test.md"), 0);
    }

    // ---- ReferenceIndex::query ----

    #[test]
    fn test_query_source_substring() {
        let conn = setup_db();
        let fm = json!({
            "references": [
                {"source": "meuorixa.wordpress.com — Yemanjá Ogunté", "url": "https://meuorixa.wordpress.com/2013/05/02/yemanja-ogunte/"},
                {"source": "other-source.com"},
            ]
        });
        sync_entry_references(&conn, "yoruba", "terms/ogunte.md", &fm, "").unwrap();

        let idx = ReferenceIndex::new(&conn);
        let rows = idx
            .query("yoruba", Some("meuorixa.wordpress.com"), None, None)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry_path, "terms/ogunte.md");
    }

    #[test]
    fn test_query_url_contains() {
        let conn = setup_db();
        let fm = json!({
            "references": [
                {"source": "S1", "url": "https://example.com/page1"},
                {"source": "S2", "url": "https://other.com/page2"},
            ]
        });
        sync_entry_references(&conn, "u1", "terms/test.md", &fm, "").unwrap();

        let idx = ReferenceIndex::new(&conn);
        let rows = idx.query("u1", None, Some("example.com"), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "S1");
    }

    #[test]
    fn test_query_fts() {
        let conn = setup_db();
        let fm = json!({"references": [{"source": "Source A"}]});
        let body = "## Referência: Source A\n\nThis text mentions maternidade and the orixá.";
        sync_entry_references(&conn, "u1", "terms/test.md", &fm, body).unwrap();

        let idx = ReferenceIndex::new(&conn);
        let rows = idx.query("u1", None, None, Some("maternidade")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "Source A");
    }

    // ---- orphan_wikilinks ----

    #[test]
    fn test_orphan_wikilinks_reports_missing_entries() {
        let conn = setup_db();

        // Seed an existing entry
        conn.execute(
            "INSERT INTO entries (path, universe_key, entry_type, frontmatter_json, payload, body_hash) \
             VALUES ('terms/ogum.md', 'u1', 'term', '{}', '{}', '')",
            [],
        )
        .unwrap();

        let fm = json!({"references": [{"source": "Source A"}]});
        let body =
            "## Referência: Source A\n\n[[terms/ogum.md|Ogum]] and [[terms/missing.md|Missing]].";
        sync_entry_references(&conn, "u1", "terms/test.md", &fm, body).unwrap();

        let idx = ReferenceIndex::new(&conn);
        let orphans = idx.orphan_wikilinks("u1").unwrap();
        assert!(
            !orphans.contains(&"terms/ogum.md".to_string()),
            "existing entry must not be orphan"
        );
        assert!(
            orphans.contains(&"terms/missing.md".to_string()),
            "missing entry must be orphan"
        );
    }

    // ---- backfill ----

    #[test]
    fn test_backfill_populates_from_entries_table() {
        let conn = setup_db();

        // Seed entries directly without going through the write path
        let fm = serde_json::to_string(&json!({
            "references": [
                {"source": "Source A", "url": "https://a.com"},
                {"source": "Source B"},
                {"source": "Source C"},
            ]
        }))
        .unwrap();
        conn.execute(
            "INSERT INTO entries (path, universe_key, entry_type, frontmatter_json, payload, body_hash) \
             VALUES ('terms/test.md', 'u1', 'term', ?1, ?1, '')",
            params![fm],
        )
        .unwrap();

        let n = backfill_references(&conn, "u1").unwrap();
        assert_eq!(n, 1, "one entry processed");
        assert_eq!(count_ref_rows(&conn, "u1", "terms/test.md"), 3);
    }
}
