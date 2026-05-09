use chrono::Utc;
use rusqlite::params;

use super::Storage;
use super::schema::parse_datetime;

impl Storage {
    pub fn create_user(
        &mut self,
        email: &str,
        display_name: &str,
    ) -> anyhow::Result<crate::models::User> {
        let id = format!("usr_{}", nanoid::nanoid!(10));
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT INTO users (id, email, display_name, tier, created_at) VALUES (?1, ?2, ?3, 'admin', ?4)",
            params![id, email, display_name, now_str],
        )?;
        // 1.46.0: every new user auto-subscribes to default universes
        // (yggdrasil today; future onboarding universes opt in via the
        // `default_for_new_users` flag).
        if let Err(e) = self.subscribe_user_to_default_universes(&id) {
            tracing::warn!("create_user: default subscriptions failed for {id}: {e}");
        }
        Ok(crate::models::User {
            id,
            email: email.to_string(),
            display_name: display_name.to_string(),
            tier: "admin".to_string(),
            created_at: now,
            usuario: None,
        })
    }

    pub fn get_user_by_email(&self, email: &str) -> Option<crate::models::User> {
        self.conn
            .query_row(
                "SELECT id, email, display_name, tier, created_at FROM users WHERE email = ?1",
                params![email],
                |row| {
                    Ok(crate::models::User {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        tier: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                        usuario: None,
                    })
                },
            )
            .ok()
    }

    pub fn get_user_by_id(&self, id: &str) -> Option<crate::models::User> {
        self.conn
            .query_row(
                "SELECT id, COALESCE(email,'') as email, display_name, tier, created_at \
                 FROM users WHERE id = ?1",
                params![id],
                |row| {
                    Ok(crate::models::User {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        tier: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                        usuario: None,
                    })
                },
            )
            .ok()
    }

    /// CO-165: Get user by usuario (username).
    pub fn get_user_by_usuario(&self, usuario: &str) -> Option<crate::models::User> {
        self.conn
            .query_row(
                "SELECT id, COALESCE(email,'') as email, display_name, tier, created_at \
                 FROM users WHERE usuario = ?1",
                params![usuario],
                |row| {
                    Ok(crate::models::User {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        tier: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                        usuario: Some(usuario.to_string()),
                    })
                },
            )
            .ok()
    }

    /// CO-165: Get user by ID along with their stored password hash.
    pub fn get_user_by_id_with_hash(
        &self,
        id: &str,
    ) -> Option<(crate::models::User, Option<String>)> {
        self.conn
            .query_row(
                "SELECT id, COALESCE(email,'') as email, display_name, tier, created_at, \
                 password_hash FROM users WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        crate::models::User {
                            id: row.get(0)?,
                            email: row.get(1)?,
                            display_name: row.get(2)?,
                            tier: row.get(3)?,
                            created_at: parse_datetime(&row.get::<_, String>(4)?),
                            usuario: None,
                        },
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .ok()
    }

    /// CO-165: Update user's password hash.
    pub fn update_password_hash(&self, user_id: &str, hash: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![hash, user_id],
        )?;
        Ok(())
    }

    // --- UAT-specific methods (CO-44) ---

    /// Get user by email along with their stored Argon2 password hash.
    /// Returns `None` if the user does not exist or has no password set.
    pub fn get_user_by_email_with_hash(
        &self,
        email: &str,
    ) -> Option<(crate::models::User, Option<String>)> {
        self.conn
            .query_row(
                "SELECT id, email, display_name, tier, created_at, password_hash \
                 FROM users WHERE email = ?1",
                params![email],
                |row| {
                    Ok((
                        crate::models::User {
                            id: row.get(0)?,
                            email: row.get(1)?,
                            display_name: row.get(2)?,
                            tier: row.get(3)?,
                            created_at: parse_datetime(&row.get::<_, String>(4)?),
                            usuario: None,
                        },
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .ok()
    }

    /// Seed the UAT `yuri@uat.local` user with an Argon2-hashed password.
    ///
    /// Idempotent: if the user already exists their password hash is updated
    /// to the supplied value (so a fresh hash is applied on each UAT startup).
    pub fn seed_uat_user(&mut self, password_hash: &str) -> anyhow::Result<()> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM users WHERE email = 'yuri@uat.local'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if exists {
            self.conn.execute(
                "UPDATE users SET password_hash = ?1, tier = 'admin' WHERE email = 'yuri@uat.local'",
                params![password_hash],
            )?;
            tracing::info!("UAT: updated yuri@uat.local password hash");
        } else {
            let now = Utc::now().to_rfc3339();
            self.conn.execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, password_hash) \
                 VALUES ('usr_yuri_uat', 'yuri@uat.local', 'yuri', 'admin', ?1, ?2)",
                params![now, password_hash],
            )?;
            tracing::info!("UAT: seeded user yuri@uat.local (tier=admin)");
        }
        Ok(())
    }

    /// Seed an admin user from env vars `CO_SEED_ADMIN_EMAIL` + `CO_SEED_ADMIN_PASSWORD_HASH`.
    ///
    /// Idempotent with drift detection:
    /// - User missing → insert (tier=admin).
    /// - User exists, hash unchanged → no-op.
    /// - User exists, hash differs → update hash + tier.
    pub fn seed_admin_user_from_env(
        &mut self,
        email: &str,
        password_hash: &str,
    ) -> anyhow::Result<()> {
        if !password_hash.starts_with("$argon2id$") {
            tracing::warn!(
                "CO_SEED_ADMIN_PASSWORD_HASH does not look like an Argon2id hash \
                 (expected '$argon2id$v=19$m=...$...'). Check your configuration."
            );
        }

        let email = email.trim().to_lowercase();

        let existing = self
            .conn
            .query_row(
                "SELECT id, password_hash FROM users WHERE email = ?1",
                params![email],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .ok();

        match existing {
            Some((user_id, ref existing_hash))
                if existing_hash.as_deref() == Some(password_hash) =>
            {
                // 1.45.0: idempotently ensure tier='admin' even when the
                // hash is unchanged — pre-collapse rows on already-deployed
                // prods need to flip from 'user'/'player'/'pro' to 'admin'.
                self.conn.execute(
                    "UPDATE users SET tier = 'admin' WHERE id = ?1 AND tier <> 'admin'",
                    params![user_id],
                )?;
                tracing::info!("admin user already seeded: {email} (hash unchanged, tier admin)");
            }
            Some((user_id, _)) => {
                self.conn.execute(
                    "UPDATE users SET password_hash = ?1, tier = 'admin' WHERE id = ?2",
                    params![password_hash, user_id],
                )?;
                tracing::info!("seeded user updated: {email} (hash refreshed, tier admin)");
            }
            None => {
                let id = format!(
                    "usr_{}",
                    &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
                );
                let now = Utc::now().to_rfc3339();
                // 1.45.0: every authenticated user is an admin. Tier collapse.
                self.conn.execute(
                    "INSERT INTO users (id, email, display_name, tier, created_at, password_hash) \
                     VALUES (?1, ?2, ?2, 'admin', ?3, ?4)",
                    params![id, email, now, password_hash],
                )?;
                tracing::info!("seeded user created: {email} (tier admin)");
            }
        }

        // Auto-attach the seed email as a verified recovery channel — no
        // extra UI step needed for forgot-password to work for the admin.
        // Idempotent; safe to call on every boot.
        let user_id: String = self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![email],
            |row| row.get(0),
        )?;
        if let Err(e) = self.ensure_email_recovery_channel(&user_id, &email) {
            tracing::warn!(
                "seed_admin_user_from_env: ensure_email_recovery_channel failed for {email}: {e}"
            );
        }
        Ok(())
    }

    /// CO-90 (preview): make the seeded admin user a member of every existing
    /// system-owned universe so the SPA shows them in the user's sidebar after
    /// login. Idempotent (`INSERT OR IGNORE`). Skips universes that don't
    /// exist yet — call this AFTER all universe seeds.
    ///
    /// Without this, a freshly-seeded admin logs in to an empty dashboard
    /// because `list_universes_for_user` only returns owned + member +
    /// subscribed universes.
    pub fn ensure_admin_universe_memberships(&mut self, email: &str) -> anyhow::Result<()> {
        let email = email.trim().to_lowercase();
        let user_id: String = match self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![email],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!(
                    "ensure_admin_universe_memberships: no user for {email} (seed first)"
                );
                return Ok(());
            }
        };

        // System universes the seeded admin should see immediately.
        // Order is irrelevant; `INSERT OR IGNORE` handles repeats.
        // co-dev and co-experience removed (CO-142 Phase C — deprecated).
        let system_keys = [
            "template",
            "quilomboaraucaria",
            "yggdrasil",
            "dados",
            // Admin content universes (seeded by seed_admin_content_universes).
            "artelonga",
            "rfq",
            "co",
            // CO-141 meaning-topology universes (seeded by
            // seed_admin_content_universes). Admin needs membership so
            // POST /assets and the asset browser work for these.
            "mbya",
            "concepts",
            "guarani-mbya",
            "portuguese",
            "yoruba",
            "languages",
            "time",
        ];
        let now = Utc::now().to_rfc3339();
        let mut added = 0usize;
        for key in system_keys {
            // Skip universes that don't exist (haven't been seeded yet).
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM universes WHERE key = ?1",
                    params![key],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if !exists {
                continue;
            }
            let n = self.conn.execute(
                "INSERT OR IGNORE INTO universe_members \
                 (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'admin', ?3)",
                params![key, user_id, now],
            )?;
            if n > 0 {
                added += 1;
            }
        }
        if added > 0 {
            tracing::info!(
                "ensure_admin_universe_memberships: added {email} to {added} universe(s) as admin"
            );
        }
        Ok(())
    }

    /// Re-home a known list of personal universes to the admin's CURRENT
    /// user_id, regardless of whether the prior owner is still in `users`.
    ///
    /// `rescue_orphan_universes` only catches truly dangling `owner_id` values
    /// (no row in `users`). But a more common failure mode is two valid users:
    /// the prior admin (still in `users` from a stale bootstrap), and the
    /// current admin (re-seeded). The universes still point at the prior
    /// admin's id, so the new admin can't see them.
    ///
    /// This function is targeted — it only touches the well-known personal
    /// universes named in the bootstrap script. Idempotent. Returns the number
    /// of universes re-homed.
    /// Free up slug claims held by `*@co.local` legacy/test users so real
    /// users can claim those slugs as their `username`. Renames any colliding
    /// `*@co.local` user's username to `legacy-<original-slug>` (e.g.
    /// `yuri` → `legacy-yuri`). Idempotent — already-renamed rows are no-ops.
    ///
    /// 2026-05-02 — closes the unique-index conflict that blocked
    /// `ensure_admin_username` from claiming `yuri` for the admin while the
    /// legacy `yuri@co.local` test user held it.
    pub fn free_legacy_co_local_usernames(&mut self) -> anyhow::Result<usize> {
        // Only touches usernames currently NOT prefixed with 'legacy-' to
        // remain idempotent across re-runs.
        let updated = self.conn.execute(
            "UPDATE users SET usuario = 'legacy-' || usuario \
             WHERE email LIKE '%@co.local' \
               AND usuario IS NOT NULL \
               AND usuario != '' \
               AND usuario NOT LIKE 'legacy-%'",
            [],
        )?;
        if updated > 0 {
            tracing::info!(
                "free_legacy_co_local_usernames: renamed {updated} legacy @co.local username(s)"
            );
        }
        Ok(updated)
    }

    /// Ensure the admin user has a username derived from their email prefix
    /// (e.g. `yuri@artelonga.com.br` → `yuri`) when currently empty.
    ///
    /// 2026-05-02 — addresses user directive "always use slug as user name by
    /// default". The unique index on `users.username` means we may collide
    /// with an existing user; on conflict we log and skip rather than break
    /// the boot path. `free_legacy_co_local_usernames` runs first to clear
    /// the common case of legacy `*@co.local` test users holding the slug.
    pub fn ensure_admin_username(&mut self, admin_email: &str) -> anyhow::Result<bool> {
        let email = admin_email.trim().to_lowercase();
        let prefix: String = email
            .split('@')
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if prefix.is_empty() {
            return Ok(false);
        }

        let current: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT username FROM users WHERE email = ?1",
                params![email],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok();
        let Some(current) = current else {
            return Ok(false);
        }; // user not seeded yet

        if current.as_deref().is_some_and(|s| !s.is_empty()) {
            return Ok(false); // already set, leave alone
        }

        // Try to claim the prefix as username. Unique index may collide with
        // a legacy/UAT user holding the same slug.
        match self.conn.execute(
            "UPDATE users SET username = ?1 WHERE email = ?2",
            params![prefix, email],
        ) {
            Ok(n) if n > 0 => {
                tracing::info!("ensure_admin_username: set username='{prefix}' for {email}");
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(e) => {
                tracing::warn!(
                    "ensure_admin_username: could not set username='{prefix}' for {email} \
                     (likely unique-index conflict with another user holding the same slug): {e}"
                );
                Ok(false)
            }
        }
    }

    pub fn ensure_admin_owns_personal_universes(
        &mut self,
        admin_email: &str,
        keys: &[&str],
    ) -> anyhow::Result<usize> {
        let admin_email = admin_email.trim().to_lowercase();
        let admin_user_id: String = match self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![admin_email],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => return Ok(0),
        };
        let now = Utc::now().to_rfc3339();
        let mut rehomed = 0usize;
        for key in keys {
            let current_owner: Option<String> = self
                .conn
                .query_row(
                    "SELECT owner_id FROM universes WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            let Some(current_owner) = current_owner else {
                continue; // universe doesn't exist
            };
            if current_owner == admin_user_id {
                // Still ensure membership row exists (defensive).
                self.conn.execute(
                    "INSERT OR IGNORE INTO universe_members \
                     (universe_key, user_id, role, joined_at) \
                     VALUES (?1, ?2, 'owner', ?3)",
                    params![key, admin_user_id, now],
                )?;
                continue;
            }
            self.conn.execute(
                "UPDATE universes SET owner_id = ?1 WHERE key = ?2",
                params![admin_user_id, key],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO universe_members \
                 (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, admin_user_id, now],
            )?;
            tracing::info!(
                "ensure_admin_owns_personal_universes: re-homed {key} from {current_owner} to {admin_email}"
            );
            rehomed += 1;
        }
        Ok(rehomed)
    }

    /// Delete anonymous-clone universes (key prefix `u-` or `anon-`) whose
    /// owner has been re-homed to the supplied admin user. These are the
    /// "Meu Co" clones that pile up on the admin's sidebar after a previous
    /// version of `rescue_orphan_universes` grabbed them.
    ///
    /// Only deletes universes whose owner currently equals the admin's user
    /// id — does NOT touch anon clones that genuinely belong to someone else,
    /// or that retain their original `anon-...` owner_id.
    ///
    /// Idempotent. Returns the number of universes removed.
    pub fn cleanup_admin_anon_clutter(&mut self, admin_email: &str) -> anyhow::Result<usize> {
        let admin_email = admin_email.trim().to_lowercase();
        let admin_user_id: String = match self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![admin_email],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => return Ok(0),
        };
        let keys: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT key FROM universes \
                 WHERE owner_id = ?1 \
                   AND (key LIKE 'u-%' OR key LIKE 'anon-%')",
            )?;
            stmt.query_map(params![admin_user_id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };
        let mut removed = 0usize;
        for key in &keys {
            let _ = self
                .conn
                .execute("DELETE FROM entries WHERE universe_key = ?1", params![key]);
            let _ = self.conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                params![key],
            );
            let n = self
                .conn
                .execute("DELETE FROM universes WHERE key = ?1", params![key])?;
            if n > 0 {
                let universe_dir = self.data_dir.join("universes").join(key);
                if universe_dir.exists() {
                    let _ = std::fs::remove_dir_all(&universe_dir);
                }
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!(
                "cleanup_admin_anon_clutter: removed {removed} stale anon-clone universe(s) from {admin_email}"
            );
        }
        Ok(removed)
    }

    /// Re-home universes whose `owner_id` no longer maps to any user — typically
    /// the result of a data wipe or migration that re-seeded the admin user
    /// with a different uuid, leaving prior universes orphaned.
    ///
    /// For every universe with a dangling `owner_id`, set the owner to the
    /// supplied admin user_id and ensure they're a member with role='owner'.
    /// Idempotent. Returns the number of universes re-homed.
    pub fn rescue_orphan_universes(&mut self, admin_email: &str) -> anyhow::Result<usize> {
        let admin_email = admin_email.trim().to_lowercase();
        let admin_user_id: String = match self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![admin_email],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => return Ok(0),
        };

        // Find universes whose owner_id has no row in users — but skip the
        // 'system' sentinel (template, quilomboaraucaria, yggdrasil, etc.)
        // AND skip anonymous-clone universes (key starts with `anon-` or
        // `u-`). Those should be deleted by `cleanup_anon_universes`, not
        // re-homed to the admin (otherwise the admin's sidebar fills up
        // with hundreds of orphaned "Meu Co" clones from past visitors).
        let orphan_keys: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT u.key FROM universes u \
                 LEFT JOIN users usr ON usr.id = u.owner_id \
                 WHERE usr.id IS NULL \
                   AND u.owner_id != 'system' \
                   AND u.key NOT LIKE 'anon-%' \
                   AND u.key NOT LIKE 'u-%'",
            )?;
            stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        let now = Utc::now().to_rfc3339();
        let mut rescued = 0usize;
        for key in &orphan_keys {
            self.conn.execute(
                "UPDATE universes SET owner_id = ?1 WHERE key = ?2",
                params![admin_user_id, key],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO universe_members \
                 (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, admin_user_id, now],
            )?;
            tracing::info!("rescue_orphan_universes: re-homed {key} to {admin_email}");
            rescued += 1;
        }
        Ok(rescued)
    }

    /// Remove all anonymous universes (keys starting with `anon-`) from the
    /// database and from the filesystem. Called on UAT startup so each session
    /// starts from a clean slate.
    ///
    /// Returns the number of universes removed.
    pub fn cleanup_anon_universes(&mut self) -> usize {
        let anon_keys: Vec<String> = {
            let mut stmt = match self
                .conn
                .prepare("SELECT key FROM universes WHERE key LIKE 'anon-%'")
            {
                Ok(s) => s,
                Err(_) => return 0,
            };
            stmt.query_map([], |row| row.get(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };

        let count = anon_keys.len();
        for key in &anon_keys {
            let _ = self
                .conn
                .execute("DELETE FROM entries WHERE universe_key = ?1", params![key]);
            let _ = self.conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                params![key],
            );
            let _ = self
                .conn
                .execute("DELETE FROM universes WHERE key = ?1", params![key]);
            let universe_dir = self.data_dir.join("universes").join(key);
            if universe_dir.exists() {
                let _ = std::fs::remove_dir_all(&universe_dir);
            }
        }

        if count > 0 {
            tracing::info!("UAT: cleaned up {} anonymous universe(s)", count);
        }
        count
    }

    /// Collect all users (with password hashes) for backup before a DB reset.
    pub fn get_all_users_with_hashes(&self) -> Vec<(crate::models::User, Option<String>)> {
        let mut stmt = match self
            .conn
            .prepare("SELECT id, email, display_name, tier, created_at, password_hash FROM users")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |row| {
            Ok((
                crate::models::User {
                    id: row.get(0)?,
                    email: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    display_name: row.get(2)?,
                    tier: row.get(3)?,
                    created_at: parse_datetime(&row.get::<_, String>(4)?),
                    usuario: None,
                },
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Re-insert users (with hashes) after a DB reset. Uses INSERT OR IGNORE to
    /// avoid duplicates if migrations re-ran and the yuri seed already ran.
    pub fn restore_users_with_hashes(&mut self, users: &[(crate::models::User, Option<String>)]) {
        for (user, hash) in users {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO users \
                 (id, email, display_name, tier, created_at, password_hash) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    user.id,
                    user.email,
                    user.display_name,
                    user.tier,
                    user.created_at.to_rfc3339(),
                    hash.as_deref(),
                ],
            );
        }
    }

    // --- Universes ---
}
