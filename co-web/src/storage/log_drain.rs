use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::LogDrainEvent;
use super::Storage;

impl Storage {
    pub fn backup_universe(&self, universe_key: &str, dest_path: &Path) -> anyhow::Result<()> {
        use rusqlite::backup::Backup;
        let uc = self.universe_pool.get_or_open(universe_key);
        let src = uc.lock().expect("universe conn lock");
        let mut dest = Connection::open(dest_path)?;
        let backup = Backup::new(&src, &mut dest)?;
        backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
        Ok(())
    }

    /// Return the size of a universe's data.db in bytes, or 0 if not yet created.
    pub fn universe_db_size(&self, universe_key: &str) -> u64 {
        std::fs::metadata(self.universe_pool.db_path(universe_key))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Search entries across multiple universes (cross-universe aggregator).
    pub fn search_entries_across_universes(
        &self,
        universe_keys: &[&str],
        query: &str,
    ) -> Vec<crate::entry_index::EntryRow> {
        let mut results = Vec::new();
        for &key in universe_keys {
            let uc = self.universe_pool.get_or_open(key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let index = crate::entry_index::EntryIndex::new(&uc_guard);
            if let Ok(entries) = index.search(key, query) {
                results.extend(entries);
            }
        }
        results
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

    // --- CO-124: Vercel Log Drain ---

    /// Return the per-universe HMAC secret used to validate Vercel drain signatures.
    /// Returns `None` when the universe does not exist.
    pub fn get_log_drain_secret(&self, universe_id: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT log_drain_secret FROM universes WHERE key = ?1",
                params![universe_id],
                |row| row.get(0),
            )
            .optional()
    }

    /// Set the per-universe Vercel Log Drain secret.
    pub fn set_log_drain_secret(&self, universe_id: &str, secret: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE universes SET log_drain_secret = ?1 WHERE key = ?2",
            params![secret, universe_id],
        )?;
        Ok(())
    }

    /// Insert a single Vercel Log Drain event, ignoring duplicates (idempotent by event_id).
    pub fn insert_log_drain_event(&self, ev: &LogDrainEvent) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO log_drain_events \
             (event_id, universe_id, received_at, source, level, message, host, path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ev.event_id,
                ev.universe_id,
                ev.received_at,
                ev.source,
                ev.level,
                ev.message,
                ev.host,
                ev.path
            ],
        )?;
        Ok(())
    }

    // --- CO-45: UAT mutation log ---

    /// Record a write operation in the uat_mutations table.
    /// Only call this when `CO_ENV=uat`.
    pub fn log_uat_mutation(
        &self,
        action: &str,
        target: &str,
        before_value: Option<&str>,
        after_value: Option<&str>,
        user_id: Option<&str>,
        metadata: Option<&str>,
    ) -> rusqlite::Result<()> {
        let ts = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO uat_mutations \
             (timestamp, user_id, action, target, before_value, after_value, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ts,
                user_id,
                action,
                target,
                before_value,
                after_value,
                metadata
            ],
        )?;
        Ok(())
    }

    /// Return all mutations with id > since_id, ordered ascending.
    pub fn get_uat_mutations_since(&self, since_id: i64) -> Vec<crate::models::UatMutation> {
        let mut stmt = match self.conn.prepare(
            "SELECT id, timestamp, user_id, action, target, before_value, after_value, metadata \
             FROM uat_mutations WHERE id > ?1 ORDER BY id ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![since_id], |row| {
            Ok(crate::models::UatMutation {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                user_id: row.get(2)?,
                action: row.get(3)?,
                target: row.get(4)?,
                before_value: row.get(5)?,
                after_value: row.get(6)?,
                metadata: row.get(7)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Return the maximum mutation id (watermark for snapshot creation).
    /// Returns 0 if no mutations exist.
    pub fn get_uat_mutations_watermark(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(id), 0) FROM uat_mutations",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    // --- Projects ---
}
