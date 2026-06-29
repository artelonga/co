//! Typed meta-DB accessors for the auth/onboarding flow (CO-433).
//!
//! Moves the raw `conn().execute/query_row` calls out of `auth/*_routes.rs`
//! into typed methods on `Storage`. These all touch the global meta-DB
//! (`users`) — the state the global `Mutex<Storage>` legitimately guards.

use rusqlite::Result;

use super::Storage;

impl Storage {
    /// CO-370: activate a pre-registered shell user on first verified login.
    /// Returns rows updated (0 if the user is not in `pre-registered` state).
    pub fn activate_pre_registered_user(&self, user_id: &str, now: &str) -> Result<usize> {
        self.conn().execute(
            "UPDATE users SET status = 'active', activated_at = ?1 \
             WHERE id = ?2 AND status = 'pre-registered'",
            rusqlite::params![now, user_id],
        )
    }

    /// Insert a freshly onboarded (email-verified) user with no password hash.
    /// `usuario` doubles as the initial `display_name` (matches the original
    /// `?2`-reuse in the route).
    pub fn insert_onboarding_user(
        &self,
        id: &str,
        usuario: &str,
        email: &str,
        now: &str,
        origin: Option<&str>,
    ) -> Result<usize> {
        self.conn().execute(
            "INSERT INTO users \
             (id, usuario, email, display_name, tier, created_at, origin, \
              status, activated_at) \
             VALUES (?1, ?2, ?3, ?2, 'player', ?4, ?5, 'active', ?4)",
            rusqlite::params![id, usuario, email, now, origin],
        )
    }
}
