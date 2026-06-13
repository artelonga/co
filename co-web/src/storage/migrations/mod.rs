//! Schema migrations for the main CO database.
//!
//! Historically this lived in a single 2.7k-LoC `migrations.rs`. CO-436 sliced
//! it into per-version-range modules (`v001_018`, `v019_031`, …) so the file is
//! navigable, while [`Storage::run_migrations`] keeps aggregating them in order.
//!
//! Behavior is unchanged: `current_version` is read **once** up front and the
//! same value is threaded into every range so a fresh DB and an existing DB
//! converge on the identical final `schema_version`. To add a migration, append
//! a new `if current_version < N` block to the latest range module (splitting
//! out a fresh module once it approaches ~500 LoC) and claim `N = max + 1` per
//! the version-claim protocol.

use super::Storage;

mod v001_018;
mod v019_031;
mod v032_041;
mod v042_045;
mod v046_056;
mod v057_072;
mod v073_076;

impl Storage {
    pub(super) fn run_migrations(&mut self) {
        // Create schema_version table
        self.conn
            .execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
            .expect("Failed to create schema_version table");

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Each range applies only the blocks whose guard `current_version < N`
        // holds, in ascending version order. `current_version` is captured once
        // above so the aggregate is equivalent to the original monolithic run.
        self.migrate_v001_018(current_version);
        self.migrate_v019_031(current_version);
        self.migrate_v032_041(current_version);
        self.migrate_v042_045(current_version);
        self.migrate_v046_056(current_version);
        self.migrate_v057_072(current_version);
        self.migrate_v073_076(current_version);
    }
}

// ---------------------------------------------------------------------------
// Helper macro for future migrations (v60+)
// ---------------------------------------------------------------------------

/// Record a migration step in both `schema_version` and `schema_versoes`.
///
/// Usage (inside the `Migrations` impl block):
/// ```ignore
/// record_migration!(self, 60, "short description of what this migration does");
/// ```
///
/// Going forward, every new migration MUST use this macro instead of a bare
/// `INSERT INTO schema_version` so that `schema_versoes` stays in sync.
#[macro_export]
macro_rules! record_migration {
    ($conn:expr, $version:expr, $desc:expr) => {
        $conn
            .execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
                rusqlite::params![$version],
            )
            .unwrap_or_else(|e| {
                panic!("record_migration v{}: schema_version insert: {e}", $version)
            });
        $conn
            .execute(
                "INSERT OR IGNORE INTO schema_versoes (versao, descricao, versao_app) \
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![$version, $desc, env!("CARGO_PKG_VERSION")],
            )
            .unwrap_or_else(|e| {
                panic!("record_migration v{}: schema_versoes insert: {e}", $version)
            });
    };
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    /// Latest migration version applied by the aggregated runner. Bump this in
    /// lockstep with the highest `if current_version < N` block (version-claim
    /// protocol) so the split stays anchored to the real schema.
    const LATEST_VERSION: i64 = 76;

    fn max_version(storage: &Storage) -> i64 {
        storage
            .conn()
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// CO-436: a fresh database runs the whole chain — now split across
    /// `v001_018`…`v073_076` and aggregated by [`Storage::run_migrations`] —
    /// and lands on the latest schema version.
    #[test]
    fn fresh_db_reaches_latest_version() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());
        assert_eq!(max_version(&storage), LATEST_VERSION);
    }

    /// CO-436: re-opening an already-migrated database re-runs the runner but
    /// every version guard sees `current_version >= N`, so the schema version is
    /// unchanged — proving the slice boundaries don't re-apply or skip anything
    /// (a fresh DB and an existing DB converge on the same version).
    #[test]
    fn rerunning_migrations_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let first = max_version(&Storage::new(tmp.path()));
        let second = max_version(&Storage::new(tmp.path()));
        assert_eq!(first, LATEST_VERSION);
        assert_eq!(first, second);
    }
}
