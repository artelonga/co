//! CO-472 — unified thread model storage.
//!
//! A **thread** is the same row as a `chat_rooms` record (a *message is a
//! comment in a thread*; comments and chat messages are one primitive). This
//! module adds the thread-shaped operations on top of the v092 columns:
//!
//! - **anchored thread** (`anchor_entry_path`, `anchor_block_id`) — the thread
//!   IS the "comments" on an entry. [`Storage::get_or_create_anchored_thread`]
//!   lazily creates it on first comment.
//! - **recursive replies** (`parent_thread_id`) — a child thread spawned from a
//!   message. [`Storage::create_reply_thread`].
//! - **member scope** (`scope`) — `universe` (all members), `subset` (explicit
//!   `chat_room_members` list), or `self` (single-member notes-to-self).
//!   [`Storage::can_access_thread`] enforces it.
//!
//! Message storage is unchanged: [`Storage::post_chat_message`] +
//! `chat_messages`. Live delivery is unchanged: the route layer publishes a
//! `ChatEvent` via `broadcast_event` (CO-468), exactly as chat does.

use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::Storage;

/// A thread = a generalized chat room. Carries the CO-472 thread columns on top
/// of the legacy room fields the SPA already consumes.
#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub id: String,
    pub universe_key: String,
    pub slug: String,
    pub created_by: String,
    pub created_at: String,
    /// `NULL` ⇒ standalone (chat room / DM / notes); set ⇒ anchored to an entry.
    pub anchor_entry_path: Option<String>,
    /// `NULL` ⇒ whole-entry anchor; set ⇒ a specific block on that entry.
    pub anchor_block_id: Option<String>,
    /// `NULL` ⇒ top-level; set ⇒ a recursive child (nested replies).
    pub parent_thread_id: Option<String>,
    /// `universe` (all members) | `subset` (explicit list) | `self` (creator).
    pub scope: String,
}

const THREAD_COLS: &str = "id, universe_key, slug, created_by, created_at, \
     anchor_entry_path, anchor_block_id, parent_thread_id, COALESCE(scope,'universe')";

fn map_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: row.get(0)?,
        universe_key: row.get(1)?,
        slug: row.get(2)?,
        created_by: row.get(3)?,
        created_at: row.get(4)?,
        anchor_entry_path: row.get(5)?,
        anchor_block_id: row.get(6)?,
        parent_thread_id: row.get(7)?,
        scope: row.get(8)?,
    })
}

impl Storage {
    /// Fetch a thread (room) row by id, including the CO-472 thread columns.
    pub fn get_thread_by_id(&self, thread_id: &str) -> Option<Thread> {
        self.conn
            .query_row(
                &format!("SELECT {THREAD_COLS} FROM chat_rooms WHERE id = ?1"),
                params![thread_id],
                map_thread,
            )
            .optional()
            .unwrap_or(None)
    }

    /// CO-472: get (or lazily create) the **anchored thread** for an entry — the
    /// thread that IS the entry's comments. `block_id = None` is the entry-level
    /// comments thread; a `Some(block_id)` anchors a per-block comments thread.
    ///
    /// Scope is `universe` (all universe members can read/post) — anchored
    /// comments follow the universe visibility gate plus membership, enforced at
    /// the route layer. Idempotent: the first comment creates the thread; later
    /// comments reuse it.
    pub fn get_or_create_anchored_thread(
        &self,
        universe_key: &str,
        anchor_entry_path: &str,
        anchor_block_id: Option<&str>,
        created_by: &str,
    ) -> anyhow::Result<Thread> {
        // Match on (universe, path, block). NULL block must match NULL, so use
        // `IS` semantics via a COALESCE sentinel comparison.
        if let Some(existing) = self
            .conn
            .query_row(
                &format!(
                    "SELECT {THREAD_COLS} FROM chat_rooms \
                     WHERE universe_key = ?1 AND anchor_entry_path = ?2 \
                       AND COALESCE(anchor_block_id,'') = COALESCE(?3,'') \
                       AND parent_thread_id IS NULL \
                     LIMIT 1"
                ),
                params![universe_key, anchor_entry_path, anchor_block_id],
                map_thread,
            )
            .optional()?
        {
            return Ok(existing);
        }

        let id = format!("thr_{}", nanoid::nanoid!(10));
        let slug = format!("comments-{}", nanoid::nanoid!(8));
        let now = Utc::now().to_rfc3339();
        let name = format!("comments:{anchor_entry_path}");
        self.conn.execute(
            "INSERT INTO chat_rooms \
             (id, universe_key, name, slug, description, created_by, created_at, is_default, kind, \
              anchor_entry_path, anchor_block_id, parent_thread_id, scope) \
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, 0, 'universe', ?7, ?8, NULL, 'universe')",
            params![
                id,
                universe_key,
                name,
                slug,
                created_by,
                now,
                anchor_entry_path,
                anchor_block_id
            ],
        )?;
        self.get_thread_by_id(&id)
            .ok_or_else(|| anyhow::anyhow!("CO-472: anchored thread vanished after insert"))
    }

    /// CO-472: read-only lookup of the anchored thread for an entry/block.
    /// `None` ⇒ no comment has been posted yet (the thread is lazily created on
    /// the first comment by [`Storage::get_or_create_anchored_thread`]).
    pub fn find_anchored_thread(
        &self,
        universe_key: &str,
        anchor_entry_path: &str,
        anchor_block_id: Option<&str>,
    ) -> Option<Thread> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {THREAD_COLS} FROM chat_rooms \
                     WHERE universe_key = ?1 AND anchor_entry_path = ?2 \
                       AND COALESCE(anchor_block_id,'') = COALESCE(?3,'') \
                       AND parent_thread_id IS NULL \
                     LIMIT 1"
                ),
                params![universe_key, anchor_entry_path, anchor_block_id],
                map_thread,
            )
            .optional()
            .unwrap_or(None)
    }

    /// CO-472: ids of the recursive child threads (nested replies) of a parent.
    pub fn list_child_thread_ids(&self, parent_thread_id: &str) -> Vec<String> {
        let mut stmt = match self.conn.prepare(
            "SELECT id FROM chat_rooms WHERE parent_thread_id = ?1 ORDER BY created_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![parent_thread_id], |r| r.get(0))
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    /// CO-472: create a **scoped thread** — `subset` (only the listed members
    /// may read/post) or `self` (a single-member notes-to-self thread). The
    /// creator is always added as a member; for `self`, `members` is ignored.
    /// Standalone (no anchor, no parent).
    pub fn create_scoped_thread(
        &self,
        universe_key: &str,
        scope: &str,
        created_by: &str,
        members: &[String],
    ) -> anyhow::Result<Thread> {
        let scope = match scope {
            "self" | "subset" | "universe" => scope,
            other => anyhow::bail!("CO-472: unknown thread scope '{other}'"),
        };
        let id = format!("thr_{}", nanoid::nanoid!(10));
        let slug = format!("thread-{}", nanoid::nanoid!(8));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO chat_rooms \
             (id, universe_key, name, slug, description, created_by, created_at, is_default, kind, \
              anchor_entry_path, anchor_block_id, parent_thread_id, scope) \
             VALUES (?1, ?2, '', ?3, NULL, ?4, ?5, 0, 'universe', NULL, NULL, NULL, ?6)",
            params![id, universe_key, slug, created_by, now, scope],
        )?;

        // Always make the creator a member. For `self` that's the only member.
        self.add_thread_member(&id, created_by)?;
        if scope == "subset" {
            for m in members {
                if m != created_by {
                    self.add_thread_member(&id, m)?;
                }
            }
        }
        self.get_thread_by_id(&id)
            .ok_or_else(|| anyhow::anyhow!("CO-472: scoped thread vanished after insert"))
    }

    /// CO-472: create a **recursive child thread** rooted at a parent thread —
    /// nested replies. Inherits the parent's universe + scope. For `subset`/
    /// `self` scope the child copies the parent's membership rows so access is
    /// consistent across the reply tree.
    pub fn create_reply_thread(
        &self,
        parent_thread_id: &str,
        created_by: &str,
    ) -> anyhow::Result<Thread> {
        let parent = self
            .get_thread_by_id(parent_thread_id)
            .ok_or_else(|| anyhow::anyhow!("CO-472: parent thread not found"))?;
        let id = format!("thr_{}", nanoid::nanoid!(10));
        let slug = format!("reply-{}", nanoid::nanoid!(8));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO chat_rooms \
             (id, universe_key, name, slug, description, created_by, created_at, is_default, kind, \
              anchor_entry_path, anchor_block_id, parent_thread_id, scope) \
             VALUES (?1, ?2, '', ?3, NULL, ?4, ?5, 0, 'universe', ?6, ?7, ?8, ?9)",
            params![
                id,
                parent.universe_key,
                slug,
                created_by,
                now,
                parent.anchor_entry_path,
                parent.anchor_block_id,
                parent_thread_id,
                parent.scope
            ],
        )?;
        // Copy membership for scoped reply trees so members keep access.
        if parent.scope == "subset" || parent.scope == "self" {
            self.conn.execute(
                "INSERT OR IGNORE INTO chat_room_members (room_id, user_id, joined_at) \
                 SELECT ?1, user_id, ?2 FROM chat_room_members WHERE room_id = ?3",
                params![id, now, parent_thread_id],
            )?;
            self.add_thread_member(&id, created_by)?;
        }
        self.get_thread_by_id(&id)
            .ok_or_else(|| anyhow::anyhow!("CO-472: reply thread vanished after insert"))
    }

    /// Add (idempotently) a member to a thread.
    pub fn add_thread_member(&self, thread_id: &str, user_id: &str) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO chat_room_members (room_id, user_id, joined_at) \
             VALUES (?1, ?2, ?3)",
            params![thread_id, user_id, now],
        )?;
        Ok(())
    }

    /// True when `user_id` has an explicit membership row in the thread.
    pub fn is_thread_member(&self, thread_id: &str, user_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM chat_room_members WHERE room_id = ?1 AND user_id = ?2",
                params![thread_id, user_id],
                |_| Ok(true),
            )
            .optional()
            .unwrap_or(None)
            .unwrap_or(false)
    }

    /// CO-472: scope-aware read/post authorization for a thread.
    ///
    /// - `universe` — any caller with a `universe_role` (member/subscriber/…)
    ///   passes; anchored comments rely on the universe visibility gate plus
    ///   this membership check, so the caller must be a universe member.
    /// - `subset` — only explicit thread members.
    /// - `self` — only the explicit member (the creator); private notes.
    ///
    /// `universe_role` is the caller's resolved role in the thread's universe
    /// (`None` ⇒ anonymous / non-member). Returns `true` if the caller may
    /// read and post in the thread.
    pub fn can_access_thread(
        &self,
        thread: &Thread,
        user_id: &str,
        has_universe_role: bool,
    ) -> bool {
        match thread.scope.as_str() {
            "self" | "subset" => self.is_thread_member(&thread.id, user_id),
            // 'universe' (and any unknown value, defensively) → universe membership.
            _ => has_universe_role,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    fn seed(storage: &Storage) {
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, created_at) VALUES ('owner','o@x','now'), \
                 ('member','m@x','now'), ('outsider','out@x','now')",
                [],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT INTO universes (key, name, description, owner_id, created_at) \
                 VALUES ('u','U','','owner','now')",
                [],
            )
            .unwrap();
    }

    #[test]
    fn anchored_thread_is_idempotent_per_entry() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        seed(&storage);

        let t1 = storage
            .get_or_create_anchored_thread("u", "content/a.md", None, "owner")
            .unwrap();
        let t2 = storage
            .get_or_create_anchored_thread("u", "content/a.md", None, "member")
            .unwrap();
        assert_eq!(t1.id, t2.id, "same entry → same anchored thread");
        assert_eq!(t1.anchor_entry_path.as_deref(), Some("content/a.md"));
        assert_eq!(t1.scope, "universe");

        // A different block id → a distinct thread.
        let t3 = storage
            .get_or_create_anchored_thread("u", "content/a.md", Some("blk-1"), "owner")
            .unwrap();
        assert_ne!(t1.id, t3.id, "per-block anchor is a separate thread");
    }

    #[test]
    fn self_thread_is_private_to_creator() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        seed(&storage);

        let t = storage
            .create_scoped_thread("u", "self", "owner", &[])
            .unwrap();
        assert_eq!(t.scope, "self");
        assert!(storage.can_access_thread(&t, "owner", true));
        assert!(
            !storage.can_access_thread(&t, "member", true),
            "self-thread is private even to other universe members"
        );
    }

    #[test]
    fn subset_thread_admits_members_only() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        seed(&storage);

        let t = storage
            .create_scoped_thread("u", "subset", "owner", &["member".into()])
            .unwrap();
        assert!(storage.can_access_thread(&t, "owner", true));
        assert!(storage.can_access_thread(&t, "member", true));
        assert!(
            !storage.can_access_thread(&t, "outsider", true),
            "non-member rejected despite having a universe role"
        );
    }

    #[test]
    fn reply_thread_is_recursive_child() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        seed(&storage);

        let parent = storage
            .get_or_create_anchored_thread("u", "content/a.md", None, "owner")
            .unwrap();
        let child = storage.create_reply_thread(&parent.id, "member").unwrap();
        assert_eq!(child.parent_thread_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(
            child.anchor_entry_path, parent.anchor_entry_path,
            "child inherits the anchor"
        );
    }
}
