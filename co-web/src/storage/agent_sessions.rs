//! CO-275: Agent session storage — one row per co-auto invocation.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::Storage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: i64,
    pub task_id: String,
    pub universe_key: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub duration_ms: i64,
    pub exit_code: i32,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    /// JSON object: `{"Read": 8, "Edit": 5, ...}`
    pub tool_calls: Option<String>,
    /// JSON array: `["spa-conventions", "rust-architecture"]`
    pub skills_loaded: Option<String>,
    pub context_chars: Option<i64>,
    pub final_commit_sha: Option<String>,
    pub pr_number: Option<i64>,
    pub model: Option<String>,
    pub co_auto_version: Option<String>,
    pub raw_log_blob_sha: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAgentSession {
    pub task_id: String,
    pub universe_key: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub duration_ms: i64,
    pub exit_code: i32,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub tool_calls: Option<String>,
    pub skills_loaded: Option<String>,
    pub context_chars: Option<i64>,
    pub final_commit_sha: Option<String>,
    pub pr_number: Option<i64>,
    pub model: Option<String>,
    pub co_auto_version: Option<String>,
    pub raw_log_blob_sha: Option<String>,
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSession> {
    Ok(AgentSession {
        id: row.get(0)?,
        task_id: row.get(1)?,
        universe_key: row.get(2)?,
        started_at: row.get(3)?,
        finished_at: row.get(4)?,
        duration_ms: row.get(5)?,
        exit_code: row.get(6)?,
        tokens_in: row.get(7)?,
        tokens_out: row.get(8)?,
        tool_calls: row.get(9)?,
        skills_loaded: row.get(10)?,
        context_chars: row.get(11)?,
        final_commit_sha: row.get(12)?,
        pr_number: row.get(13)?,
        model: row.get(14)?,
        co_auto_version: row.get(15)?,
        raw_log_blob_sha: row.get(16)?,
    })
}

impl Storage {
    pub fn insert_agent_session(&self, s: &NewAgentSession) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO agent_sessions \
             (task_id, universe_key, started_at, finished_at, duration_ms, exit_code, \
              tokens_in, tokens_out, tool_calls, skills_loaded, context_chars, \
              final_commit_sha, pr_number, model, co_auto_version, raw_log_blob_sha) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                s.task_id,
                s.universe_key,
                s.started_at,
                s.finished_at,
                s.duration_ms,
                s.exit_code,
                s.tokens_in,
                s.tokens_out,
                s.tool_calls,
                s.skills_loaded,
                s.context_chars,
                s.final_commit_sha,
                s.pr_number,
                s.model,
                s.co_auto_version,
                s.raw_log_blob_sha
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_agent_sessions(&self, task_id: &str) -> anyhow::Result<Vec<AgentSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, universe_key, started_at, finished_at, duration_ms, exit_code, \
             tokens_in, tokens_out, tool_calls, skills_loaded, context_chars, final_commit_sha, \
             pr_number, model, co_auto_version, raw_log_blob_sha \
             FROM agent_sessions WHERE task_id = ?1 ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map(params![task_id], row_to_session)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// CO-453: paginated, filtered listing across **all** tasks for the public
    /// telemetry API. Optional `model` / `since` (epoch seconds, inclusive on
    /// `started_at`) filters; newest first. `limit`/`offset` drive pagination.
    /// Read-only — reuses the CO-275 `agent_sessions` table, no new schema.
    pub fn list_agent_sessions_filtered(
        &self,
        model: Option<&str>,
        since: Option<i64>,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<AgentSession>> {
        // Sentinel filters: a NULL parameter disables the corresponding clause,
        // so every named binding is always present in the statement and the
        // filters are passed as parameters (never interpolated) — injection-safe.
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, universe_key, started_at, finished_at, duration_ms, exit_code, \
             tokens_in, tokens_out, tool_calls, skills_loaded, context_chars, final_commit_sha, \
             pr_number, model, co_auto_version, raw_log_blob_sha \
             FROM agent_sessions \
             WHERE (:model IS NULL OR model = :model) \
               AND (:since IS NULL OR started_at >= :since) \
             ORDER BY started_at DESC, id DESC LIMIT :limit OFFSET :offset",
        )?;
        let rows = stmt.query_map(
            rusqlite::named_params! {
                ":model": model,
                ":since": since,
                ":limit": limit,
                ":offset": offset,
            },
            row_to_session,
        )?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// CO-453: fetch one agent session by primary key (public telemetry API).
    pub fn get_agent_session(&self, id: i64) -> anyhow::Result<Option<AgentSession>> {
        self.conn
            .query_row(
                "SELECT id, task_id, universe_key, started_at, finished_at, duration_ms, exit_code, \
                 tokens_in, tokens_out, tool_calls, skills_loaded, context_chars, final_commit_sha, \
                 pr_number, model, co_auto_version, raw_log_blob_sha \
                 FROM agent_sessions WHERE id = ?1",
                params![id],
                row_to_session,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn latest_agent_session(&self, task_id: &str) -> anyhow::Result<Option<AgentSession>> {
        self.conn
            .query_row(
                "SELECT id, task_id, universe_key, started_at, finished_at, duration_ms, exit_code, \
                 tokens_in, tokens_out, tool_calls, skills_loaded, context_chars, final_commit_sha, \
                 pr_number, model, co_auto_version, raw_log_blob_sha \
                 FROM agent_sessions WHERE task_id = ?1 ORDER BY started_at DESC LIMIT 1",
                params![task_id],
                row_to_session,
            )
            .optional()
            .map_err(Into::into)
    }
}
