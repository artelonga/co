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

use std::sync::Arc;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::universe_pool::UniversePool;

// ---------------------------------------------------------------------------
// co:// URI resolver (CO-153)
// ---------------------------------------------------------------------------

/// Resolved components of a `co://` cross-universe reference or a plain path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossRef {
    /// Target universe key — `None` means same universe as the from-side.
    pub universe: Option<String>,
    /// Entry path within the target universe.
    pub path: String,
}

/// Parse a frontmatter ref value into a `CrossRef`.
///
/// `co://<universe>/<path>` → `CrossRef { universe: Some("universe"), path: "<path>" }`.
/// Anything else → `CrossRef { universe: None, path: s }`.
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
    /// Each element is `(relation_type, to_path, to_universe)` where
    /// `to_universe = None` means same-universe (back-compat with CO-74).
    /// Calling with an empty slice removes all existing relations. Idempotent.
    pub fn replace_all(
        &self,
        universe_key: &str,
        from_path: &str,
        relations: &[(String, String, Option<String>)],
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM entry_relations WHERE universe_key = ?1 AND from_path = ?2",
            params![universe_key, from_path],
        )?;
        if relations.is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        for (relation_type, to_path, to_universe) in relations {
            self.conn.execute(
                "INSERT OR REPLACE INTO entry_relations \
                 (universe_key, from_path, to_path, relation_type, created_at, to_universe) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    universe_key,
                    from_path,
                    to_path,
                    relation_type,
                    now,
                    to_universe
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
            "SELECT universe_key, from_path, to_path, relation_type, created_at, to_universe \
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
            "SELECT universe_key, from_path, to_path, relation_type, created_at, to_universe \
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
            "SELECT universe_key, from_path, to_path, relation_type, created_at, to_universe \
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
    })
}

// ---------------------------------------------------------------------------
// Relation extraction from frontmatter
// ---------------------------------------------------------------------------

/// Extract `(relation_type, to_path, to_universe)` triples from an entry's
/// frontmatter JSON using the manifest schema for `entry_type`.
///
/// Only fields declared as `ref` or `ref_list` in the manifest contribute rows.
/// Wikilink notation is stripped via `co::wikilink::resolve_ref_value`.
/// `co://` URIs (CO-153) are split into `(to_path, Some(to_universe))`.
/// Plain paths produce `to_universe = None` (same universe).
pub fn extract_relations(
    manifest: &co::manifest::Manifest,
    entry_type: &str,
    frontmatter: &serde_json::Value,
) -> Vec<(String, String, Option<String>)> {
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
                        relations.push((field_name.clone(), cr.path, cr.universe));
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
                                relations.push((field_name.clone(), cr.path, cr.universe));
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

/// Sync relations for a single entry: extract from frontmatter, replace in DB.
///
/// Called from entry write paths (create, update) and from vault writes.
/// Silently skips if the manifest has no schema for `entry_type`.
/// Sync relations and return the number of relations upserted (0 = all cleared).
pub fn sync_entry_relations(
    conn: &Connection,
    universe_key: &str,
    path: &str,
    entry_type: &str,
    frontmatter: &serde_json::Value,
    manifest: &co::manifest::Manifest,
) -> anyhow::Result<usize> {
    let relations = extract_relations(manifest, entry_type, frontmatter);
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
            "SELECT path, frontmatter_json FROM entries \
             WHERE universe_key = ?1 AND entry_type = ?2",
        )?;
        let entries: Vec<(String, String)> = stmt
            .query_map(params![universe_key, type_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (path, fm_json) in entries {
            if let Ok(fm) = serde_json::from_str::<serde_json::Value>(&fm_json) {
                let relations = extract_relations(manifest, type_name, &fm);
                RelationIndex::new(conn).replace_all(universe_key, &path, &relations)?;
                total += 1;
            }
        }
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use co::manifest::{
        Cardinality, ContentType, FieldDef, FieldType, Manifest, Presentation, Relationship,
    };
    use rusqlite::Connection;
    use serde_json::json;
    use std::collections::BTreeMap;

    // ---- DB helpers ----

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE entries (
                path TEXT NOT NULL,
                universe_key TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                title TEXT,
                frontmatter_json TEXT NOT NULL DEFAULT '{}',
                payload TEXT NOT NULL DEFAULT '{}',
                body TEXT NOT NULL DEFAULT '',
                body_hash TEXT NOT NULL DEFAULT '',
                created_at TEXT,
                updated_at TEXT,
                PRIMARY KEY (universe_key, path)
            );
            CREATE TABLE entry_relations (
                universe_key  TEXT NOT NULL,
                from_path     TEXT NOT NULL,
                to_path       TEXT NOT NULL,
                relation_type TEXT NOT NULL,
                created_at    TEXT NOT NULL,
                to_universe   TEXT,
                PRIMARY KEY (universe_key, from_path, to_path, relation_type)
            );
            CREATE INDEX idx_er_from ON entry_relations(universe_key, from_path, relation_type);
            CREATE INDEX idx_er_to   ON entry_relations(universe_key, to_path,   relation_type);",
        )
        .unwrap();
        conn
    }

    fn evento_manifest() -> Manifest {
        let mut schema = BTreeMap::new();
        schema.insert(
            "attendees".to_string(),
            FieldDef {
                field_type: FieldType::RefList,
                required: false,
                values: vec![],
                semantic: None,
                target: Some("pessoa".to_string()),
            },
        );
        schema.insert(
            "title".to_string(),
            FieldDef {
                field_type: FieldType::String,
                required: true,
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
            relationships: vec![Relationship {
                from: "evento.attendees".to_string(),
                to: "pessoa".to_string(),
                cardinality: Cardinality::ManyToMany,
            }],
            views: vec![],
            properties_per_type: std::collections::BTreeMap::new(),
        }
    }

    fn term_manifest() -> Manifest {
        let mut schema = BTreeMap::new();
        schema.insert(
            "concept".to_string(),
            FieldDef {
                field_type: FieldType::Ref,
                required: false,
                values: vec![],
                semantic: None,
                target: Some("concept".to_string()),
            },
        );
        schema.insert(
            "word".to_string(),
            FieldDef {
                field_type: FieldType::String,
                required: true,
                values: vec![],
                semantic: None,
                target: None,
            },
        );
        Manifest {
            schema_version: 1,
            name: "guarani-mbya".to_string(),
            content_types: vec![ContentType {
                name: "term".to_string(),
                schema,
                presentation: Presentation::default(),
                indexes: vec![],
            }],
            doc_generators: vec![],
            relationships: vec![],
            views: vec![],
        }
    }

    // ---- parse_co_uri ----

    #[test]
    fn test_parse_co_uri_cross_universe() {
        let cr = parse_co_uri("co://concepts/mother.md").unwrap();
        assert_eq!(cr.universe, Some("concepts".to_string()));
        assert_eq!(cr.path, "mother.md");
    }

    #[test]
    fn test_parse_co_uri_cross_universe_subpath() {
        let cr = parse_co_uri("co://concepts/concepts/mother.md").unwrap();
        assert_eq!(cr.universe, Some("concepts".to_string()));
        assert_eq!(cr.path, "concepts/mother.md");
    }

    #[test]
    fn test_parse_co_uri_plain_path() {
        let cr = parse_co_uri("terms/xy.md").unwrap();
        assert_eq!(cr.universe, None);
        assert_eq!(cr.path, "terms/xy.md");
    }

    #[test]
    fn test_parse_co_uri_malformed_returns_none() {
        // No slash after universe component
        assert!(parse_co_uri("co://noslash").is_none());
    }

    // ---- extract_relations ----

    #[test]
    fn test_extract_ref_list_creates_multiple_relations() {
        let manifest = evento_manifest();
        let fm = json!({"attendees": ["pessoas/yuri.md", "pessoas/ana.md"], "title": "Hackathon"});
        let rels = extract_relations(&manifest, "evento", &fm);
        assert_eq!(rels.len(), 2);
        assert!(rels.contains(&("attendees".to_string(), "pessoas/yuri.md".to_string(), None)));
        assert!(rels.contains(&("attendees".to_string(), "pessoas/ana.md".to_string(), None)));
    }

    #[test]
    fn test_extract_ref_list_resolves_wikilinks() {
        let manifest = evento_manifest();
        let fm =
            json!({"attendees": ["[[pessoas/yuri.md]]", "[[pessoas/ana.md|Ana]]"], "title": "X"});
        let rels = extract_relations(&manifest, "evento", &fm);
        assert_eq!(rels.len(), 2);
        assert!(rels.contains(&("attendees".to_string(), "pessoas/yuri.md".to_string(), None)));
        assert!(rels.contains(&("attendees".to_string(), "pessoas/ana.md".to_string(), None)));
    }

    #[test]
    fn test_extract_co_uri_populates_to_universe() {
        let manifest = term_manifest();
        let fm = json!({"concept": "co://concepts/mother.md", "word": "jaryi"});
        let rels = extract_relations(&manifest, "term", &fm);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].0, "concept");
        assert_eq!(rels[0].1, "mother.md");
        assert_eq!(rels[0].2, Some("concepts".to_string()));
    }

    #[test]
    fn test_extract_co_uri_subpath() {
        let manifest = term_manifest();
        let fm = json!({"concept": "co://concepts/concepts/mother.md", "word": "jaryi"});
        let rels = extract_relations(&manifest, "term", &fm);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].1, "concepts/mother.md");
        assert_eq!(rels[0].2, Some("concepts".to_string()));
    }

    #[test]
    fn test_extract_string_field_no_relation() {
        let manifest = evento_manifest();
        // "title" is a plain string field — wikilinks in it must not create relations
        let fm = json!({"attendees": [], "title": "[[some wikilink]]"});
        let rels = extract_relations(&manifest, "evento", &fm);
        assert!(rels.is_empty(), "non-ref field must not produce relations");
    }

    #[test]
    fn test_extract_unknown_type_returns_empty() {
        let manifest = evento_manifest();
        let fm = json!({"anything": "value"});
        let rels = extract_relations(&manifest, "unknown_type", &fm);
        assert!(rels.is_empty());
    }

    // ---- RelationIndex ----

    #[test]
    fn test_replace_all_inserts_rows() {
        let conn = setup_db();
        let idx = RelationIndex::new(&conn);
        idx.replace_all(
            "u1",
            "events/hackathon.md",
            &[
                ("attendees".to_string(), "pessoas/yuri.md".to_string(), None),
                ("attendees".to_string(), "pessoas/ana.md".to_string(), None),
            ],
        )
        .unwrap();

        let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_replace_all_idempotent() {
        let conn = setup_db();
        let idx = RelationIndex::new(&conn);
        let rels = [("attendees".to_string(), "pessoas/yuri.md".to_string(), None)];
        idx.replace_all("u1", "events/hackathon.md", &rels).unwrap();
        idx.replace_all("u1", "events/hackathon.md", &rels).unwrap();

        let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
        assert_eq!(rows.len(), 1, "duplicate replace must not add rows");
    }

    #[test]
    fn test_replace_all_removes_stale_relations() {
        let conn = setup_db();
        let idx = RelationIndex::new(&conn);
        idx.replace_all(
            "u1",
            "events/hackathon.md",
            &[
                ("attendees".to_string(), "pessoas/yuri.md".to_string(), None),
                ("attendees".to_string(), "pessoas/ana.md".to_string(), None),
            ],
        )
        .unwrap();
        // Update: only yuri remains
        idx.replace_all(
            "u1",
            "events/hackathon.md",
            &[("attendees".to_string(), "pessoas/yuri.md".to_string(), None)],
        )
        .unwrap();

        let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].to_path, "pessoas/yuri.md");
    }

    #[test]
    fn test_delete_for_entry() {
        let conn = setup_db();
        let idx = RelationIndex::new(&conn);
        idx.replace_all(
            "u1",
            "events/hackathon.md",
            &[("attendees".to_string(), "pessoas/yuri.md".to_string(), None)],
        )
        .unwrap();
        idx.delete_for_entry("u1", "events/hackathon.md").unwrap();

        let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_inbound_lookup() {
        let conn = setup_db();
        let idx = RelationIndex::new(&conn);
        idx.replace_all(
            "u1",
            "events/hackathon.md",
            &[("attendees".to_string(), "pessoas/yuri.md".to_string(), None)],
        )
        .unwrap();
        idx.replace_all(
            "u1",
            "events/conf.md",
            &[("attendees".to_string(), "pessoas/yuri.md".to_string(), None)],
        )
        .unwrap();

        let inbound = idx.inbound("u1", "pessoas/yuri.md").unwrap();
        assert_eq!(inbound.len(), 2);
        let from_paths: Vec<&str> = inbound.iter().map(|r| r.from_path.as_str()).collect();
        assert!(from_paths.contains(&"events/hackathon.md"));
        assert!(from_paths.contains(&"events/conf.md"));
    }

    #[test]
    fn test_cross_universe_inbound_via_inbound_from_other() {
        // Simulate the CO-153 scenario:
        // guarani-mbya universe has a term pointing to concepts/mother.md
        let conn = setup_db();
        let idx = RelationIndex::new(&conn);

        // Store a cross-universe relation: guarani-mbya → concepts/mother.md
        idx.replace_all(
            "guarani-mbya",
            "terms/jaryi.md",
            &[(
                "concept".to_string(),
                "mother.md".to_string(),
                Some("concepts".to_string()),
            )],
        )
        .unwrap();

        // inbound_from_other: find all rows pointing to concepts/mother.md
        let inbound = idx.inbound_from_other("concepts", "mother.md").unwrap();
        assert_eq!(inbound.len(), 1);
        assert_eq!(inbound[0].universe_key, "guarani-mbya");
        assert_eq!(inbound[0].from_path, "terms/jaryi.md");
        assert_eq!(inbound[0].relation_type, "concept");
        assert_eq!(inbound[0].to_universe, Some("concepts".to_string()));
        assert_eq!(inbound[0].to_path, "mother.md");
    }

    #[test]
    fn test_cross_universe_concept_mother_inbound() {
        // Full CO-153 acceptance test:
        // concepts/mother.md shows inbound concept relations from
        // guarani-mbya, portuguese, and yoruba.
        let conn = setup_db();
        let idx = RelationIndex::new(&conn);

        for (universe, term_path) in [
            ("guarani-mbya", "terms/jaryi.md"),
            ("portuguese", "terms/mae.md"),
            ("yoruba", "terms/iya.md"),
        ] {
            idx.replace_all(
                universe,
                term_path,
                &[(
                    "concept".to_string(),
                    "mother.md".to_string(),
                    Some("concepts".to_string()),
                )],
            )
            .unwrap();
        }

        let inbound = idx.inbound_from_other("concepts", "mother.md").unwrap();
        assert_eq!(inbound.len(), 3, "three language planes point to mother");

        let from_universes: Vec<&str> = inbound.iter().map(|r| r.universe_key.as_str()).collect();
        assert!(from_universes.contains(&"guarani-mbya"));
        assert!(from_universes.contains(&"portuguese"));
        assert!(from_universes.contains(&"yoruba"));
        assert!(inbound.iter().all(|r| r.relation_type == "concept"));
    }

    // ---- backfill ----

    #[test]
    fn test_backfill_for_manifest_creates_relations() {
        let conn = setup_db();
        // Seed an entry WITHOUT going through the write path
        conn.execute(
            "INSERT INTO entries (path, universe_key, entry_type, frontmatter_json, payload, body_hash)
             VALUES ('events/hackathon.md', 'u1', 'evento',
                     '{\"attendees\":[\"pessoas/yuri.md\"],\"title\":\"Hack\"}',
                     '{\"attendees\":[\"pessoas/yuri.md\"],\"title\":\"Hack\"}',
                     'abc')",
            [],
        )
        .unwrap();

        let manifest = evento_manifest();
        let n = backfill_for_manifest(&conn, "u1", &manifest).unwrap();
        assert_eq!(n, 1, "one entry should be processed");

        let idx = RelationIndex::new(&conn);
        let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].to_path, "pessoas/yuri.md");
        assert_eq!(rows[0].relation_type, "attendees");
        assert_eq!(rows[0].to_universe, None);
    }

    #[test]
    fn test_backfill_for_manifest_co_uri_populates_to_universe() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO entries (path, universe_key, entry_type, frontmatter_json, payload, body_hash)
             VALUES ('terms/jaryi.md', 'guarani-mbya', 'term',
                     '{\"concept\":\"co://concepts/mother.md\",\"word\":\"jaryi\"}',
                     '{\"concept\":\"co://concepts/mother.md\",\"word\":\"jaryi\"}',
                     'abc')",
            [],
        )
        .unwrap();

        let manifest = term_manifest();
        let n = backfill_for_manifest(&conn, "guarani-mbya", &manifest).unwrap();
        assert_eq!(n, 1);

        let idx = RelationIndex::new(&conn);
        let rows = idx.outbound("guarani-mbya", "terms/jaryi.md").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].to_path, "mother.md");
        assert_eq!(rows[0].to_universe, Some("concepts".to_string()));
    }

    #[test]
    fn test_backfill_no_affected_types_is_noop() {
        let conn = setup_db();
        // Manifest with no ref fields
        let manifest = Manifest {
            schema_version: 1,
            name: "X".to_string(),
            content_types: vec![ContentType {
                name: "tarefa".to_string(),
                schema: BTreeMap::new(),
                presentation: Presentation::default(),
                indexes: vec![],
            }],
            doc_generators: vec![],
            relationships: vec![],
            views: vec![],
            properties_per_type: std::collections::BTreeMap::new(),
        };
        let n = backfill_for_manifest(&conn, "u1", &manifest).unwrap();
        assert_eq!(n, 0);
    }
}
