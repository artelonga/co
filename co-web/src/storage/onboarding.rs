use chrono::Utc;
use rusqlite::params;

use super::Storage;

#[derive(Debug, Clone)]
pub struct OnboardingCode {
    pub id: String,
    pub email_lookup_hash: String,
    pub intent: String,
    pub code_hash: String,
    pub preferred_usuario: Option<String>,
    pub return_to: Option<String>,
    pub expires_at: String,
    pub consumed_at: Option<String>,
    pub attempts: i64,
    pub created_at: String,
}

impl Storage {
    pub fn create_onboarding_code(
        &self,
        email_lookup_hash: &str,
        intent: &str,
        code_hash: &str,
        preferred_usuario: Option<&str>,
        return_to: Option<&str>,
        expires_at: &str,
    ) -> anyhow::Result<()> {
        let id = format!("ob_{}", nanoid::nanoid!(16));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO onboarding_codes \
             (id, email_lookup_hash, intent, code_hash, preferred_usuario, return_to, \
              expires_at, consumed_at, attempts, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 0, ?8)",
            params![
                id,
                email_lookup_hash,
                intent,
                code_hash,
                preferred_usuario,
                return_to,
                expires_at,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn get_onboarding_code(&self, email_lookup_hash: &str) -> Option<OnboardingCode> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .query_row(
                "SELECT id, email_lookup_hash, intent, code_hash, preferred_usuario, return_to, \
                 expires_at, consumed_at, attempts, created_at \
                 FROM onboarding_codes \
                 WHERE email_lookup_hash = ?1 \
                   AND consumed_at IS NULL \
                   AND expires_at > ?2 \
                 ORDER BY created_at DESC \
                 LIMIT 1",
                params![email_lookup_hash, now],
                |row| {
                    Ok(OnboardingCode {
                        id: row.get(0)?,
                        email_lookup_hash: row.get(1)?,
                        intent: row.get(2)?,
                        code_hash: row.get(3)?,
                        preferred_usuario: row.get(4)?,
                        return_to: row.get(5)?,
                        expires_at: row.get(6)?,
                        consumed_at: row.get(7)?,
                        attempts: row.get(8)?,
                        created_at: row.get(9)?,
                    })
                },
            )
            .ok()
    }

    pub fn consume_onboarding_code(&self, id: &str) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE onboarding_codes SET consumed_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn increment_onboarding_attempts(&self, id: &str) -> anyhow::Result<i64> {
        self.conn.execute(
            "UPDATE onboarding_codes SET attempts = attempts + 1 WHERE id = ?1",
            params![id],
        )?;
        self.conn
            .query_row(
                "SELECT attempts FROM onboarding_codes WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn cleanup_expired_onboarding_codes(&self) -> usize {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "DELETE FROM onboarding_codes WHERE expires_at <= ?1",
                params![now],
            )
            .unwrap_or(0)
    }

    pub fn count_onboarding_codes_for_email(&self, email_lookup_hash: &str, since: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM onboarding_codes \
                 WHERE email_lookup_hash = ?1 AND created_at >= ?2",
                params![email_lookup_hash, since],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    pub fn count_onboarding_codes_for_ip(&self, ip_key: &str, since: &str) -> i64 {
        // IP key is stored in preferred_usuario column when intent='ip_rate_limit_key'
        // but we actually track IP rates via a dedicated column approach —
        // use the code table's `id` prefix encoded with the IP hash, or use the
        // existing rate_limit infra. Since we only have the email-hash column
        // available in this table, we use the Storage's rate_limit helpers here.
        // Actually, for simplicity, we store IP counts separately using the
        // recovery_verifications approach: look at recent onboarding_codes created
        // from this "ip". We store the client IP hash as a dedicated column would
        // require a schema change. Let's use the existing approach: we'll track
        // IP rate limits via a separate key in the same onboarding_codes table
        // by using a special `email_lookup_hash` prefix `ip:`.
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM onboarding_codes \
                 WHERE email_lookup_hash = ?1 AND created_at >= ?2",
                params![ip_key, since],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Insert a sentinel row to track IP-based rate limits using the same
    /// onboarding_codes table. The `email_lookup_hash` stores the ip-scoped key.
    pub fn record_ip_onboarding_request(&self, ip_key: &str) -> anyhow::Result<()> {
        let id = format!("ob_ip_{}", nanoid::nanoid!(12));
        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        self.conn.execute(
            "INSERT INTO onboarding_codes \
             (id, email_lookup_hash, intent, code_hash, preferred_usuario, return_to, \
              expires_at, consumed_at, attempts, created_at) \
             VALUES (?1, ?2, 'ip_tracker', '', NULL, NULL, ?3, NULL, 0, ?4)",
            params![id, ip_key, expires_at, now],
        )?;
        Ok(())
    }
}

/// Derive a `usuario` from an email local-part, lowercasing, replacing
/// non-alphanumeric chars with `-`, and deduplicating via a `-N` suffix chain.
pub fn derive_usuario_from_email(email: &str, storage: &Storage) -> String {
    let local = email.split('@').next().unwrap_or("user");
    let base: String = local
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() {
        "user".to_string()
    } else {
        base
    };

    // Truncate to 30 chars max (users.usuario constraint).
    let base = if base.len() > 28 {
        base[..28].to_string()
    } else {
        base
    };

    if storage.get_user_by_usuario(&base).is_none() {
        return base;
    }

    for n in 2..=100 {
        let candidate = format!("{base}-{n}");
        if storage.get_user_by_usuario(&candidate).is_none() {
            return candidate;
        }
    }

    // Last resort: nanoid suffix
    format!("{base}-{}", nanoid::nanoid!(6))
}
