use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::Storage;
use super::schema::ensure_table;

#[derive(Debug, Clone, Serialize)]
pub struct NotificationWithActor {
    pub id: String,
    pub event_type: String,
    pub actor: Option<NotificationActor>,
    pub summary: NotificationSummary,
    pub context: NotificationContext,
    pub created_at: String,
    pub read_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationActor {
    pub user_id: String,
    pub display_name: String,
    pub usuario: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSummary {
    pub key: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationContext {
    pub universe_key: Option<String>,
    pub room_id: Option<String>,
    pub object_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationPreferences {
    pub user_id: String,
    pub email_chat_message: bool,
    pub email_chat_dm: bool,
    pub email_invitation: bool,
    pub email_mention: bool,
    pub email_digest_freq: String,
    pub push_chat_message: bool,
    pub push_chat_dm: bool,
    pub push_invitation: bool,
    pub push_mention: bool,
    pub in_app_chat_message: bool,
    pub in_app_chat_dm: bool,
    pub in_app_invitation: bool,
    pub in_app_mention: bool,
    pub quiet_hours_start: Option<String>,
    pub quiet_hours_end: Option<String>,
    pub quiet_hours_tz: String,
    pub updated_at: String,
}

#[derive(Debug)]
pub enum NotificationError {
    Db(rusqlite::Error),
}

impl Storage {
    pub(crate) fn ensure_notification_tables(&self) {
        ensure_table(
            &self.conn,
            "user_notifications",
            "CREATE TABLE IF NOT EXISTS user_notifications (
                id              TEXT PRIMARY KEY,
                user_id         TEXT NOT NULL,
                event_type      TEXT NOT NULL,
                universe_key    TEXT,
                room_id         TEXT,
                actor_id        TEXT,
                object_id       TEXT,
                summary         TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                read_at         TEXT,
                delivered_email_at TEXT,
                delivered_push_at  TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id)
            );",
        )
        .expect("CO-199: user_notifications table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_user_notifs_user_unread
                     ON user_notifications(user_id, read_at);
                 CREATE INDEX IF NOT EXISTS idx_user_notifs_user_created
                     ON user_notifications(user_id, created_at DESC);
                 CREATE INDEX IF NOT EXISTS idx_user_notifs_pending_email
                     ON user_notifications(delivered_email_at) WHERE delivered_email_at IS NULL;",
            )
            .expect("CO-199: user_notifications indexes");

        ensure_table(
            &self.conn,
            "notification_preferences",
            "CREATE TABLE IF NOT EXISTS notification_preferences (
                user_id              TEXT PRIMARY KEY,
                email_chat_message   INTEGER NOT NULL DEFAULT 0,
                email_chat_dm        INTEGER NOT NULL DEFAULT 1,
                email_invitation     INTEGER NOT NULL DEFAULT 1,
                email_mention        INTEGER NOT NULL DEFAULT 1,
                email_digest_freq    TEXT NOT NULL DEFAULT 'daily',
                push_chat_message    INTEGER NOT NULL DEFAULT 1,
                push_chat_dm         INTEGER NOT NULL DEFAULT 1,
                push_invitation      INTEGER NOT NULL DEFAULT 1,
                push_mention         INTEGER NOT NULL DEFAULT 1,
                in_app_chat_message  INTEGER NOT NULL DEFAULT 1,
                in_app_chat_dm       INTEGER NOT NULL DEFAULT 1,
                in_app_invitation    INTEGER NOT NULL DEFAULT 1,
                in_app_mention       INTEGER NOT NULL DEFAULT 1,
                quiet_hours_start    TEXT,
                quiet_hours_end      TEXT,
                quiet_hours_tz       TEXT NOT NULL DEFAULT 'America/Sao_Paulo',
                updated_at           TEXT NOT NULL,
                FOREIGN KEY (user_id) REFERENCES users(id)
            );",
        )
        .expect("CO-199: notification_preferences table");
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_notification(
        &self,
        user_id: &str,
        event_type: &str,
        universe_key: Option<&str>,
        room_id: Option<&str>,
        actor_id: &str,
        object_id: &str,
        summary_key: &str,
        summary_params: serde_json::Value,
    ) -> Result<String, NotificationError> {
        // Idempotency: return existing id if same (user_id, event_type, object_id) within 5s.
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM user_notifications \
                 WHERE user_id = ?1 AND event_type = ?2 AND object_id = ?3 \
                 AND created_at > datetime('now', '-5 seconds')",
                params![user_id, event_type, object_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(NotificationError::Db)?;
        if let Some(id) = existing {
            return Ok(id);
        }

        let id = format!("notif_{}", nanoid::nanoid!(10));
        let now = Utc::now().to_rfc3339();
        let summary = serde_json::json!({"key": summary_key, "params": summary_params}).to_string();

        self.conn
            .execute(
                "INSERT INTO user_notifications \
                 (id, user_id, event_type, universe_key, room_id, actor_id, object_id, summary, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, user_id, event_type, universe_key, room_id, actor_id, object_id, summary, now],
            )
            .map_err(NotificationError::Db)?;

        Ok(id)
    }

    pub fn list_my_notifications(
        &self,
        user_id: &str,
        since: Option<&str>,
        limit: usize,
    ) -> (Vec<NotificationWithActor>, bool) {
        let fetch = (limit + 1) as i64;

        let cols = "n.id, n.event_type, n.actor_id, \
                    COALESCE(u.display_name, n.actor_id), COALESCE(u.usuario, ''), \
                    n.summary, n.universe_key, n.room_id, n.object_id, \
                    n.created_at, n.read_at \
                    FROM user_notifications n \
                    LEFT JOIN users u ON n.actor_id = u.id";

        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<NotificationWithActor> {
            let actor_id: Option<String> = row.get(2)?;
            let display_name: String = row.get(3)?;
            let usuario_raw: String = row.get(4)?;
            let actor = actor_id.map(|uid| NotificationActor {
                user_id: uid,
                display_name,
                usuario: if usuario_raw.is_empty() {
                    None
                } else {
                    Some(usuario_raw)
                },
            });

            let summary_str: String = row.get(5)?;
            let summary_val = serde_json::from_str::<serde_json::Value>(&summary_str)
                .unwrap_or(serde_json::Value::Null);
            let summary_key = summary_val
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let summary_params = summary_val
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

            Ok(NotificationWithActor {
                id: row.get(0)?,
                event_type: row.get(1)?,
                actor,
                summary: NotificationSummary {
                    key: summary_key,
                    params: summary_params,
                },
                context: NotificationContext {
                    universe_key: row.get(6)?,
                    room_id: row.get(7)?,
                    object_id: row.get(8)?,
                },
                created_at: row.get(9)?,
                read_at: row.get(10)?,
            })
        };

        let mut rows: Vec<NotificationWithActor> = if let Some(since_ts) = since {
            let sql = format!(
                "SELECT {cols} \
                 WHERE n.user_id = ?1 AND n.created_at > ?3 \
                 ORDER BY n.created_at DESC LIMIT ?2"
            );
            let mut stmt = self
                .conn
                .prepare(&sql)
                .expect("prepare list_my_notifications since");
            stmt.query_map(params![user_id, fetch, since_ts], map_row)
                .expect("list_my_notifications since query")
                .filter_map(|r| r.ok())
                .collect()
        } else {
            let sql = format!(
                "SELECT {cols} \
                 WHERE n.user_id = ?1 \
                 ORDER BY n.created_at DESC LIMIT ?2"
            );
            let mut stmt = self
                .conn
                .prepare(&sql)
                .expect("prepare list_my_notifications");
            stmt.query_map(params![user_id, fetch], map_row)
                .expect("list_my_notifications query")
                .filter_map(|r| r.ok())
                .collect()
        };

        let has_more = rows.len() > limit;
        if has_more {
            rows.truncate(limit);
        }
        (rows, has_more)
    }

    pub fn get_unread_count(&self, user_id: &str) -> usize {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM user_notifications WHERE user_id = ?1 AND read_at IS NULL",
                params![user_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
    }

    pub fn mark_notification_read(&self, notif_id: &str, user_id: &str) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE user_notifications SET read_at = COALESCE(read_at, ?1) \
             WHERE id = ?2 AND user_id = ?3",
            params![now, notif_id, user_id],
        )?;
        if rows == 0 {
            anyhow::bail!("notification not found");
        }
        Ok(())
    }

    pub fn mark_all_read(&self, user_id: &str) -> anyhow::Result<usize> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE user_notifications SET read_at = ?1 \
             WHERE user_id = ?2 AND read_at IS NULL",
            params![now, user_id],
        )?;
        Ok(rows)
    }

    pub fn get_preferences(&self, user_id: &str) -> NotificationPreferences {
        self.ensure_default_preferences(user_id)
            .expect("ensure_default_preferences");
        self.conn
            .query_row(
                "SELECT user_id, email_chat_message, email_chat_dm, email_invitation, \
                 email_mention, email_digest_freq, push_chat_message, push_chat_dm, \
                 push_invitation, push_mention, in_app_chat_message, in_app_chat_dm, \
                 in_app_invitation, in_app_mention, quiet_hours_start, quiet_hours_end, \
                 quiet_hours_tz, updated_at \
                 FROM notification_preferences WHERE user_id = ?1",
                params![user_id],
                |row| {
                    Ok(NotificationPreferences {
                        user_id: row.get(0)?,
                        email_chat_message: row.get::<_, i64>(1)? != 0,
                        email_chat_dm: row.get::<_, i64>(2)? != 0,
                        email_invitation: row.get::<_, i64>(3)? != 0,
                        email_mention: row.get::<_, i64>(4)? != 0,
                        email_digest_freq: row.get(5)?,
                        push_chat_message: row.get::<_, i64>(6)? != 0,
                        push_chat_dm: row.get::<_, i64>(7)? != 0,
                        push_invitation: row.get::<_, i64>(8)? != 0,
                        push_mention: row.get::<_, i64>(9)? != 0,
                        in_app_chat_message: row.get::<_, i64>(10)? != 0,
                        in_app_chat_dm: row.get::<_, i64>(11)? != 0,
                        in_app_invitation: row.get::<_, i64>(12)? != 0,
                        in_app_mention: row.get::<_, i64>(13)? != 0,
                        quiet_hours_start: row.get(14)?,
                        quiet_hours_end: row.get(15)?,
                        quiet_hours_tz: row.get(16)?,
                        updated_at: row.get(17)?,
                    })
                },
            )
            .expect("get_preferences query")
    }

    pub fn ensure_default_preferences(&self, user_id: &str) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO notification_preferences (user_id, updated_at) \
             VALUES (?1, ?2)",
            params![user_id, now],
        )?;
        Ok(())
    }

    pub fn backfill_default_preferences(&self) -> usize {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT OR IGNORE INTO notification_preferences (user_id, updated_at) \
                 SELECT id, ?1 FROM users WHERE id != 'system'",
                params![now],
            )
            .unwrap_or(0)
    }

    pub fn update_preferences(
        &self,
        user_id: &str,
        patch: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let obj = patch
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("preferences patch must be a JSON object"))?;

        if obj.is_empty() {
            return Ok(());
        }

        let bool_fields = [
            "email_chat_message",
            "email_chat_dm",
            "email_invitation",
            "email_mention",
            "push_chat_message",
            "push_chat_dm",
            "push_invitation",
            "push_mention",
            "in_app_chat_message",
            "in_app_chat_dm",
            "in_app_invitation",
            "in_app_mention",
        ];
        let text_fields = [
            "email_digest_freq",
            "quiet_hours_start",
            "quiet_hours_end",
            "quiet_hours_tz",
        ];
        let valid_digest_freqs = ["instant", "hourly", "daily", "weekly", "never"];

        let mut sets: Vec<String> = Vec::new();
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        let mut param_idx: usize = 1;

        for (key, val) in obj {
            if bool_fields.contains(&key.as_str()) {
                let b = val
                    .as_bool()
                    .ok_or_else(|| anyhow::anyhow!("field '{key}' must be a boolean"))?;
                sets.push(format!("{key} = ?{param_idx}"));
                values.push(rusqlite::types::Value::Integer(if b { 1 } else { 0 }));
                param_idx += 1;
            } else if text_fields.contains(&key.as_str()) {
                match key.as_str() {
                    "email_digest_freq" => {
                        let s = val
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("email_digest_freq must be a string"))?;
                        if !valid_digest_freqs.contains(&s) {
                            anyhow::bail!(
                                "email_digest_freq must be one of: instant, hourly, daily, weekly, never"
                            );
                        }
                        sets.push(format!("{key} = ?{param_idx}"));
                        values.push(rusqlite::types::Value::Text(s.to_string()));
                        param_idx += 1;
                    }
                    "quiet_hours_start" | "quiet_hours_end" => {
                        if val.is_null() {
                            sets.push(format!("{key} = ?{param_idx}"));
                            values.push(rusqlite::types::Value::Null);
                            param_idx += 1;
                        } else {
                            let s = val.as_str().ok_or_else(|| {
                                anyhow::anyhow!("'{key}' must be a string or null")
                            })?;
                            if !is_hhmm(s) {
                                anyhow::bail!("'{key}' must match HH:MM format or be null");
                            }
                            sets.push(format!("{key} = ?{param_idx}"));
                            values.push(rusqlite::types::Value::Text(s.to_string()));
                            param_idx += 1;
                        }
                    }
                    _ => {
                        let s = val
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("field '{key}' must be a string"))?;
                        sets.push(format!("{key} = ?{param_idx}"));
                        values.push(rusqlite::types::Value::Text(s.to_string()));
                        param_idx += 1;
                    }
                }
            }
            // unknown keys are silently ignored
        }

        if sets.is_empty() {
            return Ok(());
        }

        let now = Utc::now().to_rfc3339();
        sets.push(format!("updated_at = ?{param_idx}"));
        values.push(rusqlite::types::Value::Text(now));
        param_idx += 1;

        let sql = format!(
            "UPDATE notification_preferences SET {} WHERE user_id = ?{param_idx}",
            sets.join(", ")
        );
        values.push(rusqlite::types::Value::Text(user_id.to_string()));

        let mut stmt = self.conn.prepare(&sql)?;
        for (i, val) in values.iter().enumerate() {
            stmt.raw_bind_parameter(i + 1, val)?;
        }
        stmt.raw_execute()?;

        Ok(())
    }

    pub fn list_pending_email_notifications(
        &self,
        user_id: &str,
        max: usize,
    ) -> Vec<NotificationWithActor> {
        let fetch = max as i64;
        let cols = "n.id, n.event_type, n.actor_id, \
                    COALESCE(u.display_name, n.actor_id), COALESCE(u.usuario, ''), \
                    n.summary, n.universe_key, n.room_id, n.object_id, \
                    n.created_at, n.read_at \
                    FROM user_notifications n \
                    LEFT JOIN users u ON n.actor_id = u.id";
        let sql = format!(
            "SELECT {cols} \
             WHERE n.user_id = ?1 AND n.read_at IS NULL AND n.delivered_email_at IS NULL \
             ORDER BY n.created_at ASC LIMIT ?2"
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .expect("prepare list_pending_email_notifications");
        stmt.query_map(params![user_id, fetch], |row| {
            let actor_id: Option<String> = row.get(2)?;
            let display_name: String = row.get(3)?;
            let usuario_raw: String = row.get(4)?;
            let actor = actor_id.map(|uid| NotificationActor {
                user_id: uid,
                display_name,
                usuario: if usuario_raw.is_empty() {
                    None
                } else {
                    Some(usuario_raw)
                },
            });

            let summary_str: String = row.get(5)?;
            let summary_val = serde_json::from_str::<serde_json::Value>(&summary_str)
                .unwrap_or(serde_json::Value::Null);
            let summary_key = summary_val
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let summary_params = summary_val
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

            Ok(NotificationWithActor {
                id: row.get(0)?,
                event_type: row.get(1)?,
                actor,
                summary: NotificationSummary {
                    key: summary_key,
                    params: summary_params,
                },
                context: NotificationContext {
                    universe_key: row.get(6)?,
                    room_id: row.get(7)?,
                    object_id: row.get(8)?,
                },
                created_at: row.get(9)?,
                read_at: row.get(10)?,
            })
        })
        .expect("list_pending_email_notifications query")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn mark_notifications_email_delivered(&self, ids: &[String]) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        for id in ids {
            self.conn.execute(
                "UPDATE user_notifications SET delivered_email_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
        }
        Ok(())
    }
}

fn is_hhmm(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 5 {
        return false;
    }
    bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b':'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
}

/// Extract `@usuario` mentions from text. Returns a deduped list.
/// No regex — uses a char-by-char scan.
pub fn parse_mentions(body: &str) -> Vec<String> {
    let mut mentions: Vec<String> = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '@' {
            // Must not be preceded by an alphanumeric character.
            let preceded_by_alnum = i > 0 && chars[i - 1].is_alphanumeric();
            if !preceded_by_alnum {
                let start = i + 1;
                let mut j = start;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j > start {
                    let mention: String = chars[start..j].iter().collect();
                    if !mentions.contains(&mention) {
                        mentions.push(mention);
                    }
                }
            }
        }
        i += 1;
    }

    mentions
}
