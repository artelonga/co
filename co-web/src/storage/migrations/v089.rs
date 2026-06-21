use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-89: git-backed universes. Add the opt-in per-universe git-source
    /// configuration to `universes`. A universe with `git_source` set triggers
    /// background ingestion of its `git log` as `commit` / `profile` / `event`
    /// content entries; universes without it behave exactly as today
    /// (markdown-only). The two `git_last_synced_*` columns let incremental
    /// sync resume from the last imported SHA without re-walking history.
    ///
    /// All additive + idempotent (`ensure_column` is a guarded `ALTER TABLE`),
    /// so this is safe on both a fresh DB and the production DB. The columns are
    /// read with `COALESCE`/separate queries in `get_universe`, never folded
    /// into the base `UNIVERSE_SCHEMA`, per the migration-version-claim protocol.
    ///
    /// Migration version **v089** (train tip v088; claimed as max + 1).
    pub(super) fn migrate_v089(&mut self, current_version: i64) {
        if current_version < 89 {
            // Remote (or local) git source for ingestion, e.g.
            // 'github.com/artelonga/co.git'. NULL ⇒ markdown-only universe.
            ensure_column(&self.conn, "universes", "git_source", "TEXT")
                .expect("migration v89: universes.git_source");
            // Branch to ingest (default 'main').
            ensure_column(
                &self.conn,
                "universes",
                "git_branch",
                "TEXT NOT NULL DEFAULT 'main'",
            )
            .expect("migration v89: universes.git_branch");
            // SHA of the last commit imported on the configured branch — the
            // incremental sync cursor.
            ensure_column(&self.conn, "universes", "git_last_synced_sha", "TEXT")
                .expect("migration v89: universes.git_last_synced_sha");
            // ISO-8601 timestamp of the last successful sync.
            ensure_column(&self.conn, "universes", "git_last_synced_at", "TEXT")
                .expect("migration v89: universes.git_last_synced_at");

            crate::record_migration!(
                self.conn,
                89,
                "CO-89: git-backed universes — git_source/git_branch/git_last_synced_* on universes"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn git_columns_exist_and_default_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        // Insert a universe without touching the git columns: git_source is NULL
        // (markdown-only) and git_branch defaults to 'main'.
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, created_at) \
                 VALUES ('u1', 'u@e.com', 'U', 'now')",
                [],
            )
            .ok();
        storage
            .conn()
            .execute(
                "INSERT INTO universes (key, name, description, owner_id, created_at) \
                 VALUES ('co-dev', 'Co Dev', '', 'u1', 'now')",
                [],
            )
            .unwrap();

        let (src, branch): (Option<String>, String) = storage
            .conn()
            .query_row(
                "SELECT git_source, git_branch FROM universes WHERE key = 'co-dev'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(src, None, "git_source defaults to NULL (markdown-only)");
        assert_eq!(branch, "main", "git_branch defaults to 'main'");
    }

    #[test]
    fn git_sync_cursor_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, created_at) \
                 VALUES ('u1', 'u@e.com', 'U', 'now')",
                [],
            )
            .ok();
        storage
            .conn()
            .execute(
                "INSERT INTO universes (key, name, description, owner_id, created_at, \
                 git_source, git_last_synced_sha, git_last_synced_at) \
                 VALUES ('co-dev', 'Co Dev', '', 'u1', 'now', \
                 'github.com/artelonga/co.git', 'abc123', '2026-06-21T00:00:00Z')",
                [],
            )
            .unwrap();

        let (sha, at): (Option<String>, Option<String>) = storage
            .conn()
            .query_row(
                "SELECT git_last_synced_sha, git_last_synced_at FROM universes WHERE key = 'co-dev'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(sha.as_deref(), Some("abc123"));
        assert_eq!(at.as_deref(), Some("2026-06-21T00:00:00Z"));
    }
}
