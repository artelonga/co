//! Typed meta-DB accessors for the auth/onboarding flow (CO-433).
//!
//! Moves the raw `conn().execute/query_row` calls out of `auth/*_routes.rs`
//! into typed methods on `Storage`. These all touch the global meta-DB
//! (`users`, `quilombo_usuarios`) — the state the global `Mutex<Storage>`
//! legitimately guards.

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

    /// Recovery lookup: a quilombo user already linked to a CO user, matched by
    /// `quilombo_usuarios.usuario`. Returns the canonical CO user id.
    pub fn find_linked_co_user_by_quilombo_usuario(&self, usuario: &str) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT linked_co_user_id FROM quilombo_usuarios \
                 WHERE usuario = ?1 AND linked_co_user_id IS NOT NULL",
                rusqlite::params![usuario],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
    }

    /// Recovery lookup: an unlinked legacy quilombo user matched by `usuario`.
    /// Returns the `quilombo_usuarios.id` so the caller can lazily bridge it.
    pub fn find_unlinked_quilombo_id_by_usuario(&self, usuario: &str) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT id FROM quilombo_usuarios \
                 WHERE usuario = ?1 AND (linked_co_user_id IS NULL OR linked_co_user_id = '')",
                rusqlite::params![usuario],
                |row| row.get::<_, String>(0),
            )
            .ok()
    }

    /// Recovery lookup: a quilombo user matched by `quilombo_usuarios.email`.
    /// Returns the `quilombo_usuarios.id` for lazy bridging.
    pub fn find_quilombo_id_by_email(&self, email: &str) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT id FROM quilombo_usuarios WHERE email = ?1",
                rusqlite::params![email],
                |row| row.get(0),
            )
            .ok()
    }
}
