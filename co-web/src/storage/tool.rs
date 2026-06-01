//! Tool registry storage — CO-331
//!
//! Manages git-backed tool registrations: add, list, update pin, remove.

use chrono::Utc;
use rusqlite::params;

use super::Storage;

/// A registered tool (git-repo-backed).
#[derive(Debug, Clone)]
pub struct ToolRecord {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub remote_url: Option<String>,
    pub local_path: Option<String>,
    pub version_pin: Option<String>,
    pub entry_command: Option<String>,
    pub installed_at: Option<String>,
    pub last_updated: Option<String>,
    pub follow_main: bool,
    /// Commit SHA captured at install/update time — used by `co tool verify`.
    pub lockfile_sha: Option<String>,
}

impl Storage {
    pub fn tool_add(&mut self, record: &ToolRecord) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO tools \
             (key, name, description, remote_url, local_path, version_pin, \
              entry_command, installed_at, last_updated, follow_main, lockfile_sha) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                record.key,
                record.name,
                record.description,
                record.remote_url,
                record.local_path,
                record.version_pin,
                record.entry_command,
                record.installed_at.as_deref().unwrap_or(&now),
                record.last_updated.as_deref().unwrap_or(&now),
                record.follow_main as i64,
                record.lockfile_sha,
            ],
        )?;
        Ok(())
    }

    pub fn tool_get(&self, key: &str) -> Option<ToolRecord> {
        self.conn
            .query_row(
                "SELECT key, name, description, remote_url, local_path, version_pin, \
                 entry_command, installed_at, last_updated, follow_main, lockfile_sha \
                 FROM tools WHERE key = ?1",
                params![key],
                |row| {
                    Ok(ToolRecord {
                        key: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        remote_url: row.get(3)?,
                        local_path: row.get(4)?,
                        version_pin: row.get(5)?,
                        entry_command: row.get(6)?,
                        installed_at: row.get(7)?,
                        last_updated: row.get(8)?,
                        follow_main: row.get::<_, i64>(9)? != 0,
                        lockfile_sha: row.get(10)?,
                    })
                },
            )
            .ok()
    }

    pub fn tool_list(&self) -> Vec<ToolRecord> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT key, name, description, remote_url, local_path, version_pin, \
                 entry_command, installed_at, last_updated, follow_main, lockfile_sha \
                 FROM tools ORDER BY key",
            )
            .expect("prepare tool_list");
        stmt.query_map([], |row| {
            Ok(ToolRecord {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                remote_url: row.get(3)?,
                local_path: row.get(4)?,
                version_pin: row.get(5)?,
                entry_command: row.get(6)?,
                installed_at: row.get(7)?,
                last_updated: row.get(8)?,
                follow_main: row.get::<_, i64>(9)? != 0,
                lockfile_sha: row.get(10)?,
            })
        })
        .expect("query tool_list")
        .flatten()
        .collect()
    }

    pub fn tool_list_follow_main(&self) -> Vec<ToolRecord> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT key, name, description, remote_url, local_path, version_pin, \
                 entry_command, installed_at, last_updated, follow_main, lockfile_sha \
                 FROM tools WHERE follow_main = 1 ORDER BY key",
            )
            .expect("prepare tool_list_follow_main");
        stmt.query_map([], |row| {
            Ok(ToolRecord {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                remote_url: row.get(3)?,
                local_path: row.get(4)?,
                version_pin: row.get(5)?,
                entry_command: row.get(6)?,
                installed_at: row.get(7)?,
                last_updated: row.get(8)?,
                follow_main: row.get::<_, i64>(9)? != 0,
                lockfile_sha: row.get(10)?,
            })
        })
        .expect("query tool_list_follow_main")
        .flatten()
        .collect()
    }

    /// Update version pin, follow_main flag, and/or lockfile SHA for a tool.
    /// Only fields with `Some(...)` are written; `None` leaves the current value.
    pub fn tool_update_pin(
        &mut self,
        key: &str,
        version_pin: Option<&str>,
        follow_main: Option<bool>,
        lockfile_sha: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE tools SET \
             version_pin  = COALESCE(?1, version_pin), \
             follow_main  = COALESCE(?2, follow_main), \
             lockfile_sha = COALESCE(?3, lockfile_sha), \
             last_updated = ?4 \
             WHERE key = ?5",
            params![
                version_pin,
                follow_main.map(|f| f as i64),
                lockfile_sha,
                now,
                key,
            ],
        )?;
        anyhow::ensure!(rows > 0, "Tool '{}' not found", key);
        Ok(())
    }

    pub fn tool_remove(&mut self, key: &str) -> anyhow::Result<()> {
        let rows = self
            .conn
            .execute("DELETE FROM tools WHERE key = ?1", params![key])?;
        anyhow::ensure!(rows > 0, "Tool '{}' not found", key);
        Ok(())
    }

    /// Return keys of tools whose `entry_command` collides with the given value,
    /// excluding `exclude_key` (the tool being added/updated).
    pub fn tool_entry_command_conflicts(
        &self,
        entry_command: &str,
        exclude_key: &str,
    ) -> Vec<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT key FROM tools WHERE entry_command = ?1 AND key != ?2")
            .expect("prepare tool_entry_command_conflicts");
        stmt.query_map(params![entry_command, exclude_key], |row| row.get(0))
            .expect("query tool_entry_command_conflicts")
            .flatten()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn make_storage() -> Storage {
        let dir = tempdir().unwrap();
        Storage::new(dir.into_path())
    }

    fn sample(key: &str) -> ToolRecord {
        ToolRecord {
            key: key.to_string(),
            name: key.to_string(),
            description: None,
            remote_url: Some("https://example.com/tool.git".to_string()),
            local_path: None,
            version_pin: Some("v1.0.0".to_string()),
            entry_command: Some(key.to_string()),
            installed_at: None,
            last_updated: None,
            follow_main: false,
            lockfile_sha: Some("abc123".to_string()),
        }
    }

    #[test]
    fn add_and_get() {
        let mut s = make_storage();
        s.tool_add(&sample("mytool")).unwrap();
        let got = s.tool_get("mytool").expect("should find tool");
        assert_eq!(got.key, "mytool");
        assert_eq!(got.version_pin.as_deref(), Some("v1.0.0"));
        assert!(!got.follow_main);
    }

    #[test]
    fn list_returns_all() {
        let mut s = make_storage();
        s.tool_add(&sample("alpha")).unwrap();
        s.tool_add(&sample("beta")).unwrap();
        let list = s.tool_list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].key, "alpha");
        assert_eq!(list[1].key, "beta");
    }

    #[test]
    fn update_pin() {
        let mut s = make_storage();
        s.tool_add(&sample("mytool")).unwrap();
        s.tool_update_pin("mytool", Some("v2.0.0"), None, Some("def456"))
            .unwrap();
        let got = s.tool_get("mytool").unwrap();
        assert_eq!(got.version_pin.as_deref(), Some("v2.0.0"));
        assert_eq!(got.lockfile_sha.as_deref(), Some("def456"));
    }

    #[test]
    fn follow_main_filter() {
        let mut s = make_storage();
        let mut pinned = sample("pinned");
        pinned.follow_main = false;
        let mut tracked = sample("tracked");
        tracked.follow_main = true;
        s.tool_add(&pinned).unwrap();
        s.tool_add(&tracked).unwrap();
        let fm = s.tool_list_follow_main();
        assert_eq!(fm.len(), 1);
        assert_eq!(fm[0].key, "tracked");
    }

    #[test]
    fn remove() {
        let mut s = make_storage();
        s.tool_add(&sample("mytool")).unwrap();
        s.tool_remove("mytool").unwrap();
        assert!(s.tool_get("mytool").is_none());
        assert_eq!(s.tool_list().len(), 0);
    }

    #[test]
    fn remove_nonexistent_errors() {
        let mut s = make_storage();
        assert!(s.tool_remove("nope").is_err());
    }

    #[test]
    fn entry_command_conflict_detection() {
        let mut s = make_storage();
        s.tool_add(&sample("tool-a")).unwrap();
        let mut b = sample("tool-b");
        b.entry_command = Some("tool-a".to_string()); // same command as tool-a
        s.tool_add(&b).unwrap();
        let conflicts = s.tool_entry_command_conflicts("tool-a", "tool-a");
        assert_eq!(conflicts, vec!["tool-b"]);
    }

    #[test]
    fn duplicate_key_errors() {
        let mut s = make_storage();
        s.tool_add(&sample("mytool")).unwrap();
        assert!(s.tool_add(&sample("mytool")).is_err());
    }
}
