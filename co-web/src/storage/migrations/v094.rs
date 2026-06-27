use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-491: durable per-user WhatsApp consent record — persist EXACTLY which
    /// versioned consent document a user agreed to at link time, so the agreement
    /// survives a restart and is queryable for an LGPD request (demonstrabilidade,
    /// Art. 8º §2º). Previously the agreed version was only observable in a log
    /// line + the `link/verify` response.
    ///
    /// - `whatsapp_consent_version` — the document `version` string the user
    ///   agreed to (e.g. `whatsapp-consent/2026-06-27.v1`); `NULL` ⇒ never linked.
    /// - `whatsapp_consent_at`      — RFC 3339 timestamp of the agreement.
    /// - `whatsapp_consent_sha`     — sha256 (hex) of the rendered agreement text
    ///   for the operator, so the exact copy agreed to is reproducible/verifiable
    ///   even if a version file were ever (incorrectly) edited in place.
    ///
    /// Additive + idempotent (`ensure_column` — a guarded `ALTER TABLE`); never
    /// folded into the base `users` create. Each guard checks `current_version < N`
    /// independently so fresh and existing DBs converge.
    ///
    /// ── Cross-branch v094 coordination note ────────────────────────────────────
    /// This branch's train tip (`LATEST_VERSION`) was 93, so this migration claims
    /// `max + 1 = 94`. A **sibling** branch (`feat/CO-488`, birthday-consent
    /// columns) ALSO defines a `mod v094`. That overlap is INTENTIONAL and SAFE:
    ///   * We deliberately did NOT skip to a `v095` gap. A numeric gap is
    ///     prod-unsafe — if THIS branch merged first and the sibling later added
    ///     `v094`, every guard here uses `current_version < 94`, so on a DB already
    ///     at version 95 the sibling's `v094` block would be SILENTLY SKIPPED
    ///     (silent prod break / unapplied columns).
    ///   * Claiming the SAME `v094` instead turns the overlap into a LOUD,
    ///     compile-time `duplicate definition of mod v094` error the moment the two
    ///     branches meet at merge — forcing a clean renumber-to-`v095` of whichever
    ///     lands SECOND, with both column sets applied in order. Loud-at-merge beats
    ///     silent-in-prod.
    ///
    /// Both column sets are purely additive, so the renumber-the-second resolution
    /// is mechanical: copy the second's block to `v095`, bump `LATEST_VERSION`.
    pub(super) fn migrate_v094(&mut self, current_version: i64) {
        if current_version < 94 {
            ensure_column(&self.conn, "users", "whatsapp_consent_version", "TEXT")
                .expect("migration v94: users.whatsapp_consent_version");
            ensure_column(&self.conn, "users", "whatsapp_consent_at", "TEXT")
                .expect("migration v94: users.whatsapp_consent_at");
            ensure_column(&self.conn, "users", "whatsapp_consent_sha", "TEXT")
                .expect("migration v94: users.whatsapp_consent_sha");

            crate::record_migration!(
                self.conn,
                94,
                "CO-491: whatsapp consent version persistence (users.whatsapp_consent_version/_at/_sha)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn migration_v094_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        // Re-running across the version boundary must not duplicate the column.
        storage.migrate_v094(93);
        storage.migrate_v094(94);

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('users') \
                 WHERE name = 'whatsapp_consent_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "exactly one whatsapp_consent_version column");
    }

    #[test]
    fn consent_columns_default_null() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, created_at) VALUES ('u1', 'a@b.c', 'now')",
                [],
            )
            .unwrap();

        let (ver, at, sha): (Option<String>, Option<String>, Option<String>) = storage
            .conn()
            .query_row(
                "SELECT whatsapp_consent_version, whatsapp_consent_at, whatsapp_consent_sha \
                 FROM users WHERE id = 'u1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(ver, None, "consent version defaults NULL (never linked)");
        assert_eq!(at, None);
        assert_eq!(sha, None);
    }

    #[test]
    fn set_then_get_whatsapp_consent_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, created_at) VALUES ('u1', 'a@b.c', 'now')",
                [],
            )
            .unwrap();

        // Unset → None.
        assert_eq!(storage.get_whatsapp_consent("u1"), None);

        let updated = storage
            .set_whatsapp_consent(
                "u1",
                "whatsapp-consent/2026-06-27.v1",
                "2026-06-27T12:00:00+00:00",
                "deadbeef",
            )
            .unwrap();
        assert_eq!(updated, 1, "one row updated");

        assert_eq!(
            storage.get_whatsapp_consent("u1"),
            Some((
                "whatsapp-consent/2026-06-27.v1".to_string(),
                "2026-06-27T12:00:00+00:00".to_string(),
                "deadbeef".to_string(),
            )),
            "get returns exactly what was set"
        );

        // Unknown user → no rows updated, still None.
        assert_eq!(
            storage
                .set_whatsapp_consent("nobody", "v", "t", "s")
                .unwrap(),
            0
        );
        assert_eq!(storage.get_whatsapp_consent("nobody"), None);
    }
}
