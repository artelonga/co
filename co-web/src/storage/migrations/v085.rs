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
}
