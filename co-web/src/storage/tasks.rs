use chrono::Utc;
use rusqlite::params;
use serde_json::json;

use crate::entry_index::{EntryRow, make_entry};
use crate::models::*;

use super::Storage;
use super::schema::{
    entry_row_from_sql, entry_row_to_comment, entry_row_to_task, parse_datetime, upsert_entry_row,
};

impl Storage {
    pub fn list_tasks(&self, project_key: &str) -> Vec<Task> {
        self.list_tasks_filtered(project_key, Some(false))
    }

    pub fn list_tasks_filtered(&self, project_key: &str, archived: Option<bool>) -> Vec<Task> {
        self.list_tasks_paginated(project_key, archived, 500, 0)
    }

    pub fn list_tasks_paginated(
        &self,
        project_key: &str,
        archived: Option<bool>,
        limit: u64,
        offset: u64,
    ) -> Vec<Task> {
        let upper_key = project_key.to_uppercase();
        let limit = limit.min(500);

        // CO-77: look up universe, then query per-universe DB
        let universe_key = match self.get_project_universe_key(&upper_key) {
            Some(uk) => uk,
            None => return vec![],
        };
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");

        let sql: String;
        let rows: Vec<EntryRow>;

        match archived {
            Some(archived_val) => {
                let archived_int = if archived_val { 1 } else { 0 };
                sql = format!(
                    "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                     created_at, updated_at FROM entries \
                     WHERE entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?1 \
                     AND json_extract(frontmatter_json, '$.archived') = {} \
                     ORDER BY CAST(json_extract(frontmatter_json, '$.id') AS INTEGER) \
                     LIMIT ?2 OFFSET ?3",
                    archived_int
                );
                let mut stmt = match uc_guard.prepare(&sql) {
                    Ok(s) => s,
                    Err(_) => return vec![],
                };
                rows = stmt
                    .query_map(
                        params![upper_key, limit as i64, offset as i64],
                        entry_row_from_sql,
                    )
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r.ok())
                    .collect();
            }
            None => {
                sql = "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                       created_at, updated_at FROM entries \
                       WHERE entry_type = 'task' \
                       AND json_extract(frontmatter_json, '$.project') = ?1 \
                       ORDER BY CAST(json_extract(frontmatter_json, '$.id') AS INTEGER) \
                       LIMIT ?2 OFFSET ?3"
                    .to_string();
                let mut stmt = match uc_guard.prepare(&sql) {
                    Ok(s) => s,
                    Err(_) => return vec![],
                };
                rows = stmt
                    .query_map(
                        params![upper_key, limit as i64, offset as i64],
                        entry_row_from_sql,
                    )
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r.ok())
                    .collect();
            }
        }

        rows.into_iter()
            .filter_map(|row| entry_row_to_task(&row))
            .collect()
    }

    pub fn get_task(&self, project_key: &str, id: u64) -> Option<Task> {
        let upper_key = project_key.to_uppercase();
        let path = format!("projects/{}/{}.md", upper_key, id);
        // CO-77: look up universe, then query per-universe DB
        let universe_key = self.get_project_universe_key(&upper_key)?;
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let result = uc_guard.query_row(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries WHERE path = ?1 AND entry_type = 'task'",
            params![path],
            entry_row_from_sql,
        );
        match result {
            Ok(row) => entry_row_to_task(&row),
            Err(_) => None,
        }
    }

    pub fn create_task(&mut self, project_key: &str, create: CreateTask) -> anyhow::Result<Task> {
        let upper_key = project_key.to_uppercase();
        let project = self
            .get_project(&upper_key)
            .ok_or_else(|| anyhow::anyhow!("Project '{}' not found", upper_key))?;

        let id = project.next_id;
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());

        // Increment next_id in project entry
        self.increment_project_next_id(&upper_key, &universe_key, id + 1);

        let path = format!("projects/{}/{}.md", upper_key, id);
        let fm = json!({
            "type": "task",
            "id": id,
            "title": create.title,
            "status": create.status.to_string(),
            "priority": create.priority.to_string(),
            "due": create.due_date.map(|d| d.to_string()),
            "parent": create.parent,
            "tags": create.labels,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "assignee": create.assignee,
            "project": upper_key
        });

        let entry = make_entry(&path, fm, &create.description);
        let universe_root = self.universe_root(&universe_key);
        co::entry::write_entry(&universe_root, &entry)?;
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            upsert_entry_row(&uc_guard, &universe_key, &entry)?;
        }

        Ok(Task {
            id,
            key: format!("{}-{}", upper_key, id),
            project_key: upper_key,
            title: create.title,
            status: create.status,
            priority: create.priority,
            due_date: create.due_date,
            parent: create.parent,
            labels: create.labels,
            created_at: now,
            updated_at: now,
            description: create.description,
            archived: false,
            assignee: create.assignee,
        })
    }

    pub fn update_task(
        &mut self,
        project_key: &str,
        id: u64,
        update: UpdateTask,
    ) -> anyhow::Result<Task> {
        let mut task = self
            .get_task(project_key, id)
            .ok_or_else(|| anyhow::anyhow!("Task {}-{} not found", project_key, id))?;

        if let Some(title) = update.title {
            task.title = title;
        }
        if let Some(description) = update.description {
            task.description = description;
        }
        if let Some(status) = update.status {
            task.status = status;
        }
        if let Some(priority) = update.priority {
            task.priority = priority;
        }
        if let Some(due_date) = update.due_date {
            task.due_date = Some(due_date);
        }
        if let Some(parent) = update.parent {
            task.parent = Some(parent);
        }
        if let Some(labels) = update.labels {
            task.labels = labels;
        }
        if let Some(archived) = update.archived {
            task.archived = archived;
        }
        if update.assignee.is_some() {
            task.assignee = update.assignee;
        }

        task.updated_at = Utc::now();

        let universe_key = self
            .get_project_universe_key(&task.project_key)
            .unwrap_or_else(|| "default".to_string());

        let path = format!("projects/{}/{}.md", task.project_key, id);
        let fm = json!({
            "type": "task",
            "id": id,
            "title": task.title,
            "status": task.status.to_string(),
            "priority": task.priority.to_string(),
            "due": task.due_date.map(|d| d.to_string()),
            "parent": task.parent,
            "tags": task.labels,
            "created": task.created_at.to_rfc3339(),
            "modified": task.updated_at.to_rfc3339(),
            "archived": task.archived,
            "assignee": task.assignee,
            "project": task.project_key
        });

        let entry = make_entry(&path, fm, &task.description);
        let universe_root = self.universe_root(&universe_key);
        co::entry::write_entry(&universe_root, &entry)?;
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            upsert_entry_row(&uc_guard, &universe_key, &entry)?;
        }

        Ok(task)
    }

    pub fn delete_task(&mut self, project_key: &str, id: u64) -> anyhow::Result<()> {
        let upper_key = project_key.to_uppercase();
        self.get_task(&upper_key, id)
            .ok_or_else(|| anyhow::anyhow!("Task {}-{} not found", upper_key, id))?;

        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());
        let universe_root = self.universe_root(&universe_key);
        let path = format!("projects/{}/{}.md", upper_key, id);

        let _ = co::entry::delete_entry(&universe_root, &path);
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let _ = uc_guard.execute(
                "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
                params![universe_key, path],
            );
        }

        Ok(())
    }

    // --- Bulk Operations ---

    pub fn bulk_update_tasks(
        &mut self,
        project_key: &str,
        bulk: BulkUpdateTasks,
    ) -> anyhow::Result<Vec<Task>> {
        let upper_key = project_key.to_uppercase();
        for &task_id in &bulk.task_ids {
            let update = UpdateTask {
                title: None,
                description: None,
                status: bulk.status.clone(),
                priority: None,
                due_date: None,
                parent: None,
                labels: None,
                archived: bulk.archived,
                assignee: None,
            };
            let _ = self.update_task(&upper_key, task_id, update);
        }

        let mut result = Vec::new();
        for &task_id in &bulk.task_ids {
            if let Some(task) = self.get_task(&upper_key, task_id) {
                result.push(task);
            }
        }
        Ok(result)
    }

    pub fn bulk_delete_tasks(
        &mut self,
        project_key: &str,
        bulk: BulkDeleteTasks,
    ) -> anyhow::Result<()> {
        let upper_key = project_key.to_uppercase();
        for &task_id in &bulk.task_ids {
            let _ = self.delete_task(&upper_key, task_id);
        }
        Ok(())
    }

    // --- Comments ---

    pub fn list_comments(&self, project_key: &str, task_id: u64) -> Vec<Comment> {
        let upper_key = project_key.to_uppercase();
        let path_prefix = format!("projects/{}/comments/{}-", upper_key, task_id);

        // CO-77: look up universe, then query per-universe DB
        let universe_key = match self.get_project_universe_key(&upper_key) {
            Some(uk) => uk,
            None => return vec![],
        };
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let mut stmt = match uc_guard.prepare(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE entry_type = 'comment' AND path LIKE ?1 \
             ORDER BY created_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let like_pattern = format!("{}%", path_prefix);
        stmt.query_map(params![like_pattern], entry_row_from_sql)
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .filter_map(|row| entry_row_to_comment(&row, &upper_key, task_id))
            .collect()
    }

    pub fn create_comment(
        &mut self,
        project_key: &str,
        task_id: u64,
        create: CreateComment,
    ) -> anyhow::Result<Comment> {
        let upper_key = project_key.to_uppercase();

        // Verify task exists
        self.get_task(&upper_key, task_id)
            .ok_or_else(|| anyhow::anyhow!("Task {}-{} not found", upper_key, task_id))?;

        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());

        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Allocate id via COUNT in per-universe DB
        let id: u64 = {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let count: i64 = uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE entry_type = 'comment' \
                     AND path LIKE ?1",
                    params![format!("projects/{}/comments/{}-%%", upper_key, task_id)],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            (count + 1) as u64
        };

        let path = format!("projects/{}/comments/{}-{}.md", upper_key, task_id, id);
        let fm = json!({
            "type": "comment",
            "id": id,
            "task": task_id,
            "project": upper_key,
            "author": create.author,
            "created": now_str,
            "modified": now_str,
            "tags": []
        });

        let entry = make_entry(&path, fm, &create.body);
        let universe_root = self.universe_root(&universe_key);
        co::entry::write_entry(&universe_root, &entry)?;
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            upsert_entry_row(&uc_guard, &universe_key, &entry)?;
        }

        Ok(Comment {
            id,
            project_key: upper_key,
            task_id,
            author: create.author,
            body: create.body,
            created_at: now,
        })
    }

    // --- Activity Log (graceful fallback — table may not exist) ---

    pub fn list_activity(&self, project_key: &str, limit: u64) -> Vec<ActivityEntry> {
        let upper_key = project_key.to_uppercase();
        let mut stmt = match self.conn.prepare(
            "SELECT id, project_key, task_id, action, field, old_value, new_value, actor, created_at \
             FROM activity_log WHERE project_key = ?1 ORDER BY created_at DESC LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match stmt.query_map(params![upper_key, limit as i64], |row| {
            Ok(ActivityEntry {
                id: row.get::<_, i64>(0)? as u64,
                project_key: row.get(1)?,
                task_id: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                action: row.get(3)?,
                field: row.get(4)?,
                old_value: row.get(5)?,
                new_value: row.get(6)?,
                actor: row.get(7)?,
                created_at: parse_datetime(&row.get::<_, String>(8)?),
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    // --- Dashboard ---
}
