use rusqlite::params;
use serde_json::json;

use crate::entry_index::make_entry;

use super::Storage;
use super::schema::upsert_entry_row;

impl Storage {
    /// Migrate data from old projects/tasks/comments tables into the entries table + .md files.
    pub(crate) fn migrate_old_data_to_entries(&mut self) {
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

    // -------------------------------------------------------------------------
    // CO-77: startup migration — move entries from meta.db to per-universe DBs
    // -------------------------------------------------------------------------

    /// On first boot after CO-77, migrate all rows in `entries` (meta.db) to
    /// the appropriate per-universe `data.db`. This is a one-shot migration:
    /// once `entries` in meta.db is empty it never runs again.
    pub(super) fn maybe_migrate_entries_to_universe_dbs(&self) {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
            .unwrap_or(0);
        if count == 0 {
            return;
        }

        tracing::info!(
            "CO-77: migrating {} entries from meta.db to per-universe DBs",
            count
        );

        // Collect distinct universe keys
        let universe_keys: Vec<String> = {
            let mut stmt = match self
                .conn
                .prepare("SELECT DISTINCT universe_key FROM entries")
            {
                Ok(s) => s,
                Err(_) => return,
            };
            stmt.query_map([], |r| r.get(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };

        for uk in &universe_keys {
            let uc = self.universe_pool.get_or_open(uk);
            let uc_guard = uc.lock().expect("universe conn lock");

            type EntryRow9 = (
                String,
                String,
                String,
                Option<String>,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
            );
            // Collect rows for this universe
            let rows: Vec<EntryRow9> = {
                let mut stmt = match self.conn.prepare(
                    "SELECT path, universe_key, entry_type, title, \
                     frontmatter_json, body, body_hash, created_at, updated_at \
                     FROM entries WHERE universe_key = ?1",
                ) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                stmt.query_map(params![uk], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                })
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
            };

            for (path, uk2, entry_type, title, fm_json, body, body_hash, created_at, updated_at) in
                &rows
            {
                let _ = uc_guard.execute(
                    "INSERT OR IGNORE INTO entries \
                     (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        path, uk2, entry_type, title, fm_json, body, body_hash,
                        created_at, updated_at
                    ],
                );
                // Populate project_universe_index for project entries
                if entry_type == "project"
                    && let Ok(fm) = serde_json::from_str::<serde_json::Value>(fm_json)
                    && let Some(proj_key) = fm.get("key").and_then(|v| v.as_str())
                {
                    let _ = self.conn.execute(
                        "INSERT OR IGNORE INTO project_universe_index \
                         (project_key, universe_key) VALUES (?1, ?2)",
                        params![proj_key, uk],
                    );
                }
            }
        }

        // Clear entries from meta.db — now only in per-universe DBs
        let _ = self.conn.execute_batch("DELETE FROM entries;");
        tracing::info!("CO-77: entries migrated to per-universe DBs");
    }
}
