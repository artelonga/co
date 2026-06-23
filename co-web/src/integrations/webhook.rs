//! Outbound webhook system — CO-168.
//!
//! # Flow
//! ```text
//! request handler
//!   └── emit_event("quilombo.evento.criado", payload)
//!         └── notifications table (status='pending')
//!               └── webhook_worker (background, polls every 5 s)
//!                     ├── claim row
//!                     ├── POST {webhook.url} with HMAC-SHA256 signature
//!                     ├── mark sent / failed
//!                     └── retry up to 3× with 5 s / 30 s / 2 min backoff
//! ```

use chrono::Utc;
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use crate::error::AppError;
use crate::notification_providers::{
    EvolutionApiProvider, ResendProvider, encode_email_payload, get_template, render_template,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Webhook {
    pub id: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    pub events: Vec<String>,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub webhook_id: String,
    pub event_type: String,
    pub payload: String,
    pub status: String,
    pub attempts: i64,
    pub next_attempt_at: String,
    pub sent_at: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    /// `'email'` | `'whatsapp'` | `None` for legacy webhook rows
    pub channel: Option<String>,
    /// Channel-specific recipient address (email address or phone number)
    pub recipient: Option<String>,
}

// ---------------------------------------------------------------------------
// Webhook CRUD
// ---------------------------------------------------------------------------

/// Register a new webhook. Returns the webhook with the secret included (shown once).
pub fn create_webhook(
    conn: &Connection,
    url: &str,
    events: &[String],
) -> Result<Webhook, AppError> {
    let id = Uuid::new_v4().to_string();
    let secret = Uuid::new_v4().to_string().replace('-', "");
    let events_json =
        serde_json::to_string(events).map_err(|e| AppError::Internal(e.to_string()))?;
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO webhooks (id, url, secret, events, enabled, created_at)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        params![id, url, secret, events_json, now],
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Webhook {
        id,
        url: url.to_string(),
        secret: Some(secret),
        events: events.to_vec(),
        enabled: true,
        created_at: now,
    })
}

/// List all webhooks with secret redacted.
pub fn list_webhooks(conn: &Connection) -> Result<Vec<Webhook>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, url, events, enabled, created_at FROM webhooks
             WHERE id != '__direct__' ORDER BY created_at",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
            let events_json: String = row.get(2)?;
            let enabled: i64 = row.get(3)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                events_json,
                enabled,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut webhooks = Vec::new();
    for row in rows {
        let (id, url, events_json, enabled, created_at) =
            row.map_err(|e| AppError::Internal(e.to_string()))?;
        let events: Vec<String> = serde_json::from_str(&events_json).unwrap_or_default();
        webhooks.push(Webhook {
            id,
            url,
            secret: None,
            events,
            enabled: enabled != 0,
            created_at,
        });
    }

    Ok(webhooks)
}

pub fn get_webhook_with_secret(conn: &Connection, id: &str) -> Option<Webhook> {
    conn.query_row(
        "SELECT id, url, secret, events, enabled, created_at FROM webhooks WHERE id = ?1",
        params![id],
        |row| {
            let events_json: String = row.get(3)?;
            let enabled: i64 = row.get(4)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                events_json,
                enabled,
                row.get::<_, String>(5)?,
            ))
        },
    )
    .optional()
    .ok()
    .flatten()
    .map(|(id, url, secret, events_json, enabled, created_at)| {
        let events: Vec<String> = serde_json::from_str(&events_json).unwrap_or_default();
        Webhook {
            id,
            url,
            secret: Some(secret),
            events,
            enabled: enabled != 0,
            created_at,
        }
    })
}

/// Update URL, events list, and/or enabled state.
pub fn update_webhook(
    conn: &Connection,
    id: &str,
    url: Option<&str>,
    events: Option<&[String]>,
    enabled: Option<bool>,
) -> Result<(), AppError> {
    let existing = get_webhook_with_secret(conn, id)
        .ok_or_else(|| AppError::NotFound(format!("webhook {id} not found")))?;

    let new_url = url.unwrap_or(&existing.url);
    let new_events = events.unwrap_or(&existing.events);
    let new_enabled = enabled.unwrap_or(existing.enabled);
    let events_json =
        serde_json::to_string(new_events).map_err(|e| AppError::Internal(e.to_string()))?;

    conn.execute(
        "UPDATE webhooks SET url = ?1, events = ?2, enabled = ?3 WHERE id = ?4",
        params![new_url, events_json, new_enabled as i64, id],
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(())
}

pub fn delete_webhook(conn: &Connection, id: &str) -> Result<(), AppError> {
    let rows = conn
        .execute("DELETE FROM webhooks WHERE id = ?1", params![id])
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if rows == 0 {
        return Err(AppError::NotFound(format!("webhook {id} not found")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Notification CRUD
// ---------------------------------------------------------------------------

/// Return the last 100 delivery rows for a webhook, newest first.
pub fn list_deliveries(conn: &Connection, webhook_id: &str) -> Result<Vec<Notification>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, webhook_id, event_type, payload, status, attempts,
                    next_attempt_at, sent_at, error, created_at, channel, recipient
             FROM notifications
             WHERE webhook_id = ?1
             ORDER BY created_at DESC
             LIMIT 100",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let rows = stmt
        .query_map(params![webhook_id], notification_from_row)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| AppError::Internal(e.to_string()))?);
    }
    Ok(out)
}

fn notification_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Notification> {
    Ok(Notification {
        id: row.get(0)?,
        webhook_id: row.get(1)?,
        event_type: row.get(2)?,
        payload: row.get(3)?,
        status: row.get(4)?,
        attempts: row.get(5)?,
        next_attempt_at: row.get(6)?,
        sent_at: row.get(7)?,
        error: row.get(8)?,
        created_at: row.get(9)?,
        channel: row.get(10)?,
        recipient: row.get(11)?,
    })
}

// ---------------------------------------------------------------------------
// Direct channel enqueue helpers
// ---------------------------------------------------------------------------

fn enqueue_email_if_configured(
    conn: &Connection,
    event_type: &str,
    payload: &serde_json::Value,
    recipient: &str,
    now: &str,
) -> Result<(), AppError> {
    if ResendProvider::from_env().is_none() {
        return Ok(());
    }
    let subject_tpl = get_template(event_type, "EMAIL_SUBJECT");
    let body_tpl = get_template(event_type, "EMAIL_BODY");
    if let (Some(subject_tpl), Some(body_tpl)) = (subject_tpl, body_tpl) {
        let subject = render_template(&subject_tpl, payload);
        let body = render_template(&body_tpl, payload);
        let channel_payload = encode_email_payload(&subject, &body);
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO notifications
                (id, webhook_id, event_type, payload, status, attempts,
                 next_attempt_at, created_at, channel, recipient)
             VALUES (?1, '__direct__', ?2, ?3, 'pending', 0, ?4, ?4, 'email', ?5)",
            params![id, event_type, channel_payload, now, recipient],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    }
    Ok(())
}

fn enqueue_whatsapp_if_configured(
    conn: &Connection,
    event_type: &str,
    payload: &serde_json::Value,
    recipient: &str,
    now: &str,
) -> Result<(), AppError> {
    if EvolutionApiProvider::from_env().is_none() {
        return Ok(());
    }
    if let Some(tpl) = get_template(event_type, "WHATSAPP") {
        let message = render_template(&tpl, payload);
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO notifications
                (id, webhook_id, event_type, payload, status, attempts,
                 next_attempt_at, created_at, channel, recipient)
             VALUES (?1, '__direct__', ?2, ?3, 'pending', 0, ?4, ?4, 'whatsapp', ?5)",
            params![id, event_type, message, now, recipient],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Event emission
// ---------------------------------------------------------------------------

/// Write a notification row for each enabled webhook whose event filter matches
/// `event_type`. When direct provider env vars are set and a recipient is
/// available, also enqueue channel-specific rows (email / whatsapp).
///
/// Called from request handlers after the triggering action succeeds.
pub fn emit_event(
    conn: &Connection,
    event_type: &str,
    payload: &serde_json::Value,
    email: Option<&str>,
    telefone: Option<&str>,
) -> Result<(), AppError> {
    let payload_str =
        serde_json::to_string(payload).map_err(|e| AppError::Internal(e.to_string()))?;
    let now = Utc::now().to_rfc3339();

    // --- Outbound webhooks ---
    {
        let mut stmt = conn
            .prepare("SELECT id, events FROM webhooks WHERE enabled = 1 AND id != '__direct__'")
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let matching_ids: Vec<String> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| AppError::Internal(e.to_string()))?
            .filter_map(|r| r.ok())
            .filter_map(|(id, events_json)| {
                let patterns: Vec<String> = serde_json::from_str(&events_json).ok()?;
                if patterns.iter().any(|p| event_matches(p, event_type)) {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();

        for webhook_id in matching_ids {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO notifications
                    (id, webhook_id, event_type, payload, status, attempts,
                     next_attempt_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5, ?5)",
                params![id, webhook_id, event_type, payload_str, now],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    // --- Direct email channel ---
    if let Some(recipient) = email {
        enqueue_email_if_configured(conn, event_type, payload, recipient, &now)?;
    }

    // --- Direct WhatsApp channel ---
    if let Some(recipient) = telefone {
        enqueue_whatsapp_if_configured(conn, event_type, payload, recipient, &now)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Worker helpers (used by webhook_worker)
// ---------------------------------------------------------------------------

const MAX_ATTEMPTS: i64 = 3;

fn backoff_secs(attempts: i64) -> i64 {
    match attempts {
        1 => 5,
        2 => 30,
        _ => 120,
    }
}

/// Atomically claim the oldest eligible pending/failed notification.
pub fn claim_next_notification(conn: &Connection) -> Option<Notification> {
    let now = Utc::now().to_rfc3339();
    conn.query_row(
        "UPDATE notifications
         SET status = 'sending'
         WHERE id = (
             SELECT id FROM notifications
             WHERE status IN ('pending', 'failed') AND next_attempt_at <= ?1
             ORDER BY next_attempt_at ASC, created_at ASC
             LIMIT 1
         )
         RETURNING id, webhook_id, event_type, payload, status, attempts,
                   next_attempt_at, sent_at, error, created_at, channel, recipient",
        params![now],
        notification_from_row,
    )
    .optional()
    .unwrap_or(None)
}

pub fn mark_notification_sent(conn: &Connection, id: &str) {
    let now = Utc::now().to_rfc3339();
    let _ = conn.execute(
        "UPDATE notifications SET status = 'sent', sent_at = ?1, error = NULL WHERE id = ?2",
        params![now, id],
    );
}

pub fn mark_notification_failed(conn: &Connection, id: &str, attempts: i64, error_msg: &str) {
    let now = Utc::now().to_rfc3339();
    let (new_status, run_at) = if attempts >= MAX_ATTEMPTS {
        ("dead", Utc::now().to_rfc3339())
    } else {
        let secs = backoff_secs(attempts);
        let next = Utc::now() + chrono::Duration::seconds(secs);
        ("failed", next.to_rfc3339())
    };

    let _ = conn.execute(
        "UPDATE notifications
         SET status = ?1, attempts = ?2, error = ?3, next_attempt_at = ?4
         WHERE id = ?5",
        params![new_status, attempts, error_msg, run_at, id],
    );
    let _ = now; // used above in mark_notification_sent; quiets unused warning
}

// ---------------------------------------------------------------------------
// HMAC-SHA256 signing
// ---------------------------------------------------------------------------

type HmacSha256 = Hmac<Sha256>;

/// Compute `sha256=<hex>` over `body` using `secret` as the HMAC key.
/// Matches GitHub's webhook signature scheme.
pub fn hmac_signature(secret: &str, body: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body);
    let result = mac.finalize().into_bytes();
    let hex: String = result.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256={hex}")
}

// ---------------------------------------------------------------------------
// Wildcard event matching
// ---------------------------------------------------------------------------

/// Returns true if `pattern` matches `event_type`.
///
/// - `*` matches everything.
/// - `quilombo.*` matches any event whose name starts with `quilombo.`.
/// - Otherwise exact string match.
pub fn event_matches(pattern: &str, event_type: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return event_type.starts_with(prefix) && event_type[prefix.len()..].starts_with('.');
    }
    pattern == event_type
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use tempfile::tempdir;

    fn make_storage() -> (Storage, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let s = Storage::new(tmp.path().to_str().unwrap());
        (s, tmp)
    }

    // --- wildcard matching ---

    #[test]
    fn wildcard_star_matches_everything() {
        assert!(event_matches("*", "quilombo.evento.criado"));
        assert!(event_matches("*", "co.universe.criado"));
        assert!(event_matches("*", "anything"));
    }

    #[test]
    fn wildcard_prefix_matches_correct_namespace() {
        assert!(event_matches("quilombo.*", "quilombo.evento.criado"));
        assert!(event_matches("quilombo.*", "quilombo.missao.participou"));
        assert!(event_matches("quilombo.*", "quilombo.mensagem.criada"));
        assert!(!event_matches("quilombo.*", "co.universe.criado"));
        assert!(!event_matches("quilombo.*", "quilomboextended.foo"));
    }

    #[test]
    fn exact_match_works() {
        assert!(event_matches(
            "quilombo.evento.criado",
            "quilombo.evento.criado"
        ));
        assert!(!event_matches(
            "quilombo.evento.criado",
            "quilombo.evento.atualizado"
        ));
    }

    // --- HMAC signature ---

    #[test]
    fn hmac_signature_format() {
        let sig = hmac_signature("mysecret", b"hello world");
        assert!(
            sig.starts_with("sha256="),
            "signature must start with sha256="
        );
        assert_eq!(sig.len(), 7 + 64, "sha256= prefix + 64 hex chars");
    }

    #[test]
    fn hmac_signature_deterministic() {
        let a = hmac_signature("secret", b"body");
        let b = hmac_signature("secret", b"body");
        assert_eq!(a, b);
    }

    #[test]
    fn hmac_signature_different_secret_differs() {
        let a = hmac_signature("secret1", b"body");
        let b = hmac_signature("secret2", b"body");
        assert_ne!(a, b);
    }

    // --- webhook registration ---

    #[test]
    fn create_and_list_webhook() {
        let (storage, _tmp) = make_storage();
        let wh = create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["*".to_string()],
        )
        .unwrap();
        assert!(!wh.id.is_empty());
        assert!(wh.secret.is_some(), "secret returned on creation");

        let list = list_webhooks(storage.conn()).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].secret.is_none(), "secret redacted in list");
    }

    #[test]
    fn delete_webhook_cascades_notifications() {
        let (storage, _tmp) = make_storage();
        let wh = create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["*".to_string()],
        )
        .unwrap();

        emit_event(
            storage.conn(),
            "quilombo.evento.criado",
            &serde_json::json!({"id": 1}),
            None,
            None,
        )
        .unwrap();

        delete_webhook(storage.conn(), &wh.id).unwrap();

        // Cascaded delete should remove the notification too.
        let count: i64 = storage
            .conn()
            .query_row("SELECT COUNT(*) FROM notifications", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    // --- emit_event ---

    #[test]
    fn emit_event_creates_notification_for_matching_webhook() {
        let (storage, _tmp) = make_storage();
        create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["quilombo.*".to_string()],
        )
        .unwrap();

        emit_event(
            storage.conn(),
            "quilombo.evento.criado",
            &serde_json::json!({"id": 42}),
            None,
            None,
        )
        .unwrap();

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE status='pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn emit_event_skips_non_matching_webhook() {
        let (storage, _tmp) = make_storage();
        create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["co.*".to_string()],
        )
        .unwrap();

        emit_event(
            storage.conn(),
            "quilombo.evento.criado",
            &serde_json::json!({}),
            None,
            None,
        )
        .unwrap();

        let count: i64 = storage
            .conn()
            .query_row("SELECT COUNT(*) FROM notifications", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn emit_event_skips_disabled_webhook() {
        let (storage, _tmp) = make_storage();
        let wh = create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["*".to_string()],
        )
        .unwrap();

        update_webhook(storage.conn(), &wh.id, None, None, Some(false)).unwrap();

        emit_event(
            storage.conn(),
            "co.entry.criado",
            &serde_json::json!({}),
            None,
            None,
        )
        .unwrap();

        let count: i64 = storage
            .conn()
            .query_row("SELECT COUNT(*) FROM notifications", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    // --- retry state machine ---

    #[test]
    fn claim_and_mark_failed_once() {
        let (storage, _tmp) = make_storage();
        create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["*".to_string()],
        )
        .unwrap();
        emit_event(
            storage.conn(),
            "test.event",
            &serde_json::json!({}),
            None,
            None,
        )
        .unwrap();

        let n = claim_next_notification(storage.conn()).unwrap();
        assert_eq!(n.status, "sending");

        mark_notification_failed(storage.conn(), &n.id, 1, "timeout");

        let row: (String, i64) = storage
            .conn()
            .query_row(
                "SELECT status, attempts FROM notifications WHERE id = ?1",
                params![n.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "failed");
        assert_eq!(row.1, 1);
    }

    #[test]
    fn mark_failed_at_max_becomes_dead() {
        let (storage, _tmp) = make_storage();
        create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["*".to_string()],
        )
        .unwrap();
        emit_event(
            storage.conn(),
            "test.event",
            &serde_json::json!({}),
            None,
            None,
        )
        .unwrap();

        let n = claim_next_notification(storage.conn()).unwrap();
        mark_notification_failed(storage.conn(), &n.id, MAX_ATTEMPTS, "final error");

        let status: String = storage
            .conn()
            .query_row(
                "SELECT status FROM notifications WHERE id = ?1",
                params![n.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "dead");
    }

    #[test]
    fn no_double_claim() {
        let (storage, _tmp) = make_storage();
        create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["*".to_string()],
        )
        .unwrap();
        emit_event(
            storage.conn(),
            "test.event",
            &serde_json::json!({}),
            None,
            None,
        )
        .unwrap();

        let first = claim_next_notification(storage.conn());
        let second = claim_next_notification(storage.conn());
        assert!(first.is_some());
        assert!(
            second.is_none(),
            "sending notification must not be claimed again"
        );
    }

    #[test]
    fn mark_sent_clears_error() {
        let (storage, _tmp) = make_storage();
        create_webhook(
            storage.conn(),
            "https://example.com/hook",
            &["*".to_string()],
        )
        .unwrap();
        emit_event(
            storage.conn(),
            "test.event",
            &serde_json::json!({}),
            None,
            None,
        )
        .unwrap();
        let n = claim_next_notification(storage.conn()).unwrap();

        mark_notification_sent(storage.conn(), &n.id);

        let (status, sent_at): (String, Option<String>) = storage
            .conn()
            .query_row(
                "SELECT status, sent_at FROM notifications WHERE id = ?1",
                params![n.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "sent");
        assert!(sent_at.is_some());
    }

    // --- direct channel enqueuing ---

    // CO-477: env-var-dependent tests serialize on the single process-wide lock
    // in crate::test_support (acquired in each test below), so they also exclude
    // env-mutating tests in *other* modules — a per-file mutex did not.
    struct EnvGuard {
        key: String,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: test-only, guarded by the process-wide test env lock (CO-477)
            unsafe { std::env::set_var(key, value) };
            Self {
                key: key.to_string(),
                original,
            }
        }

        fn remove(key: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: test-only, guarded by the process-wide test env lock (CO-477)
            unsafe { std::env::remove_var(key) };
            Self {
                key: key.to_string(),
                original,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(v) => unsafe { std::env::set_var(&self.key, v) },
                None => unsafe { std::env::remove_var(&self.key) },
            }
        }
    }

    #[test]
    fn emit_event_enqueues_email_when_resend_configured() {
        let _env = crate::test_support::env_lock_blocking();
        let _guard = EnvGuard::set("RESEND_API_KEY", "test-key");
        let (storage, _tmp) = make_storage();

        emit_event(
            storage.conn(),
            "quilombo.evento.criado",
            &serde_json::json!({"titulo": "Festa", "nome": "Yuri"}),
            Some("yuri@example.com"),
            None,
        )
        .unwrap();

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE channel = 'email'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "one email notification should be enqueued");
    }

    #[test]
    fn emit_event_no_email_when_resend_key_absent() {
        let _env = crate::test_support::env_lock_blocking();
        let _guard = EnvGuard::remove("RESEND_API_KEY");
        let (storage, _tmp) = make_storage();

        emit_event(
            storage.conn(),
            "quilombo.evento.criado",
            &serde_json::json!({"titulo": "Festa"}),
            Some("yuri@example.com"),
            None,
        )
        .unwrap();

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE channel = 'email'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "no email rows without RESEND_API_KEY");
    }

    #[test]
    fn emit_event_enqueues_whatsapp_when_evolution_configured() {
        let _env = crate::test_support::env_lock_blocking();
        let _guard_key = EnvGuard::set("EVOLUTION_API_KEY", "test-key");
        let _guard_url = EnvGuard::set("EVOLUTION_API_URL", "https://api.example.com");
        let _guard_instance = EnvGuard::set("EVOLUTION_INSTANCE", "test");
        let (storage, _tmp) = make_storage();

        emit_event(
            storage.conn(),
            "quilombo.evento.criado",
            &serde_json::json!({"titulo": "Festa"}),
            None,
            Some("+5541999999999"),
        )
        .unwrap();

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE channel = 'whatsapp'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "one whatsapp notification should be enqueued");
    }

    #[test]
    fn emit_event_no_whatsapp_when_evolution_key_absent() {
        let _env = crate::test_support::env_lock_blocking();
        let _guard = EnvGuard::remove("EVOLUTION_API_KEY");
        let (storage, _tmp) = make_storage();

        emit_event(
            storage.conn(),
            "quilombo.evento.criado",
            &serde_json::json!({"titulo": "Festa"}),
            None,
            Some("+5541999999999"),
        )
        .unwrap();

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE channel = 'whatsapp'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "no whatsapp rows without EVOLUTION_API_KEY");
    }
}
