use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-448: least-privilege capability scopes for API tokens.
    ///
    /// Adds a nullable `scopes` column to `api_tokens` holding a JSON array of
    /// resolved capability strings (`recurso:ação`, e.g. `entries:read`). The
    /// hybrid model lets an issuer pass either raw capabilities or a named
    /// bundle (`read`/`write`/`admin`/`agent`); the bundle is expanded to its
    /// capability set **at issuance** and the expanded set is what we persist,
    /// so a token is auditable — you can read exactly what it can do.
    ///
    /// `NULL` (existing tokens + the legacy `create_api_token` path) means **no
    /// declared scope → inherit the owner's tier** (the all-or-nothing behavior
    /// from before CO-448). Only tokens *with* a non-NULL `scopes` value are
    /// capability-restricted, so no already-issued token (vault, co-auto
    /// reporting, …) breaks.
    ///
    /// Additive + idempotent (`ensure_column` checks `pragma_table_info`
    /// first). Added as its own migration step — NEVER in the base api_tokens
    /// batch, which runs on existing DBs before migrations and would panic the
    /// server at boot if it referenced a not-yet-added column (CO-354 trap).
    pub(super) fn migrate_v081(&mut self, current_version: i64) {
        if current_version < 81 {
            ensure_column(&self.conn, "api_tokens", "scopes", "TEXT")
                .expect("migration v81: api_tokens.scopes");

            crate::record_migration!(
                self.conn,
                81,
                "CO-448: api_tokens.scopes (JSON capability array; NULL = inherit owner tier)"
            );
        }
    }
}
