use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
use rusqlite::{Connection, params};
use serde_json::json;

use crate::entry_index::{EntryRow, make_entry};
use crate::models::*;

pub struct Storage {
    conn: Connection,
    pub data_dir: PathBuf,
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

        let mut storage = Self {
            conn,
            data_dir: data_dir.as_ref().to_path_buf(),
        };
        storage.run_migrations();
        storage
    }

    /// Returns the root directory for a universe's .md files.
    pub fn universe_root(&self, universe_key: &str) -> PathBuf {
        self.data_dir.join("universes").join(universe_key)
    }

    /// Access the underlying SQLite connection (for quilombo storage functions).
    pub fn conn(&self) -> &Connection {
        &self.conn
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

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 11 {
            self.conn
                .execute_batch(
                    "ALTER TABLE universes ADD COLUMN content_count INTEGER NOT NULL DEFAULT 0;
                     INSERT INTO schema_version (version) VALUES (11);",
                )
                .expect("Failed to run migration v11");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 12 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS entries (
                    path TEXT NOT NULL,
                    universe_key TEXT NOT NULL,
                    entry_type TEXT NOT NULL,
                    title TEXT,
                    frontmatter_json TEXT NOT NULL,
                    body TEXT NOT NULL DEFAULT '',
                    body_hash TEXT NOT NULL,
                    created_at TEXT,
                    updated_at TEXT,
                    PRIMARY KEY (universe_key, path)
                );
                CREATE INDEX IF NOT EXISTS idx_entries_type ON entries(universe_key, entry_type);
                CREATE INDEX IF NOT EXISTS idx_entries_updated ON entries(universe_key, updated_at);
                CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
                    universe_key UNINDEXED,
                    path UNINDEXED,
                    title,
                    body
                );
                INSERT INTO schema_version (version) VALUES (12);
                ",
                )
                .expect("Failed to run migration v12");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 13 {
            // Create default universe
            let _ = self.conn.execute_batch(
                "INSERT OR IGNORE INTO universes (key, name, description, owner_id, created_at, is_template, is_public, content_count) \
                 VALUES ('default', 'Default', 'Default universe', 'system', datetime('now'), 0, 0, 0);",
            );

            // Check if projects table exists
            let projects_exist: bool = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='projects'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;

            if projects_exist {
                self.migrate_old_data_to_entries();
            }

            // Drop old tables
            self.conn
                .execute_batch(
                    "DROP TABLE IF EXISTS tasks; \
                     DROP TABLE IF EXISTS comments; \
                     DROP TABLE IF EXISTS projects; \
                     DROP TABLE IF EXISTS activity_log; \
                     INSERT INTO schema_version (version) VALUES (13);",
                )
                .expect("Failed migration v13");
        }
    }

    /// Migrate data from old projects/tasks/comments tables into the entries table + .md files.
    fn migrate_old_data_to_entries(&mut self) {
        // Collect projects
        struct OldProject {
            key: String,
            name: String,
            description: String,
            next_id: i64,
            created_at: String,
            archived: i64,
            universe_key: Option<String>,
        }

        let old_projects: Vec<OldProject> = {
            let mut stmt = match self.conn.prepare(
                "SELECT key, name, description, next_id, created_at, archived, universe_key FROM projects",
            ) {
                Ok(s) => s,
                Err(_) => return,
            };
            match stmt.query_map([], |row| {
                Ok(OldProject {
                    key: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    next_id: row.get::<_, i64>(3)?,
                    created_at: row.get::<_, String>(4)?,
                    archived: row.get::<_, i64>(5)?,
                    universe_key: row.get::<_, Option<String>>(6)?,
                })
            }) {
                Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                Err(_) => return,
            }
        };

        for proj in &old_projects {
            let universe_key = proj
                .universe_key
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let universe_root = self.data_dir.join("universes").join(&universe_key);
            let path = format!("projects/{}/_project.md", proj.key);
            let fm = json!({
                "type": "project",
                "key": proj.key,
                "title": proj.name,
                "status": "active",
                "next_id": proj.next_id,
                "created": proj.created_at,
                "modified": proj.created_at,
                "archived": proj.archived != 0,
                "tags": []
            });
            let entry = make_entry(&path, fm.clone(), &proj.description);
            let _ = co::entry::write_entry(&universe_root, &entry);
            let _ = upsert_entry_row(&self.conn, &universe_key, &entry);

            // Collect tasks for this project
            struct OldTask {
                id: i64,
                title: String,
                description: String,
                status: String,
                priority: String,
                due_date: Option<String>,
                parent: Option<i64>,
                labels: String,
                created_at: String,
                updated_at: String,
                archived: i64,
                assignee: Option<String>,
            }

            let old_tasks: Vec<OldTask> = {
                let mut stmt = match self.conn.prepare(
                    "SELECT id, title, description, status, priority, due_date, parent, labels, \
                     created_at, updated_at, archived, assignee FROM tasks WHERE project_key = ?1",
                ) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                match stmt.query_map(params![proj.key], |row| {
                    Ok(OldTask {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        description: row.get(2)?,
                        status: row.get(3)?,
                        priority: row.get(4)?,
                        due_date: row.get(5)?,
                        parent: row.get(6)?,
                        labels: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        archived: row.get::<_, i64>(10)?,
                        assignee: row.get(11)?,
                    })
                }) {
                    Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                    Err(_) => continue,
                }
            };

            for task in &old_tasks {
                let task_path = format!("projects/{}/{}.md", proj.key, task.id);
                let labels: Vec<String> = serde_json::from_str(&task.labels).unwrap_or_default();
                let task_fm = json!({
                    "type": "task",
                    "id": task.id,
                    "title": task.title,
                    "status": task.status,
                    "priority": task.priority,
                    "due": task.due_date,
                    "parent": task.parent,
                    "tags": labels,
                    "created": task.created_at,
                    "modified": task.updated_at,
                    "archived": task.archived != 0,
                    "assignee": task.assignee,
                    "project": proj.key
                });
                let task_entry = make_entry(&task_path, task_fm, &task.description);
                let _ = co::entry::write_entry(&universe_root, &task_entry);
                let _ = upsert_entry_row(&self.conn, &universe_key, &task_entry);

                // Collect comments for this task
                struct OldComment {
                    id: i64,
                    author: String,
                    body: String,
                    created_at: String,
                }

                let old_comments: Vec<OldComment> = {
                    let mut stmt = match self.conn.prepare(
                        "SELECT id, author, body, created_at FROM comments \
                         WHERE project_key = ?1 AND task_id = ?2",
                    ) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    match stmt.query_map(params![proj.key, task.id], |row| {
                        Ok(OldComment {
                            id: row.get(0)?,
                            author: row.get(1)?,
                            body: row.get(2)?,
                            created_at: row.get(3)?,
                        })
                    }) {
                        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                        Err(_) => continue,
                    }
                };

                for comment in &old_comments {
                    let comment_path = format!(
                        "projects/{}/comments/{}-{}.md",
                        proj.key, task.id, comment.id
                    );
                    let comment_fm = json!({
                        "type": "comment",
                        "id": comment.id,
                        "task": task.id,
                        "project": proj.key,
                        "author": comment.author,
                        "created": comment.created_at,
                        "modified": comment.created_at,
                        "tags": []
                    });
                    let comment_entry = make_entry(&comment_path, comment_fm, &comment.body);
                    let _ = co::entry::write_entry(&universe_root, &comment_entry);
                    let _ = upsert_entry_row(&self.conn, &universe_key, &comment_entry);
                }
            }
        }
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
            .prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries WHERE entry_type = 'project' ORDER BY path",
            )
            .expect("Failed to prepare list_projects");

        stmt.query_map([], entry_row_from_sql)
            .expect("Failed to list projects")
            .filter_map(|r| r.ok())
            .filter_map(|row| entry_row_to_project(&row))
            .collect()
    }

    pub fn list_projects_for_universe(&self, universe_key: &str) -> Vec<Project> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries \
                 WHERE universe_key = ?1 AND entry_type = 'project' ORDER BY path",
            )
            .expect("Failed to prepare list_projects_for_universe");

        stmt.query_map(params![universe_key], entry_row_from_sql)
            .expect("Failed to list projects for universe")
            .filter_map(|r| r.ok())
            .filter_map(|row| entry_row_to_project(&row))
            .collect()
    }

    pub fn get_project(&self, key: &str) -> Option<Project> {
        let upper_key = key.to_uppercase();
        let path = format!("projects/{}/_project.md", upper_key);
        let result = self.conn.query_row(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE path = ?1 AND entry_type = 'project'",
            params![path],
            entry_row_from_sql,
        );
        match result {
            Ok(row) => entry_row_to_project(&row),
            Err(_) => None,
        }
    }

    pub fn create_project(&mut self, create: CreateProject) -> anyhow::Result<Project> {
        let upper_key = create.key.to_uppercase();
        if self.get_project(&upper_key).is_some() {
            anyhow::bail!("Project with key '{}' already exists", upper_key);
        }

        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let universe_key = create
            .universe_key
            .clone()
            .unwrap_or_else(|| "default".to_string());

        let path = format!("projects/{}/_project.md", upper_key);
        let fm = json!({
            "type": "project",
            "key": upper_key,
            "title": create.name,
            "status": "active",
            "next_id": 1,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "tags": []
        });

        let entry = make_entry(&path, fm, &create.description);
        let universe_root = self.universe_root(&universe_key);
        co::entry::write_entry(&universe_root, &entry)?;
        upsert_entry_row(&self.conn, &universe_key, &entry)?;

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

        // Find the universe_key
        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());
        let universe_root = self.universe_root(&universe_key);

        // Find all entries under this project
        let prefix = format!("projects/{}/", upper_key);
        let entry_paths: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT path FROM entries WHERE universe_key = ?1 AND path LIKE ?2")?;
            let like_pattern = format!("{}%", prefix);
            stmt.query_map(params![universe_key, like_pattern], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        for entry_path in &entry_paths {
            let _ = co::entry::delete_entry(&universe_root, entry_path);
            self.conn.execute(
                "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
                params![universe_key, entry_path],
            )?;
        }

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
                let mut stmt = self
                    .conn
                    .prepare(&sql)
                    .expect("Failed to prepare list_tasks");
                rows = stmt
                    .query_map(
                        params![upper_key, limit as i64, offset as i64],
                        entry_row_from_sql,
                    )
                    .expect("Failed to list tasks")
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
                let mut stmt = self
                    .conn
                    .prepare(&sql)
                    .expect("Failed to prepare list_tasks");
                rows = stmt
                    .query_map(
                        params![upper_key, limit as i64, offset as i64],
                        entry_row_from_sql,
                    )
                    .expect("Failed to list tasks")
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
        let result = self.conn.query_row(
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
        upsert_entry_row(&self.conn, &universe_key, &entry)?;

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
        upsert_entry_row(&self.conn, &universe_key, &entry)?;

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
        self.conn.execute(
            "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
            params![universe_key, path],
        )?;

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
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries \
                 WHERE entry_type = 'comment' AND path LIKE ?1 \
                 ORDER BY created_at ASC",
            )
            .expect("Failed to prepare list_comments");

        let like_pattern = format!("{}%", path_prefix);
        stmt.query_map(params![like_pattern], entry_row_from_sql)
            .expect("Failed to list comments")
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

        // Allocate id via COUNT
        let id: u64 = {
            let count: i64 = self
                .conn
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
        upsert_entry_row(&self.conn, &universe_key, &entry)?;

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

    pub fn get_dashboard(&self, project_key: &str) -> DashboardData {
        let upper_key = project_key.to_uppercase();

        let status_counts = self.get_status_counts(&upper_key);

        let today_str = chrono::Utc::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        let overdue_count: i64 = self
            .conn
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
            .unwrap_or(0);

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
        let today = chrono::Utc::now().date_naive();
        let mut result = Vec::with_capacity(8);

        for week_offset in (0i64..8).rev() {
            let week_end = today - chrono::Duration::weeks(week_offset);
            let week_label = week_end.format("%Y-W%V").to_string();
            let week_end_str = week_end.to_string();

            let total_created: i64 = self
                .conn
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
        let count = |status: &str| -> u64 {
            self.conn
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
            content_count: 0,
        })
    }

    pub fn get_universe(&self, key: &str) -> Option<crate::models::Universe> {
        self.conn
            .query_row(
                "SELECT key, name, description, owner_id, created_at, is_template, is_public, content_count \
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
                        content_count: row.get::<_, i64>(7).unwrap_or(0),
                    })
                },
            )
            .ok()
    }

    pub fn list_universes_for_user(&self, user_id: &str) -> Vec<crate::models::Universe> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT u.key, u.name, u.description, u.owner_id, u.created_at, u.is_template, u.is_public, u.content_count \
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
                content_count: row.get::<_, i64>(7).unwrap_or(0),
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

    // --- Usage gate / content count ---

    /// Return the universe_key for a given project key, or None if not found.
    pub fn get_project_universe_key(&self, project_key: &str) -> Option<String> {
        let upper = project_key.to_uppercase();
        let path = format!("projects/{}/_project.md", upper);
        self.conn
            .query_row(
                "SELECT universe_key FROM entries WHERE entry_type = 'project' AND path = ?1",
                params![path],
                |row| row.get(0),
            )
            .ok()
    }

    /// Increment content_count for a universe and return the new value.
    pub fn increment_universe_content_count(&mut self, universe_key: &str) -> i64 {
        self.conn
            .execute(
                "UPDATE universes SET content_count = content_count + 1 WHERE key = ?1",
                params![universe_key],
            )
            .ok();
        self.conn
            .query_row(
                "SELECT content_count FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Decrement content_count for a universe by `by`, flooring at 0.
    pub fn decrement_universe_content_count(&mut self, universe_key: &str, by: i64) {
        self.conn
            .execute(
                "UPDATE universes SET content_count = MAX(0, content_count - ?1) WHERE key = ?2",
                params![by, universe_key],
            )
            .ok();
    }

    /// Count comments for a specific task.
    pub fn count_task_comments(&self, project_key: &str, task_id: u64) -> i64 {
        let upper = project_key.to_uppercase();
        let like_pattern = format!("projects/{}/comments/{}-%%", upper, task_id);
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE entry_type = 'comment' AND path LIKE ?1",
                params![like_pattern],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Count all tasks and their comments for a project (used for delete_project decrement).
    pub fn count_project_content(&self, project_key: &str) -> i64 {
        let upper = project_key.to_uppercase();
        let tasks: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries \
                 WHERE entry_type = 'task' \
                 AND json_extract(frontmatter_json, '$.project') = ?1",
                params![upper],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let comments: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries \
                 WHERE entry_type = 'comment' \
                 AND json_extract(frontmatter_json, '$.project') = ?1",
                params![upper],
                |row| row.get(0),
            )
            .unwrap_or(0);
        tasks + comments
    }

    /// Claim an anonymous universe: transfer ownership to a real user.
    /// `anon_id` must match the universe's current owner_id (must start with "anon-").
    pub fn claim_universe(
        &mut self,
        slug: &str,
        user_id: &str,
        anon_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        let universe = self
            .get_universe(slug)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", slug))?;

        if !universe.owner_id.starts_with("anon-") {
            anyhow::bail!("Universe '{}' is not an anonymous universe", slug);
        }
        if universe.owner_id != anon_id {
            anyhow::bail!("Owner cookie does not match universe owner");
        }

        let now_str = Utc::now().to_rfc3339();

        // Transfer ownership
        self.conn.execute(
            "UPDATE universes SET owner_id = ?1 WHERE key = ?2",
            params![user_id, slug],
        )?;

        // Replace anon member with real user
        self.conn.execute(
            "DELETE FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
            params![slug, anon_id],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', ?3)",
            params![slug, user_id, now_str],
        )?;

        self.get_universe(slug)
            .ok_or_else(|| anyhow::anyhow!("Universe not found after claim"))
    }

    // --- Check if data exists ---

    pub fn has_data(&self) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE entry_type = 'project'",
                [],
                |row| row.get(0),
            )
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
    /// Safe to call multiple times — checks if project entry already exists.
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

        // Check if project entry already exists
        let proj_path = "projects/MP/_project.md";
        let already_seeded: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = 'template' AND path = ?1",
                params![proj_path],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if already_seeded {
            return;
        }

        // Create sample project entry
        let proj_fm = json!({
            "type": "project",
            "key": "MP",
            "title": "Meu Projeto",
            "status": "active",
            "next_id": 9,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "tags": []
        });
        let proj_entry = make_entry(
            proj_path,
            proj_fm,
            "Projeto de demonstração com todas as funcionalidades do CO",
        );
        let universe_root = self.universe_root("template");
        let _ = co::entry::write_entry(&universe_root, &proj_entry);
        let _ = upsert_entry_row(&self.conn, "template", &proj_entry);

        // 8 sample tasks
        struct SeedTask {
            id: i64,
            title: &'static str,
            description: &'static str,
            status: &'static str,
            priority: &'static str,
            labels: Vec<&'static str>,
            due_days: Option<i64>,
            parent: Option<i64>,
        }

        let tasks = [
            SeedTask {
                id: 1,
                title: "Configurar ambiente de desenvolvimento",
                description: "Configurar Docker, banco de dados local e variáveis de ambiente.",
                status: "done",
                priority: "high",
                labels: vec!["setup", "infra"],
                due_days: Some(-20),
                parent: None,
            },
            SeedTask {
                id: 2,
                title: "Desenho da arquitetura",
                description: "Definir arquitetura, escolher tecnologias e documentar decisões.",
                status: "done",
                priority: "critical",
                labels: vec!["architecture", "docs"],
                due_days: Some(-15),
                parent: None,
            },
            SeedTask {
                id: 3,
                title: "Implementar autenticação",
                description: "Implementar JWT, refresh tokens e logout seguro.",
                status: "in_progress",
                priority: "critical",
                labels: vec!["backend", "security"],
                due_days: Some(7),
                parent: None,
            },
            SeedTask {
                id: 4,
                title: "Escrever testes de integração",
                description: "Cobrir endpoints de autenticação com testes de integração.",
                status: "todo",
                priority: "medium",
                labels: vec!["testing", "backend"],
                due_days: Some(14),
                parent: Some(3),
            },
            SeedTask {
                id: 5,
                title: "Design da interface",
                description: "Criar wireframes e protótipos para os principais fluxos.",
                status: "in_review",
                priority: "high",
                labels: vec!["frontend", "design", "ux"],
                due_days: Some(5),
                parent: None,
            },
            SeedTask {
                id: 6,
                title: "Implementar componentes de UI",
                description: "Desenvolver componentes reutilizáveis baseados no design aprovado.",
                status: "in_progress",
                priority: "medium",
                labels: vec!["frontend"],
                due_days: Some(10),
                parent: Some(5),
            },
            SeedTask {
                id: 7,
                title: "Deploy de homologação",
                description: "Configurar CI/CD e fazer deploy no ambiente de homologação.",
                status: "todo",
                priority: "low",
                labels: vec!["devops", "infra"],
                due_days: Some(21),
                parent: None,
            },
            SeedTask {
                id: 8,
                title: "Documentação da API",
                description: "Documentar todos os endpoints com exemplos de requisição e resposta.",
                status: "todo",
                priority: "low",
                labels: vec!["docs", "backend"],
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
            let task_path = format!("projects/MP/{}.md", t.id);
            let labels: Vec<serde_json::Value> = t.labels.iter().map(|l| json!(l)).collect();
            let task_fm = json!({
                "type": "task",
                "id": t.id,
                "title": t.title,
                "status": t.status,
                "priority": t.priority,
                "due": due_date,
                "parent": t.parent,
                "tags": labels,
                "created": created_at,
                "modified": updated_at,
                "archived": false,
                "assignee": null,
                "project": "MP"
            });
            let task_entry = make_entry(&task_path, task_fm, t.description);
            let _ = co::entry::write_entry(&universe_root, &task_entry);
            let _ = upsert_entry_row(&self.conn, "template", &task_entry);
        }
    }

    /// Returns true if the given project belongs to a template universe.
    pub fn is_project_in_template(&self, project_key: &str) -> bool {
        let universe_key = match self.get_project_universe_key(project_key) {
            Some(k) => k,
            None => return false,
        };

        let v: i64 = self
            .conn
            .query_row(
                "SELECT is_template FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .unwrap_or(0);
        v != 0
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

        let mut cloned_entries: i64 = 0;

        // Collect source project entries
        let source_project_rows: Vec<EntryRow> = {
            let mut stmt = self.conn.prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries \
                 WHERE universe_key = ?1 AND entry_type = 'project'",
            )?;
            stmt.query_map(params![source_key], entry_row_from_sql)?
                .filter_map(|r| r.ok())
                .collect()
        };

        let new_universe_root = self.data_dir.join("universes").join(new_key);

        for proj_row in &source_project_rows {
            let old_pkey = proj_row
                .frontmatter
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let new_pkey = self.derive_unique_project_key(new_key, &old_pkey);

            // Build new project entry
            let new_proj_path = format!("projects/{}/_project.md", new_pkey);
            let mut new_proj_fm = proj_row.frontmatter.clone();
            if let Some(obj) = new_proj_fm.as_object_mut() {
                obj.insert("key".to_string(), json!(new_pkey));
                obj.insert("created".to_string(), json!(now_str));
                obj.insert("modified".to_string(), json!(now_str));
            }
            let new_proj_entry = make_entry(&new_proj_path, new_proj_fm, &proj_row.body);
            co::entry::write_entry(&new_universe_root, &new_proj_entry)?;
            upsert_entry_row(&self.conn, new_key, &new_proj_entry)?;
            cloned_entries += 1;

            // Collect source task entries
            let source_task_rows: Vec<EntryRow> = {
                let mut stmt = self.conn.prepare(
                    "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                     created_at, updated_at FROM entries \
                     WHERE universe_key = ?1 AND entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?2 \
                     AND json_extract(frontmatter_json, '$.archived') = 0",
                )?;
                stmt.query_map(params![source_key, old_pkey], entry_row_from_sql)?
                    .filter_map(|r| r.ok())
                    .collect()
            };

            for task_row in &source_task_rows {
                let task_id = task_row
                    .frontmatter
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let new_task_path = format!("projects/{}/{}.md", new_pkey, task_id);
                let mut new_task_fm = task_row.frontmatter.clone();
                if let Some(obj) = new_task_fm.as_object_mut() {
                    obj.insert("project".to_string(), json!(new_pkey));
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let new_task_entry = make_entry(&new_task_path, new_task_fm, &task_row.body);
                co::entry::write_entry(&new_universe_root, &new_task_entry)?;
                upsert_entry_row(&self.conn, new_key, &new_task_entry)?;
                cloned_entries += 1;
            }
        }

        // Set content_count
        self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = ?2",
            params![cloned_entries, new_key],
        )?;

        Ok(crate::models::Universe {
            key: new_key.to_string(),
            name: new_name.to_string(),
            description: description.to_string(),
            owner_id: owner_id.to_string(),
            created_at: now,
            is_template: false,
            is_public: false,
            content_count: cloned_entries,
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

    // --- List universe keys a user is a member of ---

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
        let universe_keys = self.list_user_universes(user_id);
        let mut result = Vec::new();
        for uk in &universe_keys {
            result.extend(self.list_projects_for_universe(uk));
        }
        result
    }

    // --- Increment project next_id ---

    fn increment_project_next_id(
        &mut self,
        project_key: &str,
        universe_key: &str,
        new_next_id: u64,
    ) {
        let path = format!("projects/{}/_project.md", project_key);
        // Read the project entry
        let result = self.conn.query_row(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries WHERE path = ?1",
            params![path],
            entry_row_from_sql,
        );
        if let Ok(row) = result {
            let mut fm = row.frontmatter.clone();
            if let Some(obj) = fm.as_object_mut() {
                obj.insert("next_id".to_string(), json!(new_next_id));
            }
            let entry = make_entry(&path, fm, &row.body);
            let universe_root = self.universe_root(universe_key);
            let _ = co::entry::write_entry(&universe_root, &entry);
            let _ = upsert_entry_row(&self.conn, universe_key, &entry);
        }
    }
}

// ---------------------------------------------------------------------------
// SQL helper — upsert a single entry into the entries table
// ---------------------------------------------------------------------------

fn upsert_entry_row(
    conn: &Connection,
    universe_key: &str,
    entry: &co::entry::Entry,
) -> anyhow::Result<()> {
    let fm_json = serde_json::to_string(&entry.frontmatter)?;
    let title: Option<&str> = entry.frontmatter.get("title").and_then(|v| v.as_str());
    let created_at = entry
        .frontmatter
        .get("created")
        .and_then(|v| v.as_str())
        .map(String::from);
    let updated_at = entry
        .frontmatter
        .get("modified")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| created_at.clone());

    conn.execute(
        "INSERT INTO entries (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(universe_key, path) DO UPDATE SET
           entry_type = excluded.entry_type,
           title = excluded.title,
           frontmatter_json = excluded.frontmatter_json,
           body = excluded.body,
           body_hash = excluded.body_hash,
           created_at = excluded.created_at,
           updated_at = excluded.updated_at",
        params![
            entry.path,
            universe_key,
            entry.entry_type,
            title,
            fm_json,
            entry.body,
            entry.body_hash,
            created_at,
            updated_at,
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn entry_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    let fm_str: String = row.get(4)?;
    let frontmatter: serde_json::Value =
        serde_json::from_str(&fm_str).unwrap_or(serde_json::Value::Object(Default::default()));
    Ok(EntryRow {
        path: row.get(0)?,
        universe_key: row.get(1)?,
        entry_type: row.get(2)?,
        title: row.get(3)?,
        frontmatter,
        body: row.get(5)?,
        body_hash: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn entry_row_to_project(row: &EntryRow) -> Option<Project> {
    let fm = &row.frontmatter;
    let key = fm.get("key").and_then(|v| v.as_str())?.to_string();
    let name = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let next_id = fm.get("next_id").and_then(|v| v.as_u64()).unwrap_or(1);
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);
    let archived = fm
        .get("archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Some(Project {
        key,
        name,
        description: row.body.clone(),
        created_at,
        next_id,
        archived,
    })
}

fn entry_row_to_task(row: &EntryRow) -> Option<Task> {
    let fm = &row.frontmatter;
    let id = fm.get("id").and_then(|v| v.as_u64())?;
    let project_key = fm
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status = parse_status(fm.get("status").and_then(|v| v.as_str()).unwrap_or("todo"));
    let priority = parse_priority(
        fm.get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("medium"),
    );
    let due_date = fm
        .get("due")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<NaiveDate>().ok());
    let parent = fm.get("parent").and_then(|v| v.as_u64());
    let labels: Vec<String> = fm
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);
    let updated_at = fm
        .get("modified")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or(created_at);
    let archived = fm
        .get("archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let assignee = fm
        .get("assignee")
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(Task {
        id,
        key: format!("{}-{}", project_key, id),
        project_key,
        title,
        status,
        priority,
        due_date,
        parent,
        labels,
        created_at,
        updated_at,
        description: row.body.clone(),
        archived,
        assignee,
    })
}

fn entry_row_to_comment(row: &EntryRow, project_key: &str, task_id: u64) -> Option<Comment> {
    let fm = &row.frontmatter;
    let id = fm.get("id").and_then(|v| v.as_u64())?;
    let author = fm
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("Anonymous")
        .to_string();
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);

    Some(Comment {
        id,
        project_key: project_key.to_string(),
        task_id,
        author,
        body: row.body.clone(),
        created_at,
    })
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

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
