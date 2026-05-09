use rusqlite::params;

use crate::entry_index::EntryRow;
use crate::models::*;

use super::Storage;
use super::schema::{entry_row_from_sql, entry_row_to_task};

impl Storage {
    pub fn get_dashboard(&self, project_key: &str) -> DashboardData {
        let upper_key = project_key.to_uppercase();

        let status_counts = self.get_status_counts(&upper_key);

        // CO-77: look up universe for per-universe queries
        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());

        let today_str = chrono::Utc::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        let overdue_count: i64 = {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?1 \
                     AND json_extract(frontmatter_json, '$.archived') = 0 \
                     AND json_extract(frontmatter_json, '$.status') != 'done' \
                     AND json_extract(frontmatter_json, '$.due') IS NOT NULL \
                     AND json_extract(frontmatter_json, '$.due') < ?2",
                    params![upper_key, today_str],
                    |row| row.get(0),
                )
                .unwrap_or(0)
        };

        let upcoming_tasks =
            self.query_tasks_entries(&upper_key, Some(false), Some("!= 'done'"), true, Some(10));
        let recently_updated = self.query_tasks_recent(&upper_key, 10);
        let velocity = self.get_velocity(&upper_key);
        let burndown = self.get_burndown(&upper_key);
        let label_distribution = self.get_label_distribution(&upper_key);
        let overdue_tasks_detail = self.get_overdue_tasks_detail(&upper_key);

        DashboardData {
            status_counts,
            overdue_count: overdue_count as u64,
            upcoming_tasks,
            recently_updated,
            velocity,
            burndown,
            label_distribution,
            overdue_tasks_detail,
        }
    }

    fn get_velocity(&self, project_key: &str) -> Vec<WeeklyVelocity> {
        let mut stmt = match self.conn.prepare(
            "SELECT strftime('%Y-W%W', created_at) as week, COUNT(*) as count \
             FROM activity_log \
             WHERE project_key = ?1 \
               AND action = 'field_changed' \
               AND field = 'status' \
               AND new_value = 'done' \
               AND date(created_at) >= date('now', '-56 days') \
             GROUP BY week \
             ORDER BY week ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match stmt.query_map(params![project_key], |row| {
            Ok(WeeklyVelocity {
                week: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    fn get_burndown(&self, project_key: &str) -> Vec<BurndownPoint> {
        // CO-77: look up universe for per-universe queries
        let universe_key = self
            .get_project_universe_key(project_key)
            .unwrap_or_else(|| "default".to_string());
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");

        let today = chrono::Utc::now().date_naive();
        let mut result = Vec::with_capacity(8);

        for week_offset in (0i64..8).rev() {
            let week_end = today - chrono::Duration::weeks(week_offset);
            let week_label = week_end.format("%Y-W%V").to_string();
            let week_end_str = week_end.to_string();

            let total_created: i64 = uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?1 \
                     AND json_extract(frontmatter_json, '$.archived') = 0 \
                     AND date(created_at) <= ?2",
                    params![project_key, week_end_str],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            // For done count we fall back to activity_log (graceful)
            let total_done: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM activity_log \
                     WHERE project_key = ?1 \
                       AND action = 'field_changed' \
                       AND field = 'status' \
                       AND new_value = 'done' \
                       AND date(created_at) <= ?2",
                    params![project_key, week_end_str],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            result.push(BurndownPoint {
                date: week_label,
                remaining: (total_created - total_done).max(0),
                completed: total_done as u64,
            });
        }

        result
    }

    fn get_label_distribution(&self, project_key: &str) -> Vec<LabelCount> {
        let mut stmt = match self.conn.prepare(
            "SELECT frontmatter_json FROM entries \
             WHERE entry_type = 'task' \
             AND json_extract(frontmatter_json, '$.project') = ?1 \
             AND json_extract(frontmatter_json, '$.archived') = 0",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let fm_strings: Vec<String> = match stmt.query_map(params![project_key], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => return vec![],
        };

        let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for fm_str in fm_strings {
            let fm: serde_json::Value =
                serde_json::from_str(&fm_str).unwrap_or(serde_json::Value::Null);
            if let Some(tags) = fm.get("tags").and_then(|v| v.as_array()) {
                for tag in tags {
                    if let Some(t) = tag.as_str()
                        && !t.is_empty()
                    {
                        *counts.entry(t.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut result: Vec<LabelCount> = counts
            .into_iter()
            .map(|(label, count)| LabelCount { label, count })
            .collect();
        result.sort_by(|a, b| b.count.cmp(&a.count));
        result.truncate(10);
        result
    }

    fn get_overdue_tasks_detail(&self, project_key: &str) -> Vec<OverdueTaskDetail> {
        let today = chrono::Utc::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let mut stmt = match self.conn.prepare(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE entry_type = 'task' \
             AND json_extract(frontmatter_json, '$.project') = ?1 \
             AND json_extract(frontmatter_json, '$.archived') = 0 \
             AND json_extract(frontmatter_json, '$.status') != 'done' \
             AND json_extract(frontmatter_json, '$.due') IS NOT NULL \
             AND json_extract(frontmatter_json, '$.due') < ?2 \
             ORDER BY json_extract(frontmatter_json, '$.due') ASC LIMIT 20",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let rows: Vec<EntryRow> =
            match stmt.query_map(params![project_key, today_str], entry_row_from_sql) {
                Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                Err(_) => return vec![],
            };

        rows.into_iter()
            .filter_map(|row| {
                let task = entry_row_to_task(&row)?;
                let due = task.due_date?;
                let days_overdue = (today - due).num_days();
                Some(OverdueTaskDetail {
                    id: task.id,
                    key: task.key,
                    title: task.title,
                    due_date: due.to_string(),
                    days_overdue,
                    priority: task.priority.to_string(),
                })
            })
            .collect()
    }

    fn get_status_counts(&self, project_key: &str) -> StatusCounts {
        // CO-77: look up universe for per-universe queries
        let universe_key = self
            .get_project_universe_key(project_key)
            .unwrap_or_else(|| "default".to_string());
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");

        let count = |status: &str| -> u64 {
            uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?1 \
                     AND json_extract(frontmatter_json, '$.archived') = 0 \
                     AND json_extract(frontmatter_json, '$.status') = ?2",
                    params![project_key, status],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0) as u64
        };

        let todo = count("todo");
        let in_progress = count("in_progress");
        let in_review = count("in_review");
        let done = count("done");

        StatusCounts {
            todo,
            in_progress,
            in_review,
            done,
            total: todo + in_progress + in_review + done,
        }
    }

    /// Query tasks with due date within the next 7 days (upcoming).
    fn query_tasks_entries(
        &self,
        project_key: &str,
        archived: Option<bool>,
        status_condition: Option<&str>,
        upcoming_only: bool,
        limit: Option<u64>,
    ) -> Vec<Task> {
        let archived_filter = match archived {
            Some(true) => "AND json_extract(frontmatter_json, '$.archived') = 1".to_string(),
            Some(false) => "AND json_extract(frontmatter_json, '$.archived') = 0".to_string(),
            None => String::new(),
        };
        let status_filter = status_condition
            .map(|c| format!("AND json_extract(frontmatter_json, '$.status') {}", c))
            .unwrap_or_default();
        let upcoming_filter = if upcoming_only {
            "AND json_extract(frontmatter_json, '$.due') IS NOT NULL \
             AND json_extract(frontmatter_json, '$.due') BETWEEN date('now') AND date('now', '+7 days')"
                .to_string()
        } else {
            String::new()
        };
        let limit_clause = limit.map(|l| format!("LIMIT {}", l)).unwrap_or_default();

        let sql = format!(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE entry_type = 'task' \
             AND json_extract(frontmatter_json, '$.project') = ?1 \
             {} {} {} \
             ORDER BY json_extract(frontmatter_json, '$.due') ASC {}",
            archived_filter, status_filter, upcoming_filter, limit_clause
        );

        let mut stmt = match self.conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match stmt.query_map(params![project_key], entry_row_from_sql) {
            Ok(rows) => rows
                .filter_map(|r| r.ok())
                .filter_map(|row| entry_row_to_task(&row))
                .collect(),
            Err(_) => vec![],
        }
    }

    fn query_tasks_recent(&self, project_key: &str, limit: u64) -> Vec<Task> {
        let sql = "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                   created_at, updated_at FROM entries \
                   WHERE entry_type = 'task' \
                   AND json_extract(frontmatter_json, '$.project') = ?1 \
                   AND json_extract(frontmatter_json, '$.archived') = 0 \
                   ORDER BY updated_at DESC \
                   LIMIT ?2";

        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match stmt.query_map(params![project_key, limit as i64], entry_row_from_sql) {
            Ok(rows) => rows
                .filter_map(|r| r.ok())
                .filter_map(|row| entry_row_to_task(&row))
                .collect(),
            Err(_) => vec![],
        }
    }

    // --- Users ---
}
