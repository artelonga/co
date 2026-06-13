use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-204: chat message origin telemetry.
    ///
    /// Adds a nullable `origin_universe_key` column to `chat_messages` — the
    /// universe context the sender was *browsing* when they hit Send. This is
    /// distinct from `chat_rooms.universe_key` (where the room lives): for DM
    /// rooms the room has no universe, so the origin is the only breadcrumb of
    /// "where did this message come from".
    ///
    /// Existing rows get `NULL` (no backfill — we can't reconstruct the
    /// sender's at-the-time context). An index on the column backs the
    /// admin origin-breakdown aggregation.
    ///
    /// Additive + idempotent (`ensure_column` checks `pragma_table_info`
    /// first). Added as its own migration step — NEVER in the base chat-table
    /// batch, which runs on existing DBs before migrations and would panic the
    /// server at boot if it referenced a not-yet-added column (CO-354 trap).
    pub(super) fn migrate_v080(&mut self, current_version: i64) {
        if current_version < 80 {
            ensure_column(&self.conn, "chat_messages", "origin_universe_key", "TEXT")
                .expect("migration v80: chat_messages.origin_universe_key");
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_chat_messages_origin \
                     ON chat_messages(origin_universe_key);",
                )
                .expect("migration v80: idx_chat_messages_origin");

            crate::record_migration!(
                self.conn,
                80,
                "CO-204: chat_messages.origin_universe_key (sender's universe context; NULL = unknown/pre-CO-204) + index"
            );
        }
    }
}
