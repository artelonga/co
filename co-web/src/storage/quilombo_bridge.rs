use chrono::Utc;
use rusqlite::params;

use super::Storage;

impl Storage {
    /// CO-172 Phase 1: Idempotently create or find a CO `users` row for a
    /// quilombo user. Priority:
    /// 1. `linked_co_user_id` already set → return it (no-op).
    /// 2. Email matches an existing `users.email` → return that user's id.
    /// 3. INSERT a new `users` row and return the new id.
    ///
    /// Does NOT write `linked_co_user_id`; call `link_quilombo_to_co` after.
    pub fn ensure_co_user_for_quilombo(&self, quilombo_user_id: &str) -> anyhow::Result<String> {
        struct QRow {
            linked_co_user_id: Option<String>,
            email: Option<String>,
            senha_hash: String,
            usuario: String,
            nome: String,
        }
        let q = self
            .conn
            .query_row(
                "SELECT linked_co_user_id, email, senha_hash, usuario, nome \
                 FROM quilombo_usuarios WHERE id = ?1",
                params![quilombo_user_id],
                |row| {
                    Ok(QRow {
                        linked_co_user_id: row.get(0)?,
                        email: row.get(1)?,
                        senha_hash: row.get(2)?,
                        usuario: row.get(3)?,
                        nome: row.get(4)?,
                    })
                },
            )
            .map_err(|e| anyhow::anyhow!("quilombo user {quilombo_user_id} not found: {e}"))?;

        // 1. Already linked.
        if let Some(co_id) = q.linked_co_user_id {
            return Ok(co_id);
        }

        // 2. Email match → link to existing CO user.
        if let Some(ref email) = q.email {
            let existing_id: Option<String> = self
                .conn
                .query_row(
                    "SELECT id FROM users WHERE email = ?1",
                    params![email],
                    |row| row.get(0),
                )
                .ok();
            if let Some(co_id) = existing_id {
                return Ok(co_id);
            }
        }

        // 3. Create new CO user (tier='player', reuse Argon2id hash).
        let co_id = format!("usr_{}", nanoid::nanoid!(10));
        let now = Utc::now().to_rfc3339();
        // Try with usuario first; fall back to NULL on unique-index collision.
        let inserted = self.conn.execute(
            "INSERT INTO users \
             (id, email, display_name, tier, created_at, password_hash, usuario) \
             VALUES (?1, ?2, ?3, 'player', ?4, ?5, ?6)",
            params![
                co_id,
                q.email.as_deref(),
                q.nome,
                now,
                q.senha_hash,
                q.usuario
            ],
        );
        if inserted.is_err() {
            self.conn.execute(
                "INSERT INTO users \
                 (id, email, display_name, tier, created_at, password_hash) \
                 VALUES (?1, ?2, ?3, 'player', ?4, ?5)",
                params![co_id, q.email.as_deref(), q.nome, now, q.senha_hash],
            )?;
        }

        Ok(co_id)
    }

    /// CO-172: Set `linked_co_user_id` on a `quilombo_usuarios` row.
    pub fn link_quilombo_to_co(
        &self,
        quilombo_user_id: &str,
        co_user_id: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE quilombo_usuarios SET linked_co_user_id = ?1 WHERE id = ?2",
            params![co_user_id, quilombo_user_id],
        )?;
        Ok(())
    }

    /// CO-172 Phase 4: After a CO password reset, mirror the new hash to all
    /// linked `quilombo_usuarios` rows. Returns the number of updated rows.
    pub fn mirror_password_to_quilombo(
        &self,
        co_user_id: &str,
        new_hash: &str,
    ) -> anyhow::Result<usize> {
        let n = self.conn.execute(
            "UPDATE quilombo_usuarios SET senha_hash = ?1 WHERE linked_co_user_id = ?2",
            params![new_hash, co_user_id],
        )?;
        Ok(n)
    }

    /// CO-172 Phase 2: For every quilombo user with a set email and a linked CO
    /// user, ensure their email is a verified CO recovery channel.
    /// Idempotent; runs on every boot.
    pub fn backfill_quilombo_recovery_channels(&self) -> usize {
        let mut stmt = match self.conn.prepare(
            "SELECT linked_co_user_id, email FROM quilombo_usuarios \
             WHERE email IS NOT NULL AND email != '' \
               AND linked_co_user_id IS NOT NULL AND linked_co_user_id != ''",
        ) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();

        let mut count = 0usize;
        for (co_user_id, email) in rows {
            if let Err(e) = self.ensure_email_recovery_channel(&co_user_id, &email) {
                tracing::warn!(
                    co_user_id = %co_user_id,
                    "backfill_quilombo_recovery_channels: {e}"
                );
                continue;
            }
            count += 1;
        }
        count
    }
}
