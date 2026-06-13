use super::super::Storage;

impl Storage {
    /// CO-54: idempotency + conflict resolution — the `entry_versions` audit
    /// trail.
    ///
    /// Every `PUT` snapshots the *previous* content of an entry into this table
    /// before overwriting it (worst-case data-loss prevention). The table is the
    /// backing store for `GET …/entries/versions` (manual recovery) and for the
    /// "crash during write doesn't lose data" guarantee.
    ///
    /// Additive, backwards-compatible. Created as its own migration step rather
    /// than in the base `UNIVERSE_SCHEMA` batch — that batch runs on existing
    /// DBs before migrations, so referencing a not-yet-added table there would
    /// panic the server at boot (CO-354 trap).
    pub(super) fn migrate_v078(&mut self, current_version: i64) {
        if current_version < 78 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS entry_versions (
                        id               INTEGER PRIMARY KEY AUTOINCREMENT,
                        universe_key     TEXT NOT NULL,
                        path             TEXT NOT NULL,
                        version          INTEGER NOT NULL,
                        body             TEXT NOT NULL,
                        frontmatter_json TEXT NOT NULL,
                        hash             TEXT NOT NULL,
                        actor            TEXT,
                        timestamp        TEXT NOT NULL
                    );
                    -- History lookups + next-version computation both scan by
                    -- (universe_key, path) newest-first.
                    CREATE INDEX IF NOT EXISTS idx_entry_versions_lookup
                        ON entry_versions (universe_key, path, version DESC);",
                )
                .expect("migration v78: entry_versions table + index");

            crate::record_migration!(
                self.conn,
                78,
                "CO-54: entry_versions audit trail (pre-overwrite snapshots + version history)"
            );
        }
    }
}
