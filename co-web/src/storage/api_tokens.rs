use chrono::Utc;
use rusqlite::params;
use sha2::{Digest, Sha256};

use super::Storage;
use super::schema::{row_to_recovery_channel, row_to_recovery_verification};

fn hash_token(token: &str) -> String {
    let hash = Sha256::digest(token.as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// CO-448: parse the stored `api_tokens.scopes` JSON column.
///
/// `NULL` / absent ⇒ `None` (inherit owner tier). A malformed value is treated
/// as `None` too (fail-open to the legacy behavior) rather than locking the
/// token out — but well-formed JSON arrays round-trip exactly.
fn parse_scopes(raw: Option<String>) -> Option<Vec<String>> {
    let raw = raw?;
    serde_json::from_str::<Vec<String>>(&raw).ok()
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
    ///
    /// Leaves `scopes` NULL → the token inherits the owner's tier (pre-CO-448
    /// all-or-nothing behavior). For a least-privilege token, use
    /// [`create_api_token_with_scopes`](Self::create_api_token_with_scopes).
    pub fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
    ) -> anyhow::Result<crate::vault_routes::ApiToken> {
        self.create_api_token_inner(user_id, name, None)
    }

    /// CO-448: create a least-privilege API token whose access is restricted to
    /// the resolved capability set (bundles already expanded, persisted as a
    /// JSON array). A leaked secret then grants only this set, never owner-tier
    /// admin. An empty set is still a *declared* scope (grants nothing) — to get
    /// the inherit-tier behavior, use [`create_api_token`](Self::create_api_token).
    pub fn create_api_token_with_scopes(
        &self,
        user_id: &str,
        name: &str,
        scopes: &std::collections::BTreeSet<String>,
    ) -> anyhow::Result<crate::vault_routes::ApiToken> {
        let ordered: Vec<String> = scopes.iter().cloned().collect();
        self.create_api_token_inner(user_id, name, Some(ordered))
    }

    /// CO-401: idempotently register a **known-value**, capability-scoped API
    /// token (least-privilege, CO-448 scopes).
    ///
    /// Unlike [`create_api_token_with_scopes`](Self::create_api_token_with_scopes)
    /// — which mints a fresh random secret — the raw token value is supplied by
    /// the caller. This materializes the `CO_STAGING_ADMIN_TOKEN` secret (shared
    /// with CI) as a least-privilege `api_tokens` row at staging boot, so the
    /// staging suite authenticates with exactly the capabilities CO-374
    /// exercises and a leaked secret grants only that scope set — never
    /// owner-tier admin.
    ///
    /// Idempotent + rotation-safe: any prior token for this `user_id`/`name`
    /// whose hash differs (a rotated secret) is purged first, then the current
    /// secret is upserted (`token_hash` carries a partial UNIQUE index, so a
    /// re-run with the same secret is a no-op while a scope change is applied).
    pub fn seed_scoped_api_token(
        &self,
        user_id: &str,
        name: &str,
        raw_token: &str,
        scopes: &std::collections::BTreeSet<String>,
    ) -> anyhow::Result<()> {
        let token_hash = hash_token(raw_token);
        let token_prefix: String = raw_token.chars().take(11).collect();
        let now = Utc::now();
        let created = now.to_rfc3339();
        // Long-lived (1 year) — rotated via the documented runbook, not expiry.
        let expires = (now + chrono::Duration::days(365)).to_rfc3339();
        let ordered: Vec<String> = scopes.iter().cloned().collect();
        let scopes_json = serde_json::to_string(&ordered)?;
        let id = format!("seed-{}", &token_hash[..16.min(token_hash.len())]);
        let token_placeholder = format!("hashed:{id}");

        // Rotation: drop a previous seeded token for this owner+name when the
        // secret value changed (different hash), so only the live secret remains.
        self.conn.execute(
            "DELETE FROM api_tokens WHERE user_id = ?1 AND name = ?2 AND token_hash <> ?3",
            params![user_id, name, token_hash],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO api_tokens \
             (id, user_id, name, token, token_hash, token_prefix, created_at, expires_at, scopes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                user_id,
                name,
                token_placeholder,
                token_hash,
                token_prefix,
                created,
                expires,
                scopes_json
            ],
        )?;
        // Keep scopes/expiry current across redeploys (the suite's scope set may
        // widen as more surfaces adopt `Scoped<C>`).
        self.conn.execute(
            "UPDATE api_tokens SET user_id = ?1, name = ?2, scopes = ?3, expires_at = ?4 \
             WHERE token_hash = ?5",
            params![user_id, name, scopes_json, expires, token_hash],
        )?;
        Ok(())
    }

    fn create_api_token_inner(
        &self,
        user_id: &str,
        name: &str,
        scopes: Option<Vec<String>>,
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
        // CO-448: NULL scopes (legacy path) → inherit owner tier; a JSON array
        // (even empty) → least-privilege restriction to that capability set.
        let scopes_json = scopes.as_ref().map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "INSERT INTO api_tokens \
             (id, user_id, name, token, token_hash, token_prefix, created_at, expires_at, scopes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                user_id,
                name,
                token_placeholder,
                token_hash,
                token_prefix,
                now_str,
                exp_str,
                scopes_json
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
            scopes,
        })
    }

    /// List API tokens for a user (raw token never returned).
    pub fn list_api_tokens(
        &self,
        user_id: &str,
    ) -> anyhow::Result<Vec<crate::vault_routes::ApiToken>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_id, name, token_hash, token_prefix, created_at, expires_at, last_used_at, scopes \
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
                row.get::<_, Option<String>>(8)?,
            ))
        })?;
        let mut tokens = vec![];
        for row in rows.filter_map(|r| r.ok()) {
            let (
                id,
                uid,
                name,
                token_hash,
                token_prefix,
                created_str,
                expires_str,
                last_used_str,
                scopes_raw,
            ) = row;
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
                scopes: parse_scopes(scopes_raw),
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
            "SELECT id, user_id, name, token_hash, token_prefix, created_at, expires_at, last_used_at, scopes \
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
                    row.get::<_, Option<String>>(8)?,
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
                scopes_raw,
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
                    scopes: parse_scopes(scopes_raw),
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
        let lookup_hash = crate::recovery_crypto::compute_lookup_hash(
            &normalized,
            &crate::infra::secrets::EnvSecretsProvider,
        );
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

        let (ciphertext, nonce) = crate::recovery_crypto::encrypt_channel_value(
            normalized.as_bytes(),
            &crate::infra::secrets::EnvSecretsProvider,
        )
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

#[cfg(test)]
mod seed_scoped_token_tests {
    use super::Storage;
    use crate::auth::capabilities::{grants, resolve_scopes};

    /// CO-401: a seeded scoped token round-trips by its raw value, carries its
    /// least-privilege scope set, reaches admin surfaces it was granted (via the
    /// capability), and does NOT reach capabilities outside the scope.
    #[test]
    fn seeded_scoped_token_grants_only_its_scope() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let uid = storage.ensure_staging_admin_user().unwrap();

        // The exact least-privilege set the staging suite uses: admin *reads*
        // the suite exercises, but NOT chat:write / agent:dispatch / deployments.
        let scopes = resolve_scopes(&[
            "entries:read".into(),
            "entries:write".into(),
            "universes:read".into(),
            "universes:write".into(),
            "gestao:read".into(),
            "funnel:read".into(),
            "chat:read".into(),
            "telemetry:read".into(),
        ])
        .unwrap();

        let raw = "co_staging_suite_fixture_secret_value_123456";
        storage
            .seed_scoped_api_token(&uid, "staging-suite", raw, &scopes)
            .unwrap();

        let fetched = storage.get_api_token_by_value(raw).unwrap().expect("token");
        assert_eq!(fetched.user_id, uid);
        let resolved: std::collections::BTreeSet<String> = fetched
            .scopes
            .expect("scoped, not NULL")
            .into_iter()
            .collect();

        // In-scope admin surface reached via capability (chat:read is admin-surface).
        assert!(grants(&resolved, "chat:read"), "must reach chat:read");
        assert!(grants(&resolved, "funnel:read"), "must reach funnel:read");
        assert!(
            grants(&resolved, "entries:write"),
            "must reach entries:write"
        );

        // Out-of-scope: never granted, even though the owner is admin-tier.
        assert!(!grants(&resolved, "chat:write"), "chat:write out of scope");
        assert!(
            !grants(&resolved, "agent:dispatch"),
            "agent:dispatch out of scope"
        );
        assert!(
            !grants(&resolved, "deployments:read"),
            "deployments:read out of scope"
        );
    }

    /// CO-401: re-seeding the same secret is a no-op (one row); rotating to a new
    /// secret retires the old token and leaves exactly the new one.
    #[test]
    fn seed_scoped_token_is_idempotent_and_rotation_safe() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        let uid = storage.ensure_staging_admin_user().unwrap();
        let scopes = resolve_scopes(&["chat:read".into()]).unwrap();

        storage
            .seed_scoped_api_token(&uid, "staging-suite", "co_secret_one", &scopes)
            .unwrap();
        storage
            .seed_scoped_api_token(&uid, "staging-suite", "co_secret_one", &scopes)
            .unwrap();
        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM api_tokens WHERE name = 'staging-suite'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "idempotent: same secret → single row");

        // Rotate the secret.
        storage
            .seed_scoped_api_token(&uid, "staging-suite", "co_secret_two", &scopes)
            .unwrap();
        let count_after: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM api_tokens WHERE name = 'staging-suite'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_after, 1, "rotation retires the old secret");
        assert!(
            storage
                .get_api_token_by_value("co_secret_one")
                .unwrap()
                .is_none(),
            "old secret no longer valid after rotation"
        );
        assert!(
            storage
                .get_api_token_by_value("co_secret_two")
                .unwrap()
                .is_some(),
            "new secret valid after rotation"
        );
    }
}
