use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::Storage;
use super::schema::ensure_table;

#[derive(Debug, Clone, Serialize)]
pub struct PushSubscription {
    pub id: String,
    pub user_id: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub last_success_at: Option<String>,
    pub failure_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushSubscriptionInfo {
    pub id: String,
    pub user_agent: Option<String>,
    pub created_at: String,
}

impl Storage {
    pub(crate) fn ensure_push_subscriptions_table(&self) {
        ensure_table(
            &self.conn,
            "push_subscriptions",
            "CREATE TABLE IF NOT EXISTS push_subscriptions (
                id              TEXT PRIMARY KEY,
                user_id         TEXT NOT NULL,
                endpoint        TEXT NOT NULL UNIQUE,
                p256dh          TEXT NOT NULL,
                auth            TEXT NOT NULL,
                user_agent      TEXT,
                created_at      TEXT NOT NULL,
                last_success_at TEXT,
                failure_count   INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (user_id) REFERENCES users(id)
            );",
        )
        .expect("CO-201: push_subscriptions table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_push_subscriptions_user \
                 ON push_subscriptions(user_id);",
            )
            .expect("CO-201: push_subscriptions index");
    }

    pub fn upsert_push_subscription(
        &self,
        user_id: &str,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        user_agent: Option<&str>,
    ) -> anyhow::Result<String> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM push_subscriptions WHERE user_id = ?1 AND endpoint = ?2",
                params![user_id, endpoint],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = existing {
            return Ok(id);
        }

        let id = format!("pushsub_{}", nanoid::nanoid!(10));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO push_subscriptions \
             (id, user_id, endpoint, p256dh, auth, user_agent, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, user_id, endpoint, p256dh, auth, user_agent, now],
        )?;
        Ok(id)
    }

    pub fn delete_push_subscription(&self, id: &str, user_id: &str) -> anyhow::Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM push_subscriptions WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_push_subscription_by_endpoint(&self, endpoint: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM push_subscriptions WHERE endpoint = ?1",
            params![endpoint],
        )?;
        Ok(())
    }

    pub fn list_push_subscriptions_for_user(&self, user_id: &str) -> Vec<PushSubscriptionInfo> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, user_agent, created_at FROM push_subscriptions \
                 WHERE user_id = ?1 ORDER BY created_at ASC",
            )
            .expect("prepare list_push_subscriptions");
        stmt.query_map(params![user_id], |row| {
            Ok(PushSubscriptionInfo {
                id: row.get(0)?,
                user_agent: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .expect("list_push_subscriptions query")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn list_all_push_subscriptions_for_user(&self, user_id: &str) -> Vec<PushSubscription> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, user_id, endpoint, p256dh, auth, user_agent, created_at, \
                 last_success_at, failure_count \
                 FROM push_subscriptions WHERE user_id = ?1 \
                 ORDER BY created_at ASC",
            )
            .expect("prepare list_all_push_subscriptions");
        stmt.query_map(params![user_id], |row| {
            Ok(PushSubscription {
                id: row.get(0)?,
                user_id: row.get(1)?,
                endpoint: row.get(2)?,
                p256dh: row.get(3)?,
                auth: row.get(4)?,
                user_agent: row.get(5)?,
                created_at: row.get(6)?,
                last_success_at: row.get(7)?,
                failure_count: row.get(8)?,
            })
        })
        .expect("list_all_push_subscriptions query")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn list_pending_push_notifications(
        &self,
        user_id: &str,
        max: usize,
    ) -> Vec<crate::storage::notifications::NotificationWithActor> {
        let fetch = max as i64;
        let cols = "n.id, n.event_type, n.actor_id, \
                    COALESCE(u.display_name, n.actor_id), COALESCE(u.usuario, ''), \
                    n.summary, n.universe_key, n.room_id, n.object_id, \
                    n.created_at, n.read_at \
                    FROM user_notifications n \
                    LEFT JOIN users u ON n.actor_id = u.id";
        let sql = format!(
            "SELECT {cols} \
             WHERE n.user_id = ?1 AND n.read_at IS NULL AND n.delivered_push_at IS NULL \
             ORDER BY n.created_at ASC LIMIT ?2"
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .expect("prepare list_pending_push_notifications");
        stmt.query_map(params![user_id, fetch], |row| {
            use crate::storage::notifications::*;
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
        .expect("list_pending_push_notifications query")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn mark_notifications_push_delivered(&self, ids: &[String]) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        for id in ids {
            self.conn.execute(
                "UPDATE user_notifications SET delivered_push_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
        }
        Ok(())
    }

    /// Returns (user_id, display_name) for users with pending push notifications
    /// and at least one active push subscription.
    pub fn list_users_with_pending_push_notifications(&self) -> Vec<(String, String)> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT n.user_id, COALESCE(u.display_name, '') \
                 FROM user_notifications n \
                 JOIN users u ON n.user_id = u.id \
                 JOIN push_subscriptions ps ON n.user_id = ps.user_id \
                 WHERE n.delivered_push_at IS NULL AND n.read_at IS NULL",
            )
            .expect("prepare list_users_with_pending_push_notifications");
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("list_users_with_pending_push_notifications query")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn increment_push_failure_count(&self, sub_id: &str) -> anyhow::Result<i64> {
        self.conn.execute(
            "UPDATE push_subscriptions SET failure_count = failure_count + 1 WHERE id = ?1",
            params![sub_id],
        )?;
        let new_count: i64 = self.conn.query_row(
            "SELECT failure_count FROM push_subscriptions WHERE id = ?1",
            params![sub_id],
            |row| row.get(0),
        )?;
        Ok(new_count)
    }

    pub fn update_push_subscription_last_success(&self, sub_id: &str) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE push_subscriptions \
             SET last_success_at = ?1, failure_count = 0 \
             WHERE id = ?2",
            params![now, sub_id],
        )?;
        Ok(())
    }
}
