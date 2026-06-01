//! CO-334: Storage layer for cross-repo release notes aggregation.

use rusqlite::params;

use crate::error::AppError;

use super::Storage;

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReleaseNoteRow {
    pub repo: String,
    pub version: String,
    pub date: String, // ISO date "YYYY-MM-DD"
    pub theme: Option<String>,
    pub body_md: String,
    pub body_text: String, // markdown-stripped, for FTS later
    pub ingested_at: i64,  // unix seconds
}

#[derive(Debug, Clone)]
pub struct RepoSummary {
    pub repo: String,
    pub latest_version: String,
    pub latest_date: String,
}

// ---------------------------------------------------------------------------
// Storage methods
// ---------------------------------------------------------------------------

impl Storage {
    /// Upsert a batch of release notes. Idempotent: (repo, version) is the unique key.
    pub fn upsert_release_notes_batch(&self, rows: &[ReleaseNoteRow]) -> Result<usize, AppError> {
        for row in rows {
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO release_notes \
                     (repo, version, date, theme, body_md, body_text, ingested_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        row.repo,
                        row.version,
                        row.date,
                        row.theme,
                        row.body_md,
                        row.body_text,
                        row.ingested_at,
                    ],
                )
                .map_err(|e| AppError::Internal(format!("release_notes upsert: {e}")))?;
        }
        Ok(rows.len())
    }

    /// Interleaved release feed, newest date first. Optional repo/since filters.
    pub fn query_release_feed(
        &self,
        repo: Option<&str>,
        since: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Vec<ReleaseNoteRow> {
        let offset = page.saturating_sub(1) * per_page;
        let base = "SELECT repo, version, date, theme, body_md, body_text, ingested_at \
                    FROM release_notes";
        let tail = format!("ORDER BY date DESC, repo ASC LIMIT {per_page} OFFSET {offset}");

        fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReleaseNoteRow> {
            Ok(ReleaseNoteRow {
                repo: row.get(0)?,
                version: row.get(1)?,
                date: row.get(2)?,
                theme: row.get(3)?,
                body_md: row.get(4)?,
                body_text: row.get(5)?,
                ingested_at: row.get(6)?,
            })
        }

        let result = match (repo, since) {
            (None, None) => {
                let sql = format!("{base} {tail}");
                self.conn.prepare(&sql).and_then(|mut s| {
                    s.query_map([], map_row)
                        .map(|r| r.filter_map(|x| x.ok()).collect())
                })
            }
            (Some(r), None) => {
                let sql = format!("{base} WHERE repo = ?1 {tail}");
                self.conn.prepare(&sql).and_then(|mut s| {
                    s.query_map(params![r], map_row)
                        .map(|r| r.filter_map(|x| x.ok()).collect())
                })
            }
            (None, Some(since)) => {
                let sql = format!("{base} WHERE date >= ?1 {tail}");
                self.conn.prepare(&sql).and_then(|mut stmt| {
                    stmt.query_map(params![since], map_row)
                        .map(|r| r.filter_map(|x| x.ok()).collect())
                })
            }
            (Some(r), Some(since)) => {
                let sql = format!("{base} WHERE repo = ?1 AND date >= ?2 {tail}");
                self.conn.prepare(&sql).and_then(|mut stmt| {
                    stmt.query_map(params![r, since], map_row)
                        .map(|r| r.filter_map(|x| x.ok()).collect())
                })
            }
        };

        result.unwrap_or_default()
    }

    /// One row per repo: the latest release (by date, then any version on that date).
    pub fn query_release_repos(&self) -> Vec<RepoSummary> {
        let sql = "SELECT r.repo, r.version, r.date \
                   FROM release_notes r \
                   WHERE r.date = (SELECT MAX(r2.date) FROM release_notes r2 WHERE r2.repo = r.repo) \
                   GROUP BY r.repo \
                   ORDER BY r.date DESC";

        let result = self.conn.prepare(sql).and_then(|mut s| {
            s.query_map([], |row| {
                Ok(RepoSummary {
                    repo: row.get(0)?,
                    latest_version: row.get(1)?,
                    latest_date: row.get(2)?,
                })
            })
            .map(|r| r.filter_map(|x| x.ok()).collect())
        });

        result.unwrap_or_default()
    }

    /// All universes that have a local_repo_path set. Used by the refresh worker.
    pub fn query_universe_repo_paths(&self) -> Vec<(String, String)> {
        let sql = "SELECT key, local_repo_path FROM universes WHERE local_repo_path IS NOT NULL";

        let result = self.conn.prepare(sql).and_then(|mut s| {
            s.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map(|r| r.filter_map(|x| x.ok()).collect())
        });

        result.unwrap_or_default()
    }
}
