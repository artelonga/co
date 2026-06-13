use super::super::Storage;
use super::super::schema::ensure_column;

impl Storage {
    /// CO-366: billing / payment wiring (register → paid).
    ///
    /// Adds four billing columns to `users` and the `billing_events` audit
    /// table. Additive, backwards-compatible. The columns are added through
    /// [`ensure_column`] (its own step) rather than the base `UNIVERSE_SCHEMA`
    /// batch — that batch runs on existing DBs before migrations, so referencing
    /// a not-yet-added column there panics the server at boot (CO-354 trap).
    ///
    /// New columns are read via explicit `SELECT`s (never `.ok()`-swallowed) so
    /// a missing column fails loudly rather than aliasing to "row not found".
    pub(super) fn migrate_v077(&mut self, current_version: i64) {
        if current_version < 77 {
            // tier_paid_at  — ISO-8601 timestamp the user converted to paid.
            ensure_column(&self.conn, "users", "tier_paid_at", "TEXT")
                .expect("migration v77: users.tier_paid_at");
            // tier_plan     — 'free' | 'starter' | 'pro' | 'enterprise'.
            ensure_column(&self.conn, "users", "tier_plan", "TEXT")
                .expect("migration v77: users.tier_plan");
            // billing_provider     — 'hostinger' | 'manual' | 'pix' | 'stripe'.
            ensure_column(&self.conn, "users", "billing_provider", "TEXT")
                .expect("migration v77: users.billing_provider");
            // billing_external_id  — provider's customer/subscription id.
            ensure_column(&self.conn, "users", "billing_external_id", "TEXT")
                .expect("migration v77: users.billing_external_id");

            // Audit trail of every billing event the system observes
            // (checkout_created, payment_succeeded, payment_failed,
            // subscription_canceled). `payload_json` keeps the (redacted)
            // provider payload for forensics.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS billing_events (
                        id           TEXT PRIMARY KEY,
                        user_id      TEXT NOT NULL,
                        event_type   TEXT NOT NULL,
                        provider     TEXT NOT NULL,
                        payload_json TEXT,
                        created_at   TEXT NOT NULL,
                        FOREIGN KEY (user_id) REFERENCES users(id)
                    );
                    CREATE INDEX IF NOT EXISTS idx_billing_events_user
                        ON billing_events (user_id, created_at DESC);
                    CREATE INDEX IF NOT EXISTS idx_billing_events_type
                        ON billing_events (event_type, created_at DESC);",
                )
                .expect("migration v77: billing_events table + indexes");

            crate::record_migration!(
                self.conn,
                77,
                "CO-366: users billing columns + billing_events (register → paid wiring)"
            );
        }
    }
}
