//! Typed meta-DB accessors for the security subsystem (CO-433).
//!
//! Moves the raw `conn().prepare/execute/query_row` calls that used to live in
//! `security/routes.rs` and the security subscribers into typed methods on
//! `Storage`, so handlers and subscribers never speak SQL directly. All of
//! these operate on the global meta-DB (`security_findings`,
//! `security_scan_counts`, and the `entries` PBI rows), which is what the
//! global `Mutex<Storage>` legitimately guards.

use rusqlite::{Result, params};

use super::Storage;

/// One row from `security_findings` — the storage-layer projection. The route
/// layer maps this into its `FindingResponse` wire type.
#[derive(Debug, Clone)]
pub struct SecurityFindingRow {
    pub id: String,
    pub pr_number: i64,
    pub severity: String,
    pub category: String,
    pub file_path: String,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub description: String,
    pub cwe: Option<String>,
    pub cve_match: Option<String>,
    pub suggested_patch: Option<String>,
    pub detected_at: String,
    pub resolved_at: Option<String>,
    pub resolution_kind: Option<String>,
    pub resolution_pr: Option<i64>,
    pub resolved_by: Option<String>,
}

const FINDING_COLUMNS: &str = "id, pr_number, severity, category, file_path, \
     line_start, line_end, description, cwe, cve_match, \
     suggested_patch, detected_at, resolved_at, resolution_kind, \
     resolution_pr, resolved_by";

fn map_finding(row: &rusqlite::Row<'_>) -> Result<SecurityFindingRow> {
    Ok(SecurityFindingRow {
        id: row.get(0)?,
        pr_number: row.get(1)?,
        severity: row.get(2)?,
        category: row.get(3)?,
        file_path: row.get(4)?,
        line_start: row.get(5)?,
        line_end: row.get(6)?,
        description: row.get(7)?,
        cwe: row.get(8)?,
        cve_match: row.get(9)?,
        suggested_patch: row.get(10)?,
        detected_at: row.get(11)?,
        resolved_at: row.get(12)?,
        resolution_kind: row.get(13)?,
        resolution_pr: row.get(14)?,
        resolved_by: row.get(15)?,
    })
}

impl Storage {
    /// List security findings, filtered by optional severity and resolved flag,
    /// newest first. `limit`/`offset` are caller-clamped.
    pub fn list_security_findings(
        &self,
        severity: Option<&str>,
        resolved_flag: Option<i64>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SecurityFindingRow>> {
        let sql = format!(
            "SELECT {FINDING_COLUMNS} \
             FROM security_findings \
             WHERE (?1 IS NULL OR severity = ?1) \
               AND (?2 IS NULL \
                    OR (?2 = 1 AND resolved_at IS NOT NULL) \
                    OR (?2 = 0 AND resolved_at IS NULL)) \
             ORDER BY detected_at DESC LIMIT ?3 OFFSET ?4"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![severity, resolved_flag, limit, offset], map_finding)?;
        Ok(rows.flatten().collect())
    }

    /// Fetch a single finding by id. `Ok(None)` if absent.
    pub fn get_security_finding(&self, id: &str) -> Result<Option<SecurityFindingRow>> {
        let sql = format!("SELECT {FINDING_COLUMNS} FROM security_findings WHERE id = ?1");
        match self.conn().query_row(&sql, params![id], map_finding) {
            Ok(f) => Ok(Some(f)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Resolve a finding. Returns the number of rows updated (0 if not found or
    /// already resolved).
    pub fn resolve_security_finding(
        &self,
        id: &str,
        now: &str,
        resolution_kind: &str,
        resolution_pr: Option<i64>,
        resolved_by: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "UPDATE security_findings \
             SET resolved_at = ?1, resolution_kind = ?2, resolution_pr = ?3, resolved_by = ?4 \
             WHERE id = ?5 AND resolved_at IS NULL",
            params![now, resolution_kind, resolution_pr, resolved_by, id],
        )
    }

    /// Count unresolved findings.
    pub fn count_unresolved_findings(&self) -> i64 {
        self.conn()
            .query_row(
                "SELECT COUNT(*) FROM security_findings WHERE resolved_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    /// Count unresolved high/critical findings (the release-gate blockers).
    pub fn count_unresolved_high_critical_findings(&self) -> i64 {
        self.conn()
            .query_row(
                "SELECT COUNT(*) FROM security_findings \
                 WHERE resolved_at IS NULL AND severity IN ('high','critical')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    /// Atomically increment today's scan counter and return the new count.
    pub fn increment_security_scan_count(&self, scan_day: &str) -> Result<i64> {
        self.conn().query_row(
            "INSERT INTO security_scan_counts (scan_day, count) VALUES (?1, 1) \
             ON CONFLICT(scan_day) DO UPDATE SET count = count + 1 \
             RETURNING count",
            params![scan_day],
            |r| r.get(0),
        )
    }

    /// Persist a detected finding (idempotent on id).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_security_finding(
        &self,
        id: &str,
        pr_number: i64,
        severity: &str,
        category: &str,
        file_path: &str,
        line_start: Option<i64>,
        line_end: Option<i64>,
        description: &str,
        cwe: Option<&str>,
        cve_match: Option<&str>,
        suggested_patch: Option<&str>,
        detected_at: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "INSERT OR IGNORE INTO security_findings \
             (id, pr_number, severity, category, file_path, line_start, line_end, \
              description, cwe, cve_match, suggested_patch, detected_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                pr_number,
                severity,
                category,
                file_path,
                line_start,
                line_end,
                description,
                cwe,
                cve_match,
                suggested_patch,
                detected_at,
            ],
        )
    }

    /// Insert a security PBI as an `entries` row (idempotent on path). The
    /// `entries` table is keyed by (universe_key, path) and has no `id` column.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_security_pbi(
        &self,
        path: &str,
        universe_key: &str,
        title: &str,
        frontmatter_json: &str,
        body: &str,
        body_hash: &str,
        now: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "INSERT OR IGNORE INTO entries \
             (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
              created_at, updated_at) \
             VALUES (?1, ?2, 'pbi', ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                path,
                universe_key,
                title,
                frontmatter_json,
                body,
                body_hash,
                now,
            ],
        )
    }
}
