use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-78: job queue + worker pool — generalize the CO-72 `jobs` table from a
    /// single-kind doc-gen queue into a multi-kind, rate-limited, metered work
    /// queue (`doc_gen`, `index_rebuild`, `changelog_regen`, `sync_pull`,
    /// `webhook_dispatch`, `cleanup`).
    ///
    /// The base table + the `(status, run_at)`, `(universe_key)` and unique
    /// `dedupe_key` indexes already exist from v25 (CO-72). This migration adds
    /// only what the worker pool needs on top, all additive + idempotent:
    ///
    /// * `idx_jobs_status_run_at_universe` — the composite claim index the spec
    ///   asks for: `(status, run_at, universe_key)`. The claim query orders by
    ///   `run_at, created_at` filtered on `status = 'pending'`; carrying
    ///   `universe_key` in the same index keeps per-universe queue scans
    ///   covering as the queue grows.
    /// * `idx_jobs_universe_kind_created` — backs the CO-80 per-universe,
    ///   per-kind hourly rate-limit count (`WHERE universe_key = ? AND kind = ?
    ///   AND created_at >= ?`).
    /// * `jobs.timeout_secs` — per-job wall-time budget (NULL ⇒ fall back to the
    ///   per-kind default, default 5 min). Stamped at enqueue from the kind so a
    ///   heavy `index_rebuild` can carry a longer budget than a `webhook_dispatch`.
    ///
    /// Migration version pinned to **v088** (train tip v085; CO-458 → v086,
    /// CO-449 → v087). All job-queue schema lives in this single file.
    pub(super) fn migrate_v088(&mut self, current_version: i64) {
        if current_version < 88 {
            // Per-job timeout budget (NULL = use the per-kind default).
            ensure_column(&self.conn, "jobs", "timeout_secs", "INTEGER")
                .expect("migration v88: jobs.timeout_secs");

            // Composite claim index + rate-limit index. `CREATE INDEX IF NOT
            // EXISTS` is idempotent, so this is safe to re-run.
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_jobs_status_run_at_universe
                         ON jobs(status, run_at, universe_key);
                     CREATE INDEX IF NOT EXISTS idx_jobs_universe_kind_created
                         ON jobs(universe_key, kind, created_at);",
                )
                .expect("migration v88: job-queue indexes");

            crate::record_migration!(
                self.conn,
                88,
                "CO-78: job-queue worker pool — timeout_secs + claim/rate-limit indexes"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn timeout_secs_column_exists_and_defaults_null() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        // A job row inserted without timeout_secs leaves it NULL (⇒ per-kind default).
        storage
            .conn()
            .execute(
                "INSERT INTO jobs (id, universe_key, kind, payload, status, attempts, \
                                   created_at, run_at) \
                 VALUES ('j1', 'u1', 'doc_gen', '{}', 'pending', 0, 'now', 'now')",
                [],
            )
            .unwrap();
        let timeout: Option<i64> = storage
            .conn()
            .query_row("SELECT timeout_secs FROM jobs WHERE id = 'j1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(timeout, None, "timeout_secs defaults to NULL");
    }

    #[test]
    fn claim_and_rate_limit_indexes_exist() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'index' AND name IN \
                   ('idx_jobs_status_run_at_universe', 'idx_jobs_universe_kind_created')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "both CO-78 job-queue indexes must exist");
    }
}
