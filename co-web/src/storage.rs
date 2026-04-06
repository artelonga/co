use std::path::Path;

use chrono::{NaiveDate, Utc};
use rusqlite::{Connection, params};

use crate::models::*;

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        std::fs::create_dir_all(data_dir.as_ref()).expect("Failed to create data directory");
        let db_path = data_dir.as_ref().join("co.db");
        let conn = Connection::open(&db_path).expect("Failed to open database");

        // Enable WAL mode for concurrent reads
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .expect("Failed to enable WAL mode");
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("Failed to enable foreign keys");

        let mut storage = Self { conn };
        storage.run_migrations();
        storage
    }

    fn run_migrations(&mut self) {
        // Create schema_version table
        self.conn
            .execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
            .expect("Failed to create schema_version table");

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 1 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS projects (
                    key TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    next_id INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS tasks (
                    project_key TEXT NOT NULL,
                    id INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT 'todo',
                    priority TEXT NOT NULL DEFAULT 'medium',
                    due_date TEXT,
                    parent INTEGER,
                    labels TEXT NOT NULL DEFAULT '[]',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (project_key, id),
                    FOREIGN KEY (project_key) REFERENCES projects(key)
                );

                CREATE TABLE IF NOT EXISTS comments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_key TEXT NOT NULL,
                    task_id INTEGER NOT NULL,
                    author TEXT NOT NULL DEFAULT 'Anonymous',
                    body TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (project_key, task_id) REFERENCES tasks(project_key, id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS activity_log (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_key TEXT NOT NULL,
                    task_id INTEGER,
                    action TEXT NOT NULL,
                    field TEXT,
                    old_value TEXT,
                    new_value TEXT,
                    actor TEXT NOT NULL DEFAULT 'system',
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (project_key) REFERENCES projects(key)
                );

                INSERT INTO schema_version (version) VALUES (1);
                ",
                )
                .expect("Failed to run migration v1");
        }

        if current_version < 2 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS users (
                    id TEXT PRIMARY KEY,
                    email TEXT UNIQUE NOT NULL,
                    display_name TEXT NOT NULL DEFAULT '',
                    tier TEXT NOT NULL DEFAULT 'player',
                    created_at TEXT NOT NULL
                );
                INSERT INTO schema_version (version) VALUES (2);
                ",
                )
                .expect("Failed to run migration v2");
        }

        // Quilombo community tables (v3, v4)
        crate::quilombo_storage::run_quilombo_migrations(&self.conn);

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 5 {
            self.conn
                .execute_batch(
                    "ALTER TABLE tasks ADD COLUMN assignee TEXT;
                     INSERT INTO schema_version (version) VALUES (5);",
                )
                .expect("Failed to run migration v5");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 6 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS universes (
                    key TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    owner_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (owner_id) REFERENCES users(id)
                );
                INSERT INTO schema_version (version) VALUES (6);
                ",
                )
                .expect("Failed to run migration v6");
        }

        if current_version < 7 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS universe_members (
                    universe_key TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    role TEXT NOT NULL DEFAULT 'member',
                    joined_at TEXT NOT NULL,
                    PRIMARY KEY (universe_key, user_id),
                    FOREIGN KEY (universe_key) REFERENCES universes(key) ON DELETE CASCADE,
                    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
                );
                INSERT INTO schema_version (version) VALUES (7);
                ",
                )
                .expect("Failed to run migration v7");
        }

        if current_version < 8 {
            self.conn
                .execute_batch(
                    "ALTER TABLE projects ADD COLUMN universe_key TEXT REFERENCES universes(key);
                     INSERT INTO schema_version (version) VALUES (8);",
                )
                .expect("Failed to run migration v8");
        }

        if current_version < 9 {
            // Recreate universe_members without the FK on user_id so that
            // quilombo users (stored in quilombo_usuarios, not users) can be members.
            self.conn
                .execute_batch(
                    "
                CREATE TABLE universe_members_new (
                    universe_key TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    role TEXT NOT NULL DEFAULT 'member',
                    joined_at TEXT NOT NULL,
                    PRIMARY KEY (universe_key, user_id),
                    FOREIGN KEY (universe_key) REFERENCES universes(key) ON DELETE CASCADE
                );
                INSERT INTO universe_members_new SELECT * FROM universe_members;
                DROP TABLE universe_members;
                ALTER TABLE universe_members_new RENAME TO universe_members;
                INSERT INTO schema_version (version) VALUES (9);
                ",
                )
                .expect("Failed to run migration v9");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 10 {
            // Rebuild universes without FK on owner_id (support anonymous/system owners)
            // and add is_template + is_public columns.
            self.conn
                .execute_batch(
                    "
                CREATE TABLE universes_new (
                    key TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    owner_id TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL,
                    is_template INTEGER NOT NULL DEFAULT 0,
                    is_public INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO universes_new (key, name, description, owner_id, created_at, is_template, is_public)
                    SELECT key, name, description, owner_id, created_at, 0, 0 FROM universes;
                DROP TABLE universes;
                ALTER TABLE universes_new RENAME TO universes;
                INSERT INTO schema_version (version) VALUES (10);
                ",
                )
                .expect("Failed to run migration v10");
        }
    }

    /// List universe keys a user is a member of.
    pub fn list_user_universes(&self, user_id: &str) -> Vec<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT universe_key FROM universe_members WHERE user_id = ?1")
            .expect("Failed to prepare list_user_universes");
        stmt.query_map(rusqlite::params![user_id], |row| row.get(0))
            .expect("Failed to query user universes")
            .filter_map(|r| r.ok())
            .collect()
    }

    /// List projects the user can see: those in their universes.
    pub fn list_projects_for_user(&self, user_id: &str) -> Vec<crate::models::Project> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.key, p.name, p.description, p.next_id, p.created_at, p.archived \
             FROM projects p \
             JOIN universe_members um ON um.universe_key = p.universe_key \
             WHERE um.user_id = ?1 \
             ORDER BY p.key",
            )
            .expect("Failed to prepare list_projects_for_user");
        stmt.query_map(rusqlite::params![user_id], |row| {
            Ok(crate::models::Project {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                next_id: row.get::<_, i64>(3)? as u64,
                created_at: crate::storage::parse_datetime(&row.get::<_, String>(4)?),
                archived: row.get::<_, i64>(5)? != 0,
            })
        })
        .expect("Failed to list projects for user")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Access the underlying SQLite connection (for quilombo storage functions).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn schema_version(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    // --- Projects ---

    pub fn list_projects(&self) -> Vec<Project> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, name, description, next_id, created_at, archived FROM projects ORDER BY key")
            .expect("Failed to prepare list_projects");

        stmt.query_map([], |row| {
            Ok(Project {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                next_id: row.get::<_, i64>(3)? as u64,
                created_at: parse_datetime(&row.get::<_, String>(4)?),
                archived: row.get::<_, i64>(5)? != 0,
            })
        })
        .expect("Failed to list projects")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn list_projects_for_universe(&self, universe_key: &str) -> Vec<Project> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT key, name, description, next_id, created_at, archived \
                 FROM projects WHERE universe_key = ?1 ORDER BY key",
            )
            .expect("Failed to prepare list_projects_for_universe");

        stmt.query_map(params![universe_key], |row| {
            Ok(Project {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                next_id: row.get::<_, i64>(3)? as u64,
                created_at: parse_datetime(&row.get::<_, String>(4)?),
                archived: row.get::<_, i64>(5)? != 0,
            })
        })
        .expect("Failed to list projects for universe")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn get_project(&self, key: &str) -> Option<Project> {
        let upper_key = key.to_uppercase();
        self.conn
            .query_row(
                "SELECT key, name, description, next_id, created_at, archived FROM projects WHERE key = ?1",
                params![upper_key],
                |row| {
                    Ok(Project {
                        key: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        next_id: row.get::<_, i64>(3)? as u64,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                        archived: row.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .ok()
    }

    pub fn create_project(&mut self, create: CreateProject) -> anyhow::Result<Project> {
        let upper_key = create.key.to_uppercase();
        if self.get_project(&upper_key).is_some() {
            anyhow::bail!("Project with key '{}' already exists", upper_key);
        }

        let now = Utc::now();
        let now_str = now.to_rfc3339();

        self.conn.execute(
            "INSERT INTO projects (key, name, description, next_id, created_at, universe_key) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![upper_key, create.name, create.description, 1i64, now_str, create.universe_key],
        )?;

        self.log_activity(
            &upper_key,
            None,
            "project_created",
            None,
            None,
            Some(&create.name),
        );

        Ok(Project {
            name: create.name,
            key: upper_key,
            description: create.description,
            created_at: now,
            next_id: 1,
            archived: false,
        })
    }

    pub fn delete_project(&mut self, key: &str) -> anyhow::Result<()> {
        let upper_key = key.to_uppercase();
        if self.get_project(&upper_key).is_none() {
            anyhow::bail!("Project '{}' not found", upper_key);
        }

        self.conn.execute(
            "DELETE FROM activity_log WHERE project_key = ?1",
            params![upper_key],
        )?;
        self.conn.execute(
            "DELETE FROM tasks WHERE project_key = ?1",
            params![upper_key],
        )?;
        self.conn
            .execute("DELETE FROM projects WHERE key = ?1", params![upper_key])?;

        Ok(())
    }

    // --- Tasks ---

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

        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match archived {
            Some(archived_val) => (
                "SELECT id, title, description, status, priority, due_date, parent, labels, created_at, updated_at, archived, project_key, assignee \
                 FROM tasks WHERE project_key = ?1 AND archived = ?2 ORDER BY id LIMIT ?3 OFFSET ?4".to_string(),
                vec![Box::new(upper_key), Box::new(archived_val as i64), Box::new(limit as i64), Box::new(offset as i64)],
            ),
            None => (
                "SELECT id, title, description, status, priority, due_date, parent, labels, created_at, updated_at, archived, project_key, assignee \
                 FROM tasks WHERE project_key = ?1 ORDER BY id LIMIT ?2 OFFSET ?3".to_string(),
                vec![Box::new(upper_key), Box::new(limit as i64), Box::new(offset as i64)],
            ),
        };

        let mut stmt = self
            .conn
            .prepare(&sql)
            .expect("Failed to prepare list_tasks");
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        stmt.query_map(params_refs.as_slice(), |row| {
            let project_key: String = row.get(11)?;
            let id: i64 = row.get(0)?;
            Ok(Task {
                id: id as u64,
                key: format!("{}-{}", project_key, id),
                project_key: project_key.clone(),
                title: row.get(1)?,
                description: row.get(2)?,
                status: parse_status(&row.get::<_, String>(3)?),
                priority: parse_priority(&row.get::<_, String>(4)?),
                due_date: row
                    .get::<_, Option<String>>(5)?
                    .and_then(|s| s.parse::<NaiveDate>().ok()),
                parent: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                labels: parse_labels(&row.get::<_, String>(7)?),
                created_at: parse_datetime(&row.get::<_, String>(8)?),
                updated_at: parse_datetime(&row.get::<_, String>(9)?),
                archived: row.get::<_, i64>(10)? != 0,
                assignee: row.get(12)?,
            })
        })
        .expect("Failed to list tasks")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn get_task(&self, project_key: &str, id: u64) -> Option<Task> {
        let upper_key = project_key.to_uppercase();
        self.conn
            .query_row(
                "SELECT id, title, description, status, priority, due_date, parent, labels, created_at, updated_at, archived, project_key, assignee \
                 FROM tasks WHERE project_key = ?1 AND id = ?2",
                params![upper_key, id as i64],
                |row| {
                    let project_key: String = row.get(11)?;
                    let id: i64 = row.get(0)?;
                    Ok(Task {
                        id: id as u64,
                        key: format!("{}-{}", project_key, id),
                        project_key: project_key.clone(),
                        title: row.get(1)?,
                        description: row.get(2)?,
                        status: parse_status(&row.get::<_, String>(3)?),
                        priority: parse_priority(&row.get::<_, String>(4)?),
                        due_date: row.get::<_, Option<String>>(5)?.and_then(|s| s.parse::<NaiveDate>().ok()),
                        parent: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                        labels: parse_labels(&row.get::<_, String>(7)?),
                        created_at: parse_datetime(&row.get::<_, String>(8)?),
                        updated_at: parse_datetime(&row.get::<_, String>(9)?),
                        archived: row.get::<_, i64>(10)? != 0,
                        assignee: row.get(12)?,
                    })
                },
            )
            .ok()
    }

    pub fn create_task(&mut self, project_key: &str, create: CreateTask) -> anyhow::Result<Task> {
        let upper_key = project_key.to_uppercase();
        let project = self
            .get_project(&upper_key)
            .ok_or_else(|| anyhow::anyhow!("Project '{}' not found", upper_key))?;

        let id = project.next_id;
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let labels_json = serde_json::to_string(&create.labels)?;

        // Increment next_id
        self.conn.execute(
            "UPDATE projects SET next_id = ?1 WHERE key = ?2",
            params![(id + 1) as i64, upper_key],
        )?;

        self.conn.execute(
            "INSERT INTO tasks (project_key, id, title, description, status, priority, due_date, parent, labels, created_at, updated_at, assignee) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                upper_key,
                id as i64,
                create.title,
                create.description,
                create.status.to_string(),
                create.priority.to_string(),
                create.due_date.map(|d| d.to_string()),
                create.parent.map(|p| p as i64),
                labels_json,
                now_str,
                now_str,
                create.assignee,
            ],
        )?;

        self.log_activity(
            &upper_key,
            Some(id),
            "task_created",
            None,
            None,
            Some(&create.title),
        );

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

        if let Some(title) = &update.title {
            self.log_activity(
                &task.project_key,
                Some(id),
                "field_changed",
                Some("title"),
                Some(&task.title),
                Some(title),
            );
            task.title = title.clone();
        }
        if let Some(description) = &update.description {
            task.description = description.clone();
        }
        if let Some(status) = &update.status {
            self.log_activity(
                &task.project_key,
                Some(id),
                "field_changed",
                Some("status"),
                Some(&task.status.to_string()),
                Some(&status.to_string()),
            );
            task.status = status.clone();
        }
        if let Some(priority) = &update.priority {
            self.log_activity(
                &task.project_key,
                Some(id),
                "field_changed",
                Some("priority"),
                Some(&task.priority.to_string()),
                Some(&priority.to_string()),
            );
            task.priority = priority.clone();
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
            if archived != task.archived {
                let action = if archived {
                    "task_archived"
                } else {
                    "task_unarchived"
                };
                self.log_activity(&task.project_key, Some(id), action, None, None, None);
            }
            task.archived = archived;
        }
        if update.assignee.is_some() {
            task.assignee = update.assignee;
        }

        task.updated_at = Utc::now();
        let labels_json = serde_json::to_string(&task.labels)?;

        self.conn.execute(
            "UPDATE tasks SET title = ?1, description = ?2, status = ?3, priority = ?4, \
             due_date = ?5, parent = ?6, labels = ?7, updated_at = ?8, archived = ?9, assignee = ?10 \
             WHERE project_key = ?11 AND id = ?12",
            params![
                task.title,
                task.description,
                task.status.to_string(),
                task.priority.to_string(),
                task.due_date.map(|d| d.to_string()),
                task.parent.map(|p| p as i64),
                labels_json,
                task.updated_at.to_rfc3339(),
                task.archived as i64,
                task.assignee,
                task.project_key,
                id as i64,
            ],
        )?;

        Ok(task)
    }

    pub fn delete_task(&mut self, project_key: &str, id: u64) -> anyhow::Result<()> {
        let upper_key = project_key.to_uppercase();
        let task = self
            .get_task(&upper_key, id)
            .ok_or_else(|| anyhow::anyhow!("Task {}-{} not found", upper_key, id))?;

        self.conn.execute(
            "DELETE FROM tasks WHERE project_key = ?1 AND id = ?2",
            params![upper_key, id as i64],
        )?;

        self.log_activity(
            &upper_key,
            Some(id),
            "task_deleted",
            None,
            None,
            Some(&task.title),
        );

        Ok(())
    }

    // --- Bulk Operations ---

    pub fn bulk_update_tasks(
        &mut self,
        project_key: &str,
        bulk: BulkUpdateTasks,
    ) -> anyhow::Result<Vec<Task>> {
        let upper_key = project_key.to_uppercase();
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        for &task_id in &bulk.task_ids {
            if let Some(status) = &bulk.status {
                self.conn.execute(
                    "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE project_key = ?3 AND id = ?4",
                    params![status.to_string(), now_str, upper_key, task_id as i64],
                )?;
                self.log_activity(
                    &upper_key,
                    Some(task_id),
                    "field_changed",
                    Some("status"),
                    None,
                    Some(&status.to_string()),
                );
            }
            if let Some(archived) = bulk.archived {
                self.conn.execute(
                    "UPDATE tasks SET archived = ?1, updated_at = ?2 WHERE project_key = ?3 AND id = ?4",
                    params![archived as i64, now_str, upper_key, task_id as i64],
                )?;
                let action = if archived {
                    "task_archived"
                } else {
                    "task_unarchived"
                };
                self.log_activity(&upper_key, Some(task_id), action, None, None, None);
            }
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
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_key, task_id, author, body, created_at \
                 FROM comments WHERE project_key = ?1 AND task_id = ?2 ORDER BY created_at ASC",
            )
            .expect("Failed to prepare list_comments");

        stmt.query_map(params![upper_key, task_id as i64], |row| {
            Ok(Comment {
                id: row.get::<_, i64>(0)? as u64,
                project_key: row.get(1)?,
                task_id: row.get::<_, i64>(2)? as u64,
                author: row.get(3)?,
                body: row.get(4)?,
                created_at: parse_datetime(&row.get::<_, String>(5)?),
            })
        })
        .expect("Failed to list comments")
        .filter_map(|r| r.ok())
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

        let now = Utc::now();
        let now_str = now.to_rfc3339();

        self.conn.execute(
            "INSERT INTO comments (project_key, task_id, author, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![upper_key, task_id as i64, create.author, create.body, now_str],
        )?;

        let id = self.conn.last_insert_rowid() as u64;

        self.log_activity(
            &upper_key,
            Some(task_id),
            "comment_added",
            None,
            None,
            Some(&create.author),
        );

        Ok(Comment {
            id,
            project_key: upper_key,
            task_id,
            author: create.author,
            body: create.body,
            created_at: now,
        })
    }

    // --- Activity Log ---

    pub fn list_activity(&self, project_key: &str, limit: u64) -> Vec<ActivityEntry> {
        let upper_key = project_key.to_uppercase();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_key, task_id, action, field, old_value, new_value, actor, created_at \
                 FROM activity_log WHERE project_key = ?1 ORDER BY created_at DESC LIMIT ?2",
            )
            .expect("Failed to prepare list_activity");

        stmt.query_map(params![upper_key, limit as i64], |row| {
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
        })
        .expect("Failed to list activity")
        .filter_map(|r| r.ok())
        .collect()
    }

    fn log_activity(
        &self,
        project_key: &str,
        task_id: Option<u64>,
        action: &str,
        field: Option<&str>,
        old_value: Option<&str>,
        new_value: Option<&str>,
    ) {
        let now_str = Utc::now().to_rfc3339();
        let _ = self.conn.execute(
            "INSERT INTO activity_log (project_key, task_id, action, field, old_value, new_value, actor, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                project_key,
                task_id.map(|v| v as i64),
                action,
                field,
                old_value,
                new_value,
                "system",
                now_str,
            ],
        );
    }

    // --- Dashboard ---

    pub fn get_dashboard(&self, project_key: &str) -> DashboardData {
        let upper_key = project_key.to_uppercase();

        let status_counts = self.get_status_counts(&upper_key);

        let overdue_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE project_key = ?1 AND archived = 0 AND status != 'done' \
                 AND due_date IS NOT NULL AND due_date < date('now')",
                params![upper_key],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let upcoming_tasks = self.query_tasks(
            "SELECT id, title, description, status, priority, due_date, parent, labels, created_at, updated_at, archived, project_key, assignee \
             FROM tasks WHERE project_key = ?1 AND archived = 0 AND status != 'done' \
             AND due_date IS NOT NULL AND due_date BETWEEN date('now') AND date('now', '+7 days') \
             ORDER BY due_date ASC LIMIT 10",
            &upper_key,
        );

        let recently_updated = self.query_tasks(
            "SELECT id, title, description, status, priority, due_date, parent, labels, created_at, updated_at, archived, project_key, assignee \
             FROM tasks WHERE project_key = ?1 AND archived = 0 \
             ORDER BY updated_at DESC LIMIT 10",
            &upper_key,
        );

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
        let today = chrono::Utc::now().date_naive();
        let mut result = Vec::with_capacity(8);

        for week_offset in (0i64..8).rev() {
            let week_end = today - chrono::Duration::weeks(week_offset);
            let week_label = week_end.format("%Y-W%V").to_string();
            let week_end_str = week_end.to_string();

            let total_created: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM tasks WHERE project_key = ?1 AND archived = 0 \
                     AND date(created_at) <= ?2",
                    params![project_key, week_end_str],
                    |row| row.get(0),
                )
                .unwrap_or(0);

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
        let mut stmt = match self
            .conn
            .prepare("SELECT labels FROM tasks WHERE project_key = ?1 AND archived = 0")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let label_strings: Vec<String> =
            match stmt.query_map(params![project_key], |row| row.get(0)) {
                Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                Err(_) => return vec![],
            };

        let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for s in label_strings {
            let labels: Vec<String> = serde_json::from_str(&s).unwrap_or_default();
            for label in labels {
                if !label.is_empty() {
                    *counts.entry(label).or_insert(0) += 1;
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
        let tasks = self.query_tasks(
            "SELECT id, title, description, status, priority, due_date, parent, labels, \
             created_at, updated_at, archived, project_key, assignee \
             FROM tasks WHERE project_key = ?1 AND archived = 0 AND status != 'done' \
             AND due_date IS NOT NULL AND due_date < date('now') \
             ORDER BY due_date ASC LIMIT 20",
            project_key,
        );

        tasks
            .into_iter()
            .filter_map(|t| {
                let due = t.due_date?;
                let days_overdue = (today - due).num_days();
                Some(OverdueTaskDetail {
                    id: t.id,
                    key: t.key,
                    title: t.title,
                    due_date: due.to_string(),
                    days_overdue,
                    priority: t.priority.to_string(),
                })
            })
            .collect()
    }

    fn get_status_counts(&self, project_key: &str) -> StatusCounts {
        let count = |status: &str| -> u64 {
            self.conn
                .query_row(
                    "SELECT COUNT(*) FROM tasks WHERE project_key = ?1 AND archived = 0 AND status = ?2",
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

    fn query_tasks(&self, sql: &str, project_key: &str) -> Vec<Task> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .expect("Failed to prepare query_tasks");
        stmt.query_map(params![project_key], |row| {
            let project_key: String = row.get(11)?;
            let id: i64 = row.get(0)?;
            Ok(Task {
                id: id as u64,
                key: format!("{}-{}", project_key, id),
                project_key: project_key.clone(),
                title: row.get(1)?,
                description: row.get(2)?,
                status: parse_status(&row.get::<_, String>(3)?),
                priority: parse_priority(&row.get::<_, String>(4)?),
                due_date: row
                    .get::<_, Option<String>>(5)?
                    .and_then(|s| s.parse::<NaiveDate>().ok()),
                parent: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                labels: parse_labels(&row.get::<_, String>(7)?),
                created_at: parse_datetime(&row.get::<_, String>(8)?),
                updated_at: parse_datetime(&row.get::<_, String>(9)?),
                archived: row.get::<_, i64>(10)? != 0,
                assignee: row.get(12)?,
            })
        })
        .expect("Failed to query tasks")
        .filter_map(|r| r.ok())
        .collect()
    }

    // --- Users ---

    pub fn create_user(
        &mut self,
        email: &str,
        display_name: &str,
    ) -> anyhow::Result<crate::models::User> {
        let id = format!("usr_{}", nanoid::nanoid!(10));
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT INTO users (id, email, display_name, tier, created_at) VALUES (?1, ?2, ?3, 'player', ?4)",
            params![id, email, display_name, now_str],
        )?;
        Ok(crate::models::User {
            id,
            email: email.to_string(),
            display_name: display_name.to_string(),
            tier: "player".to_string(),
            created_at: now,
        })
    }

    pub fn get_user_by_email(&self, email: &str) -> Option<crate::models::User> {
        self.conn
            .query_row(
                "SELECT id, email, display_name, tier, created_at FROM users WHERE email = ?1",
                params![email],
                |row| {
                    Ok(crate::models::User {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        tier: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                    })
                },
            )
            .ok()
    }

    pub fn get_user_by_id(&self, id: &str) -> Option<crate::models::User> {
        self.conn
            .query_row(
                "SELECT id, email, display_name, tier, created_at FROM users WHERE id = ?1",
                params![id],
                |row| {
                    Ok(crate::models::User {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        tier: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                    })
                },
            )
            .ok()
    }

    // --- Universes ---

    pub fn create_universe(
        &mut self,
        create: crate::models::CreateUniverse,
        owner_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        if self.get_universe(&create.key).is_some() {
            anyhow::bail!("Universe '{}' already exists", create.key);
        }
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT INTO universes (key, name, description, owner_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![create.key, create.name, create.description, owner_id, now_str],
        )?;
        // Owner is automatically a member
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) VALUES (?1, ?2, 'owner', ?3)",
            params![create.key, owner_id, now_str],
        )?;
        Ok(crate::models::Universe {
            key: create.key,
            name: create.name,
            description: create.description,
            owner_id: owner_id.to_string(),
            created_at: now,
            is_template: false,
            is_public: false,
        })
    }

    pub fn get_universe(&self, key: &str) -> Option<crate::models::Universe> {
        self.conn
            .query_row(
                "SELECT key, name, description, owner_id, created_at, is_template, is_public \
                 FROM universes WHERE key = ?1",
                params![key],
                |row| {
                    Ok(crate::models::Universe {
                        key: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        owner_id: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                        is_template: row.get::<_, i64>(5)? != 0,
                        is_public: row.get::<_, i64>(6)? != 0,
                    })
                },
            )
            .ok()
    }

    pub fn list_universes_for_user(&self, user_id: &str) -> Vec<crate::models::Universe> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT u.key, u.name, u.description, u.owner_id, u.created_at, u.is_template, u.is_public \
                 FROM universes u \
                 JOIN universe_members um ON um.universe_key = u.key \
                 WHERE um.user_id = ?1 \
                 ORDER BY u.created_at ASC",
            )
            .expect("Failed to prepare list_universes_for_user");
        stmt.query_map(params![user_id], |row| {
            Ok(crate::models::Universe {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                owner_id: row.get(3)?,
                created_at: parse_datetime(&row.get::<_, String>(4)?),
                is_template: row.get::<_, i64>(5)? != 0,
                is_public: row.get::<_, i64>(6)? != 0,
            })
        })
        .expect("Failed to list universes for user")
        .filter_map(|r| r.ok())
        .collect()
    }

    // --- Universe Members ---

    pub fn is_universe_member(&self, universe_key: &str, user_id: &str) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
                params![universe_key, user_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    pub fn list_universe_members(&self, universe_key: &str) -> Vec<crate::models::UniverseMember> {
        let mut stmt = self.conn.prepare(
            "SELECT um.universe_key, um.user_id, um.role, um.joined_at, u.email, u.display_name \
             FROM universe_members um \
             LEFT JOIN users u ON um.user_id = u.id \
             WHERE um.universe_key = ?1 \
             ORDER BY um.joined_at ASC",
        ).expect("Failed to prepare list_universe_members");
        stmt.query_map(params![universe_key], |row| {
            Ok(crate::models::UniverseMember {
                universe_key: row.get(0)?,
                user_id: row.get(1)?,
                role: row.get(2)?,
                joined_at: parse_datetime(&row.get::<_, String>(3)?),
                email: row.get(4)?,
                display_name: row.get(5)?,
            })
        })
        .expect("Failed to list universe members")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn add_universe_member(
        &mut self,
        universe_key: &str,
        user_id: &str,
        role: &str,
    ) -> anyhow::Result<crate::models::UniverseMember> {
        if self.get_universe(universe_key).is_none() {
            anyhow::bail!("Universe '{}' not found", universe_key);
        }
        // Note: user_id may refer to a quilombo user (not in the users table) — no FK check.
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) VALUES (?1, ?2, ?3, ?4)",
            params![universe_key, user_id, role, now_str],
        )?;
        let member = self.conn.query_row(
            "SELECT um.universe_key, um.user_id, um.role, um.joined_at, u.email, u.display_name \
             FROM universe_members um LEFT JOIN users u ON um.user_id = u.id \
             WHERE um.universe_key = ?1 AND um.user_id = ?2",
            params![universe_key, user_id],
            |row| {
                Ok(crate::models::UniverseMember {
                    universe_key: row.get(0)?,
                    user_id: row.get(1)?,
                    role: row.get(2)?,
                    joined_at: parse_datetime(&row.get::<_, String>(3)?),
                    email: row.get(4)?,
                    display_name: row.get(5)?,
                })
            },
        )?;
        Ok(member)
    }

    pub fn remove_universe_member(
        &mut self,
        universe_key: &str,
        user_id: &str,
    ) -> anyhow::Result<()> {
        // Prevent removing the owner
        let universe = self
            .get_universe(universe_key)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", universe_key))?;
        if universe.owner_id == user_id {
            anyhow::bail!("Cannot remove the owner from a universe");
        }
        self.conn.execute(
            "DELETE FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
            params![universe_key, user_id],
        )?;
        Ok(())
    }

    // --- Check if data exists ---

    pub fn has_data(&self) -> bool {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .unwrap_or(0);
        count > 0
    }

    // --- Template universe ---

    /// Returns true if a template universe already exists (seed already ran).
    pub fn template_exists(&self) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM universes WHERE is_template = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    /// Seed the template universe with "Meu Projeto" and 8 sample tasks.
    /// Safe to call multiple times — uses INSERT OR IGNORE.
    pub fn seed_template_universe(&mut self) {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Template universe
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public) \
             VALUES ('template', 'CO', \
             'Universo de demonstração — todas as funcionalidades do CO', \
             'system', ?1, 1, 1)",
            params![now_str],
        );

        // Sample project
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO projects \
             (key, name, description, next_id, created_at, universe_key) \
             VALUES ('MP', 'Meu Projeto', \
             'Projeto de demonstração com todas as funcionalidades do CO', \
             9, ?1, 'template')",
            params![now_str],
        );

        // 8 sample tasks showcasing all features
        struct SeedTask {
            id: i64,
            title: &'static str,
            description: &'static str,
            status: &'static str,
            priority: &'static str,
            labels: &'static str,
            due_days: Option<i64>, // positive = future, negative = past
            parent: Option<i64>,
        }

        let tasks = [
            SeedTask {
                id: 1,
                title: "Configurar ambiente de desenvolvimento",
                description: "Configurar Docker, banco de dados local e variáveis de ambiente.",
                status: "done",
                priority: "high",
                labels: r#"["setup","infra"]"#,
                due_days: Some(-20),
                parent: None,
            },
            SeedTask {
                id: 2,
                title: "Desenho da arquitetura",
                description: "Definir arquitetura, escolher tecnologias e documentar decisões.",
                status: "done",
                priority: "critical",
                labels: r#"["architecture","docs"]"#,
                due_days: Some(-15),
                parent: None,
            },
            SeedTask {
                id: 3,
                title: "Implementar autenticação",
                description: "Implementar JWT, refresh tokens e logout seguro.",
                status: "in_progress",
                priority: "critical",
                labels: r#"["backend","security"]"#,
                due_days: Some(7),
                parent: None,
            },
            SeedTask {
                id: 4,
                title: "Escrever testes de integração",
                description: "Cobrir endpoints de autenticação com testes de integração.",
                status: "todo",
                priority: "medium",
                labels: r#"["testing","backend"]"#,
                due_days: Some(14),
                parent: Some(3),
            },
            SeedTask {
                id: 5,
                title: "Design da interface",
                description: "Criar wireframes e protótipos para os principais fluxos.",
                status: "in_review",
                priority: "high",
                labels: r#"["frontend","design","ux"]"#,
                due_days: Some(5),
                parent: None,
            },
            SeedTask {
                id: 6,
                title: "Implementar componentes de UI",
                description: "Desenvolver componentes reutilizáveis baseados no design aprovado.",
                status: "in_progress",
                priority: "medium",
                labels: r#"["frontend"]"#,
                due_days: Some(10),
                parent: Some(5),
            },
            SeedTask {
                id: 7,
                title: "Deploy de homologação",
                description: "Configurar CI/CD e fazer deploy no ambiente de homologação.",
                status: "todo",
                priority: "low",
                labels: r#"["devops","infra"]"#,
                due_days: Some(21),
                parent: None,
            },
            SeedTask {
                id: 8,
                title: "Documentação da API",
                description: "Documentar todos os endpoints com exemplos de requisição e resposta.",
                status: "todo",
                priority: "low",
                labels: r#"["docs","backend"]"#,
                due_days: Some(30),
                parent: None,
            },
        ];

        for t in &tasks {
            let created_at = (now - chrono::Duration::days(30 - t.id * 3)).to_rfc3339();
            let updated_at = (now - chrono::Duration::days(5)).to_rfc3339();
            let due_date: Option<String> = t.due_days.map(|d| {
                (now + chrono::Duration::days(d))
                    .format("%Y-%m-%d")
                    .to_string()
            });
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO tasks \
                 (project_key, id, title, description, status, priority, \
                  due_date, parent, labels, created_at, updated_at, archived) \
                 VALUES ('MP', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
                params![
                    t.id,
                    t.title,
                    t.description,
                    t.status,
                    t.priority,
                    due_date,
                    t.parent,
                    t.labels,
                    created_at,
                    updated_at,
                ],
            );
        }
    }

    /// Returns true if the given project belongs to a template universe.
    pub fn is_project_in_template(&self, project_key: &str) -> bool {
        let upper = project_key.to_uppercase();
        // Get universe_key for the project, then check is_template
        let universe_key: Option<String> = self
            .conn
            .query_row(
                "SELECT universe_key FROM projects WHERE key = ?1",
                params![upper],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        match universe_key {
            None => false,
            Some(ukey) => {
                let v: i64 = self
                    .conn
                    .query_row(
                        "SELECT is_template FROM universes WHERE key = ?1",
                        params![ukey],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                v != 0
            }
        }
    }

    /// List projects for a universe that has `is_public = 1`.
    /// Returns Err if the universe doesn't exist or is not public.
    pub fn list_projects_for_public_universe(
        &self,
        universe_key: &str,
    ) -> anyhow::Result<Vec<crate::models::Project>> {
        let is_public: i64 = self
            .conn
            .query_row(
                "SELECT is_public FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("Universe '{}' not found", universe_key))?;

        if is_public == 0 {
            anyhow::bail!("Universe '{}' is not public", universe_key);
        }

        Ok(self.list_projects_for_universe(universe_key))
    }

    /// Clone a universe: copy all its projects and tasks into a new universe.
    /// The new universe is NOT a template and is private by default.
    pub fn clone_universe(
        &mut self,
        source_key: &str,
        new_key: &str,
        new_name: &str,
        description: &str,
        owner_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        if self.get_universe(new_key).is_some() {
            anyhow::bail!("Universe '{}' already exists", new_key);
        }
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Create the new universe
        self.conn.execute(
            "INSERT INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
            params![new_key, new_name, description, owner_id, now_str],
        )?;

        // Add owner as member
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members \
             (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', ?3)",
            params![new_key, owner_id, now_str],
        )?;

        // Collect source projects
        let source_projects: Vec<(String, String, String, i64)> = {
            let mut stmt = self.conn.prepare(
                "SELECT key, name, description, next_id \
                 FROM projects WHERE universe_key = ?1",
            )?;
            stmt.query_map(params![source_key], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect()
        };

        for (old_pkey, pname, pdesc, next_id) in &source_projects {
            let new_pkey = self.derive_unique_project_key(new_key, old_pkey);

            self.conn.execute(
                "INSERT INTO projects \
                 (key, name, description, next_id, created_at, universe_key) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![new_pkey, pname, pdesc, next_id, now_str, new_key],
            )?;

            // Copy tasks (collect as raw SQL, then re-insert under new project key)
            type TaskRow = (
                i64,
                String,
                String,
                String,
                String,
                Option<String>,
                Option<i64>,
                String,
            );
            let task_rows: Vec<TaskRow> = {
                let mut stmt = self.conn.prepare(
                    "SELECT id, title, description, status, priority, \
                     due_date, parent, labels \
                     FROM tasks WHERE project_key = ?1 AND archived = 0",
                )?;
                stmt.query_map(params![old_pkey], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect()
            };

            for (id, title, desc, status, priority, due_date, parent, labels) in task_rows {
                self.conn.execute(
                    "INSERT INTO tasks \
                     (project_key, id, title, description, status, priority, \
                      due_date, parent, labels, created_at, updated_at, archived) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
                    params![
                        new_pkey, id, title, desc, status, priority, due_date, parent, labels,
                        now_str, now_str,
                    ],
                )?;
            }
        }

        Ok(crate::models::Universe {
            key: new_key.to_string(),
            name: new_name.to_string(),
            description: description.to_string(),
            owner_id: owner_id.to_string(),
            created_at: now,
            is_template: false,
            is_public: false,
        })
    }

    /// Derive a unique project key for a clone, based on the universe key + original project key.
    fn derive_unique_project_key(&self, universe_key: &str, original_key: &str) -> String {
        // Take up to 4 alphanumeric chars from universe key (uppercase) + original key, max 10
        let prefix: String = universe_key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(4)
            .collect::<String>()
            .to_uppercase();
        let base: String = format!("{}{}", prefix, original_key)
            .chars()
            .take(10)
            .collect();

        // If unique, use as-is; otherwise append a number
        if self.get_project(&base).is_none() {
            return base;
        }
        for i in 2u32..=99 {
            let candidate: String = format!("{}{}", base, i).chars().take(10).collect();
            if self.get_project(&candidate).is_none() {
                return candidate;
            }
        }
        // Fallback: uuid-based suffix (shouldn't happen in practice)
        format!("{}{}", &base[..6.min(base.len())], nanoid::nanoid!(4))
            .chars()
            .take(10)
            .collect()
    }
}

// --- Parsing helpers ---

pub fn parse_datetime(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_status(s: &str) -> TaskStatus {
    match s {
        "todo" => TaskStatus::Todo,
        "in_progress" => TaskStatus::InProgress,
        "in_review" => TaskStatus::InReview,
        "done" => TaskStatus::Done,
        _ => TaskStatus::Todo,
    }
}

fn parse_priority(s: &str) -> Priority {
    match s {
        "low" => Priority::Low,
        "medium" => Priority::Medium,
        "high" => Priority::High,
        "critical" => Priority::Critical,
        _ => Priority::Medium,
    }
}

fn parse_labels(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

// --- Seed Data ---

pub fn seed_data(storage: &mut Storage) {
    use chrono::NaiveDate;

    let ds = CreateProject {
        name: "Design System".into(),
        key: "DS".into(),
        description: "Shared component library and design tokens".into(),
        ..Default::default()
    };
    storage.create_project(ds).unwrap();

    let api = CreateProject {
        name: "Backend API".into(),
        key: "API".into(),
        description: "Core REST API and data services".into(),
        ..Default::default()
    };
    storage.create_project(api).unwrap();

    // --- Design System tasks ---
    let ds_tasks = vec![
        CreateTask {
            title: "Define visual identity".into(),
            description: "Create logo, color palette, and typography for the design system.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 1),
            parent: None,
            labels: vec!["design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Build component showcase".into(),
            description: "Develop a web-based showcase of all available components and patterns."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 15),
            parent: None,
            labels: vec!["web".into(), "design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Organize first design review".into(),
            description:
                "Schedule review session, prepare demos, and gather feedback from stakeholders."
                    .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 20),
            parent: None,
            labels: vec!["review".into()],
            assignee: None,
        },
        CreateTask {
            title: "Produce component catalog".into(),
            description:
                "Document each component with usage examples, props, and accessibility notes."
                    .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 1),
            parent: None,
            labels: vec!["docs".into()],
            assignee: None,
        },
        CreateTask {
            title: "Set up documentation site".into(),
            description: "Deploy a static site with guidelines and a monthly content calendar."
                .into(),
            status: TaskStatus::Done,
            priority: Priority::Low,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 10),
            parent: None,
            labels: vec!["marketing".into()],
            assignee: None,
        },
        CreateTask {
            title: "Select color palette".into(),
            description: "Define primary and secondary colors aligned with the project identity."
                .into(),
            status: TaskStatus::InReview,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 25),
            parent: Some(1),
            labels: vec!["design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Design logo".into(),
            description: "Create 3 logo proposals for team vote.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 28),
            parent: Some(1),
            labels: vec!["design".into()],
            assignee: None,
        },
    ];

    for task in ds_tasks {
        storage.create_task("ds", task).unwrap();
    }

    // --- Backend API tasks ---
    let api_tasks = vec![
        CreateTask {
            title: "Database schema design".into(),
            description: "Design and document the relational schema for all core entities.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 30),
            parent: None,
            labels: vec!["database".into(), "urgent".into()],
            assignee: None,
        },
        CreateTask {
            title: "API documentation".into(),
            description: "Write OpenAPI specs and usage guides for every endpoint.".into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 15),
            parent: None,
            labels: vec!["docs".into()],
            assignee: None,
        },
        CreateTask {
            title: "Authentication module".into(),
            description: "Implement JWT-based auth with refresh tokens and role-based access."
                .into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 1),
            parent: None,
            labels: vec!["security".into(), "auth".into()],
            assignee: None,
        },
        CreateTask {
            title: "Rate limiting and throttling".into(),
            description: "Add per-endpoint rate limits and IP-based throttling to protect the API."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 7, 1),
            parent: None,
            labels: vec!["security".into()],
            assignee: None,
        },
        CreateTask {
            title: "CI/CD pipeline setup".into(),
            description:
                "Configure automated testing, linting, and deployment for the API service.".into(),
            status: TaskStatus::InReview,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 15),
            parent: None,
            labels: vec!["devops".into()],
            assignee: None,
        },
        CreateTask {
            title: "Write migration scripts".into(),
            description: "Create versioned SQL migrations for the initial schema.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 10),
            parent: Some(1),
            labels: vec!["database".into()],
            assignee: None,
        },
        CreateTask {
            title: "Integration test suite".into(),
            description: "Build end-to-end tests covering all critical API workflows.".into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 1),
            parent: Some(3),
            labels: vec!["testing".into()],
            assignee: None,
        },
        CreateTask {
            title: "Load testing workshop".into(),
            description:
                "Run load tests to identify bottlenecks and establish performance baselines.".into(),
            status: TaskStatus::Done,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 8),
            parent: Some(1),
            labels: vec!["testing".into(), "performance".into()],
            assignee: None,
        },
    ];

    for task in api_tasks {
        storage.create_task("api", task).unwrap();
    }

    // --- Platform ---
    let plt = CreateProject {
        name: "Platform".into(),
        key: "PLT".into(),
        description: "Unified platform for management and collaboration".into(),
        ..Default::default()
    };
    storage.create_project(plt).unwrap();

    let plt_tasks = vec![
        CreateTask {
            title: "Initial Launch".into(),
            description: "Launch epic: prepare and publish the first versions of the product."
                .into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 30),
            parent: None,
            labels: vec!["epic".into(), "launch".into()],
            assignee: None,
        },
        CreateTask {
            title: "Internal MVP".into(),
            description: "Minimum viable version for internal team use. Validate core \
                           workflows, identify critical bugs, and collect feedback before \
                           the public launch."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 15),
            parent: Some(1),
            labels: vec!["mvp".into()],
            assignee: None,
        },
        CreateTask {
            title: "Public MVP".into(),
            description: "First public version of the product. Incorporate fixes from the \
                           internal MVP, prepare onboarding, documentation, and production \
                           infrastructure."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 30),
            parent: Some(1),
            labels: vec!["mvp".into(), "public".into()],
            assignee: None,
        },
    ];

    for task in plt_tasks {
        storage.create_task("plt", task).unwrap();
    }
}
