use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-401: mark synthetic (staging fixture) leads so acquisition-funnel
    /// analytics can exclude them.
    ///
    /// Adds a `synthetic` flag to `leads` (0 = real, 1 = fixture). The staging
    /// seeder inserts pre-baked funnel/lead fixtures with `synthetic = 1`; the
    /// acquisition-funnel rollup (`query_funnel_steps`) filters them out so the
    /// staging numbers stay honest. Real signups/contact-form leads never set
    /// the column, so the `DEFAULT 0` keeps every existing row counted.
    ///
    /// Additive + idempotent (`ensure_column` checks `pragma_table_info`
    /// first). Added as its own migration step — never folded into the base
    /// `leads` batch, which runs on existing DBs before migrations and would
    /// panic the server at boot if it referenced a not-yet-added column
    /// (CO-354 trap).
    pub(super) fn migrate_v081(&mut self, current_version: i64) {
        if current_version < 81 {
            ensure_column(
                &self.conn,
                "leads",
                "synthetic",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v81: leads.synthetic");
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_leads_synthetic \
                     ON leads(synthetic);",
                )
                .expect("migration v81: idx_leads_synthetic");

            crate::record_migration!(
                self.conn,
                81,
                "CO-401: leads.synthetic flag (1 = staging fixture, excluded from funnel analytics) + index"
            );
        }
    }
}
