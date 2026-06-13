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
mod v077;
mod v078;
mod v079;

/// CO-446: minimum free bytes required on the data volume before the boot path
/// runs migrations. Default 200 MiB. Override with `CO_MIGRATION_MIN_FREE_BYTES`.
const DEFAULT_MIGRATION_MIN_FREE_BYTES: u64 = 200 * 1024 * 1024;

/// CO-446: a controlled migration failure carrying a human-readable reason.
///
/// Returned from [`Storage::run_migrations`] instead of panicking, so a full
/// `/data` (the recurring 2026-06-11/2026-06-13 outage) surfaces as a clear,
/// operator-facing boot error rather than a cryptic SQLite panic backtrace and
/// an `exit 101` crash-loop.
#[derive(Debug)]
pub struct MigrationError {
    /// Which step failed (`preflight` / `schema_version` / `migrate`).
    pub stage: &'static str,
    /// Underlying error rendered for humans (SQLite/IO message or panic text).
    pub reason: String,
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "migrations failed ({}): {}", self.stage, self.reason)
    }
}

impl std::error::Error for MigrationError {}

/// CO-446: pure disk-headroom check (no real filesystem, so it is unit-testable).
///
/// `free_bytes = None` (statvfs unavailable / non-Linux dev + CI) disables the
/// guard — missing telemetry must never block a boot.
fn check_headroom(free_bytes: Option<u64>, min_bytes: u64) -> Result<(), MigrationError> {
    if let Some(free) = free_bytes
        && free < min_bytes
    {
        return Err(MigrationError {
            stage: "preflight",
            reason: format!(
                "insufficient disk for migrations: {free} free < {min_bytes} min \
                 (extend the volume or raise CO_MIGRATION_MIN_FREE_BYTES)"
            ),
        });
    }
    Ok(())
}

/// CO-446: run the legacy migration chain (which still uses `expect()` /
/// `record_migration!` internally) under a catch boundary, converting any panic
/// — e.g. a `SQLITE_FULL` "database or disk is full" mid-`ALTER TABLE` after the
/// pre-flight passed — into a controlled [`MigrationError`] instead of letting
/// it unwind out of the process into a crash-loop.
fn catch_migration_panic<F: FnOnce()>(f: F) -> Result<(), MigrationError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => Ok(()),
        Err(payload) => Err(MigrationError {
            stage: "migrate",
            reason: panic_payload_string(payload),
        }),
    }
}

fn panic_payload_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown migration panic".to_string()
    }
}

impl Storage {
    /// CO-446: run all schema migrations, degrading to a [`MigrationError`]
    /// instead of panicking the boot process on a full / failing disk.
    pub(super) fn run_migrations(&mut self) -> Result<(), MigrationError> {
        // Create schema_version table (the first write — a full disk fails here).
        self.conn
            .execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
            .map_err(|e| MigrationError {
                stage: "schema_version",
                reason: e.to_string(),
            })?;

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // CO-446 pre-flight: refuse to start migrations without disk headroom so
        // the failure is an explicit ERROR, not a half-applied migration that
        // panics deep in SQLite.
        let min_free = {
            use crate::infra::secrets::SecretsProviderExt;
            crate::infra::secrets::global().get_parsed(
                "CO_MIGRATION_MIN_FREE_BYTES",
                DEFAULT_MIGRATION_MIN_FREE_BYTES,
            )
        };
        let free =
            crate::platform::disk_monitor::disk_stats(&self.data_dir).map(|(avail, _)| avail);
        if let Err(e) = check_headroom(free, min_free) {
            tracing::error!(
                stage = e.stage,
                reason = %e.reason,
                "CO-446: aborting boot before migrations — {e}"
            );
            return Err(e);
        }

        // Each range applies only the blocks whose guard `current_version < N`
        // holds, in ascending version order. `current_version` is captured once
        // above so the aggregate is equivalent to the original monolithic run.
        //
        // CO-446: the chain runs under a catch boundary so a write failure that
        // slips past the pre-flight (disk fills *during* migration) becomes a
        // controlled MigrationError + clear ERROR log — never `exit 101`.
        catch_migration_panic(|| {
            self.migrate_v001_018(current_version);
            self.migrate_v019_031(current_version);
            self.migrate_v032_041(current_version);
            self.migrate_v042_045(current_version);
            self.migrate_v046_056(current_version);
            self.migrate_v057_072(current_version);
            self.migrate_v073_076(current_version);
            self.migrate_v077(current_version);
            self.migrate_v078(current_version);
            self.migrate_v079(current_version);
        })
        .inspect_err(|e| {
            tracing::error!(
                stage = e.stage,
                reason = %e.reason,
                "CO-446: database migration failed — aborting boot (no crash-loop)"
            );
        })
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
//
// CO-446: a write here can fail with `SQLITE_FULL` ("database or disk is full")
// on a full `/data`. We emit a clear `ERROR` log first, then abort — the abort
// is caught by [`catch_migration_panic`] in `run_migrations`, which turns it
// into a controlled boot failure instead of an `exit 101` crash-loop.
#[macro_export]
macro_rules! record_migration {
    ($conn:expr, $version:expr, $desc:expr) => {
        $conn
            .execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
                rusqlite::params![$version],
            )
            .unwrap_or_else(|e| {
                tracing::error!(
                    version = $version,
                    error = %e,
                    "CO-446: record_migration schema_version insert failed (disk full?) — aborting boot"
                );
                panic!("record_migration v{}: schema_version insert: {e}", $version)
            });
        $conn
            .execute(
                "INSERT OR IGNORE INTO schema_versoes (versao, descricao, versao_app) \
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![$version, $desc, env!("CARGO_PKG_VERSION")],
            )
            .unwrap_or_else(|e| {
                tracing::error!(
                    version = $version,
                    error = %e,
                    "CO-446: record_migration schema_versoes insert failed (disk full?) — aborting boot"
                );
                panic!("record_migration v{}: schema_versoes insert: {e}", $version)
            });
    };
}

#[cfg(test)]
mod co446_disk_full_tests {
    use super::*;
    use crate::storage::Storage;
    use crate::universe_pool::UniversePool;
    use std::sync::Arc;

    /// CO-446: the pure headroom guard rejects a low-disk boot with a readable
    /// reason, allows a healthy one, and is disabled when telemetry is absent.
    #[test]
    fn check_headroom_blocks_only_when_below_min() {
        let min = 200 * 1024 * 1024;

        // Below the floor → controlled error mentioning both numbers.
        let err = check_headroom(Some(10 * 1024 * 1024), min).unwrap_err();
        assert_eq!(err.stage, "preflight");
        let msg = err.to_string();
        assert!(
            msg.contains("insufficient disk for migrations"),
            "must be operator-readable: {msg}"
        );

        // Above the floor → OK.
        assert!(check_headroom(Some(min + 1), min).is_ok());

        // No telemetry (non-Linux / statvfs failed) → guard disabled, never block.
        assert!(check_headroom(None, min).is_ok());
    }

    /// CO-446: a panic from the legacy migration chain (e.g. `record_migration!`
    /// hitting `SQLITE_FULL`) is caught and converted into a readable
    /// `MigrationError` rather than unwinding out into a crash-loop.
    #[test]
    fn panic_in_chain_becomes_readable_error() {
        let err =
            catch_migration_panic(|| panic!("record_migration v78: database or disk is full"))
                .unwrap_err();
        assert_eq!(err.stage, "migrate");
        assert!(
            err.to_string().to_lowercase().contains("disk is full"),
            "panic message must survive into the error: {err}"
        );

        // The happy path stays Ok.
        assert!(catch_migration_panic(|| {}).is_ok());
    }

    /// CO-446 end-to-end: a full disk during boot migrations degrades to an
    /// `Err` (readable) instead of panicking the process / `exit 101`.
    ///
    /// `PRAGMA max_page_count` caps the database at ~2 pages, so the first
    /// migration write returns SQLITE_FULL ("database or disk is full") — a
    /// deterministic stand-in for the volume that filled in prod.
    #[test]
    fn run_migrations_degrades_on_full_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("meta.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA page_size=512; PRAGMA max_page_count=2;")
            .unwrap();

        let mut storage = Storage {
            conn,
            universe_pool: Arc::new(UniversePool::new(tmp.path(), 16)),
            data_dir: tmp.path().to_path_buf(),
        };

        let result = storage.run_migrations();
        assert!(
            result.is_err(),
            "a full disk must degrade to Err, not panic / exit 101"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.to_lowercase().contains("full") || msg.contains("migrations failed"),
            "boot error must be readable: {msg}"
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    /// Latest migration version applied by the aggregated runner. Bump this in
    /// lockstep with the highest `if current_version < N` block (version-claim
    /// protocol) so the split stays anchored to the real schema.
    const LATEST_VERSION: i64 = 79;

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
