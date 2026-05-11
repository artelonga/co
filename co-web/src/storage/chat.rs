use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::Storage;
use super::schema::{ensure_column, ensure_table};

#[derive(Debug, Clone, Serialize)]
pub struct ChatRoom {
    pub id: String,
    pub universe_key: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub created_by: String,
    pub created_at: String,
    pub archived_at: Option<String>,
    pub kind: String,
}

// ---------------------------------------------------------------------------
// CO-198: DM types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DmRoom {
    pub room_id: String,
    pub slug: String,
    pub other_user: ChatAuthor,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DmLastMessage {
    pub id: String,
    pub author_id: String,
    pub body_preview: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DmThreadPreview {
    pub room_id: String,
    pub slug: String,
    pub other_user: ChatAuthor,
    pub last_message: Option<DmLastMessage>,
    pub unread_count: i64,
    pub last_read_at: Option<String>,
    pub muted_until: Option<String>,
}

#[derive(Debug)]
pub enum DmError {
    SelfDm,
    Blocked,
    PolicyDenied,
    UserNotFound,
    Db(rusqlite::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatAuthor {
    pub user_id: String,
    pub display_name: String,
    pub usuario: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessageWithAuthor {
    pub id: String,
    pub author: ChatAuthor,
    pub body: String,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub deleted_at: Option<String>,
    pub reply_to_id: Option<String>,
}

#[derive(Debug)]
pub enum EditError {
    NotFound,
    NotAuthor,
    AlreadyDeleted,
    EditWindowExpired,
    BodyTooLong,
    BodyEmpty,
    Db(rusqlite::Error),
}

#[derive(Debug)]
pub enum DeleteError {
    NotFound,
    NotAuthorAndNotMod,
    AlreadyDeleted,
    Db(rusqlite::Error),
}

fn slugify(name: &str) -> String {
    let raw: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    raw.split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

impl Storage {
    pub(crate) fn ensure_chat_tables(&self) {
        ensure_table(
            &self.conn,
            "chat_rooms",
            "CREATE TABLE IF NOT EXISTS chat_rooms (
                id              TEXT PRIMARY KEY,
                universe_key    TEXT NOT NULL,
                name            TEXT NOT NULL,
                slug            TEXT NOT NULL,
                description     TEXT,
                created_by      TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                archived_at     TEXT,
                is_default      INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (universe_key) REFERENCES universes(key)
            );",
        )
        .expect("CO-193: chat_rooms table");
        self.conn
            .execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_rooms_universe_slug
                     ON chat_rooms(universe_key, slug);
                 CREATE INDEX IF NOT EXISTS idx_chat_rooms_universe_archived
                     ON chat_rooms(universe_key, archived_at);",
            )
            .expect("CO-193: chat_rooms indexes");

        ensure_table(
            &self.conn,
            "chat_messages",
            "CREATE TABLE IF NOT EXISTS chat_messages (
                id              TEXT PRIMARY KEY,
                room_id         TEXT NOT NULL,
                author_id       TEXT NOT NULL,
                body            TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                edited_at       TEXT,
                deleted_at      TEXT,
                reply_to_id     TEXT,
                FOREIGN KEY (room_id)     REFERENCES chat_rooms(id),
                FOREIGN KEY (author_id)   REFERENCES users(id),
                FOREIGN KEY (reply_to_id) REFERENCES chat_messages(id)
            );",
        )
        .expect("CO-193: chat_messages table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_chat_messages_room_created
                     ON chat_messages(room_id, created_at);
                 CREATE INDEX IF NOT EXISTS idx_chat_messages_room_deleted
                     ON chat_messages(room_id, deleted_at);",
            )
            .expect("CO-193: chat_messages indexes");

        // CO-198: add `kind` column to chat_rooms (universe | dm).
        ensure_column(
            &self.conn,
            "chat_rooms",
            "kind",
            "TEXT NOT NULL DEFAULT 'universe'",
        )
        .expect("CO-198: chat_rooms.kind");

        // CO-198: generic membership table (subsumes universe + dm read-state).
        ensure_table(
            &self.conn,
            "chat_room_members",
            "CREATE TABLE IF NOT EXISTS chat_room_members (
                room_id      TEXT NOT NULL,
                user_id      TEXT NOT NULL,
                last_read_at TEXT,
                muted_until  TEXT,
                joined_at    TEXT NOT NULL,
                PRIMARY KEY (room_id, user_id),
                FOREIGN KEY (room_id) REFERENCES chat_rooms(id)
            );",
        )
        .expect("CO-198: chat_room_members table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_chat_room_members_user
                     ON chat_room_members(user_id);",
            )
            .expect("CO-198: chat_room_members index");

        // CO-198: dm_policy column on users.
        ensure_column(
            &self.conn,
            "users",
            "dm_policy",
            "TEXT NOT NULL DEFAULT 'shared-universe'",
        )
        .expect("CO-198: users.dm_policy");

        // CO-198: user_blocks table.
        ensure_table(
            &self.conn,
            "user_blocks",
            "CREATE TABLE IF NOT EXISTS user_blocks (
                blocker_id TEXT NOT NULL,
                blocked_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                reason     TEXT,
                PRIMARY KEY (blocker_id, blocked_id)
            );",
        )
        .expect("CO-198: user_blocks table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_user_blocks_blocked
                     ON user_blocks(blocked_id);",
            )
            .expect("CO-198: user_blocks index");

        // CO-198: seed sentinel rows needed for the FK chain
        // chat_rooms.universe_key='dm' → universes.key='dm' → users.id='system'.
        // These are stable no-op anchors: INSERT OR IGNORE means they only insert once.
        // email is NULL (not '') to avoid the UNIQUE users.email constraint.
        self.conn
            .execute_batch(
                "INSERT OR IGNORE INTO users (id, email, display_name, tier, created_at) \
                 VALUES ('system', NULL, 'System', 'admin', '2020-01-01T00:00:00Z');
                 INSERT OR IGNORE INTO universes (key, name, description, owner_id, created_at) \
                 VALUES ('dm', '', '', 'system', '2020-01-01T00:00:00Z');",
            )
            .expect("CO-198: sentinel user+universe for DM FK anchor");
    }

    /// Insert a `general` room for `universe_key` if one does not exist yet.
    pub fn ensure_default_room(&self, universe_key: &str) -> anyhow::Result<()> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM chat_rooms WHERE universe_key = ?1 AND slug = 'general'",
                params![universe_key],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !exists {
            let id = format!("room_{}", nanoid::nanoid!(10));
            let now = Utc::now().to_rfc3339();
            let created_by: String = self
                .conn
                .query_row(
                    "SELECT owner_id FROM universes WHERE key = ?1",
                    params![universe_key],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "system".to_string());
            self.conn.execute(
                "INSERT OR IGNORE INTO chat_rooms \
                 (id, universe_key, name, slug, description, created_by, created_at, is_default, kind) \
                 VALUES (?1, ?2, 'Geral', 'general', NULL, ?3, ?4, 1, 'universe')",
                params![id, universe_key, created_by, now],
            )?;
        }
        Ok(())
    }

    pub fn list_chat_rooms(&self, universe_key: &str, include_archived: bool) -> Vec<ChatRoom> {
        let sql = if include_archived {
            "SELECT id, universe_key, name, slug, description, is_default, created_by, \
             created_at, archived_at, COALESCE(kind,'universe') FROM chat_rooms \
             WHERE universe_key = ?1 ORDER BY is_default DESC, created_at ASC"
        } else {
            "SELECT id, universe_key, name, slug, description, is_default, created_by, \
             created_at, archived_at, COALESCE(kind,'universe') FROM chat_rooms \
             WHERE universe_key = ?1 AND archived_at IS NULL \
             ORDER BY is_default DESC, created_at ASC"
        };
        let mut stmt = self.conn.prepare(sql).expect("prepare list_chat_rooms");
        stmt.query_map(params![universe_key], |row| {
            Ok(ChatRoom {
                id: row.get(0)?,
                universe_key: row.get(1)?,
                name: row.get(2)?,
                slug: row.get(3)?,
                description: row.get(4)?,
                is_default: row.get::<_, i64>(5)? != 0,
                created_by: row.get(6)?,
                created_at: row.get(7)?,
                archived_at: row.get(8)?,
                kind: row.get(9)?,
            })
        })
        .expect("list_chat_rooms query")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn create_chat_room(
        &self,
        universe_key: &str,
        name: &str,
        description: Option<&str>,
        created_by: &str,
    ) -> anyhow::Result<ChatRoom> {
        let slug = slugify(name);
        if slug.is_empty() {
            anyhow::bail!("room name produces empty slug");
        }
        let id = format!("room_{}", nanoid::nanoid!(10));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO chat_rooms \
             (id, universe_key, name, slug, description, created_by, created_at, is_default, kind) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'universe')",
            params![id, universe_key, name, slug, description, created_by, now],
        )?;
        Ok(ChatRoom {
            id,
            universe_key: universe_key.to_string(),
            name: name.to_string(),
            slug,
            description: description.map(|s| s.to_string()),
            is_default: false,
            created_by: created_by.to_string(),
            created_at: now,
            archived_at: None,
            kind: "universe".to_string(),
        })
    }

    pub fn get_chat_room_by_slug(&self, universe_key: &str, slug: &str) -> Option<ChatRoom> {
        self.conn
            .query_row(
                "SELECT id, universe_key, name, slug, description, is_default, created_by, \
                 created_at, archived_at, COALESCE(kind,'universe') FROM chat_rooms \
                 WHERE universe_key = ?1 AND slug = ?2",
                params![universe_key, slug],
                |row| {
                    Ok(ChatRoom {
                        id: row.get(0)?,
                        universe_key: row.get(1)?,
                        name: row.get(2)?,
                        slug: row.get(3)?,
                        description: row.get(4)?,
                        is_default: row.get::<_, i64>(5)? != 0,
                        created_by: row.get(6)?,
                        created_at: row.get(7)?,
                        archived_at: row.get(8)?,
                        kind: row.get(9)?,
                    })
                },
            )
            .ok()
    }

    /// Resolve a DM room by its canonical slug (kind='dm').
    pub fn get_dm_room_by_slug(&self, slug: &str) -> Option<ChatRoom> {
        self.conn
            .query_row(
                "SELECT id, universe_key, name, slug, description, is_default, created_by, \
                 created_at, archived_at, COALESCE(kind,'universe') FROM chat_rooms \
                 WHERE kind = 'dm' AND slug = ?1",
                params![slug],
                |row| {
                    Ok(ChatRoom {
                        id: row.get(0)?,
                        universe_key: row.get(1)?,
                        name: row.get(2)?,
                        slug: row.get(3)?,
                        description: row.get(4)?,
                        is_default: row.get::<_, i64>(5)? != 0,
                        created_by: row.get(6)?,
                        created_at: row.get(7)?,
                        archived_at: row.get(8)?,
                        kind: row.get(9)?,
                    })
                },
            )
            .ok()
    }

    /// Returns up to `limit` messages (clamped to 200) ordered newest-first.
    /// When `before` is supplied, only messages older than that message are returned.
    /// The returned vec has at most `limit` items; `has_more` should be checked by
    /// the caller by passing `limit + 1` and then truncating.
    pub fn list_chat_messages(
        &self,
        room_id: &str,
        before: Option<&str>,
        limit: usize,
    ) -> Vec<ChatMessageWithAuthor> {
        let fetch = limit.min(200).saturating_add(1) as i64;

        let cursor_ts: Option<String> = before.and_then(|msg_id| {
            self.conn
                .query_row(
                    "SELECT created_at FROM chat_messages WHERE id = ?1",
                    params![msg_id],
                    |row| row.get(0),
                )
                .ok()
        });

        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<ChatMessageWithAuthor> {
            let deleted_at: Option<String> = row.get(7)?;
            let raw_body: String = row.get(4)?;
            let body = if deleted_at.is_some() {
                "[mensagem removida]".to_string()
            } else {
                raw_body
            };
            let usuario_raw: String = row.get(3)?;
            Ok(ChatMessageWithAuthor {
                id: row.get(0)?,
                author: ChatAuthor {
                    user_id: row.get(1)?,
                    display_name: row.get(2)?,
                    usuario: if usuario_raw.is_empty() {
                        None
                    } else {
                        Some(usuario_raw)
                    },
                },
                body,
                created_at: row.get(5)?,
                edited_at: row.get(6)?,
                deleted_at,
                reply_to_id: row.get(8)?,
            })
        };

        let cols = "m.id, m.author_id, COALESCE(u.display_name, m.author_id), \
                    COALESCE(u.usuario, ''), m.body, m.created_at, m.edited_at, \
                    m.deleted_at, m.reply_to_id \
                    FROM chat_messages m LEFT JOIN users u ON m.author_id = u.id";

        if let Some(cat) = cursor_ts {
            let sql = format!(
                "SELECT {cols} \
                 WHERE m.room_id = ?1 AND m.created_at < ?3 \
                 ORDER BY m.created_at DESC LIMIT ?2"
            );
            let mut stmt = self
                .conn
                .prepare(&sql)
                .expect("prepare list_chat_messages cursor");
            stmt.query_map(params![room_id, fetch, cat], map_row)
                .expect("list_chat_messages cursor query")
                .filter_map(|r| r.ok())
                .collect()
        } else {
            let sql = format!(
                "SELECT {cols} \
                 WHERE m.room_id = ?1 \
                 ORDER BY m.created_at DESC LIMIT ?2"
            );
            let mut stmt = self.conn.prepare(&sql).expect("prepare list_chat_messages");
            stmt.query_map(params![room_id, fetch], map_row)
                .expect("list_chat_messages query")
                .filter_map(|r| r.ok())
                .collect()
        }
    }

    pub fn post_chat_message(
        &self,
        room_id: &str,
        author_id: &str,
        body: &str,
        reply_to_id: Option<&str>,
    ) -> anyhow::Result<String> {
        let id = format!("msg_{}", nanoid::nanoid!(10));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO chat_messages (id, room_id, author_id, body, created_at, reply_to_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, room_id, author_id, body, now, reply_to_id],
        )?;
        Ok(id)
    }

    /// Fetch a single message with its author, used by the WS broadcast path.
    pub fn get_chat_message_by_id(&self, msg_id: &str) -> Option<ChatMessageWithAuthor> {
        let cols = "m.id, m.author_id, COALESCE(u.display_name, m.author_id), \
                    COALESCE(u.usuario, ''), m.body, m.created_at, m.edited_at, \
                    m.deleted_at, m.reply_to_id \
                    FROM chat_messages m LEFT JOIN users u ON m.author_id = u.id";
        let sql = format!("SELECT {cols} WHERE m.id = ?1");
        self.conn
            .query_row(&sql, params![msg_id], |row| {
                let deleted_at: Option<String> = row.get(7)?;
                let raw_body: String = row.get(4)?;
                let body = if deleted_at.is_some() {
                    "[mensagem removida]".to_string()
                } else {
                    raw_body
                };
                let usuario_raw: String = row.get(3)?;
                Ok(ChatMessageWithAuthor {
                    id: row.get(0)?,
                    author: ChatAuthor {
                        user_id: row.get(1)?,
                        display_name: row.get(2)?,
                        usuario: if usuario_raw.is_empty() {
                            None
                        } else {
                            Some(usuario_raw)
                        },
                    },
                    body,
                    created_at: row.get(5)?,
                    edited_at: row.get(6)?,
                    deleted_at,
                    reply_to_id: row.get(8)?,
                })
            })
            .ok()
    }

    /// Return `(display_name, usuario)` for a user, used for WS presence events.
    pub fn get_user_display_info(&self, user_id: &str) -> Option<(String, Option<String>)> {
        self.conn
            .query_row(
                "SELECT display_name, usuario FROM users WHERE id = ?1",
                params![user_id],
                |row| {
                    let display_name: String = row.get(0)?;
                    let usuario: Option<String> = row.get(1)?;
                    Ok((display_name, usuario.filter(|s| !s.is_empty())))
                },
            )
            .ok()
    }

    /// Edit a chat message body. Enforces authorship and edit-window checks.
    pub fn edit_chat_message(
        &self,
        message_id: &str,
        author_id: &str,
        new_body: &str,
        edit_window: Duration,
    ) -> Result<ChatMessageWithAuthor, EditError> {
        let trimmed = new_body.trim();
        if trimmed.is_empty() {
            return Err(EditError::BodyEmpty);
        }
        if trimmed.len() > 4000 {
            return Err(EditError::BodyTooLong);
        }

        let raw = self
            .conn
            .query_row(
                "SELECT author_id, created_at, deleted_at FROM chat_messages WHERE id = ?1",
                params![message_id],
                |row| {
                    let a: String = row.get(0)?;
                    let c: String = row.get(1)?;
                    let d: Option<String> = row.get(2)?;
                    Ok((a, c, d))
                },
            )
            .optional()
            .map_err(EditError::Db)?;

        let (stored_author, created_at, deleted_at) = raw.ok_or(EditError::NotFound)?;

        if deleted_at.is_some() {
            return Err(EditError::AlreadyDeleted);
        }
        if stored_author != author_id {
            return Err(EditError::NotAuthor);
        }

        let created = chrono::DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        if Utc::now() - created > edit_window {
            return Err(EditError::EditWindowExpired);
        }

        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE chat_messages SET body = ?1, edited_at = ?2 WHERE id = ?3",
                params![trimmed, now, message_id],
            )
            .map_err(EditError::Db)?;

        self.get_chat_message_by_id(message_id)
            .ok_or(EditError::NotFound)
    }

    /// Soft-delete a chat message. Returns the `deleted_at` timestamp.
    pub fn delete_chat_message(
        &self,
        message_id: &str,
        caller_id: &str,
        caller_can_moderate: bool,
    ) -> Result<DateTime<Utc>, DeleteError> {
        let raw = self
            .conn
            .query_row(
                "SELECT author_id, deleted_at FROM chat_messages WHERE id = ?1",
                params![message_id],
                |row| {
                    let a: String = row.get(0)?;
                    let d: Option<String> = row.get(1)?;
                    Ok((a, d))
                },
            )
            .optional()
            .map_err(DeleteError::Db)?;

        let (stored_author, deleted_at) = raw.ok_or(DeleteError::NotFound)?;

        if deleted_at.is_some() {
            return Err(DeleteError::AlreadyDeleted);
        }
        if stored_author != caller_id && !caller_can_moderate {
            return Err(DeleteError::NotAuthorAndNotMod);
        }

        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn
            .execute(
                "UPDATE chat_messages SET deleted_at = ?1 WHERE id = ?2",
                params![now_str, message_id],
            )
            .map_err(DeleteError::Db)?;

        Ok(now)
    }

    // -----------------------------------------------------------------------
    // CO-198: DM methods
    // -----------------------------------------------------------------------

    fn dm_canonical_slug(a: &str, b: &str) -> String {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        format!("dm-{lo}-{hi}")
    }

    /// Returns true if either user has blocked the other.
    pub fn is_blocked_either_way(&self, user_a: &str, user_b: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM user_blocks \
                 WHERE (blocker_id = ?1 AND blocked_id = ?2) \
                    OR (blocker_id = ?2 AND blocked_id = ?1)",
                params![user_a, user_b],
                |_| Ok(true),
            )
            .optional()
            .unwrap_or(None)
            .unwrap_or(false)
    }

    /// Returns true if the caller is blocked by the target (target blocked caller).
    fn is_blocked_by(&self, target_id: &str, caller_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM user_blocks WHERE blocker_id = ?1 AND blocked_id = ?2",
                params![target_id, caller_id],
                |_| Ok(true),
            )
            .optional()
            .unwrap_or(None)
            .unwrap_or(false)
    }

    /// Check whether sender_id can open a DM with target_id.
    pub fn can_dm(&self, sender_id: &str, target_id: &str) -> bool {
        if sender_id == target_id {
            return false;
        }
        if self.is_blocked_by(target_id, sender_id) {
            return false;
        }
        let policy: String = self
            .conn
            .query_row(
                "SELECT COALESCE(dm_policy,'shared-universe') FROM users WHERE id = ?1",
                params![target_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "shared-universe".to_string());

        match policy.as_str() {
            "everyone" => true,
            "nobody" => false,
            _ => {
                // shared-universe: check if they share at least one universe_members row
                self.conn
                    .query_row(
                        "SELECT 1 FROM universe_members a \
                         JOIN universe_members b ON a.universe_key = b.universe_key \
                         WHERE a.user_id = ?1 AND b.user_id = ?2 LIMIT 1",
                        params![sender_id, target_id],
                        |_| Ok(true),
                    )
                    .optional()
                    .unwrap_or(None)
                    .unwrap_or(false)
            }
        }
    }

    /// Returns `(dm_policy, display_name, usuario)` for a user.
    fn get_user_dm_info(&self, user_id: &str) -> Option<(String, String, Option<String>)> {
        self.conn
            .query_row(
                "SELECT COALESCE(dm_policy,'shared-universe'), \
                        COALESCE(display_name,id), \
                        usuario \
                 FROM users WHERE id = ?1",
                params![user_id],
                |row| {
                    let policy: String = row.get(0)?;
                    let display_name: String = row.get(1)?;
                    let usuario: Option<String> = row.get(2)?;
                    Ok((policy, display_name, usuario.filter(|s| !s.is_empty())))
                },
            )
            .ok()
    }

    /// Idempotent: open (or return existing) DM room between caller and other.
    pub fn open_dm(&self, caller_id: &str, other_user_id: &str) -> Result<DmRoom, DmError> {
        if caller_id == other_user_id {
            return Err(DmError::SelfDm);
        }

        let (policy, other_display, other_usuario) = self
            .get_user_dm_info(other_user_id)
            .ok_or(DmError::UserNotFound)?;

        // Block check (target blocked caller)
        if self.is_blocked_by(other_user_id, caller_id) {
            return Err(DmError::Blocked);
        }

        match policy.as_str() {
            "nobody" => return Err(DmError::PolicyDenied),
            "shared-universe" => {
                let shared: bool = self
                    .conn
                    .query_row(
                        "SELECT 1 FROM universe_members a \
                         JOIN universe_members b ON a.universe_key = b.universe_key \
                         WHERE a.user_id = ?1 AND b.user_id = ?2 LIMIT 1",
                        params![caller_id, other_user_id],
                        |_| Ok(true),
                    )
                    .optional()
                    .unwrap_or(None)
                    .unwrap_or(false);
                if !shared {
                    return Err(DmError::PolicyDenied);
                }
            }
            _ => {} // "everyone" — allow
        }

        let slug = Self::dm_canonical_slug(caller_id, other_user_id);
        let other_author = ChatAuthor {
            user_id: other_user_id.to_string(),
            display_name: other_display,
            usuario: other_usuario,
        };

        // Check if room already exists
        if let Some(room) = self.get_dm_room_by_slug(&slug) {
            return Ok(DmRoom {
                room_id: room.id,
                slug,
                other_user: other_author,
                created: false,
            });
        }

        // Create new DM room + 2 membership rows
        let room_id = format!("room_dm_{}", nanoid::nanoid!(10));
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO chat_rooms \
                 (id, universe_key, name, slug, description, created_by, created_at, is_default, kind) \
                 VALUES (?1, 'dm', '', ?2, NULL, ?3, ?4, 0, 'dm')",
                params![room_id, slug, caller_id, now],
            )
            .map_err(DmError::Db)?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO chat_room_members (room_id, user_id, joined_at) \
                 VALUES (?1, ?2, ?3)",
                params![room_id, caller_id, now],
            )
            .map_err(DmError::Db)?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO chat_room_members (room_id, user_id, joined_at) \
                 VALUES (?1, ?2, ?3)",
                params![room_id, other_user_id, now],
            )
            .map_err(DmError::Db)?;

        Ok(DmRoom {
            room_id,
            slug,
            other_user: other_author,
            created: true,
        })
    }

    /// Returns all DM threads for a user, ordered by last message desc.
    pub fn list_my_dms(&self, user_id: &str) -> Vec<DmThreadPreview> {
        // Find all DM rooms where user is a member
        let room_rows: Vec<(String, String, Option<String>, Option<String>)> = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT r.id, r.slug, crm.last_read_at, crm.muted_until \
                     FROM chat_rooms r \
                     JOIN chat_room_members crm ON crm.room_id = r.id \
                     WHERE crm.user_id = ?1 AND r.kind = 'dm'",
                )
                .expect("prepare list_my_dms rooms");
            stmt.query_map(params![user_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .expect("list_my_dms rooms query")
            .filter_map(|r| r.ok())
            .collect()
        };

        let mut previews: Vec<DmThreadPreview> = room_rows
            .into_iter()
            .filter_map(|(room_id, slug, last_read_at, muted_until)| {
                // Determine the other user from the slug pattern or members table
                let other_user_id: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT user_id FROM chat_room_members \
                         WHERE room_id = ?1 AND user_id != ?2 LIMIT 1",
                        params![room_id, user_id],
                        |row| row.get(0),
                    )
                    .ok();

                let other_user_id = other_user_id?;
                let (_, other_display, other_usuario) = self.get_user_dm_info(&other_user_id)?;

                // Last message
                let last_message: Option<DmLastMessage> = self
                    .conn
                    .query_row(
                        "SELECT id, author_id, body, created_at FROM chat_messages \
                         WHERE room_id = ?1 AND deleted_at IS NULL \
                         ORDER BY created_at DESC LIMIT 1",
                        params![room_id],
                        |row| {
                            let body: String = row.get(2)?;
                            let preview = body.chars().take(80).collect();
                            Ok(DmLastMessage {
                                id: row.get(0)?,
                                author_id: row.get(1)?,
                                body_preview: preview,
                                created_at: row.get(3)?,
                            })
                        },
                    )
                    .ok();

                // Unread count
                let unread_count: i64 = if let Some(ref lra) = last_read_at {
                    self.conn
                        .query_row(
                            "SELECT COUNT(*) FROM chat_messages \
                             WHERE room_id = ?1 AND deleted_at IS NULL \
                             AND author_id != ?2 AND created_at > ?3",
                            params![room_id, user_id, lra],
                            |row| row.get(0),
                        )
                        .unwrap_or(0)
                } else {
                    self.conn
                        .query_row(
                            "SELECT COUNT(*) FROM chat_messages \
                             WHERE room_id = ?1 AND deleted_at IS NULL AND author_id != ?2",
                            params![room_id, user_id],
                            |row| row.get(0),
                        )
                        .unwrap_or(0)
                };

                Some(DmThreadPreview {
                    room_id,
                    slug,
                    other_user: ChatAuthor {
                        user_id: other_user_id,
                        display_name: other_display,
                        usuario: other_usuario,
                    },
                    last_message,
                    unread_count,
                    last_read_at,
                    muted_until,
                })
            })
            .collect();

        // Order by last_message.created_at DESC (rooms with no messages go last)
        previews.sort_by(|a, b| {
            let a_ts = a
                .last_message
                .as_ref()
                .map(|m| m.created_at.as_str())
                .unwrap_or("");
            let b_ts = b
                .last_message
                .as_ref()
                .map(|m| m.created_at.as_str())
                .unwrap_or("");
            b_ts.cmp(a_ts)
        });

        previews
    }

    /// Mark DM thread as read for the caller (sets last_read_at = now).
    pub fn mark_dm_read(&self, room_id: &str, user_id: &str) -> anyhow::Result<String> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE chat_room_members SET last_read_at = ?1 WHERE room_id = ?2 AND user_id = ?3",
            params![now, room_id, user_id],
        )?;
        Ok(now)
    }

    /// Mute or unmute a DM thread for the caller.
    pub fn mute_dm(&self, room_id: &str, user_id: &str, until: Option<&str>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE chat_room_members SET muted_until = ?1 WHERE room_id = ?2 AND user_id = ?3",
            params![until, room_id, user_id],
        )?;
        Ok(())
    }

    /// Update the caller's DM policy.
    pub fn set_dm_policy(&self, user_id: &str, policy: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE users SET dm_policy = ?1 WHERE id = ?2",
            params![policy, user_id],
        )?;
        Ok(())
    }

    /// Block a user. Returns true if a new block was created.
    pub fn block_user(
        &self,
        blocker_id: &str,
        blocked_id: &str,
        reason: Option<&str>,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "INSERT OR IGNORE INTO user_blocks (blocker_id, blocked_id, created_at, reason) \
             VALUES (?1, ?2, ?3, ?4)",
            params![blocker_id, blocked_id, now, reason],
        )?;
        Ok(rows > 0)
    }

    /// Unblock a user. Returns true if a block was removed.
    pub fn unblock_user(&self, blocker_id: &str, blocked_id: &str) -> anyhow::Result<bool> {
        let rows = self.conn.execute(
            "DELETE FROM user_blocks WHERE blocker_id = ?1 AND blocked_id = ?2",
            params![blocker_id, blocked_id],
        )?;
        Ok(rows > 0)
    }

    /// Returns true if the caller is a member of the DM room (via chat_room_members).
    pub fn is_dm_member(&self, room_id: &str, user_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM chat_room_members WHERE room_id = ?1 AND user_id = ?2",
                params![room_id, user_id],
                |_| Ok(true),
            )
            .optional()
            .unwrap_or(None)
            .unwrap_or(false)
    }

    /// Boot-time backfill: insert a chat_room_members row for every universe_members row.
    pub fn backfill_chat_room_members_from_universe_members(&self) -> usize {
        // For each universe room, insert membership rows for all universe_members
        // who don't yet have a chat_room_members row.
        let result = self.conn.execute_batch(
            "INSERT OR IGNORE INTO chat_room_members (room_id, user_id, joined_at)
             SELECT cr.id, um.user_id, COALESCE(um.joined_at, DATETIME('now'))
             FROM chat_rooms cr
             JOIN universe_members um ON um.universe_key = cr.universe_key
             WHERE cr.kind = 'universe' OR cr.kind IS NULL",
        );
        if let Err(e) = result {
            tracing::warn!("CO-198: backfill chat_room_members: {e}");
            return 0;
        }
        // Count inserted rows (approximate: count distinct room/user pairs now)
        self.conn
            .query_row("SELECT COUNT(*) FROM chat_room_members", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize
    }

    /// Insert a `general` room for every universe that lacks one. Returns the number inserted.
    pub fn backfill_default_rooms(&self) -> usize {
        let keys: Vec<String> = {
            let mut stmt = match self.conn.prepare("SELECT key FROM universes") {
                Ok(s) => s,
                Err(_) => return 0,
            };
            stmt.query_map([], |row| row.get(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };

        let mut count = 0usize;
        for key in &keys {
            let already: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM chat_rooms WHERE universe_key = ?1 AND slug = 'general'",
                    params![key],
                    |_| Ok(true),
                )
                .optional()
                .unwrap_or(None)
                .unwrap_or(false);
            if !already {
                if let Err(e) = self.ensure_default_room(key) {
                    tracing::warn!(universe_key = %key, "backfill ensure_default_room: {e}");
                } else {
                    count += 1;
                }
            }
        }
        count
    }
}
