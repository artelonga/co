use chrono::Utc;
use rusqlite::params;
use sha2::{Digest, Sha256};

use super::Storage;
use super::schema::{row_to_recovery_channel, row_to_recovery_verification};

fn hash_token(token: &str) -> String {
    let hash = Sha256::digest(token.as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

impl Storage {
    pub fn get_entry_body(&self, universe_key: &str, path: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT body FROM entries WHERE universe_key = ?1 AND path = ?2",
                params![universe_key, path],
                |row| row.get::<_, String>(0),
            )
            .ok()
    }

    /// Persist the markdown body of an entry (CRDT idle / last-disconnect save).
    pub fn update_entry_body(
        &self,
        universe_key: &str,
        path: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        // Simple hash: polynomial rolling hash — used only for change detection.
        let hash = {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in body.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            format!("{h:016x}")
        };
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE entries SET body = ?1, body_hash = ?2, updated_at = ?3 \
             WHERE universe_key = ?4 AND path = ?5",
            params![body, hash, now, universe_key, path],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // API tokens (CO-35 — Vault REST API)
    // -------------------------------------------------------------------------

    /// Create a new long-lived API token (90 days) for the given user.
    /// The raw token is returned once in `ApiToken.token`; only its SHA-256
    /// hash is persisted — the plaintext is never written to the database.
    pub fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
    ) -> anyhow::Result<crate::vault_routes::ApiToken> {
        let id = nanoid::nanoid!(21);
        let raw_token = format!("co_{}", nanoid::nanoid!(40));
        let token_hash = hash_token(&raw_token);
        let token_prefix: String = raw_token.chars().take(11).collect(); // "co_" + 8 chars
        let now = Utc::now();
        let expires_at = now + chrono::Duration::days(90);
        let now_str = now.to_rfc3339();
        let exp_str = expires_at.to_rfc3339();
        // `token` column retains a NOT NULL constraint from migration v15.
        // We store an empty placeholder so the constraint is satisfied while
        // ensuring no plaintext token lives in the DB (the real secret is
        // in `token_hash`). The UNIQUE index on `token` is satisfied because
        // the column value is unique per-row via the id-prefixed placeholder.
        let token_placeholder = format!("hashed:{id}");
        self.conn.execute(
            "INSERT INTO api_tokens \
             (id, user_id, name, token, token_hash, token_prefix, created_at, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                user_id,
                name,
                token_placeholder,
                token_hash,
                token_prefix,
                now_str,
                exp_str
            ],
        )?;
        Ok(crate::vault_routes::ApiToken {
            id,
            user_id: user_id.to_string(),
            name: name.to_string(),
            token: Some(raw_token), // returned once; not persisted
            token_hash,
            token_prefix,
            created_at: now,
            expires_at,
            last_used_at: None,
        })
    }

    /// List API tokens for a user (raw token never returned).
    pub fn list_api_tokens(
        &self,
        user_id: &str,
    ) -> anyhow::Result<Vec<crate::vault_routes::ApiToken>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_id, name, token_hash, token_prefix, created_at, expires_at, last_used_at \
             FROM api_tokens WHERE user_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;
        let mut tokens = vec![];
        for row in rows.filter_map(|r| r.ok()) {
            let (id, uid, name, token_hash, token_prefix, created_str, expires_str, last_used_str) =
                row;
            let created_at = created_str
                .parse::<chrono::DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now());
            let expires_at = expires_str
                .parse::<chrono::DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now());
            let last_used_at = last_used_str
                .as_deref()
                .and_then(|s| s.parse::<chrono::DateTime<Utc>>().ok());
            tokens.push(crate::vault_routes::ApiToken {
                id,
                user_id: uid,
                name,
                token: None,
                token_hash,
                token_prefix,
                created_at,
                expires_at,
                last_used_at,
            });
        }
        Ok(tokens)
    }

    /// Revoke an API token by id. Returns true if deleted.
    pub fn delete_api_token(&self, id: &str, user_id: &str) -> anyhow::Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM api_tokens WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    /// Look up a token by value; check expiry. Updates `last_used_at`.
    /// The incoming raw token is hashed with SHA-256 before the DB lookup —
    /// no plaintext token is ever compared directly against stored data.
    pub fn get_api_token_by_value(
        &self,
        token: &str,
    ) -> anyhow::Result<Option<crate::vault_routes::ApiToken>> {
        let incoming_hash = hash_token(token);
        let result = self.conn.query_row(
            "SELECT id, user_id, name, token_hash, token_prefix, created_at, expires_at, last_used_at \
             FROM api_tokens WHERE token_hash = ?1",
            params![incoming_hash],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        );
        match result {
            Ok((
                id,
                uid,
                name,
                token_hash,
                token_prefix,
                created_str,
                expires_str,
                last_used_str,
            )) => {
                let created_at = created_str
                    .parse::<chrono::DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now());
                let expires_at = expires_str
                    .parse::<chrono::DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now());
                if expires_at < Utc::now() {
                    return Ok(None); // expired
                }
                let last_used_at = last_used_str
                    .as_deref()
                    .and_then(|s| s.parse::<chrono::DateTime<Utc>>().ok());
                let _ = self.conn.execute(
                    "UPDATE api_tokens SET last_used_at = ?1 WHERE id = ?2",
                    params![Utc::now().to_rfc3339(), id],
                );
                Ok(Some(crate::vault_routes::ApiToken {
                    id,
                    user_id: uid,
                    name,
                    token: None,
                    token_hash,
                    token_prefix,
                    created_at,
                    expires_at,
                    last_used_at,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // -------------------------------------------------------------------------
    // CO-165: Recovery channel storage methods
    // -------------------------------------------------------------------------

    pub fn create_recovery_channel(
        &self,
        user_id: &str,
        channel_type: &str,
        ciphertext: Vec<u8>,
        nonce: [u8; 12],
        lookup_hash: &str,
    ) -> anyhow::Result<String> {
        let id = format!("rc_{}", nanoid::nanoid!(10));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO user_recovery_channels \
             (id, user_id, channel_type, value_ciphertext, value_nonce, value_lookup_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, user_id, channel_type, ciphertext, nonce.as_slice(), lookup_hash, now],
        )?;
        Ok(id)
    }

    pub fn get_recovery_channel(&self, id: &str) -> Option<crate::models::RecoveryChannel> {
        self.conn
            .query_row(
                "SELECT id, user_id, channel_type, value_ciphertext, value_nonce, \
                 value_lookup_hash, verified_at, created_at, last_used_at, lockout_until \
                 FROM user_recovery_channels WHERE id = ?1",
                params![id],
                row_to_recovery_channel,
            )
            .ok()
    }

    pub fn get_recovery_channels_for_user(
        &self,
        user_id: &str,
    ) -> Vec<crate::models::RecoveryChannel> {
        let mut stmt = match self.conn.prepare(
            "SELECT id, user_id, channel_type, value_ciphertext, value_nonce, \
             value_lookup_hash, verified_at, created_at, last_used_at, lockout_until \
             FROM user_recovery_channels WHERE user_id = ?1 ORDER BY created_at",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![user_id], row_to_recovery_channel)
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Ensure `user_id` has a verified email recovery channel for `email`.
    ///
    /// If a channel with the same `(channel_type='email', lookup_hash)` already
    /// exists for this user, it's left untouched (and verified if it wasn't).
    /// Otherwise a fresh row is inserted with `verified_at = now()`.
    ///
    /// "Verified by virtue of being how the user signed in" — the email on
    /// `users.email` was either typed at signup or set by the admin seed; we
    /// trust it the same way `password-login` already does. This means
    /// `forgot-password` works for any user with a `users.email` set, no
    /// additional add-channel dance required.
    pub fn ensure_email_recovery_channel(&self, user_id: &str, email: &str) -> anyhow::Result<()> {
        let normalized = crate::recovery_crypto::normalize_channel_value("email", email);
        if normalized.is_empty() {
            return Ok(());
        }
        let lookup_hash = crate::recovery_crypto::compute_lookup_hash(&normalized);
        let now = Utc::now().to_rfc3339();

        // Existing row for this (user, email)? Just bring it up to verified.
        let existing_id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM user_recovery_channels \
                 WHERE user_id = ?1 AND channel_type = 'email' AND value_lookup_hash = ?2",
                params![user_id, lookup_hash],
                |row| row.get(0),
            )
            .ok();

        if let Some(id) = existing_id {
            self.conn.execute(
                "UPDATE user_recovery_channels SET verified_at = COALESCE(verified_at, ?1) WHERE id = ?2",
                params![now, id],
            )?;
            return Ok(());
        }

        let (ciphertext, nonce) =
            crate::recovery_crypto::encrypt_channel_value(normalized.as_bytes())
                .map_err(|e| anyhow::anyhow!("encrypt email for recovery channel: {e}"))?;
        let id = format!("rc_{}", nanoid::nanoid!(10));
        self.conn.execute(
            "INSERT INTO user_recovery_channels \
             (id, user_id, channel_type, value_ciphertext, value_nonce, value_lookup_hash, \
              verified_at, created_at) \
             VALUES (?1, ?2, 'email', ?3, ?4, ?5, ?6, ?6)",
            params![id, user_id, ciphertext, nonce.as_slice(), lookup_hash, now],
        )?;
        Ok(())
    }

    /// For every user with a non-empty `users.email`, make sure they have a
    /// verified email recovery channel for that address. Runs on every boot;
    /// each per-user call is idempotent (see `ensure_email_recovery_channel`).
    pub fn backfill_email_recovery_channels(&self) -> usize {
        let mut stmt = match self
            .conn
            .prepare("SELECT id, email FROM users WHERE email IS NOT NULL AND email <> ''")
        {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();

        let mut count = 0usize;
        for (user_id, email) in rows {
            if let Err(e) = self.ensure_email_recovery_channel(&user_id, &email) {
                tracing::warn!(
                    user_id = %user_id,
                    "ensure_email_recovery_channel failed: {e}"
                );
                continue;
            }
            count += 1;
        }
        count
    }

    pub fn find_verified_channel_by_lookup_hash(
        &self,
        channel_type: &str,
        lookup_hash: &str,
    ) -> Option<crate::models::RecoveryChannel> {
        self.conn
            .query_row(
                "SELECT id, user_id, channel_type, value_ciphertext, value_nonce, \
                 value_lookup_hash, verified_at, created_at, last_used_at, lockout_until \
                 FROM user_recovery_channels \
                 WHERE channel_type = ?1 AND value_lookup_hash = ?2 AND verified_at IS NOT NULL",
                params![channel_type, lookup_hash],
                row_to_recovery_channel,
            )
            .ok()
    }

    pub fn verify_recovery_channel(
        &self,
        channel_id: &str,
        verified_at: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE user_recovery_channels SET verified_at = ?1 WHERE id = ?2",
            params![verified_at, channel_id],
        )?;
        Ok(())
    }

    pub fn delete_recovery_channel(&self, channel_id: &str, user_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM user_recovery_channels WHERE id = ?1 AND user_id = ?2",
            params![channel_id, user_id],
        )?;
        Ok(())
    }

    pub fn set_channel_lockout(&self, channel_id: &str, lockout_until: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE user_recovery_channels SET lockout_until = ?1 WHERE id = ?2",
            params![lockout_until, channel_id],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // CO-165: Recovery verification storage methods
    // -------------------------------------------------------------------------

    pub fn create_recovery_verification(
        &self,
        channel_id: &str,
        user_id: &str,
        purpose: &str,
        code_hash: &str,
        expires_at: &str,
    ) -> anyhow::Result<String> {
        let id = format!("rv_{}", nanoid::nanoid!(10));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO recovery_verifications \
             (id, channel_id, user_id, purpose, code_hash, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, channel_id, user_id, purpose, code_hash, expires_at, now],
        )?;
        Ok(id)
    }

    pub fn get_active_verification(
        &self,
        channel_id: &str,
        purpose: &str,
    ) -> Option<crate::models::RecoveryVerification> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .query_row(
                "SELECT id, channel_id, user_id, purpose, code_hash, expires_at, \
                 consumed_at, attempts, created_at \
                 FROM recovery_verifications \
                 WHERE channel_id = ?1 AND purpose = ?2 AND consumed_at IS NULL \
                 AND expires_at > ?3 \
                 ORDER BY created_at DESC LIMIT 1",
                params![channel_id, purpose, now],
                row_to_recovery_verification,
            )
            .ok()
    }

    pub fn increment_verification_attempts(&self, id: &str) -> anyhow::Result<i64> {
        self.conn.execute(
            "UPDATE recovery_verifications SET attempts = attempts + 1 WHERE id = ?1",
            params![id],
        )?;
        let count: i64 = self.conn.query_row(
            "SELECT attempts FROM recovery_verifications WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn consume_verification(&self, id: &str) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE recovery_verifications SET consumed_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    /// Expire a verification by setting expires_at to the past.
    pub fn expire_verification(&self, id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE recovery_verifications SET expires_at = '2000-01-01T00:00:00Z' WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn count_recent_verifications_for_channel(&self, channel_id: &str, since: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM recovery_verifications \
                 WHERE channel_id = ?1 AND created_at >= ?2",
                params![channel_id, since],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    // -------------------------------------------------------------------------
    // CO-165: Password reset token storage methods
    // -------------------------------------------------------------------------

    pub fn create_reset_token(
        &self,
        token_hash: &str,
        user_id: &str,
        channel_id: &str,
        expires_at: &str,
    ) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO password_reset_tokens \
             (token_hash, user_id, channel_id, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![token_hash, user_id, channel_id, expires_at, now],
        )?;
        Ok(())
    }

    pub fn get_reset_token(&self, token_hash: &str) -> Option<crate::models::PasswordResetToken> {
        self.conn
            .query_row(
                "SELECT token_hash, user_id, channel_id, expires_at, consumed_at, created_at \
                 FROM password_reset_tokens WHERE token_hash = ?1",
                params![token_hash],
                |row| {
                    Ok(crate::models::PasswordResetToken {
                        token_hash: row.get(0)?,
                        user_id: row.get(1)?,
                        channel_id: row.get(2)?,
                        expires_at: row.get(3)?,
                        consumed_at: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .ok()
    }

    pub fn consume_reset_token(&self, token_hash: &str) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE password_reset_tokens SET consumed_at = ?1 WHERE token_hash = ?2",
            params![now, token_hash],
        )?;
        Ok(())
    }
}
