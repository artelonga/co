use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-413: bidirectional bridge universes.
    ///
    /// Adds `source_mode` to `universes` — the per-universe write policy for
    /// event-bus-backed universes:
    ///
    /// - `'read-only'` (default) — CO rejects writes (current CO-383 behavior).
    /// - `'bidirectional'` — CO accepts writes and re-emits `entry.{created,
    ///   updated,deleted}` to the federated bus as CO-origin edits (`source =
    ///   co-edit`), which the YG-97 echo-filter recognizes to avoid a sync loop.
    ///
    /// `yggdrasil` keeps `source_kind = 'event-bus'` (set in v68) and now gets
    /// `source_mode = 'read-only'` by the column default, so its behavior is
    /// unchanged until an operator explicitly flips it to bidirectional. No
    /// destructive data migration.
    ///
    /// CO-401 took v82 (`leads.is_synthetic`); this task claims **v83**.
    ///
    /// Additive + idempotent (`ensure_column` checks `pragma_table_info` first).
    /// `NOT NULL DEFAULT 'read-only'` backfills every existing row to read-only.
    pub(super) fn migrate_v083(&mut self, current_version: i64) {
        if current_version < 83 {
            ensure_column(
                &self.conn,
                "universes",
                "source_mode",
                "TEXT NOT NULL DEFAULT 'read-only'",
            )
            .expect("migration v83: universes.source_mode");

            crate::record_migration!(
                self.conn,
                83,
                "CO-413: universes.source_mode (bidirectional event-bus bridge write policy)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn source_mode_column_defaults_to_read_only_and_loads_via_get_universe() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());

        // A freshly-inserted universe row gets `source_mode = 'read-only'` from
        // the v83 column default — i.e. the gate stays closed by default.
        storage.ensure_local_universe("u-default", "Default", false);
        let u = storage.get_universe("u-default").expect("universe exists");
        assert_eq!(u.source_mode.as_deref(), Some("read-only"));

        // Marking a universe as a bidirectional event-bus source round-trips
        // through get_universe so the write gate / capability flag can read it.
        storage
            .conn()
            .execute(
                "UPDATE universes SET source_kind = 'event-bus', source_mode = 'bidirectional' \
                 WHERE key = 'u-default'",
                [],
            )
            .unwrap();
        let u = storage.get_universe("u-default").expect("universe exists");
        assert_eq!(u.source_kind.as_deref(), Some("event-bus"));
        assert_eq!(u.source_mode.as_deref(), Some("bidirectional"));
    }
}
