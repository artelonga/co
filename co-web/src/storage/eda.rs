//! Typed meta-DB accessors for the EDA subsystem (CO-433).
//!
//! Moves the raw `conn().execute/prepare/query_row` calls out of `eda/mod.rs`,
//! the EDA bridge, and the EDA subscribers into typed methods on `Storage`.
//! All operate on the global meta-DB (`event_log`, `bridge_state`,
//! `task_status_log`) — the state the global `Mutex<Storage>` legitimately
//! guards.

use chrono::Utc;
use rusqlite::Result;

use super::Storage;

/// A raw `event_log` row, as stored. The EDA bridge maps this into a domain
/// `Event` (parsing visibility + timestamps in the bridge layer).
#[derive(Debug, Clone)]
pub struct EventLogRow {
    pub id: String,
    pub event_type: String,
    pub universe_key: Option<String>,
    pub user_id: Option<String>,
    pub payload_json: String,
    pub visibility: String,
    pub created_at: String,
}

impl Storage {
    /// Delete `event_log` rows older than 30 days. Returns rows pruned.
    pub fn prune_event_log_older_than_30_days(&self) -> Result<usize> {
        self.conn().execute(
            "DELETE FROM event_log WHERE created_at < datetime('now', '-30 days')",
            [],
        )
    }

    /// Append an event to `event_log` (idempotent on id).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_event_log(
        &self,
        id: &str,
        event_type: &str,
        universe_key: Option<&str>,
        user_id: Option<&str>,
        payload_json: &str,
        visibility: &str,
        created_at: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "INSERT OR IGNORE INTO event_log \
             (id, event_type, universe_key, user_id, payload_json, visibility, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                id,
                event_type,
                universe_key,
                user_id,
                payload_json,
                visibility,
                created_at,
            ],
        )
    }

    /// Load up to 1000 `event_log` rows published after `since_id` (exclusive),
    /// in ULID (chronological) order.
    pub fn load_event_log_since(&self, since_id: &str) -> Result<Vec<EventLogRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, event_type, universe_key, user_id, payload_json, visibility, created_at
             FROM event_log
             WHERE id > ?1
             ORDER BY id ASC
             LIMIT 1000",
        )?;
        let rows = stmt.query_map(rusqlite::params![since_id], |row| {
            Ok(EventLogRow {
                id: row.get(0)?,
                event_type: row.get(1)?,
                universe_key: row.get(2)?,
                user_id: row.get(3)?,
                payload_json: row.get(4)?,
                visibility: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Upsert the bridge connection state for a `source:target` pair.
    pub fn upsert_bridge_state(
        &self,
        source: &str,
        target: &str,
        state: &str,
        last_event_id: Option<&str>,
    ) -> Result<usize> {
        let id = format!("{source}:{target}");
        let now = Utc::now().to_rfc3339();
        self.conn().execute(
            "INSERT INTO bridge_state (id, source_deployment, target_deployment,
                last_delivered_event_id, last_connected_at, last_disconnected_at, state)
             VALUES (?1, ?2, ?3, ?4,
                     CASE WHEN ?5 = 'connected' THEN ?6 ELSE NULL END,
                     CASE WHEN ?5 = 'disconnected' THEN ?6 ELSE NULL END,
                     ?5)
             ON CONFLICT(id) DO UPDATE SET
                 state = excluded.state,
                 last_connected_at = CASE WHEN ?5 = 'connected' THEN ?6 ELSE last_connected_at END,
                 last_disconnected_at = CASE WHEN ?5 = 'disconnected' THEN ?6 ELSE last_disconnected_at END,
                 last_delivered_event_id = COALESCE(?4, last_delivered_event_id)",
            rusqlite::params![id, source, target, last_event_id, state, now],
        )
    }

    /// The last delivered event id recorded for a `source:target` bridge pair.
    pub fn bridge_last_delivered_event_id(&self, source: &str, target: &str) -> Option<String> {
        let id = format!("{source}:{target}");
        self.conn()
            .query_row(
                "SELECT last_delivered_event_id FROM bridge_state WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
    }

    /// Append a task status transition to `task_status_log` (idempotent on id).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_task_status_log(
        &self,
        id: &str,
        universe_key: &str,
        entry_path: &str,
        status_from: Option<&str>,
        status_to: &str,
        trigger: &str,
        triggered_at: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "INSERT OR IGNORE INTO task_status_log \
             (id, universe_key, entry_path, status_from, status_to, trigger, triggered_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                id,
                universe_key,
                entry_path,
                status_from,
                status_to,
                trigger,
                triggered_at,
            ],
        )
    }
}
