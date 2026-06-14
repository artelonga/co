use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-401: mark synthetic (staging-fixture) leads so the acquisition-funnel
    /// rollup can exclude them.
    ///
    /// Adds a `is_synthetic` flag to `leads` (default `0` → real lead). The
    /// staging seeder inserts its funnel/lead fixtures with `is_synthetic = 1`,
    /// and `funnel_routes::query_funnel_steps` filters them out, so seeding the
    /// staging suite's test data never pollutes the real analytics metrics.
    ///
    /// CO-448 took v81 (`api_tokens.scopes`); this task claims **v82**.
    ///
    /// Additive + idempotent (`ensure_column` checks `pragma_table_info` first).
    /// `NOT NULL DEFAULT 0` backfills every existing row to "real" — the only
    /// rows that ever carry `1` are the staging fixtures.
    pub(super) fn migrate_v082(&mut self, current_version: i64) {
        if current_version < 82 {
            ensure_column(
                &self.conn,
                "leads",
                "is_synthetic",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v82: leads.is_synthetic");

            crate::record_migration!(
                self.conn,
                82,
                "CO-401: leads.is_synthetic (exclude staging fixture leads from funnel/analytics rollups)"
            );
        }
    }
}
