use chrono::Utc;
use rusqlite::params;
use serde_json::json;

use crate::entry_index::make_entry;
use crate::models::*;

use super::Storage;
use super::schema::{entry_row_from_sql, entry_row_to_project, upsert_entry_row};

impl Storage {
    pub fn list_projects(&self) -> Vec<Project> {
        // CO-77: fan out to all universes listed in the project_universe_index
        let universe_keys: Vec<String> = {
            let mut stmt = match self
                .conn
                .prepare("SELECT DISTINCT universe_key FROM project_universe_index")
            {
                Ok(s) => s,
                Err(_) => return vec![],
            };
            stmt.query_map([], |r| r.get(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };
        let mut result = Vec::new();
        for uk in &universe_keys {
            result.extend(self.list_projects_for_universe(uk));
        }
        result.sort_by(|a, b| a.key.cmp(&b.key));
        result
    }

    pub fn list_projects_for_universe(&self, universe_key: &str) -> Vec<Project> {
        let uc = self.universe_pool.get_or_open(universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let mut stmt = match uc_guard.prepare(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE universe_key = ?1 AND entry_type = 'project' ORDER BY path",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![universe_key], entry_row_from_sql)
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .filter_map(|row| entry_row_to_project(&row))
            .collect()
    }

    pub fn get_project(&self, key: &str) -> Option<Project> {
        let upper_key = key.to_uppercase();
        let path = format!("projects/{}/_project.md", upper_key);

        // CO-77: look up universe via index, then query per-universe DB
        let universe_key = self.get_project_universe_key(&upper_key)?;
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let result = uc_guard.query_row(
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
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            upsert_entry_row(&uc_guard, &universe_key, &entry)?;
        }
        // Register in routing index
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) VALUES (?1, ?2)",
            params![upper_key, universe_key],
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

        // Find the universe_key
        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());
        let universe_root = self.universe_root(&universe_key);

        // Find all entries under this project
        let prefix = format!("projects/{}/", upper_key);
        let entry_paths: Vec<String> = {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let mut stmt = uc_guard
                .prepare("SELECT path FROM entries WHERE universe_key = ?1 AND path LIKE ?2")?;
            let like_pattern = format!("{}%", prefix);
            stmt.query_map(params![universe_key, like_pattern], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            for entry_path in &entry_paths {
                let _ = co::entry::delete_entry(&universe_root, entry_path);
                let _ = uc_guard.execute(
                    "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
                    params![universe_key, entry_path],
                );
            }
        }

        // Remove from routing index
        let _ = self.conn.execute(
            "DELETE FROM project_universe_index WHERE project_key = ?1",
            params![upper_key],
        );

        Ok(())
    }

    // --- Tasks ---
}
