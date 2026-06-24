use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-487: birthday greetings — opt-in consent + birthday on the user, so a
    /// daily job can send a WhatsApp "feliz aniversário" on the day.
    ///
    /// - `birthday` — `MM-DD` (year-agnostic match), `NULL` ⇒ unset.
    /// - `birthday_consent` — `0`/`1`; the LGPD opt-in captured at login.
    /// - `whatsapp` — the recipient number; `NULL` ⇒ no channel, no send.
    /// - `birthday_greeted_year` — last year greeted; idempotency (one/year).
    ///
    /// Additive + idempotent (`ensure_column` — guarded `ALTER`); never folded
    /// into the base `users` create. Migration **v093** (tip v092, max+1). Each
    /// guard checks `current_version < N` so fresh/existing DBs converge.
    pub(super) fn migrate_v093(&mut self, current_version: i64) {
        if current_version < 93 {
            ensure_column(&self.conn, "users", "birthday", "TEXT")
                .expect("migration v93: users.birthday");
            ensure_column(
                &self.conn,
                "users",
                "birthday_consent",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v93: users.birthday_consent");
            ensure_column(&self.conn, "users", "whatsapp", "TEXT")
                .expect("migration v93: users.whatsapp");
            ensure_column(&self.conn, "users", "birthday_greeted_year", "INTEGER")
                .expect("migration v93: users.birthday_greeted_year");

            crate::record_migration!(
                self.conn,
                93,
                "CO-487: birthday greetings (users.birthday/birthday_consent/whatsapp/birthday_greeted_year)"
            );
        }
    }
}
