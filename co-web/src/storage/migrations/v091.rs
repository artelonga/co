use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-93: universe-type architecture — add the `accepts_proposals` opt-in
    /// flag that turns a private universe into a **private-dynamic** one
    /// (subscribers may submit pending edits via the CO-60 proposal flow).
    ///
    /// The three first-class universe types (`public-static` / `private-static` /
    /// `private-dynamic`, plus the system-owned `template`) are *derived*
    /// deterministically from the existing `visibility` column plus this flag by
    /// [`crate::models::Universe::universe_type`]. We deliberately do NOT add a
    /// fourth raw `universe_type` column: the legacy
    /// `is_template`/`is_public`/`requires_login` trio already drifted from
    /// `visibility` and had to be reconciled in v20, so the canonical type stays
    /// computed instead of denormalized.
    ///
    /// Only `accepts_proposals` is genuinely new state. The column is additive +
    /// idempotent (`ensure_column`); existing rows default to `0`
    /// (private-static / public-static), so behavior is unchanged until an owner
    /// opts a universe into the dynamic proposal flow.
    ///
    /// Migration version is **v091**: v090 was claimed by CO-81 (object-storage
    /// GC cursor, merged to main first), so this sibling migration renumbered
    /// from v090 to v091 on rebase to avoid the collision. Each guard checks
    /// `current_version < N` independently, so fresh and existing DBs converge
    /// regardless of sibling-merge order.
    pub(super) fn migrate_v091(&mut self, current_version: i64) {
        if current_version < 91 {
            ensure_column(
                &self.conn,
                "universes",
                "accepts_proposals",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v91: universes.accepts_proposals");

            crate::record_migration!(
                self.conn,
                91,
                "CO-93: universes.accepts_proposals (private-dynamic opt-in for universe-type model)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn accepts_proposals_column_exists_and_defaults_zero() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        // Seed a user + universe without the flag → defaults to 0 (false).
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, created_at) VALUES ('u1', 'a@b.c', 'now')",
                [],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT INTO universes (key, name, description, owner_id, created_at) \
                 VALUES ('k1', 'K', '', 'u1', 'now')",
                [],
            )
            .unwrap();

        let accepts: i64 = storage
            .conn()
            .query_row(
                "SELECT accepts_proposals FROM universes WHERE key = 'k1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            accepts, 0,
            "accepts_proposals defaults to 0 (private-static)"
        );
    }

    #[test]
    fn accepts_proposals_migration_is_idempotent() {
        // Re-running the migration on a DB already at v91 is a no-op (the column
        // already exists; ensure_column tolerates that).
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        storage.migrate_v091(90);
        storage.migrate_v091(91);

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('universes') WHERE name = 'accepts_proposals'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "exactly one accepts_proposals column");
    }
}
