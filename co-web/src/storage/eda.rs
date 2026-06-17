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

    /// Transport (`bridge.*`) event types that flooded `event_log`.
    const TRANSPORT_TYPES_SQL: &'static str = "'bridge.event_sent','bridge.event_received'";

    /// meta.db size in bytes (`page_count * page_size`).
    pub fn db_size_bytes(&self) -> i64 {
        let conn = self.conn();
        let pc: i64 = conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap_or(0);
        let ps: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap_or(0);
        pc * ps
    }

    /// Count remaining `bridge.*` transport rows in `event_log`.
    pub fn count_event_log_transport_rows(&self) -> i64 {
        self.conn()
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM event_log WHERE event_type IN ({})",
                    Self::TRANSPORT_TYPES_SQL
                ),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    /// Delete one batch of `bridge.*` transport rows, then truncate the WAL so
    /// disk use stays bounded. Returns rows deleted (0 when none remain).
    ///
    /// Designed to be called in a loop by a caller that **re-acquires the storage
    /// lock per batch** (so request handlers interleave between batches instead of
    /// blocking for the whole multi-GB delete).
    pub fn delete_event_log_transport_batch(&self, limit: usize) -> Result<usize> {
        let n = self.conn().execute(
            &format!(
                "DELETE FROM event_log WHERE rowid IN \
                 (SELECT rowid FROM event_log WHERE event_type IN ({}) LIMIT {limit})",
                Self::TRANSPORT_TYPES_SQL
            ),
            [],
        )?;
        if n > 0 {
            let _ = self.conn().execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        Ok(n)
    }

    /// Full `VACUUM` — rewrites the live remainder compactly (temp ≈ live size,
    /// not the bloated file size, so it fits a tight volume) and activates the
    /// `auto_vacuum=INCREMENTAL` latent since v087. Holds the DB lock for its
    /// duration; the caller must run it off the async runtime (`spawn_blocking`).
    pub fn vacuum(&self) -> Result<()> {
        self.conn().execute_batch("VACUUM;")
    }

    /// Single-lock convenience wrapper (delete all transport rows + VACUUM) for
    /// tests and the off-box compact path. The boot task instead drives the
    /// granular methods with per-batch lock release — see
    /// `eda::event_log_reclaim_boot_task`. Returns `(rows_deleted, before, after)`.
    pub fn reclaim_event_log_transport_bloat(&self) -> Result<(usize, i64, i64)> {
        const BATCH: usize = 20_000;
        let before = self.db_size_bytes();
        if self.count_event_log_transport_rows() < BATCH as i64 / 4 {
            return Ok((0, before, before));
        }
        let mut deleted = 0usize;
        loop {
            let n = self.delete_event_log_transport_batch(BATCH)?;
            deleted += n;
            if n == 0 {
                break;
            }
        }
        self.vacuum()?;
        Ok((deleted, before, self.db_size_bytes()))
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

#[cfg(test)]
mod tests {
    use super::Storage;

    #[test]
    fn reclaim_drops_transport_rows_keeps_domain_and_shrinks() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        // Enough transport rows to exceed the no-op threshold, plus domain rows.
        for i in 0..6_000 {
            storage
                .insert_event_log(
                    &format!("t{i}"),
                    "bridge.event_sent",
                    None,
                    None,
                    "{}",
                    "Public",
                    "2026-06-17T00:00:00Z",
                )
                .unwrap();
        }
        for i in 0..50 {
            storage
                .insert_event_log(
                    &format!("d{i}"),
                    "entry.updated",
                    Some("neuro"),
                    None,
                    "{}",
                    "Public",
                    "2026-06-17T00:00:00Z",
                )
                .unwrap();
        }

        let (deleted, _before, _after) = storage.reclaim_event_log_transport_bloat().unwrap();
        assert_eq!(deleted, 6_000, "all transport rows deleted");

        let transport: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM event_log WHERE event_type LIKE 'bridge.%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(transport, 0, "no transport rows remain");

        let domain: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM event_log WHERE event_type = 'entry.updated'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(domain, 50, "domain rows preserved");
    }

    #[test]
    fn reclaim_is_a_noop_when_not_bloated() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        storage
            .insert_event_log(
                "d1",
                "entry.updated",
                None,
                None,
                "{}",
                "Public",
                "2026-06-17T00:00:00Z",
            )
            .unwrap();
        let (deleted, before, after) = storage.reclaim_event_log_transport_bloat().unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(before, after, "no-op leaves the file untouched");
    }
}
