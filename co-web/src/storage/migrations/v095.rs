use super::super::Storage;

impl Storage {
    /// CO-496 (tier-3 scalability): the auth/ownership hot-path indexes the
    /// scalability review flagged as missing on `meta.db`. Every one of these is
    /// a full-table scan today, so they bound concurrency under load:
    ///
    /// - `idx_users_whatsapp` — `get_user_id_by_whatsapp` (WhatsApp link/verify
    ///   uniqueness check, CO-490) scans `users` for every link attempt. Partial
    ///   (`WHERE whatsapp IS NOT NULL`) because the vast majority of users have no
    ///   linked number, so the index stays tiny while still covering the equality
    ///   lookup (the probe value is always non-NULL).
    /// - `idx_universes_owner` — "universes owned by user X" (dashboard / gestão
    ///   listing) scans `universes(owner_id)`.
    /// - `idx_universe_members_user` — "which universes is user X a member of"
    ///   scans `universe_members(user_id)`. The PK is `(universe_key, user_id)`,
    ///   so it cannot serve a `user_id`-leading probe; this index does.
    ///
    /// All three are `CREATE INDEX IF NOT EXISTS` over columns that already exist
    /// on every deployed DB (whatsapp added v093; owner_id + user_id predate the
    /// split), so the migration is purely additive and idempotent — safe to run on
    /// a fresh DB and on the largest production `meta.db` alike.
    ///
    /// ── Cross-branch version coordination ───────────────────────────────────
    /// The train tip on this branch is `LATEST_VERSION = 94` (CO-491's v094), so
    /// this claims `max + 1 = 95`. Per the version-claim protocol a SHARED number
    /// is the safe collision (loud `duplicate mod` at merge → renumber-the-second)
    /// vs. a numeric GAP (silently skipped on a DB already past the gap).
    pub(super) fn migrate_v095(&mut self, current_version: i64) {
        if current_version < 95 {
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_users_whatsapp \
                         ON users(whatsapp) WHERE whatsapp IS NOT NULL; \
                     CREATE INDEX IF NOT EXISTS idx_universes_owner \
                         ON universes(owner_id); \
                     CREATE INDEX IF NOT EXISTS idx_universe_members_user \
                         ON universe_members(user_id);",
                )
                .expect("migration v95: CO-496 auth/ownership indexes");

            crate::record_migration!(
                self.conn,
                95,
                "CO-496: auth/ownership indexes (idx_users_whatsapp, idx_universes_owner, idx_universe_members_user)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    fn index_exists(storage: &Storage, name: &str) -> bool {
        storage
            .conn()
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name = ?1",
                rusqlite::params![name],
                |_| Ok(true),
            )
            .unwrap_or(false)
    }

    /// A fresh DB reaches v095 and has all three CO-496 indexes.
    #[test]
    fn migration_v095_creates_all_three_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        assert!(index_exists(&storage, "idx_users_whatsapp"));
        assert!(index_exists(&storage, "idx_universes_owner"));
        assert!(index_exists(&storage, "idx_universe_members_user"));
    }

    /// Re-running across the version boundary must not error or duplicate.
    #[test]
    fn migration_v095_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        storage.migrate_v095(94);
        storage.migrate_v095(95);
        assert!(index_exists(&storage, "idx_users_whatsapp"));
    }

    /// The whatsapp-uniqueness lookup planner picks the partial index rather than
    /// scanning the table — the concrete scalability win CO-496 targets.
    #[test]
    fn whatsapp_lookup_uses_index_not_scan() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        let plan: String = storage
            .conn()
            .query_row(
                "EXPLAIN QUERY PLAN SELECT id FROM users WHERE whatsapp = '5541999999999'",
                [],
                |r| r.get::<_, String>(3),
            )
            .unwrap();
        assert!(
            plan.contains("idx_users_whatsapp"),
            "expected the partial index to be used, got plan: {plan}"
        );
    }
}
