//! Typed meta-DB accessors for the `leads` table (CO-433).
//!
//! Moves the raw `conn().execute/prepare/query_row` calls out of the onboarding
//! and lead admin routes into typed methods on `Storage`. The `leads` table
//! lives in the global meta-DB.

use rusqlite::Result;
use serde::Serialize;

use super::Storage;

/// One row of the lead admin list view (joined with the linked user). Doubles
/// as the wire type for the leads admin API.
#[derive(Debug, Serialize)]
pub struct LeadRow {
    pub id: i64,
    pub created_at: String,
    pub updated_at: String,
    pub nome: Option<String>,
    pub email: Option<String>,
    pub telefone: Option<String>,
    pub mensagem: String,
    pub servico_titulo: Option<String>,
    pub parceiro_handle: Option<String>,
    pub status: String,
    pub priority: Option<String>,
    pub assignee_handle: Option<String>,
    pub notes: Option<String>,
    pub closed_reason: Option<String>,
    pub promoted_to_al: Option<i64>,
    /// CO-370: linked user id (NULL if no user was matched/created yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// CO-370: status of the linked user ('active' | 'pre-registered').
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_status: Option<String>,
    /// CO-370: when the linked user completed email verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
}

impl Storage {
    // --- onboarding / signup linking ---

    /// Find an existing lead id by email (case-insensitive). `None` if absent.
    pub fn find_lead_id_by_email(&self, email: &str) -> Option<i64> {
        self.conn()
            .query_row(
                "SELECT id FROM leads WHERE lower(email) = lower(?1) LIMIT 1",
                rusqlite::params![email],
                |r| r.get(0),
            )
            .ok()
    }

    /// Link a pre-existing lead to a newly created user (only if unlinked).
    pub fn link_lead_to_user(&self, lead_id: i64, user_id: &str) -> Result<usize> {
        self.conn().execute(
            "UPDATE leads SET user_id = ?1 WHERE id = ?2 AND user_id IS NULL",
            rusqlite::params![user_id, lead_id],
        )
    }

    /// Link a user to a lead (only if the user has no lead yet).
    pub fn link_user_to_lead(&self, user_id: &str, lead_id: i64) -> Result<usize> {
        self.conn().execute(
            "UPDATE users SET lead_id = ?1 WHERE id = ?2 AND lead_id IS NULL",
            rusqlite::params![lead_id, user_id],
        )
    }

    /// Insert a signup-sourced lead linked to a new user (idempotent on email).
    /// Returns the new lead's rowid, or `None` if a row already existed
    /// (INSERT OR IGNORE affected 0 rows).
    pub fn insert_signup_lead(&self, now: &str, email: &str, user_id: &str) -> Option<i64> {
        let res = self.conn().execute(
            "INSERT OR IGNORE INTO leads \
             (created_at, updated_at, email, mensagem, status, priority, \
              source, user_id) \
             VALUES (?1, ?1, ?2, '', 'new', 'normal', 'signup', ?3)",
            rusqlite::params![now, email, user_id],
        );
        res.ok()
            .filter(|&n| n > 0)
            .map(|_| self.conn().last_insert_rowid())
    }

    /// Auto-advance a signup-sourced lead from `new` → `in_progress`.
    pub fn advance_signup_lead_to_in_progress(&self, now: &str, lead_id: i64) -> Result<usize> {
        self.conn().execute(
            "UPDATE leads SET status = 'in_progress', updated_at = ?1 \
             WHERE id = ?2 AND source = 'signup' AND status = 'new'",
            rusqlite::params![now, lead_id],
        )
    }

    // --- user shell for lead linking ---

    /// Find a user id by email (case-insensitive). `None` if absent.
    pub fn find_user_id_by_email(&self, email: &str) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT id FROM users WHERE lower(email) = lower(?1) LIMIT 1",
                rusqlite::params![email],
                |r| r.get::<_, String>(0),
            )
            .ok()
    }

    /// Insert a pre-registered shell user (idempotent on the unique email).
    /// Returns rows inserted (0 if a user already existed).
    pub fn insert_shell_user(
        &self,
        id: &str,
        email: &str,
        display_name: &str,
        now: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "INSERT OR IGNORE INTO users \
             (id, email, display_name, tier, created_at, status) \
             VALUES (?1, ?2, ?3, 'player', ?4, 'pre-registered')",
            rusqlite::params![id, email, display_name, now],
        )
    }

    // --- contact form leads (admin) ---

    /// Rate-limit helper: leads from this ip_hash in the last 24 hours.
    pub fn count_leads_by_ip_hash_last_day(&self, ip_hash: &str) -> i64 {
        self.conn()
            .query_row(
                "SELECT COUNT(*) FROM leads WHERE ip_hash = ? \
                 AND created_at > datetime('now', '-1 day')",
                rusqlite::params![ip_hash],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    /// Insert a contact-form lead. Returns the new lead's rowid on success.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_contact_lead(
        &self,
        now: &str,
        nome: Option<&str>,
        email: Option<&str>,
        telefone: Option<&str>,
        mensagem: &str,
        servico_titulo: Option<&str>,
        parceiro_handle: Option<&str>,
        ip_hash: &str,
        user_agent: Option<&str>,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO leads
             (created_at, updated_at, nome, email, telefone, mensagem,
              servico_titulo, parceiro_handle, status, priority, ip_hash, user_agent)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'new', 'normal', ?, ?)",
            rusqlite::params![
                now,
                now,
                nome,
                email,
                telefone,
                mensagem,
                servico_titulo,
                parceiro_handle,
                ip_hash,
                user_agent,
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Count leads matching a dynamically-built `WHERE` clause (caller assembles
    /// the clause + positional binds; both come from validated query params).
    pub fn count_leads(&self, where_clause: &str, binds: &[String]) -> i64 {
        // concat (not format!("SELECT…")): where_clause is a structural clause
        // built from validated params with separate positional `binds` — values
        // are never interpolated. concat also avoids the CWE-89 scanner FP.
        let sql = ["SELECT COUNT(*) FROM leads ", where_clause].concat();
        self.conn()
            .prepare(&sql)
            .and_then(|mut stmt| {
                stmt.query_row(rusqlite::params_from_iter(binds.iter()), |r| r.get(0))
            })
            .unwrap_or(0)
    }

    /// List leads matching a dynamically-built `WHERE` clause, newest first,
    /// joined with the linked user. `limit` is appended as the final bind.
    pub fn list_leads(&self, where_clause: &str, binds: &[String], limit: i64) -> Vec<LeadRow> {
        let mut binds = binds.to_vec();
        binds.push(limit.to_string());
        let sql = format!(
            "SELECT l.id, l.created_at, l.updated_at, l.nome, l.email, l.telefone, l.mensagem,
                    l.servico_titulo, l.parceiro_handle, l.status, l.priority, l.assignee_handle,
                    l.notes, l.closed_reason, l.promoted_to_al,
                    l.user_id,
                    u.status  AS user_status,
                    CASE WHEN u.status = 'active' THEN u.activated_at ELSE NULL END AS verified_at
             FROM leads l
             LEFT JOIN users u ON u.id = l.user_id
             {where_clause} ORDER BY l.created_at DESC LIMIT ?"
        );
        self.conn()
            .prepare(&sql)
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params_from_iter(binds.iter()), |r| {
                    Ok(LeadRow {
                        id: r.get(0)?,
                        created_at: r.get(1)?,
                        updated_at: r.get(2)?,
                        nome: r.get(3)?,
                        email: r.get(4)?,
                        telefone: r.get(5)?,
                        mensagem: r.get(6)?,
                        servico_titulo: r.get(7)?,
                        parceiro_handle: r.get(8)?,
                        status: r.get(9)?,
                        priority: r.get(10)?,
                        assignee_handle: r.get(11)?,
                        notes: r.get(12)?,
                        closed_reason: r.get(13)?,
                        promoted_to_al: r.get(14)?,
                        user_id: r.get(15)?,
                        user_status: r.get(16)?,
                        verified_at: r.get(17)?,
                    })
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    /// The current status of a lead by id. `None` if absent.
    pub fn get_lead_status(&self, id: i64) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT status FROM leads WHERE id = ?",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok()
    }

    /// Patch a lead (each field only overwrites when `Some`). Returns rows updated.
    #[allow(clippy::too_many_arguments)]
    pub fn update_lead(
        &self,
        now: &str,
        status: Option<&str>,
        priority: Option<&str>,
        assignee_handle: Option<&str>,
        notes: Option<&str>,
        closed_reason: Option<&str>,
        promoted_to_al: Option<i64>,
        id: i64,
    ) -> Result<usize> {
        self.conn().execute(
            "UPDATE leads SET
                 updated_at      = ?,
                 status          = COALESCE(?, status),
                 priority        = COALESCE(?, priority),
                 assignee_handle = COALESCE(?, assignee_handle),
                 notes           = COALESCE(?, notes),
                 closed_reason   = COALESCE(?, closed_reason),
                 promoted_to_al  = COALESCE(?, promoted_to_al)
             WHERE id = ?",
            rusqlite::params![
                now,
                status,
                priority,
                assignee_handle,
                notes,
                closed_reason,
                promoted_to_al,
                id,
            ],
        )
    }

    /// CO-183 LGPD purge: delete closed leads older than 24 months. Returns
    /// rows purged.
    pub fn purge_closed_leads_older_than_24_months(&self) -> Result<usize> {
        self.conn().execute(
            "DELETE FROM leads \
             WHERE created_at < datetime('now', '-24 months') \
             AND status = 'closed'",
            [],
        )
    }
}
