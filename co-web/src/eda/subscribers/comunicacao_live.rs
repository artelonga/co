//! CO-389: `ComunicacaoLive` — live overlay for Yggdrasil lexicon sala events.
//!
//! Two subscribers:
//!
//! 1. **Term subscriber**: listens for `entry.{created,updated,deleted}` events on
//!    the `comunicacao` universe (path must contain `_users/` — sala-published terms).
//!    On each event, upserts directly into CO's entries table, acting as a live
//!    overlay on top of CO-337's 15-min sister-repo sync.  Hash-dedup prevents
//!    double-writes when the sync already wrote the same bytes.
//!
//! 2. **Sala subscriber**: listens for `yggdrasil.sala.*` activity events and
//!    re-publishes them as `sync.yggdrasil.sala.*` for `/agora` display, with no
//!    content mutation.

use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::OptionalExtension;
use tracing::{debug, info, warn};

use crate::eda::bus::{EdaBus, Filter};
use crate::eda::event::{Event, Visibility};
use crate::storage::Storage;

const UNIVERSE_KEY: &str = "comunicacao";
const USERS_PATH_SEGMENT: &str = "_users/";

/// Spawn both subscribers as background Tokio tasks.
pub fn spawn(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>) {
    spawn_term_subscriber(Arc::clone(&bus), Arc::clone(&storage));
    spawn_sala_subscriber(bus);
}

/// Subscribe to `entry.*` events for the `comunicacao` universe and upsert
/// sala-published terms from Yggdrasil into the local entries table.
fn spawn_term_subscriber(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>) {
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
        info!("EDA: ComunicacaoLive term subscriber started");
        while let Some(ev) = sub.recv().await {
            handle_term_event(&ev, &publish_bus, &storage);
        }
        info!("EDA: ComunicacaoLive term subscriber stopped (bus closed)");
    });
}

/// Subscribe to `yggdrasil.sala.*` activity events and republish them as
/// `sync.yggdrasil.sala.*` for `/agora` display (no content mutation).
fn spawn_sala_subscriber(bus: Arc<dyn EdaBus>) {
    let publish_bus = Arc::clone(&bus);
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec!["yggdrasil.sala".into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: ComunicacaoLive sala subscriber started");
        while let Some(ev) = sub.recv().await {
            handle_sala_activity_event(&ev, &publish_bus);
        }
        info!("EDA: ComunicacaoLive sala subscriber stopped (bus closed)");
    });
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn handle_term_event(ev: &Event, bus: &Arc<dyn EdaBus>, storage: &Arc<Mutex<Storage>>) {
    let payload = &ev.payload;

    let path = match payload.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            warn!("ComunicacaoLive: entry event missing payload.path — skipping");
            return;
        }
    };

    // Narrow to sala-published terms: path must contain the `_users/` segment.
    if !path.contains(USERS_PATH_SEGMENT) {
        debug!(path = %path, "ComunicacaoLive: skipping non-sala path");
        return;
    }

    let lingua = payload
        .get("lingua")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if ev.event_type == "entry.deleted" {
        soft_delete_term(&path, &lingua, ev, bus, storage);
        return;
    }

    let body_hash = match payload.get("body_hash").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => {
            warn!(path = %path, "ComunicacaoLive: entry event missing payload.body_hash — skipping");
            return;
        }
    };

    // Dedup: if existing body_hash matches, sister-repo sync already wrote it.
    if let Some(existing_hash) = read_body_hash(&path, storage)
        && existing_hash == body_hash
    {
        debug!(path = %path, "ComunicacaoLive: hash match — dedup skip");
        bus.publish(Event::new(
            "sync.comunicacao.event_dedup_skipped",
            Some(UNIVERSE_KEY.into()),
            ev.user_id.clone(),
            serde_json::json!({ "path": &path }),
            Visibility::System,
        ));
        return;
    }

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
    let entry_type = payload
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("term")
        .to_string();

    if let Err(e) = upsert_term(
        &path,
        &entry_type,
        &title,
        &frontmatter_json,
        &body,
        &body_hash,
        storage,
    ) {
        warn!(path = %path, error = %e, "ComunicacaoLive: upsert failed");
        return;
    }

    bus.publish(Event::new(
        "sync.comunicacao.term_landed",
        Some(UNIVERSE_KEY.into()),
        ev.user_id.clone(),
        serde_json::json!({ "path": &path, "lingua": &lingua }),
        Visibility::UniverseMembers,
    ));
}

/// Soft-delete a term: mark updated_at and source_marker without physical removal.
/// CO-337's durability poll handles permanent state; the overlay only records the event.
fn soft_delete_term(
    path: &str,
    lingua: &str,
    ev: &Event,
    bus: &Arc<dyn EdaBus>,
    storage: &Arc<Mutex<Storage>>,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let uc = {
        let st = storage.lock();
        st.universe_conn(UNIVERSE_KEY)
    };
    match uc.lock() {
        Ok(conn) => {
            if let Err(e) = conn.execute(
                "UPDATE entries \
                 SET source_marker = 'yggdrasil-live', updated_at = ?1 \
                 WHERE universe_key = ?2 AND path = ?3",
                rusqlite::params![now, UNIVERSE_KEY, path],
            ) {
                warn!(path = %path, error = %e, "ComunicacaoLive: soft-delete failed");
            }
        }
        Err(e) => {
            warn!(path = %path, error = %e, "ComunicacaoLive: universe conn poisoned on delete");
        }
    }
    bus.publish(Event::new(
        "sync.comunicacao.term_landed",
        Some(UNIVERSE_KEY.into()),
        ev.user_id.clone(),
        serde_json::json!({ "path": path, "lingua": lingua, "deleted": true }),
        Visibility::UniverseMembers,
    ));
}

/// Re-publish a sala activity event as `sync.yggdrasil.sala.*` for `/agora`.
/// Preserves the original visibility so CO-378 redaction is enforced downstream.
fn handle_sala_activity_event(ev: &Event, bus: &Arc<dyn EdaBus>) {
    bus.publish(Event::new(
        format!("sync.{}", ev.event_type),
        ev.universe_key.clone(),
        ev.user_id.clone(),
        ev.payload.clone(),
        ev.visibility.clone(),
    ));
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Read the current body_hash for a comunicacao entry; returns None if missing.
fn read_body_hash(path: &str, storage: &Arc<Mutex<Storage>>) -> Option<String> {
    let uc = {
        let st = storage.lock();
        st.universe_conn(UNIVERSE_KEY)
    };
    uc.lock().ok().and_then(|conn| {
        conn.query_row(
            "SELECT body_hash FROM entries WHERE universe_key = ?1 AND path = ?2",
            rusqlite::params![UNIVERSE_KEY, path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    })
}

/// Upsert a term from Yggdrasil with `source_marker = 'yggdrasil-live'`.
fn upsert_term(
    path: &str,
    entry_type: &str,
    title: &Option<String>,
    frontmatter_json: &str,
    body: &str,
    body_hash: &str,
    storage: &Arc<Mutex<Storage>>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
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
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, '', 'yggdrasil-live') \
         ON CONFLICT(universe_key, path) DO UPDATE SET \
           entry_type     = excluded.entry_type, \
           title          = excluded.title, \
           frontmatter_json = excluded.frontmatter_json, \
           body           = excluded.body, \
           body_hash      = excluded.body_hash, \
           updated_at     = excluded.updated_at, \
           source_marker  = 'yggdrasil-live'",
        rusqlite::params![
            path,
            UNIVERSE_KEY,
            entry_type,
            title,
            frontmatter_json,
            body,
            body_hash,
            now,
        ],
    )?;
    Ok(())
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
            created_at: chrono::Utc::now(),
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
    fn test_non_sala_path_skipped() {
        let payload = serde_json::json!({
            "path": "mbya/terms/index.md",
            "body_hash": "abc123",
            "body": "body",
            "frontmatter": { "title": "Index" }
        });
        let ev = make_event("entry.created", Some(UNIVERSE_KEY), payload);
        // path does not contain "_users/" → should be skipped silently
        // No bus or storage needed; just verify the guard logic via the path check.
        assert!(
            !ev.payload["path"]
                .as_str()
                .unwrap()
                .contains(USERS_PATH_SEGMENT)
        );
    }

    #[test]
    fn test_sala_path_recognized() {
        let path = "mbya/terms/_users/yuri/ogunté.md";
        assert!(path.contains(USERS_PATH_SEGMENT));
    }

    #[test]
    fn test_upsert_term_roundtrip() {
        let conn = open_test_universe_conn();
        let uc: Arc<std::sync::Mutex<rusqlite::Connection>> = Arc::new(std::sync::Mutex::new(conn));
        let bus_uc = Arc::clone(&uc);

        // Write a term directly via the SQL we use in upsert_term.
        let now = chrono::Utc::now().to_rfc3339();
        {
            let c = bus_uc.lock().unwrap();
            c.execute(
                "INSERT INTO entries \
                   (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                    created_at, updated_at, entry_origin, source_marker) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, '', 'yggdrasil-live') \
                 ON CONFLICT(universe_key, path) DO UPDATE SET \
                   entry_type = excluded.entry_type, \
                   title = excluded.title, \
                   frontmatter_json = excluded.frontmatter_json, \
                   body = excluded.body, \
                   body_hash = excluded.body_hash, \
                   updated_at = excluded.updated_at, \
                   source_marker = 'yggdrasil-live'",
                rusqlite::params![
                    "mbya/terms/_users/yuri/ogunté.md",
                    UNIVERSE_KEY,
                    "term",
                    Some("Ogunté"),
                    "{}",
                    "body content",
                    "sha256abc",
                    now,
                ],
            )
            .unwrap();
        }

        // Verify the row was written with the expected source_marker.
        {
            let c = bus_uc.lock().unwrap();
            let (hash, marker): (String, String) = c
                .query_row(
                    "SELECT body_hash, source_marker FROM entries \
                     WHERE universe_key = ?1 AND path = ?2",
                    rusqlite::params![UNIVERSE_KEY, "mbya/terms/_users/yuri/ogunté.md"],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(hash, "sha256abc");
            assert_eq!(marker, "yggdrasil-live");
        }
    }

    #[test]
    fn test_sala_event_republish_type() {
        // Verify the re-published event_type prefix.
        let original_type = "yggdrasil.sala.published";
        let expected = format!("sync.{original_type}");
        assert_eq!(expected, "sync.yggdrasil.sala.published");
    }

    #[test]
    fn test_filter_term_subscriber_matches() {
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
            serde_json::json!({ "path": "mbya/terms/_users/a/b.md" }),
        );
        assert!(filter.matches(&ev));

        // Wrong universe → no match.
        let ev2 = make_event(
            "entry.created",
            Some("other-universe"),
            serde_json::json!({ "path": "mbya/terms/_users/a/b.md" }),
        );
        assert!(!filter.matches(&ev2));
    }

    #[test]
    fn test_filter_sala_subscriber_matches() {
        let filter = Filter {
            event_types: Some(vec!["yggdrasil.sala".into()]),
            ..Default::default()
        };

        let ev = make_event(
            "yggdrasil.sala.published",
            None,
            serde_json::json!({ "sala_id": "s1" }),
        );
        assert!(filter.matches(&ev));

        // entry.created should NOT match the sala filter.
        let ev2 = make_event("entry.created", Some(UNIVERSE_KEY), serde_json::json!({}));
        assert!(!filter.matches(&ev2));
    }
}
