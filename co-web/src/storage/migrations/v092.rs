use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-472: unified thread model — generalize `chat_rooms` into a **thread**.
    ///
    /// A *message is a comment in a thread*; comments and chat messages are the
    /// same primitive. Rather than build a parallel `comments` table, CO-472
    /// adds the columns a chat room needs to also serve as:
    ///
    /// - **anchored thread** — `anchor_entry_path` (and optional
    ///   `anchor_block_id`) point the thread at an entry; that anchored thread
    ///   IS the "comments" on that content. `NULL` ⇒ a standalone chat room / DM
    ///   / notes thread (the pre-CO-472 behavior, unchanged).
    /// - **recursive replies** — `parent_thread_id` links a child thread to the
    ///   message it spawned from; `NULL` ⇒ a top-level thread.
    /// - **member scope** — `scope`: `universe` (all universe members — the
    ///   legacy default, preserved), `subset` (only rows in `chat_room_members`
    ///   may read/post), or `self` (a single-member notes-to-self thread).
    ///
    /// All four columns are additive + idempotent (`ensure_column` — a guarded
    /// `ALTER TABLE`), defaulting to the legacy semantics so every existing chat
    /// room keeps working as an unanchored, all-universe-scope thread. Never
    /// folded into the base `chat_rooms` create (which also adds them, for fresh
    /// DBs) per the migration-version-claim protocol.
    ///
    /// Migration version **v092** (train tip v091; claimed as max + 1). Each
    /// guard checks `current_version < N` independently so fresh and existing
    /// DBs converge regardless of sibling-merge order.
    pub(super) fn migrate_v092(&mut self, current_version: i64) {
        if current_version < 92 {
            // NULL ⇒ standalone (chat room / DM / notes); set ⇒ anchored to an entry.
            ensure_column(&self.conn, "chat_rooms", "anchor_entry_path", "TEXT")
                .expect("migration v92: chat_rooms.anchor_entry_path");
            // NULL ⇒ whole-entry anchor; set ⇒ a specific block on that entry.
            ensure_column(&self.conn, "chat_rooms", "anchor_block_id", "TEXT")
                .expect("migration v92: chat_rooms.anchor_block_id");
            // NULL ⇒ top-level thread; set ⇒ a recursive child (nested replies).
            ensure_column(&self.conn, "chat_rooms", "parent_thread_id", "TEXT")
                .expect("migration v92: chat_rooms.parent_thread_id");
            // 'universe' (all members) | 'subset' (explicit list) | 'self' (creator only).
            ensure_column(
                &self.conn,
                "chat_rooms",
                "scope",
                "TEXT NOT NULL DEFAULT 'universe'",
            )
            .expect("migration v92: chat_rooms.scope");

            // Index the anchor so the `comments` view (anchored thread for an
            // entry) is a single indexed lookup, not a table scan.
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_chat_rooms_anchor
                         ON chat_rooms(universe_key, anchor_entry_path);
                     CREATE INDEX IF NOT EXISTS idx_chat_rooms_parent
                         ON chat_rooms(parent_thread_id);",
                )
                .expect("migration v92: chat_rooms thread indexes");

            crate::record_migration!(
                self.conn,
                92,
                "CO-472: chat_rooms thread columns (anchor_entry_path/anchor_block_id/parent_thread_id/scope)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn thread_columns_exist_with_legacy_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        // Seed a user + universe + a chat room via the existing path.
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, created_at) VALUES ('u1', 'a@b.c', 'now')",
                [],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT INTO universes (key, name, description, owner_id, created_at) \
                 VALUES ('k1', 'K', '', 'u1', 'now')",
                [],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT INTO chat_rooms \
                 (id, universe_key, name, slug, created_by, created_at) \
                 VALUES ('r1', 'k1', 'R', 'r', 'u1', 'now')",
                [],
            )
            .unwrap();

        let (anchor, block, parent, scope): (
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        ) = storage
            .conn()
            .query_row(
                "SELECT anchor_entry_path, anchor_block_id, parent_thread_id, scope \
                 FROM chat_rooms WHERE id = 'r1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(anchor, None, "anchor_entry_path defaults NULL (standalone)");
        assert_eq!(block, None, "anchor_block_id defaults NULL");
        assert_eq!(parent, None, "parent_thread_id defaults NULL (top-level)");
        assert_eq!(scope, "universe", "scope defaults to all-universe-members");
    }

    #[test]
    fn migration_v092_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        storage.migrate_v092(91);
        storage.migrate_v092(92);

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('chat_rooms') \
                 WHERE name = 'anchor_entry_path'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "exactly one anchor_entry_path column");
    }
}
