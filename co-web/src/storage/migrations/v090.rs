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
    /// Migration version pinned to **v090**: v089 is claimed by CO-89
    /// (git-backed universes, sibling PR #299, now on main). The numeric gap is
    /// benign — each guard checks `current_version < N` independently and
    /// `current_version` is read once up front, so a fresh DB and an existing DB
    /// converge regardless of which sibling migration ran first.
    pub(super) fn migrate_v090(&mut self, current_version: i64) {
        if current_version < 90 {
            ensure_column(
                &self.conn,
                "universes",
                "accepts_proposals",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v90: universes.accepts_proposals");

            crate::record_migration!(
                self.conn,
                90,
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
        // Re-running the migration on a DB already at v90 is a no-op (the column
        // already exists; ensure_column tolerates that).
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        storage.migrate_v090(89);
        storage.migrate_v090(90);

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
