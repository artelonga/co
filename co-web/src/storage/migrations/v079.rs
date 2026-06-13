use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-96: universe soft-delete + archive support.
    ///
    /// Adds two nullable timestamp columns to `universes`:
    /// - `deleted_at`  — set when a universe is sent to the trash (recoverable
    ///   for a 30-day window; cleared on restore). `NULL` = not deleted.
    /// - `archived_at` — set when a universe is archived (soft-hidden from the
    ///   sidebar but recoverable). `NULL` = not archived.
    ///
    /// Both are filtered out of every universe-listing query so a deleted or
    /// archived universe disappears from the sidebar, public listings, search
    /// and discovery — but the row and its data survive for recovery via the
    /// trash view.
    ///
    /// Additive + idempotent (`ensure_column` checks `pragma_table_info` first).
    /// Added as its own migration step — NEVER in the base `UNIVERSE_SCHEMA`
    /// batch, which runs on existing DBs before migrations and would panic the
    /// server at boot if it referenced a not-yet-added column (CO-354 trap).
    pub(super) fn migrate_v079(&mut self, current_version: i64) {
        if current_version < 79 {
            ensure_column(&self.conn, "universes", "deleted_at", "TEXT")
                .expect("migration v79: universes.deleted_at");
            ensure_column(&self.conn, "universes", "archived_at", "TEXT")
                .expect("migration v79: universes.archived_at");

            crate::record_migration!(
                self.conn,
                79,
                "CO-96: universes.deleted_at + archived_at (soft-delete + archive; NULL = active)"
            );
        }
    }
}
