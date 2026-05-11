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

    /// CO-184: reverse bridge — given a CO user, idempotently ensure a
    /// `quilombo_usuarios` row exists and is linked to that CO user.
    /// Returns the quilombo user id.
    ///
    /// Resolution priority:
    /// 1. A row with `linked_co_user_id = co_user_id` already exists → return it.
    /// 2. A row with matching `email` exists → set its `linked_co_user_id` and return it.
    /// 3. Insert a fresh placeholder row (`papel = 'membro'`,
    ///    `senha_hash = '!provisorio!'` — same sentinel as QB-6 for accounts
    ///    that authenticate via CO session and don't use the legacy quilombo
    ///    password login). `usuario` is taken from the CO `usuario` column
    ///    (or email local-part) with a `-N` dedupe suffix on collision.
    ///
    /// Called from every CO sign-in path so users always have a quilombo
    /// identity available — closes the loop opened by CO-172 (quilombo→CO)
    /// in the other direction (CO→quilombo).
    pub fn ensure_quilombo_user_for_co(&self, co_user_id: &str) -> anyhow::Result<String> {
        // 1. Already linked.
        let already: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM quilombo_usuarios WHERE linked_co_user_id = ?1",
                params![co_user_id],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = already {
            return Ok(id);
        }

        // Pull what we know about the CO user.
        struct CoRow {
            email: Option<String>,
            usuario: Option<String>,
            display_name: String,
        }
        let co = self
            .conn
            .query_row(
                "SELECT email, usuario, COALESCE(display_name, '') FROM users WHERE id = ?1",
                params![co_user_id],
                |row| {
                    Ok(CoRow {
                        email: row.get(0)?,
                        usuario: row.get(1)?,
                        display_name: row.get(2)?,
                    })
                },
            )
            .map_err(|e| anyhow::anyhow!("CO user {co_user_id} not found: {e}"))?;

        // 2. Email match — link an existing quilombo user that happens to
        //    share the email.
        if let Some(ref email) = co.email
            && !email.is_empty()
        {
            let by_email: Option<String> = self
                .conn
                .query_row(
                    "SELECT id FROM quilombo_usuarios WHERE email = ?1",
                    params![email],
                    |row| row.get(0),
                )
                .ok();
            if let Some(q_id) = by_email {
                let _ = self.link_quilombo_to_co(&q_id, co_user_id);
                return Ok(q_id);
            }
        }

        // 3. Insert. Pick a unique `usuario` — try the CO usuario, then
        //    email local-part, with `-N` dedupe.
        let base = co.usuario.clone().unwrap_or_else(|| {
            co.email
                .as_deref()
                .and_then(|e| e.split('@').next())
                .unwrap_or("user")
                .to_string()
        });
        let base = base.trim().to_lowercase();
        let mut candidate = base.clone();
        let mut n = 2;
        while self
            .conn
            .query_row(
                "SELECT 1 FROM quilombo_usuarios WHERE usuario = ?1",
                params![candidate],
                |_| Ok(true),
            )
            .unwrap_or(false)
        {
            candidate = format!("{base}-{n}");
            n += 1;
            if n > 100 {
                candidate = format!("{base}-{}", nanoid::nanoid!(6));
                break;
            }
        }

        let q_id = uuid::Uuid::new_v4().to_string();
        let nome = if co.display_name.is_empty() {
            candidate.clone()
        } else {
            co.display_name.clone()
        };
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO quilombo_usuarios \
             (id, usuario, nome, senha_hash, criado_em, atualizado_em, email, linked_co_user_id, papel) \
             VALUES (?1, ?2, ?3, '!provisorio!', ?4, ?4, ?5, ?6, 'membro')",
            params![q_id, candidate, nome, now, co.email, co_user_id],
        )?;

        tracing::info!(
            "CO-184 reverse bridge: created quilombo_usuarios {q_id} ({candidate}) for CO user {co_user_id}"
        );
        Ok(q_id)
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

    /// Mirror `users.email` onto any linked `quilombo_usuarios` row that
    /// currently has no email — ensuring the quilombo identity carries the
    /// recovery address. Username is NEVER touched. Returns the number of
    /// rows updated. If a quilombo row already has a different non-empty
    /// email, it is left alone (the user can set it via quilombo's own
    /// profile flow).
    pub fn mirror_email_to_quilombo(
        &self,
        co_user_id: &str,
        new_email: &str,
    ) -> anyhow::Result<usize> {
        let normalized = new_email.trim().to_lowercase();
        if normalized.is_empty() {
            return Ok(0);
        }
        let n = self.conn.execute(
            "UPDATE quilombo_usuarios \
             SET email = ?1, atualizado_em = ?2 \
             WHERE linked_co_user_id = ?3 \
               AND (email IS NULL OR email = '' OR LOWER(email) = ?1)",
            params![normalized, chrono::Utc::now().to_rfc3339(), co_user_id],
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
