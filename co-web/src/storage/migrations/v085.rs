use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-399: sala scope expansion — add `scope` to `workspace_states`.
    ///
    /// A sala is no longer bound to a single universe. `/sala` (every
    /// caller-visible universe) and `/sala?u=a,b` (an explicit subset) render the
    /// SAME canvas, so a persisted layout must key on its *scope*, not one
    /// universe key. This column carries the canonical scope identifier:
    ///   · `''`  (empty)  — a legacy single-universe row, keyed by `universe_key`
    ///                      as before (these rows are untouched — purely additive).
    ///   · `'*'`          — every universe visible to the caller (the `/sala` view).
    ///   · `'a,b,c'`      — a normalized (sorted, de-duplicated) subset.
    ///
    /// The identifier is deterministic and machine-local-state-free so Wave 7
    /// (v3.3 cross-device sync) can op-log these rows by scope-hash. Multi-scope
    /// rows store `universe_key = '@scope:<scope>'` (a sentinel that cannot collide
    /// with a real, validated universe key) so the existing
    /// `UNIQUE (universe_key, workspace_slug, user_id)` constraint enforces
    /// one-row-per-(scope, user) without a table rebuild.
    ///
    /// CO-338 took v84 (`universes.surface_dns`); this task claims **v85**.
    ///
    /// Additive + idempotent (`ensure_column` checks `pragma_table_info` first).
    /// `NOT NULL DEFAULT ''` backfills every existing row to the legacy
    /// single-universe semantics.
    pub(super) fn migrate_v085(&mut self, current_version: i64) {
        if current_version < 85 {
            // Self-contained guard (CO-330 invariant): `workspace_states` is only
            // CREATEd inside v70's `if current_version < 70` block, which a DB
            // already past v70 never re-runs — so `ensure_column` here would try to
            // ALTER a possibly-missing table ("no such table: workspace_states").
            // Recreate it defensively (no-op where it exists) before altering, the
            // same self-containment fix applied to v088/jobs after the prod outage.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS workspace_states (
                       id             TEXT PRIMARY KEY,
                       universe_key   TEXT NOT NULL,
                       workspace_slug TEXT NOT NULL,
                       user_id        TEXT NOT NULL,
                       layout_json    TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
                       is_public      INTEGER NOT NULL DEFAULT 0,
                       share_token    TEXT,
                       created_at     TEXT NOT NULL,
                       updated_at     TEXT NOT NULL,
                       UNIQUE (universe_key, workspace_slug, user_id)
                     );
                     CREATE INDEX IF NOT EXISTS idx_workspace_states_user
                       ON workspace_states (user_id);
                     CREATE INDEX IF NOT EXISTS idx_workspace_states_token
                       ON workspace_states (share_token) WHERE share_token IS NOT NULL;",
                )
                .expect("migration v85: ensure workspace_states base table");
            ensure_column(
                &self.conn,
                "workspace_states",
                "scope",
                "TEXT NOT NULL DEFAULT ''",
            )
            .expect("migration v85: workspace_states.scope");

            crate::record_migration!(
                self.conn,
                85,
                "CO-399: workspace_states.scope (multi-universe sala scope key)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn scope_column_defaults_to_empty_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        // A legacy single-universe row inserted without `scope` backfills to ''.
        storage
            .conn()
            .execute(
                "INSERT INTO workspace_states \
                 (id, universe_key, workspace_slug, user_id, layout_json, is_public, \
                  created_at, updated_at) \
                 VALUES ('id-1', 'mbya', 'default', 'user-1', '{}', 0, 'now', 'now')",
                [],
            )
            .unwrap();
        let scope: String = storage
            .conn()
            .query_row(
                "SELECT scope FROM workspace_states WHERE id = 'id-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            scope, "",
            "legacy rows must backfill to the empty (single) scope"
        );

        // A multi-scope row carries its scope identifier verbatim.
        storage
            .conn()
            .execute(
                "INSERT INTO workspace_states \
                 (id, universe_key, workspace_slug, user_id, layout_json, is_public, scope, \
                  created_at, updated_at) \
                 VALUES ('id-2', '@scope:a,b', 'default', 'user-1', '{}', 0, 'a,b', 'now', 'now')",
                [],
            )
            .unwrap();
        let scope: String = storage
            .conn()
            .query_row(
                "SELECT scope FROM workspace_states WHERE id = 'id-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(scope, "a,b");
    }

    #[test]
    fn recreates_workspace_states_when_missing() {
        // CO-330 regression: workspace_states' only CREATE is gated behind v70, so
        // a DB past v70 without the table must still migrate cleanly at v85.
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        storage
            .conn()
            .execute_batch("DROP TABLE workspace_states;")
            .expect("drop workspace_states to simulate prod");

        storage.migrate_v085(84); // current_version < 85 → must recreate, then alter

        storage
            .conn()
            .execute(
                "INSERT INTO workspace_states \
                 (id, universe_key, workspace_slug, user_id, layout_json, is_public, scope, \
                  created_at, updated_at) \
                 VALUES ('r1', 'u', 'default', 'usr', '{}', 0, '*', 'now', 'now')",
                [],
            )
            .expect("workspace_states must exist with scope after v85");
    }
}
