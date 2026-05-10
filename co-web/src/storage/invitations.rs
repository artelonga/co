use chrono::Utc;
use rusqlite::params;

use super::Storage;
use super::schema::parse_datetime;

/// A universe invitation row.
#[derive(Debug, Clone)]
pub struct Invitation {
    pub token_hash: String,
    pub universe_key: String,
    pub invited_by: String,
    pub invited_email: Option<String>,
    pub invited_user_id: Option<String>,
    pub role: String,
    pub expires_at: chrono::DateTime<Utc>,
    pub consumed_at: Option<chrono::DateTime<Utc>>,
    pub revoked_at: Option<chrono::DateTime<Utc>>,
    pub created_at: chrono::DateTime<Utc>,
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(input.as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

fn row_to_invitation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Invitation> {
    Ok(Invitation {
        token_hash: row.get(0)?,
        universe_key: row.get(1)?,
        invited_by: row.get(2)?,
        invited_email: row.get(3)?,
        invited_user_id: row.get(4)?,
        role: row.get(5)?,
        expires_at: parse_datetime(&row.get::<_, String>(6)?),
        consumed_at: row
            .get::<_, Option<String>>(7)?
            .as_deref()
            .map(parse_datetime),
        revoked_at: row
            .get::<_, Option<String>>(8)?
            .as_deref()
            .map(parse_datetime),
        created_at: parse_datetime(&row.get::<_, String>(9)?),
    })
}

const SELECT_COLS: &str = "token_hash, universe_key, invited_by, invited_email, invited_user_id, \
     role, expires_at, consumed_at, revoked_at, created_at";

impl Storage {
    /// Mint a new single-use invitation token. Returns `(token_hash, raw_token)`.
    /// The raw token must only be sent to the recipient — never returned to the API caller.
    pub fn create_invitation(
        &self,
        universe_key: &str,
        invited_by: &str,
        invited_email: Option<&str>,
        invited_user_id: Option<&str>,
        role: &str,
    ) -> anyhow::Result<(String, String)> {
        if invited_email.is_none() && invited_user_id.is_none() {
            anyhow::bail!("at least one of invited_email or invited_user_id is required");
        }
        let raw_token = nanoid::nanoid!(32);
        let token_hash = sha256_hex(&raw_token);
        let now = Utc::now();
        let expires_at = (now + chrono::Duration::days(14)).to_rfc3339();
        self.conn.execute(
            "INSERT INTO universe_invitations \
             (token_hash, universe_key, invited_by, invited_email, invited_user_id, role, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                token_hash,
                universe_key,
                invited_by,
                invited_email,
                invited_user_id,
                role,
                expires_at,
                now.to_rfc3339()
            ],
        )?;
        Ok((token_hash, raw_token))
    }

    pub fn get_invitation_by_token(&self, token_hash: &str) -> Option<Invitation> {
        self.conn
            .query_row(
                &format!("SELECT {SELECT_COLS} FROM universe_invitations WHERE token_hash = ?1"),
                params![token_hash],
                row_to_invitation,
            )
            .ok()
    }

    pub fn consume_invitation(&self, token_hash: &str) -> anyhow::Result<()> {
        let now_str = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE universe_invitations SET consumed_at = ?1 WHERE token_hash = ?2",
            params![now_str, token_hash],
        )?;
        Ok(())
    }

    pub fn list_invitations_for_universe(&self, universe_key: &str) -> Vec<Invitation> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {SELECT_COLS} FROM universe_invitations \
                 WHERE universe_key = ?1 ORDER BY created_at DESC"
            ))
            .expect("prepare list_invitations_for_universe");
        stmt.query_map(params![universe_key], row_to_invitation)
            .expect("list_invitations_for_universe query")
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn list_invitations_for_email(&self, email: &str) -> Vec<Invitation> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {SELECT_COLS} FROM universe_invitations \
                 WHERE LOWER(invited_email) = LOWER(?1) ORDER BY created_at DESC"
            ))
            .expect("prepare list_invitations_for_email");
        stmt.query_map(params![email], row_to_invitation)
            .expect("list_invitations_for_email query")
            .filter_map(|r| r.ok())
            .collect()
    }

    /// List pending invitations for a user matched by user_id OR email.
    /// Returns non-consumed, non-revoked, non-expired invitations only.
    pub fn list_invitations_for_me(&self, user_id: &str, email: &str) -> Vec<Invitation> {
        let now = Utc::now().to_rfc3339();
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {SELECT_COLS} FROM universe_invitations \
                 WHERE (invited_user_id = ?1 OR LOWER(invited_email) = LOWER(?2)) \
                 AND consumed_at IS NULL AND revoked_at IS NULL AND expires_at > ?3 \
                 ORDER BY created_at DESC"
            ))
            .expect("prepare list_invitations_for_me");
        stmt.query_map(params![user_id, email, now], row_to_invitation)
            .expect("list_invitations_for_me query")
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Hash a raw token the same way create_invitation does — for lookups from URL paths.
    pub fn hash_invitation_token(raw_token: &str) -> String {
        sha256_hex(raw_token)
    }
}
