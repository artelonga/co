//! CO-426: Usage-session storage — fleet-wide token/cost telemetry.
//!
//! One row per co-auto Claude invocation (producer: CO-425's stream-json
//! capture). Distinct from the CO-275 `agent_sessions` table: this is the
//! cross-machine usage ledger that feeds the Gestão usage dashboard and the
//! CO-427 downshift policy. Lives in the meta DB (global fleet telemetry).

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::Storage;

/// A new usage row to persist. `started_at`/`ended_at`/`reported_at` are epoch
/// seconds; `cost_usd` is `None` for subscription/keychain auth (no USD reported).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewUsageSession {
    pub task_key: String,
    pub universe_key: String,
    pub machine: String,
    pub model: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    pub cost_usd: Option<f64>,
    pub started_at: i64,
    pub ended_at: i64,
    pub outcome: String,
    pub reported_at: i64,
    /// CO-437: tool name→count map, stored as a JSON object string. `None` when
    /// the producer reported no tool usage.
    pub tool_uses: Option<String>,
    /// CO-437: the PR link co-auto opened for this session, if any.
    pub pr_url: Option<String>,
    /// CO-437: number of agent turns, from the stream-json `result` event.
    pub turns: i64,
}

/// The dimension to group a usage summary by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageDimension {
    Universe,
    Model,
    Machine,
    Task,
}

impl UsageDimension {
    /// Parse the `by=` query param. Defaults are decided by the caller.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "universe" => Some(Self::Universe),
            "model" => Some(Self::Model),
            "machine" => Some(Self::Machine),
            "task" => Some(Self::Task),
            _ => None,
        }
    }

    /// The whitelisted column name. Never interpolate user input — this enum is
    /// the only thing that reaches the SQL string, so the GROUP BY is injection-safe.
    fn column(self) -> &'static str {
        match self {
            Self::Universe => "universe_key",
            Self::Model => "model",
            Self::Machine => "machine",
            Self::Task => "task_key",
        }
    }
}

/// One aggregated group (a universe / model / machine / task) over a window.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct UsageGroup {
    pub key: String,
    pub sessions: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
}

/// Window-wide totals across every session in the window.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct UsageTotals {
    pub sessions: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
}

/// One row in the fleet "projetos" roll-up — usage accumulated per universe,
/// enriched with board task counts and last deploy (best-effort joins).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct UsageProjectRow {
    pub universe_key: String,
    pub name: Option<String>,
    pub sessions: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
    pub last_session_at: i64,
    pub tasks_todo: i64,
    pub tasks_in_progress: i64,
    pub last_deploy_at: Option<String>,
}

/// One cell of the model×universe (or any two-dimension) cross-tab.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct UsageCell {
    /// Row key (e.g. the universe).
    pub row: String,
    /// Column key (e.g. the model).
    pub col: String,
    pub sessions: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
}

/// CO-437: one usage session row, including the enrichment metadata, for the
/// per-session dashboard listing. `tool_uses` is the raw JSON string as stored.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct UsageSessionRow {
    pub id: i64,
    pub task_key: String,
    pub universe_key: String,
    pub machine: String,
    pub model: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
    pub started_at: i64,
    pub ended_at: i64,
    pub outcome: String,
    pub tool_uses: Option<String>,
    pub pr_url: Option<String>,
    pub turns: i64,
}

impl Storage {
    /// Insert a usage row. Returns the new row id.
    pub fn insert_usage_session(&self, s: &NewUsageSession) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO usage_sessions \
             (task_key, universe_key, machine, model, tokens_in, tokens_out, \
              tokens_cache_read, tokens_cache_write, cost_usd, started_at, ended_at, \
              outcome, reported_at, tool_uses, pr_url, turns) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                s.task_key,
                s.universe_key,
                s.machine,
                s.model,
                s.tokens_in,
                s.tokens_out,
                s.tokens_cache_read,
                s.tokens_cache_write,
                s.cost_usd,
                s.started_at,
                s.ended_at,
                s.outcome,
                s.reported_at,
                s.tool_uses,
                s.pr_url,
                s.turns,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Aggregate usage grouped by a dimension, over sessions started at/after
    /// `since` (epoch seconds). Ordered by total tokens descending.
    pub fn usage_summary(&self, since: i64, by: UsageDimension) -> anyhow::Result<Vec<UsageGroup>> {
        let col = by.column();
        // `col` is one of four hard-coded column names (see UsageDimension) —
        // never user input — so this format! is injection-safe.
        let sql = format!(
            "SELECT {col} AS k, \
                    COUNT(*) AS sessions, \
                    COALESCE(SUM(tokens_in), 0), \
                    COALESCE(SUM(tokens_out), 0), \
                    COALESCE(SUM(tokens_cache_read), 0), \
                    COALESCE(SUM(tokens_cache_write), 0), \
                    SUM(cost_usd) \
             FROM usage_sessions \
             WHERE started_at >= ?1 \
             GROUP BY {col} \
             ORDER BY (COALESCE(SUM(tokens_in), 0) + COALESCE(SUM(tokens_out), 0) \
                     + COALESCE(SUM(tokens_cache_read), 0) \
                     + COALESCE(SUM(tokens_cache_write), 0)) DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since], |row| {
            let tokens_in: i64 = row.get(2)?;
            let tokens_out: i64 = row.get(3)?;
            let cache_read: i64 = row.get(4)?;
            let cache_write: i64 = row.get(5)?;
            Ok(UsageGroup {
                key: row.get(0)?,
                sessions: row.get(1)?,
                tokens_in,
                tokens_out,
                tokens_cache_read: cache_read,
                tokens_cache_write: cache_write,
                total_tokens: tokens_in + tokens_out + cache_read + cache_write,
                cost_usd: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// CO-437: cross-tabulate usage by two dimensions (e.g. universe × model)
    /// over sessions started at/after `since`. Returns one [`UsageCell`] per
    /// non-empty `(row, col)` pair, ordered by total tokens descending. Both
    /// column names come from the [`UsageDimension`] whitelist, so the
    /// `format!` is injection-safe.
    pub fn usage_cross(
        &self,
        since: i64,
        row: UsageDimension,
        col: UsageDimension,
    ) -> anyhow::Result<Vec<UsageCell>> {
        let row_col = row.column();
        let col_col = col.column();
        let sql = format!(
            "SELECT {row_col} AS r, {col_col} AS c, \
                    COUNT(*) AS sessions, \
                    COALESCE(SUM(tokens_in), 0), \
                    COALESCE(SUM(tokens_out), 0), \
                    COALESCE(SUM(tokens_cache_read), 0), \
                    COALESCE(SUM(tokens_cache_write), 0), \
                    SUM(cost_usd) \
             FROM usage_sessions \
             WHERE started_at >= ?1 \
             GROUP BY {row_col}, {col_col} \
             ORDER BY (COALESCE(SUM(tokens_in), 0) + COALESCE(SUM(tokens_out), 0) \
                     + COALESCE(SUM(tokens_cache_read), 0) \
                     + COALESCE(SUM(tokens_cache_write), 0)) DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since], |row| {
            let tokens_in: i64 = row.get(3)?;
            let tokens_out: i64 = row.get(4)?;
            let cache_read: i64 = row.get(5)?;
            let cache_write: i64 = row.get(6)?;
            Ok(UsageCell {
                row: row.get(0)?,
                col: row.get(1)?,
                sessions: row.get(2)?,
                tokens_in,
                tokens_out,
                tokens_cache_read: cache_read,
                tokens_cache_write: cache_write,
                total_tokens: tokens_in + tokens_out + cache_read + cache_write,
                cost_usd: row.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// CO-437: the most recent usage sessions (started at/after `since`),
    /// newest first, capped at `limit`. Carries the enrichment metadata
    /// (`tool_uses`, `pr_url`, `turns`) for the per-session dashboard listing.
    /// New columns are read by explicit index — never `.ok()`-swallowed — so a
    /// missing migration fails loudly rather than masquerading as "no rows".
    pub fn recent_usage_sessions(
        &self,
        since: i64,
        limit: i64,
    ) -> anyhow::Result<Vec<UsageSessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_key, universe_key, machine, model, \
                    tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, \
                    cost_usd, started_at, ended_at, outcome, tool_uses, pr_url, turns \
             FROM usage_sessions \
             WHERE started_at >= ?1 \
             ORDER BY started_at DESC, id DESC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![since, limit], |row| {
            let tokens_in: i64 = row.get(5)?;
            let tokens_out: i64 = row.get(6)?;
            let cache_read: i64 = row.get(7)?;
            let cache_write: i64 = row.get(8)?;
            Ok(UsageSessionRow {
                id: row.get(0)?,
                task_key: row.get(1)?,
                universe_key: row.get(2)?,
                machine: row.get(3)?,
                model: row.get(4)?,
                tokens_in,
                tokens_out,
                tokens_cache_read: cache_read,
                tokens_cache_write: cache_write,
                total_tokens: tokens_in + tokens_out + cache_read + cache_write,
                cost_usd: row.get(9)?,
                started_at: row.get(10)?,
                ended_at: row.get(11)?,
                outcome: row.get(12)?,
                tool_uses: row.get(13)?,
                pr_url: row.get(14)?,
                turns: row.get(15)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Sum every token/cost field across the window (sessions started at/after
    /// `since`). Drives the "current window" consumption bars.
    pub fn usage_window_totals(&self, since: i64) -> anyhow::Result<UsageTotals> {
        self.conn
            .query_row(
                "SELECT COUNT(*), \
                        COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0), \
                        COALESCE(SUM(tokens_cache_read), 0), COALESCE(SUM(tokens_cache_write), 0), \
                        SUM(cost_usd) \
                 FROM usage_sessions WHERE started_at >= ?1",
                params![since],
                |row| {
                    let tokens_in: i64 = row.get(1)?;
                    let tokens_out: i64 = row.get(2)?;
                    let cache_read: i64 = row.get(3)?;
                    let cache_write: i64 = row.get(4)?;
                    Ok(UsageTotals {
                        sessions: row.get(0)?,
                        tokens_in,
                        tokens_out,
                        tokens_cache_read: cache_read,
                        tokens_cache_write: cache_write,
                        total_tokens: tokens_in + tokens_out + cache_read + cache_write,
                        cost_usd: row.get(5)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    /// Fleet "projetos" roll-up: one row per universe seen in usage (since
    /// `since`), enriched with the universe display name and last deploy
    /// (`deployment_snapshots` where `unit == universe_key`) from the meta DB,
    /// plus board task counts (todo / in_progress) pulled best-effort from each
    /// universe's *own* DB (the `tasks`/`projects` tables were dropped from the
    /// meta DB when entries moved to per-universe DBs). A universe with no board
    /// or no deploy simply reports 0 / null, never an error.
    pub fn usage_projects(&self, since: i64) -> anyhow::Result<Vec<UsageProjectRow>> {
        // Pass 1: aggregate from the meta DB only (no cross-DB joins here).
        let mut rows: Vec<UsageProjectRow> = {
            let mut stmt = self.conn.prepare(
                "SELECT us.universe_key, \
                        (SELECT name FROM universes WHERE key = us.universe_key), \
                        COUNT(*), \
                        COALESCE(SUM(us.tokens_in), 0), COALESCE(SUM(us.tokens_out), 0), \
                        COALESCE(SUM(us.tokens_cache_read), 0), COALESCE(SUM(us.tokens_cache_write), 0), \
                        SUM(us.cost_usd), \
                        MAX(us.started_at), \
                        (SELECT NULLIF(last_deploy_at, '') FROM deployment_snapshots \
                            WHERE unit = us.universe_key) \
                 FROM usage_sessions us \
                 WHERE us.started_at >= ?1 \
                 GROUP BY us.universe_key \
                 ORDER BY (COALESCE(SUM(us.tokens_in), 0) + COALESCE(SUM(us.tokens_out), 0) \
                         + COALESCE(SUM(us.tokens_cache_read), 0) \
                         + COALESCE(SUM(us.tokens_cache_write), 0)) DESC",
            )?;
            let mapped = stmt.query_map(params![since], |row| {
                let tokens_in: i64 = row.get(3)?;
                let tokens_out: i64 = row.get(4)?;
                let cache_read: i64 = row.get(5)?;
                let cache_write: i64 = row.get(6)?;
                Ok(UsageProjectRow {
                    universe_key: row.get(0)?,
                    name: row.get(1)?,
                    sessions: row.get(2)?,
                    tokens_in,
                    tokens_out,
                    tokens_cache_read: cache_read,
                    tokens_cache_write: cache_write,
                    total_tokens: tokens_in + tokens_out + cache_read + cache_write,
                    cost_usd: row.get(7)?,
                    last_session_at: row.get(8)?,
                    tasks_todo: 0,
                    tasks_in_progress: 0,
                    last_deploy_at: row.get(9)?,
                })
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        };

        // Pass 2: best-effort per-universe board counts from each universe DB.
        for row in &mut rows {
            let (todo, in_progress) = self.universe_board_counts(&row.universe_key);
            row.tasks_todo = todo;
            row.tasks_in_progress = in_progress;
        }

        Ok(rows)
    }

    /// Count entries with `status` of `todo` / `in_progress` in a universe's own
    /// `frontmatter_index`. Best-effort: any failure (universe never opened, no
    /// such table, lock poisoned) yields `(0, 0)` — never propagates an error,
    /// so a single bad universe can't break the whole fleet roll-up.
    fn universe_board_counts(&self, universe_key: &str) -> (i64, i64) {
        let Ok(conn_arc) = self.try_universe_conn(universe_key) else {
            return (0, 0);
        };
        let Ok(conn) = conn_arc.lock() else {
            return (0, 0);
        };
        let count = |status: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM frontmatter_index WHERE status = ?1",
                params![status],
                |r| r.get(0),
            )
            .unwrap_or(0)
        };
        (count("todo"), count("in_progress"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use tempfile::tempdir;

    fn sample(
        task: &str,
        universe: &str,
        machine: &str,
        model: &str,
        started: i64,
    ) -> NewUsageSession {
        NewUsageSession {
            task_key: task.into(),
            universe_key: universe.into(),
            machine: machine.into(),
            model: model.into(),
            tokens_in: 1000,
            tokens_out: 500,
            tokens_cache_read: 200,
            tokens_cache_write: 100,
            cost_usd: Some(0.25),
            started_at: started,
            ended_at: started + 60,
            outcome: "success".into(),
            reported_at: started + 61,
            tool_uses: Some(r#"{"Read":3,"Edit":1}"#.into()),
            pr_url: Some(format!("https://github.com/artelonga/co/pull/{started}")),
            turns: 4,
        }
    }

    #[test]
    fn insert_and_summarize_by_model() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        storage
            .insert_usage_session(&sample("CO-1", "co", "m1", "opus", 1000))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-2", "co", "m1", "opus", 1100))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-3", "ygg", "m2", "sonnet", 1200))
            .unwrap();

        let groups = storage.usage_summary(0, UsageDimension::Model).unwrap();
        assert_eq!(groups.len(), 2);
        // opus has two sessions → ordered first by total tokens.
        let opus = groups.iter().find(|g| g.key == "opus").unwrap();
        assert_eq!(opus.sessions, 2);
        assert_eq!(opus.tokens_in, 2000);
        assert_eq!(opus.total_tokens, 2 * (1000 + 500 + 200 + 100));
        assert_eq!(opus.cost_usd, Some(0.5));
    }

    #[test]
    fn summary_window_excludes_old_sessions() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        storage
            .insert_usage_session(&sample("CO-1", "co", "m1", "opus", 100))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-2", "co", "m1", "opus", 5000))
            .unwrap();

        // Window cutoff at 1000 keeps only the second session.
        let totals = storage.usage_window_totals(1000).unwrap();
        assert_eq!(totals.sessions, 1);
        assert_eq!(totals.tokens_in, 1000);
    }

    #[test]
    fn summary_by_universe_and_machine() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        storage
            .insert_usage_session(&sample("CO-1", "co", "m1", "opus", 1000))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-2", "ygg", "m2", "sonnet", 1100))
            .unwrap();

        let by_uni = storage.usage_summary(0, UsageDimension::Universe).unwrap();
        assert_eq!(by_uni.len(), 2);
        let by_machine = storage.usage_summary(0, UsageDimension::Machine).unwrap();
        assert_eq!(by_machine.len(), 2);
    }

    #[test]
    fn projects_rollup_groups_by_universe() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        storage
            .insert_usage_session(&sample("CO-1", "co", "m1", "opus", 1000))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-2", "co", "m2", "sonnet", 2000))
            .unwrap();

        let projects = storage.usage_projects(0).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].universe_key, "co");
        assert_eq!(projects[0].sessions, 2);
        assert_eq!(projects[0].last_session_at, 2000);
        // No board / deploy rows in a fresh DB — best-effort enrichment is zero/null.
        assert_eq!(projects[0].tasks_todo, 0);
        assert_eq!(projects[0].last_deploy_at, None);
    }

    #[test]
    fn cross_tab_groups_by_universe_and_model() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        storage
            .insert_usage_session(&sample("CO-1", "co", "m1", "opus", 1000))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-2", "co", "m1", "opus", 1100))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-3", "co", "m1", "sonnet", 1200))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-4", "ygg", "m2", "opus", 1300))
            .unwrap();

        let cells = storage
            .usage_cross(0, UsageDimension::Universe, UsageDimension::Model)
            .unwrap();
        // (co,opus), (co,sonnet), (ygg,opus) → three non-empty cells.
        assert_eq!(cells.len(), 3);
        let co_opus = cells
            .iter()
            .find(|c| c.row == "co" && c.col == "opus")
            .unwrap();
        assert_eq!(co_opus.sessions, 2);
        assert_eq!(co_opus.tokens_in, 2000);
        assert_eq!(co_opus.total_tokens, 2 * (1000 + 500 + 200 + 100));
        assert_eq!(co_opus.cost_usd, Some(0.5));
    }

    #[test]
    fn recent_sessions_carry_enrichment_metadata() {
        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        storage
            .insert_usage_session(&sample("CO-1", "co", "m1", "opus", 1000))
            .unwrap();
        storage
            .insert_usage_session(&sample("CO-2", "co", "m1", "sonnet", 2000))
            .unwrap();

        let rows = storage.recent_usage_sessions(0, 10).unwrap();
        assert_eq!(rows.len(), 2);
        // Newest first.
        assert_eq!(rows[0].task_key, "CO-2");
        assert_eq!(rows[0].turns, 4);
        assert_eq!(rows[0].tool_uses.as_deref(), Some(r#"{"Read":3,"Edit":1}"#));
        assert_eq!(
            rows[0].pr_url.as_deref(),
            Some("https://github.com/artelonga/co/pull/2000")
        );
        // Limit is honoured.
        assert_eq!(storage.recent_usage_sessions(0, 1).unwrap().len(), 1);
    }

    #[test]
    fn dimension_parse_rejects_unknown() {
        assert_eq!(UsageDimension::parse("model"), Some(UsageDimension::Model));
        assert_eq!(UsageDimension::parse("bogus"), None);
    }
}
