//! Typed meta-DB accessors for the `feedback` table (CO-433).
//!
//! The feedback subsystem predates the storage-trait conventions and still has
//! a wide raw-SQL surface in its routes; CO-433 moves the request-path
//! `conn().execute`/`conn().prepare` call(s) flagged by the no-raw-SQL gate
//! into typed methods here. Remaining `let conn = storage.conn()` read helpers
//! in `feedback_routes.rs` are a follow-up (not request-path writes the gate
//! targets).

use rusqlite::Result;

use super::Storage;

impl Storage {
    /// Backfill a feedback item's linked-issue summary (only if still unset).
    /// Returns rows updated.
    pub fn set_feedback_linked_summary(&self, id: &str, summary: &str) -> Result<usize> {
        self.conn().execute(
            "UPDATE feedback SET linked_summary = ?1 WHERE id = ?2 AND linked_summary IS NULL",
            rusqlite::params![summary, id],
        )
    }
}
