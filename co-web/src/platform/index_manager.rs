//! Expression index manager for CO-71 generic JSON payload.
//!
//! Manages per-universe SQLite expression indexes declared in the universe
//! manifest (`_universe.yaml`).  When a manifest declares
//! `indexes: [status, due_at]` for a content type, this module creates:
//!
//! ```sql
//! CREATE INDEX IF NOT EXISTS idx_co71_<universe>_<field>
//!     ON entries(universe_key, json_extract(payload, '$.<field>'));
//! ```
//!
//! Index creation is always run on a background thread to avoid blocking the
//! HTTP request that triggered the manifest update.

use std::path::PathBuf;

use rusqlite::{Connection, params};

use co::manifest::ContentType;

/// Prefix used for all CO-71-managed indexes; enables safe diffing in `sqlite_master`.
const INDEX_PREFIX: &str = "idx_co71";

/// Sanitize a string for use in an index name (replace non-alphanumeric chars).
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

/// Compute the index name for `(universe_key, field)`.
fn index_name(universe_key: &str, field: &str) -> String {
    format!(
        "{INDEX_PREFIX}_{}_{}",
        sanitize(universe_key),
        sanitize(field)
    )
}

/// Return `true` if `field` is a safe SQL identifier (alphanumeric + underscore).
///
/// We never interpolate unsanitized user data into SQL; this guard ensures
/// that index field names from the manifest cannot carry SQL injection.
fn is_safe_identifier(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

// ---------------------------------------------------------------------------
// IndexManager
// ---------------------------------------------------------------------------

/// Manages expression indexes for a universe on an open [`Connection`].
pub struct IndexManager<'a> {
    conn: &'a Connection,
}

impl<'a> IndexManager<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Return the set of fields currently indexed for `universe_key`.
    ///
    /// Reads `sqlite_master` for indexes whose names match the CO-71 prefix.
    pub fn existing_fields(&self, universe_key: &str) -> anyhow::Result<Vec<String>> {
        let prefix = format!("{INDEX_PREFIX}_{}_", sanitize(universe_key));
        let mut stmt = self.conn.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE ?1 || '%'",
        )?;
        let names: Vec<String> = stmt
            .query_map(params![prefix], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let fields = names
            .into_iter()
            .filter_map(|name| {
                // strip the prefix to recover the field name
                name.strip_prefix(&prefix).map(String::from)
            })
            .collect();

        Ok(fields)
    }

    /// Create expression indexes for all fields declared in `content_types`
    /// that do not already exist.
    ///
    /// Only fields with safe identifier names are indexed.
    pub fn apply_indexes(
        &self,
        universe_key: &str,
        content_types: &[ContentType],
    ) -> anyhow::Result<()> {
        let existing: std::collections::HashSet<String> =
            self.existing_fields(universe_key)?.into_iter().collect();

        let mut declared: std::collections::HashSet<String> = std::collections::HashSet::new();
        for ct in content_types {
            for field in &ct.indexes {
                declared.insert(field.clone());
            }
        }

        for field in &declared {
            if existing.contains(field) {
                continue;
            }
            if !is_safe_identifier(field) {
                tracing::warn!(
                    universe = universe_key,
                    field,
                    "skipping index for unsafe field name"
                );
                continue;
            }
            let name = index_name(universe_key, field);
            // Safety: `name` is sanitized; `field` is validated as safe identifier above.
            let ddl = format!(
                "CREATE INDEX IF NOT EXISTS {name} \
                 ON entries(universe_key, json_extract(payload, '$.{field}'))"
            );
            self.conn.execute_batch(&ddl)?;
            tracing::info!(universe = universe_key, field, "created expression index");
        }

        Ok(())
    }

    /// Drop expression indexes for `universe_key` that are no longer declared
    /// in `content_types`.
    pub fn drop_stale_indexes(
        &self,
        universe_key: &str,
        content_types: &[ContentType],
    ) -> anyhow::Result<()> {
        let existing = self.existing_fields(universe_key)?;

        let declared: std::collections::HashSet<String> = content_types
            .iter()
            .flat_map(|ct| ct.indexes.iter().cloned())
            .collect();

        for field in existing {
            if !declared.contains(&field) {
                let name = index_name(universe_key, &field);
                self.conn
                    .execute_batch(&format!("DROP INDEX IF EXISTS {name}"))?;
                tracing::info!(
                    universe = universe_key,
                    field,
                    "dropped stale expression index"
                );
            }
        }

        Ok(())
    }

    /// Apply the full diff: create new indexes and drop removed ones.
    pub fn sync_indexes(
        &self,
        universe_key: &str,
        content_types: &[ContentType],
    ) -> anyhow::Result<()> {
        self.drop_stale_indexes(universe_key, content_types)?;
        self.apply_indexes(universe_key, content_types)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Background backfill
// ---------------------------------------------------------------------------

/// Spawn a background thread that opens `db_path` and applies manifest indexes
/// for `universe_key`.  Returns immediately without blocking the caller.
///
/// This ensures that a new index can be built without blocking HTTP writes.
pub fn apply_manifest_indexes_background(
    db_path: PathBuf,
    universe_key: String,
    content_types: Vec<ContentType>,
) {
    std::thread::spawn(move || match Connection::open(&db_path) {
        Ok(conn) => {
            let manager = IndexManager::new(&conn);
            if let Err(e) = manager.sync_indexes(&universe_key, &content_types) {
                tracing::warn!(
                    universe = %universe_key,
                    error = %e,
                    "background index sync failed"
                );
            } else {
                tracing::info!(universe = %universe_key, "background index sync complete");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "background index sync: failed to open DB");
        }
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use co::manifest::{ContentType, FieldDef, FieldType, Presentation};
    use rusqlite::{Connection, OptionalExtension};
    use std::collections::BTreeMap;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (24);
             CREATE TABLE entries (
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
             );",
        )
        .unwrap();
        conn
    }

    fn make_ct(name: &str, indexes: &[&str]) -> ContentType {
        ContentType {
            name: name.to_string(),
            schema: BTreeMap::new(),
            presentation: Presentation::default(),
            indexes: indexes.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn make_ct_with_fields(
        name: &str,
        fields: &[(&str, FieldType)],
        indexes: &[&str],
    ) -> ContentType {
        let mut schema = BTreeMap::new();
        for (fname, ft) in fields {
            schema.insert(
                fname.to_string(),
                FieldDef {
                    field_type: ft.clone(),
                    required: false,
                    values: if matches!(ft, FieldType::Enum) {
                        vec!["todo".into(), "done".into()]
                    } else {
                        vec![]
                    },
                    semantic: None,
                    target: if matches!(ft, FieldType::Ref | FieldType::RefList) {
                        Some("other".to_string())
                    } else {
                        None
                    },
                },
            );
        }
        ContentType {
            name: name.to_string(),
            schema,
            presentation: Presentation::default(),
            indexes: indexes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_apply_indexes_creates_index() {
        let conn = setup_db();
        let manager = IndexManager::new(&conn);
        let ct = make_ct("tarefa", &["status"]);

        manager.apply_indexes("myuniverse", &[ct]).unwrap();

        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_co71_myuniverse_status'",
                [],
                |_| Ok(true),
            )
            .optional()
            .unwrap()
            .unwrap_or(false);
        assert!(exists, "index should exist after apply_indexes");
    }

    #[test]
    fn test_apply_indexes_idempotent() {
        let conn = setup_db();
        let manager = IndexManager::new(&conn);
        let ct = make_ct("tarefa", &["status"]);

        // Apply twice — should not fail
        manager
            .apply_indexes("myuniverse", std::slice::from_ref(&ct))
            .unwrap();
        manager.apply_indexes("myuniverse", &[ct]).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_co71_myuniverse_status'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "duplicate index must not be created");
    }

    #[test]
    fn test_drop_stale_indexes() {
        let conn = setup_db();
        let manager = IndexManager::new(&conn);

        // Create with two indexes
        let ct_old = make_ct("tarefa", &["status", "due_at"]);
        manager.apply_indexes("myuniverse", &[ct_old]).unwrap();

        // Now sync with only one
        let ct_new = make_ct("tarefa", &["status"]);
        manager.drop_stale_indexes("myuniverse", &[ct_new]).unwrap();

        let due_at_exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_co71_myuniverse_due_at'",
                [],
                |_| Ok(true),
            )
            .optional()
            .unwrap()
            .unwrap_or(false);
        assert!(!due_at_exists, "due_at index should be dropped");

        let status_exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_co71_myuniverse_status'",
                [],
                |_| Ok(true),
            )
            .optional()
            .unwrap()
            .unwrap_or(false);
        assert!(status_exists, "status index should remain");
    }

    #[test]
    fn test_sync_indexes_full_diff() {
        let conn = setup_db();
        let manager = IndexManager::new(&conn);

        // Start with status + due_at
        let ct_v1 = make_ct("tarefa", &["status", "due_at"]);
        manager.sync_indexes("myuniverse", &[ct_v1]).unwrap();

        // Manifest update: remove due_at, add assignee
        let ct_v2 = make_ct("tarefa", &["status", "assignee"]);
        manager.sync_indexes("myuniverse", &[ct_v2]).unwrap();

        let check = |name: &str| -> bool {
            conn.query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
                params![name],
                |_| Ok(true),
            )
            .optional()
            .unwrap()
            .unwrap_or(false)
        };

        assert!(check("idx_co71_myuniverse_status"), "status must remain");
        assert!(
            check("idx_co71_myuniverse_assignee"),
            "assignee must be created"
        );
        assert!(
            !check("idx_co71_myuniverse_due_at"),
            "due_at must be dropped"
        );
    }

    #[test]
    fn test_index_used_in_query_plan() {
        let conn = setup_db();
        let manager = IndexManager::new(&conn);

        // Seed an entry
        conn.execute(
            "INSERT INTO entries (path, universe_key, entry_type, frontmatter_json, payload, body_hash)
             VALUES ('tasks/1.md', 'u1', 'tarefa', '{\"status\":\"todo\"}', '{\"status\":\"todo\"}', 'abc')",
            [],
        )
        .unwrap();

        let ct = make_ct_with_fields("tarefa", &[("status", FieldType::Enum)], &["status"]);
        manager.apply_indexes("u1", &[ct]).unwrap();

        // EXPLAIN QUERY PLAN must mention the index
        let mut stmt = conn
            .prepare(
                "EXPLAIN QUERY PLAN \
                 SELECT path FROM entries \
                 WHERE universe_key = 'u1' AND json_extract(payload, '$.status') = 'todo'",
            )
            .unwrap();
        let plan: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        let plan_str = plan.join(" ");
        assert!(
            plan_str.contains("idx_co71"),
            "query plan should use the managed index, got: {plan_str}"
        );
    }

    #[test]
    fn test_unsafe_field_name_skipped() {
        let conn = setup_db();
        let manager = IndexManager::new(&conn);

        // Field name with SQL injection attempt
        let ct = make_ct("tarefa", &["status; DROP TABLE entries; --"]);
        // Should not panic or create a bad index
        manager.apply_indexes("myuniverse", &[ct]).unwrap();

        // No index should exist
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name LIKE 'idx_co71_myuniverse_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "unsafe field names must not create indexes");
    }

    #[test]
    fn test_existing_fields_returns_created_fields() {
        let conn = setup_db();
        let manager = IndexManager::new(&conn);
        let ct = make_ct("tarefa", &["status", "due_at"]);
        manager.apply_indexes("myuniverse", &[ct]).unwrap();

        let fields = manager.existing_fields("myuniverse").unwrap();
        let mut fields = fields;
        fields.sort();
        assert_eq!(fields, vec!["due_at", "status"]);
    }

    #[test]
    fn test_indexes_isolated_per_universe() {
        let conn = setup_db();
        let manager = IndexManager::new(&conn);

        let ct = make_ct("tarefa", &["status"]);
        manager
            .apply_indexes("universe_a", std::slice::from_ref(&ct))
            .unwrap();
        manager.apply_indexes("universe_b", &[ct]).unwrap();

        // Drop for universe_a must not affect universe_b
        manager.drop_stale_indexes("universe_a", &[]).unwrap();

        let b_exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_co71_universe_b_status'",
                [],
                |_| Ok(true),
            )
            .optional()
            .unwrap()
            .unwrap_or(false);
        assert!(
            b_exists,
            "universe_b index must not be affected by universe_a drop"
        );
    }
}
