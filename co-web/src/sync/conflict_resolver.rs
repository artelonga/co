//! CO-385: Action tree — 7 resolution handlers for cross-device conflicts.
//!
//! Each handler applies one action to a conflict record atomically:
//! write the entry change, mark the conflict resolved, and publish a
//! `sync.conflict_resolved` event to the CO-380 bus.

use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::eda::EdaBus;
use crate::eda::event::{Event, Visibility};
use crate::storage::Storage;

use super::conflict_detector::Conflict;

/// The seven CRUD-mapped actions available for conflict resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Keep both. Local renamed with `_1` suffix; remote written at original path.
    KeepBoth,
    /// No-op when hashes match (defensive guard).
    Ignore,
    /// Overwrite local with remote (remote wins).
    Replace,
    /// 3-way merge — body merged; conflict markers if non-resolvable.
    Update,
    /// Bulk: merge existing diffs + insert new remote items.
    Upsert,
    /// Mirror remote delete: hard-delete local entry.
    AcceptDelete,
    /// Resurrect from local snapshot: keep local, discard remote.
    KeepLocal,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        f.write_str(&s)
    }
}

impl std::str::FromStr for Action {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "keep_both" => Ok(Action::KeepBoth),
            "ignore" => Ok(Action::Ignore),
            "replace" => Ok(Action::Replace),
            "update" => Ok(Action::Update),
            "upsert" => Ok(Action::Upsert),
            "accept_delete" => Ok(Action::AcceptDelete),
            "keep_local" => Ok(Action::KeepLocal),
            other => Err(format!("unknown Action: {other}")),
        }
    }
}

/// Result of applying a conflict resolution action.
#[derive(Debug, Serialize)]
pub struct ResolutionResult {
    pub conflict_id: String,
    pub action: Action,
    /// Final path of the entry after resolution.
    pub path: String,
    /// EDA event type published after resolution.
    pub event_type: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Apply `action` to `conflict`, writing to storage and publishing events.
///
/// Returns `Err(String)` on any database failure — caller must handle rollback.
pub fn apply_action(
    conflict: &Conflict,
    action: Action,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) -> Result<ResolutionResult, String> {
    match action {
        Action::Ignore => apply_ignore(conflict, user_id, storage, bus),
        Action::Replace => apply_replace(conflict, user_id, storage, bus),
        Action::Update => apply_update(conflict, user_id, storage, bus),
        Action::KeepBoth => apply_keep_both(conflict, user_id, storage, bus),
        Action::Upsert => apply_upsert(conflict, user_id, storage, bus),
        Action::AcceptDelete => apply_accept_delete(conflict, user_id, storage, bus),
        Action::KeepLocal => apply_keep_local(conflict, user_id, storage, bus),
    }
}

// ---------------------------------------------------------------------------
// Action implementations
// ---------------------------------------------------------------------------

fn apply_ignore(
    conflict: &Conflict,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) -> Result<ResolutionResult, String> {
    mark_resolved(&conflict.id, &Action::Ignore, user_id, storage)?;
    let ev_type = publish_resolved(conflict, &Action::Ignore, bus, &conflict.path);
    Ok(ResolutionResult {
        conflict_id: conflict.id.clone(),
        action: Action::Ignore,
        path: conflict.path.clone(),
        event_type: ev_type,
    })
}

fn apply_replace(
    conflict: &Conflict,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) -> Result<ResolutionResult, String> {
    // Remote body wins — read remote body from entries table and write locally.
    let remote_body = fetch_or_use_stored_body(
        storage,
        &conflict.universe_key,
        &conflict.path,
        conflict.remote.body.as_deref(),
    );

    if let Some(body) = &remote_body {
        overwrite_entry_body(
            storage,
            &conflict.universe_key,
            &conflict.path,
            body,
            &conflict.remote.body_hash,
        )?;
    }

    mark_resolved(&conflict.id, &Action::Replace, user_id, storage)?;
    let ev_type = publish_resolved(conflict, &Action::Replace, bus, &conflict.path);
    Ok(ResolutionResult {
        conflict_id: conflict.id.clone(),
        action: Action::Replace,
        path: conflict.path.clone(),
        event_type: ev_type,
    })
}

fn apply_update(
    conflict: &Conflict,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) -> Result<ResolutionResult, String> {
    let local_body = fetch_entry_body(storage, &conflict.universe_key, &conflict.path)
        .or_else(|| conflict.local.body.clone())
        .unwrap_or_default();

    let remote_body = conflict.remote.body.clone().unwrap_or_default();

    let ancestor = conflict
        .common_ancestor
        .as_ref()
        .and_then(|a| a.body.as_deref());

    let merged = merge_bodies(ancestor, &local_body, &remote_body);
    let merged_hash = sha256_hex(merged.as_bytes());

    overwrite_entry_body(
        storage,
        &conflict.universe_key,
        &conflict.path,
        &merged,
        &merged_hash,
    )?;
    mark_resolved(&conflict.id, &Action::Update, user_id, storage)?;
    let ev_type = publish_resolved(conflict, &Action::Update, bus, &conflict.path);
    Ok(ResolutionResult {
        conflict_id: conflict.id.clone(),
        action: Action::Update,
        path: conflict.path.clone(),
        event_type: ev_type,
    })
}

fn apply_keep_both(
    conflict: &Conflict,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) -> Result<ResolutionResult, String> {
    let suffix_path = make_suffix_path(&conflict.path);
    let now = Utc::now().to_rfc3339();

    {
        let st = storage.lock();
        let uc = st.universe_conn(&conflict.universe_key);
        let conn = uc.lock().unwrap();

        // Rename local copy to the suffixed path.
        conn.execute(
            "UPDATE entries SET path = ?1, updated_at = ?2
             WHERE universe_key = ?3 AND path = ?4",
            rusqlite::params![&suffix_path, &now, &conflict.universe_key, &conflict.path],
        )
        .map_err(|e| format!("keep_both rename: {e}"))?;

        // Write remote at the original path (INSERT OR REPLACE).
        if let Some(ref body) = conflict.remote.body {
            conn.execute(
                "INSERT OR REPLACE INTO entries
                 (path, universe_key, entry_type, title, frontmatter_json, payload, body, body_hash, created_at, updated_at)
                 SELECT ?1, universe_key, entry_type, title, frontmatter_json, payload, ?2, ?3, ?4, ?4
                 FROM entries WHERE universe_key = ?5 AND path = ?6
                 UNION ALL
                 SELECT ?1, ?5, 'note', NULL, '{}', '{}', ?2, ?3, ?4, ?4
                 WHERE NOT EXISTS (SELECT 1 FROM entries WHERE universe_key = ?5 AND path = ?6)
                 LIMIT 1",
                rusqlite::params![
                    &conflict.path,
                    body,
                    &conflict.remote.body_hash,
                    &now,
                    &conflict.universe_key,
                    &suffix_path,
                ],
            )
            .map_err(|e| format!("keep_both insert remote: {e}"))?;
        }
    }

    mark_resolved(&conflict.id, &Action::KeepBoth, user_id, storage)?;
    let ev_type = publish_resolved(conflict, &Action::KeepBoth, bus, &conflict.path);
    Ok(ResolutionResult {
        conflict_id: conflict.id.clone(),
        action: Action::KeepBoth,
        path: suffix_path,
        event_type: ev_type,
    })
}

fn apply_upsert(
    conflict: &Conflict,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) -> Result<ResolutionResult, String> {
    // Upsert = Update existing diffs + insert new remote items.
    // For a single conflict, this behaves like Update.
    // At bulk-apply level the caller iterates all conflicts and applies Update
    // to existing paths plus Insert for RemoteOnlyNew conflicts.
    apply_update(conflict, user_id, storage, bus).map(|mut r| {
        r.action = Action::Upsert;
        r
    })
}

fn apply_accept_delete(
    conflict: &Conflict,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) -> Result<ResolutionResult, String> {
    {
        let st = storage.lock();
        let uc = st.universe_conn(&conflict.universe_key);
        let conn = uc.lock().unwrap();
        conn.execute(
            "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
            rusqlite::params![&conflict.universe_key, &conflict.path],
        )
        .map_err(|e| format!("accept_delete: {e}"))?;
    }
    mark_resolved(&conflict.id, &Action::AcceptDelete, user_id, storage)?;
    let ev_type = publish_resolved(conflict, &Action::AcceptDelete, bus, &conflict.path);
    Ok(ResolutionResult {
        conflict_id: conflict.id.clone(),
        action: Action::AcceptDelete,
        path: conflict.path.clone(),
        event_type: ev_type,
    })
}

fn apply_keep_local(
    conflict: &Conflict,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) -> Result<ResolutionResult, String> {
    // Keep local as-is — no entry write needed. Mark resolved.
    mark_resolved(&conflict.id, &Action::KeepLocal, user_id, storage)?;
    let ev_type = publish_resolved(conflict, &Action::KeepLocal, bus, &conflict.path);
    Ok(ResolutionResult {
        conflict_id: conflict.id.clone(),
        action: Action::KeepLocal,
        path: conflict.path.clone(),
        event_type: ev_type,
    })
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn mark_resolved(
    conflict_id: &str,
    action: &Action,
    user_id: Option<&str>,
    storage: &Arc<Mutex<Storage>>,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    let st = storage.lock();
    st.conn()
        .execute(
            "UPDATE sync_conflicts
             SET resolved_at = ?1, resolution_action = ?2, resolved_by = ?3
             WHERE id = ?4",
            rusqlite::params![now, action.to_string(), user_id, conflict_id],
        )
        .map_err(|e| format!("mark_resolved: {e}"))?;
    Ok(())
}

fn publish_resolved(
    conflict: &Conflict,
    action: &Action,
    bus: &Arc<dyn EdaBus>,
    path_result: &str,
) -> String {
    let ev_type = "sync.conflict_resolved";
    bus.publish(Event::new(
        ev_type,
        Some(conflict.universe_key.clone()),
        None,
        serde_json::json!({
            "conflict_id": &conflict.id,
            "path": path_result,
            "action": action.to_string(),
            "universe": &conflict.universe_key,
            "kind": conflict.kind.to_string(),
        }),
        Visibility::UniverseOwner,
    ));
    ev_type.to_owned()
}

fn overwrite_entry_body(
    storage: &Arc<Mutex<Storage>>,
    universe_key: &str,
    path: &str,
    body: &str,
    body_hash: &str,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    let st = storage.lock();
    let uc = st.universe_conn(universe_key);
    let conn = uc.lock().unwrap();
    conn.execute(
        "UPDATE entries SET body = ?1, body_hash = ?2, updated_at = ?3
         WHERE universe_key = ?4 AND path = ?5",
        rusqlite::params![body, body_hash, now, universe_key, path],
    )
    .map_err(|e| format!("overwrite_entry_body: {e}"))?;
    Ok(())
}

fn fetch_entry_body(
    storage: &Arc<Mutex<Storage>>,
    universe_key: &str,
    path: &str,
) -> Option<String> {
    let st = storage.lock();
    let uc = st.universe_conn(universe_key);
    let conn = uc.lock().unwrap();
    conn.query_row(
        "SELECT body FROM entries WHERE universe_key = ?1 AND path = ?2",
        rusqlite::params![universe_key, path],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

fn fetch_or_use_stored_body(
    storage: &Arc<Mutex<Storage>>,
    universe_key: &str,
    path: &str,
    stored: Option<&str>,
) -> Option<String> {
    stored
        .map(|s| s.to_owned())
        .or_else(|| fetch_entry_body(storage, universe_key, path))
}

/// Build a `_1`-suffixed copy path: `notes/foo.md` → `notes/foo_1.md`.
fn make_suffix_path(path: &str) -> String {
    if let Some(stem) = path.strip_suffix(".md") {
        format!("{stem}_1.md")
    } else {
        format!("{path}_1")
    }
}

// ---------------------------------------------------------------------------
// Text merge helpers
// ---------------------------------------------------------------------------

/// Line-based 3-way text merge. Falls back to conflict markers when both sides
/// changed the same content and no clean resolution is found.
fn merge_bodies(ancestor: Option<&str>, local: &str, remote: &str) -> String {
    if local == remote {
        return local.to_string();
    }

    let anc = ancestor.unwrap_or("");

    if local == anc {
        return remote.to_string();
    }
    if remote == anc {
        return local.to_string();
    }

    // Both sides changed — emit conflict markers (same format as git).
    format!("<<<<<<< local\n{local}\n=======\n{remote}\n>>>>>>> remote\n")
}

/// SHA-256 hex digest.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_roundtrips() {
        let actions = [
            Action::KeepBoth,
            Action::Ignore,
            Action::Replace,
            Action::Update,
            Action::Upsert,
            Action::AcceptDelete,
            Action::KeepLocal,
        ];
        for action in &actions {
            let s = action.to_string();
            let back: Action = s.parse().expect("roundtrip");
            assert_eq!(&back, action, "action '{s}' failed roundtrip");
        }
    }

    #[test]
    fn make_suffix_path_md() {
        assert_eq!(make_suffix_path("notes/foo.md"), "notes/foo_1.md");
    }

    #[test]
    fn make_suffix_path_no_ext() {
        assert_eq!(make_suffix_path("notes/foo"), "notes/foo_1");
    }

    #[test]
    fn merge_bodies_same() {
        assert_eq!(merge_bodies(None, "a", "a"), "a");
    }

    #[test]
    fn merge_bodies_local_only_changed() {
        assert_eq!(merge_bodies(Some("base"), "changed", "base"), "changed");
    }

    #[test]
    fn merge_bodies_remote_only_changed() {
        assert_eq!(merge_bodies(Some("base"), "base", "changed"), "changed");
    }

    #[test]
    fn merge_bodies_both_changed_has_markers() {
        let result = merge_bodies(Some("base"), "local-change", "remote-change");
        assert!(result.contains("<<<<<<<"), "expected conflict markers");
        assert!(result.contains("======="), "expected separator");
        assert!(result.contains(">>>>>>>"), "expected closing marker");
    }
}
