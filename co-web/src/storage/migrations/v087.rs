use super::super::Storage;

impl Storage {
    /// CO-449: telemetry cold-tier archival manifest + incremental-vacuum opt-in.
    ///
    /// The `meta.db` OLTP database is dominated by `telemetry_events` (append-only,
    /// high volume — 11 GB in prod 2026-06-14). The owner decision is **keep all
    /// data** but move the cold window (> `CO_TELEMETRY_HOT_DAYS`, default 90d) out
    /// of SQLite into per-month **Parquet** files (zstd, DuckDB-readable). This
    /// migration adds the bookkeeping side:
    ///
    /// · `telemetry_archives` — one row per archived month for traceability and
    ///   dedupe (the archival job skips a month already listed here). `s3_key`
    ///   holds the archive's path; local-first today (`telemetry-archive/year=…`),
    ///   an S3 key once CO-81 lands — the column name is forward-looking.
    ///
    /// · `PRAGMA auto_vacuum = INCREMENTAL` — so the archival job can reclaim the
    ///   freed pages with `PRAGMA incremental_vacuum` **without** the 2× peak a full
    ///   `VACUUM` needs (critical: prod has ~5.6 GB free for an 11 GB DB). On an
    ///   existing database the mode change only takes effect after **one** full
    ///   `VACUUM` (run during a maintenance window with an extended volume — see
    ///   `docs/OPERATIONS.md`); on a fresh database it is effective immediately.
    ///   Setting it is harmless either way, so it is safe to run here.
    ///
    /// CO-458 takes v86; this task claims **v87**. Additive + idempotent
    /// (`CREATE TABLE IF NOT EXISTS`, and the pragma is a no-op when already set).
    pub(super) fn migrate_v087(&mut self, current_version: i64) {
        if current_version < 87 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS telemetry_archives (
                        id          INTEGER PRIMARY KEY AUTOINCREMENT,
                        year        INTEGER NOT NULL,
                        month       INTEGER NOT NULL,
                        s3_key      TEXT NOT NULL,
                        rows        INTEGER NOT NULL,
                        sha256      TEXT NOT NULL,
                        bytes       INTEGER NOT NULL,
                        archived_at TEXT NOT NULL,
                        UNIQUE(year, month)
                    );",
                )
                .expect("migration v87: telemetry_archives");

            // Opt into incremental auto-vacuum so the archival job reclaims space
            // page-by-page (no 2× rebuild peak). Best-effort: on a populated DB
            // this is latent until the operator runs a one-time full VACUUM.
            let _ = self.conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;");

            crate::record_migration!(
                self.conn,
                87,
                "CO-449: telemetry_archives manifest + auto_vacuum=INCREMENTAL"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn telemetry_archives_table_exists_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        storage
            .conn()
            .execute(
                "INSERT INTO telemetry_archives \
                 (year, month, s3_key, rows, sha256, bytes, archived_at) \
                 VALUES (2026, 1, 'year=2026/month=01/part-abc.parquet', 42, 'deadbeef', 1024, 'now')",
                [],
            )
            .unwrap();

        let (rows, key): (i64, String) = storage
            .conn()
            .query_row(
                "SELECT rows, s3_key FROM telemetry_archives WHERE year = 2026 AND month = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows, 42);
        assert_eq!(key, "year=2026/month=01/part-abc.parquet");
    }

    #[test]
    fn archives_month_is_unique() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        let insert = || {
            storage.conn().execute(
                "INSERT INTO telemetry_archives \
                 (year, month, s3_key, rows, sha256, bytes, archived_at) \
                 VALUES (2026, 1, 'k', 1, 's', 1, 'now')",
                [],
            )
        };
        assert!(insert().is_ok());
        assert!(
            insert().is_err(),
            "duplicate (year, month) must be rejected"
        );
    }
}
