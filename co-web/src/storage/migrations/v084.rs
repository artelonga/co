use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-338: surface keys — add `surface_dns` to `universes`.
    ///
    /// A deployable unit (a universe served at its own DNS) carries a
    /// `surface_dns` host (e.g. `yggdrasil.artelonga.com.br`). A nested
    /// sub-universe leaves it `NULL` and inherits a deploying ancestor's DNS.
    ///
    /// Together with the existing `parent_key` (CO-98) column, this is the data
    /// the `core::surface` resolver walks to turn a `key::path` reference into a
    /// live deployment URL — and the source the deployment-snapshot worker reads
    /// instead of its hardcoded unit list.
    ///
    /// CO-413 took v83 (`universes.source_mode`); this task claims **v84**.
    ///
    /// Additive + idempotent (`ensure_column` checks `pragma_table_info` first).
    /// Nullable with no default — absence means "inherits an ancestor".
    pub(super) fn migrate_v084(&mut self, current_version: i64) {
        if current_version < 84 {
            ensure_column(&self.conn, "universes", "surface_dns", "TEXT")
                .expect("migration v84: universes.surface_dns");

            crate::record_migration!(
                self.conn,
                84,
                "CO-338: universes.surface_dns (surface-key deployment resolution)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn surface_dns_defaults_to_null_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());

        // A freshly-inserted universe row has no surface_dns (inherits ancestor).
        storage.ensure_local_universe("u-leaf", "Leaf", false);
        let u = storage.get_universe("u-leaf").expect("universe exists");
        assert_eq!(u.surface_dns, None);

        // Promoting it to a deployable surface round-trips through get_universe.
        storage
            .conn()
            .execute(
                "UPDATE universes SET surface_dns = 'leaf.artelonga.com.br' WHERE key = 'u-leaf'",
                [],
            )
            .unwrap();
        let u = storage.get_universe("u-leaf").expect("universe exists");
        assert_eq!(u.surface_dns.as_deref(), Some("leaf.artelonga.com.br"));
    }
}
