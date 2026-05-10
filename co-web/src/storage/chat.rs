use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::Storage;
use super::schema::ensure_table;

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
                 (id, universe_key, name, slug, description, created_by, created_at, is_default) \
                 VALUES (?1, ?2, 'Geral', 'general', NULL, ?3, ?4, 1)",
                params![id, universe_key, created_by, now],
            )?;
        }
        Ok(())
    }

    pub fn list_chat_rooms(&self, universe_key: &str, include_archived: bool) -> Vec<ChatRoom> {
        let sql = if include_archived {
            "SELECT id, universe_key, name, slug, description, is_default, created_by, \
             created_at, archived_at FROM chat_rooms WHERE universe_key = ?1 \
             ORDER BY is_default DESC, created_at ASC"
        } else {
            "SELECT id, universe_key, name, slug, description, is_default, created_by, \
             created_at, archived_at FROM chat_rooms WHERE universe_key = ?1 \
             AND archived_at IS NULL ORDER BY is_default DESC, created_at ASC"
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
             (id, universe_key, name, slug, description, created_by, created_at, is_default) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
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
        })
    }

    pub fn get_chat_room_by_slug(&self, universe_key: &str, slug: &str) -> Option<ChatRoom> {
        self.conn
            .query_row(
                "SELECT id, universe_key, name, slug, description, is_default, created_by, \
                 created_at, archived_at FROM chat_rooms \
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
