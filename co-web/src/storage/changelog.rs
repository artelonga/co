use rusqlite::params;

use crate::error::AppError;

use super::Storage;

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ChangelogCacheRow {
    pub version: String,
    pub ticket: String,
    pub entry_type: String,
    pub title: String,
    pub pr_number: Option<i64>,
    pub pr_size: Option<i64>,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub commit_sha: Option<String>,
    pub author: Option<String>,
}

// ---------------------------------------------------------------------------
// Storage methods
// ---------------------------------------------------------------------------

impl Storage {
    /// Load all rows from `changelog_cache` ordered by version desc.
    pub fn query_changelog_cache_all(&self) -> Vec<ChangelogCacheRow> {
        let mut stmt = match self.conn.prepare(
            "SELECT version, ticket, entry_type, title, pr_number, pr_size, \
             additions, deletions, commit_sha, author \
             FROM changelog_cache \
             ORDER BY version DESC, ticket ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        stmt.query_map([], |row| {
            Ok(ChangelogCacheRow {
                version: row.get(0)?,
                ticket: row.get(1)?,
                entry_type: row.get(2)?,
                title: row.get(3)?,
                pr_number: row.get(4)?,
                pr_size: row.get(5)?,
                additions: row.get(6)?,
                deletions: row.get(7)?,
                commit_sha: row.get(8)?,
                author: row.get(9)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Insert or replace rows into `changelog_cache`, returning the count upserted.
    pub fn upsert_changelog_cache_batch(
        &self,
        rows: &[ChangelogCacheRow],
    ) -> Result<usize, AppError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        for row in rows {
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO changelog_cache \
                     (version, ticket, entry_type, title, pr_number, pr_size, \
                      additions, deletions, commit_sha, author, indexed_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        row.version,
                        row.ticket,
                        row.entry_type,
                        row.title,
                        row.pr_number,
                        row.pr_size,
                        row.additions,
                        row.deletions,
                        row.commit_sha,
                        row.author,
                        now,
                    ],
                )
                .map_err(|e| AppError::Internal(format!("changelog cache upsert: {e}")))?;
        }
        Ok(rows.len())
    }
}
