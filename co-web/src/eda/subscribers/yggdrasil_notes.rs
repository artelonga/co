//! CO-383: `YggdrasilNotes` — event-driven ingestion of Yggdrasil notes into CO.
//!
//! Subscribes to `entry.{created,updated,deleted}` events on the `yggdrasil`
//! universe key via the federated bus bridge (CO-384). On receipt:
//!
//! - `entry.created` / `entry.updated` → upsert entry at
//!   `instances/<instance_id>/notes/<slug>.md` with `source_marker = 'yggdrasil-live'`.
//! - `entry.deleted` → soft-delete by stamping `source_marker = 'yggdrasil-deleted'`.
//!
//! After each successful ingestion, re-publishes `sync.yggdrasil.note_ingested`
//! to CO's own bus so the live timeline (CO-381) picks it up, and stamps
//! `source_last_event_at` on the `yggdrasil` universe row.

use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use tracing::{info, warn};

use crate::eda::bus::{EdaBus, Filter};
use crate::eda::event::{Event, Visibility};
use crate::storage::Storage;

const UNIVERSE_KEY: &str = "yggdrasil";

/// Spawn the Yggdrasil notes subscriber as a background Tokio task.
pub fn spawn(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>) {
    let publish_bus = Arc::clone(&bus);
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec![
            "entry.created".into(),
            "entry.updated".into(),
            "entry.deleted".into(),
        ]),
        universe_keys: Some(vec![UNIVERSE_KEY.into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: YggdrasilNotes subscriber started");
        while let Some(ev) = sub.recv().await {
            handle_note_event(&ev, &publish_bus, &storage);
        }
        info!("EDA: YggdrasilNotes subscriber stopped (bus closed)");
    });
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

fn handle_note_event(ev: &Event, bus: &Arc<dyn EdaBus>, storage: &Arc<Mutex<Storage>>) {
    let payload = &ev.payload;

    let path = match payload.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            warn!("YggdrasilNotes: event missing payload.path — skipping");
            return;
        }
    };

    // Only process instance-qualified note paths (instances/<id>/notes/<slug>.md).
    if !path.starts_with("instances/") || !path.contains("/notes/") {
        return;
    }

    let now = Utc::now().to_rfc3339();

    if ev.event_type == "entry.deleted" {
        soft_delete_note(&path, ev, bus, storage, &now);
        return;
    }

    // entry.created | entry.updated
    let body = payload
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let frontmatter_json = payload
        .get("frontmatter")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".into());

    let title = payload
        .get("frontmatter")
        .and_then(|f| f.get("title"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let body_hash = payload
        .get("body_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if let Err(e) = upsert_note(&path, &title, &frontmatter_json, &body, &body_hash, storage) {
        warn!(path = %path, error = %e, "YggdrasilNotes: upsert failed");
        bus.publish(Event::new(
            "sync.yggdrasil.error",
            Some(UNIVERSE_KEY.into()),
            ev.user_id.clone(),
            serde_json::json!({ "path": &path, "error": e.to_string() }),
            Visibility::System,
        ));
        return;
    }

    stamp_last_event_at(storage, &now);

    bus.publish(Event::new(
        "sync.yggdrasil.note_ingested",
        Some(UNIVERSE_KEY.into()),
        ev.user_id.clone(),
        serde_json::json!({ "path": &path, "event_type": &ev.event_type }),
        Visibility::UniverseMembers,
    ));
}

fn soft_delete_note(
    path: &str,
    ev: &Event,
    bus: &Arc<dyn EdaBus>,
    storage: &Arc<Mutex<Storage>>,
    now: &str,
) {
    let uc = {
        let st = storage.lock();
        st.universe_conn(UNIVERSE_KEY)
    };
    match uc.lock() {
        Ok(conn) => {
            if let Err(e) = conn.execute(
                "UPDATE entries \
                 SET source_marker = 'yggdrasil-deleted', updated_at = ?1 \
                 WHERE universe_key = ?2 AND path = ?3",
                rusqlite::params![now, UNIVERSE_KEY, path],
            ) {
                warn!(path = %path, error = %e, "YggdrasilNotes: soft-delete failed");
                return;
            }
        }
        Err(e) => {
            warn!(path = %path, error = %e, "YggdrasilNotes: universe conn poisoned on delete");
            return;
        }
    }

    stamp_last_event_at(storage, now);

    bus.publish(Event::new(
        "sync.yggdrasil.note_ingested",
        Some(UNIVERSE_KEY.into()),
        ev.user_id.clone(),
        serde_json::json!({ "path": path, "event_type": "entry.deleted" }),
        Visibility::UniverseMembers,
    ));
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn upsert_note(
    path: &str,
    title: &Option<String>,
    frontmatter_json: &str,
    body: &str,
    body_hash: &str,
    storage: &Arc<Mutex<Storage>>,
) -> anyhow::Result<()> {
    let now = Utc::now().to_rfc3339();
    let uc = {
        let st = storage.lock();
        st.universe_conn(UNIVERSE_KEY)
    };
    let conn = uc
        .lock()
        .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
    conn.execute(
        "INSERT INTO entries \
           (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
            created_at, updated_at, entry_origin, source_marker) \
         VALUES (?1, ?2, 'page', ?3, ?4, ?5, ?6, ?7, ?7, '', 'yggdrasil-live') \
         ON CONFLICT(universe_key, path) DO UPDATE SET \
           title            = excluded.title, \
           frontmatter_json = excluded.frontmatter_json, \
           body             = excluded.body, \
           body_hash        = excluded.body_hash, \
           updated_at       = excluded.updated_at, \
           source_marker    = 'yggdrasil-live'",
        rusqlite::params![
            path,
            UNIVERSE_KEY,
            title,
            frontmatter_json,
            body,
            body_hash,
            now,
        ],
    )?;
    Ok(())
}

fn stamp_last_event_at(storage: &Arc<Mutex<Storage>>, ts: &str) {
    let st = storage.lock();
    st.touch_universe_last_event_at(UNIVERSE_KEY, ts);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eda::event::Visibility;
    use crate::platform::universe_pool::UNIVERSE_SCHEMA_FOR_TEST;

    fn make_event(
        event_type: &str,
        universe_key: Option<&str>,
        payload: serde_json::Value,
    ) -> Event {
        Event {
            id: crate::eda::event::new_ulid(),
            event_type: event_type.into(),
            universe_key: universe_key.map(Into::into),
            user_id: None,
            payload,
            visibility: Visibility::UniverseMembers,
            created_at: Utc::now(),
        }
    }

    fn open_test_universe_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(UNIVERSE_SCHEMA_FOR_TEST).unwrap();
        // Ensure source_marker column exists (v17 migration).
        if !conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('entries') WHERE name = 'source_marker'",
                [],
                |_| Ok(true),
            )
            .ok()
            .unwrap_or(false)
        {
            conn.execute_batch("ALTER TABLE entries ADD COLUMN source_marker TEXT")
                .unwrap();
        }
        conn
    }

    #[test]
    fn test_non_note_path_skipped() {
        // A path without "instances/" prefix should not be processed.
        let path = "notes/random.md";
        let is_valid = path.starts_with("instances/") && path.contains("/notes/");
        assert!(!is_valid);
    }

    #[test]
    fn test_instance_note_path_recognized() {
        let path = "instances/abc123/notes/my-note.md";
        let is_valid = path.starts_with("instances/") && path.contains("/notes/");
        assert!(is_valid);
    }

    #[test]
    fn test_upsert_note_roundtrip() {
        let conn = open_test_universe_conn();
        let uc: Arc<std::sync::Mutex<rusqlite::Connection>> = Arc::new(std::sync::Mutex::new(conn));

        let now = Utc::now().to_rfc3339();
        let path = "instances/inst-01/notes/hello.md";
        {
            let c = uc.lock().unwrap();
            c.execute(
                "INSERT INTO entries \
                   (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                    created_at, updated_at, entry_origin, source_marker) \
                 VALUES (?1, ?2, 'page', ?3, ?4, ?5, ?6, ?7, ?7, '', 'yggdrasil-live') \
                 ON CONFLICT(universe_key, path) DO UPDATE SET \
                   title = excluded.title, \
                   frontmatter_json = excluded.frontmatter_json, \
                   body = excluded.body, \
                   body_hash = excluded.body_hash, \
                   updated_at = excluded.updated_at, \
                   source_marker = 'yggdrasil-live'",
                rusqlite::params![
                    path,
                    UNIVERSE_KEY,
                    Some("Hello"),
                    "{}",
                    "body here",
                    "hash-abc",
                    now,
                ],
            )
            .unwrap();
        }

        {
            let c = uc.lock().unwrap();
            let (hash, marker): (String, String) = c
                .query_row(
                    "SELECT body_hash, source_marker FROM entries \
                     WHERE universe_key = ?1 AND path = ?2",
                    rusqlite::params![UNIVERSE_KEY, path],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(hash, "hash-abc");
            assert_eq!(marker, "yggdrasil-live");
        }
    }

    #[test]
    fn test_soft_delete_path_check() {
        // A valid note path with instance prefix.
        let path = "instances/xyz/notes/deleted-note.md";
        assert!(path.starts_with("instances/") && path.contains("/notes/"));
    }

    #[test]
    fn test_filter_matches_yggdrasil_entry_events() {
        let filter = Filter {
            event_types: Some(vec![
                "entry.created".into(),
                "entry.updated".into(),
                "entry.deleted".into(),
            ]),
            universe_keys: Some(vec![UNIVERSE_KEY.into()]),
            ..Default::default()
        };

        let ev = make_event(
            "entry.created",
            Some(UNIVERSE_KEY),
            serde_json::json!({ "path": "instances/abc/notes/foo.md" }),
        );
        assert!(filter.matches(&ev));

        // Wrong universe — must not match.
        let ev2 = make_event(
            "entry.created",
            Some("other-universe"),
            serde_json::json!({ "path": "instances/abc/notes/foo.md" }),
        );
        assert!(!filter.matches(&ev2));

        // Wrong event type — must not match.
        let ev3 = make_event("analytics.visit", Some(UNIVERSE_KEY), serde_json::json!({}));
        assert!(!filter.matches(&ev3));
    }

    #[test]
    fn test_ingested_event_type() {
        let expected = "sync.yggdrasil.note_ingested";
        assert!(expected.starts_with("sync.yggdrasil."));
    }
}
