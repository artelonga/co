//! CO-422: `DegradationAlerter` — emails the operator on degraded-but-alive events.
//!
//! Subscribes to the three degradation event types on the EDA bus and sends one
//! alert email per event type per debounce window (default 2 h, configurable via
//! `CO_ALERT_DEBOUNCE_HOURS`).  Without `RESEND_API_KEY` a WARN is logged once at
//! startup and the subscriber becomes a no-op mailer — it never panics.
//!
//! Covered events
//! --------------
//! | Event type                | Source                |
//! |---------------------------|-----------------------|
//! | `backup.skipped_low_disk` | backup worker (CO-405)|
//! | `universe.unavailable`    | universe pool (CO-406)|
//! | `system.disk_pressure`    | disk monitor (CO-422) |

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tracing::{debug, info, warn};

use crate::eda::bus::{EdaBus, Filter};
use crate::eda::event::Event;

// ---------------------------------------------------------------------------
// AlertMailer trait
// ---------------------------------------------------------------------------

/// Injectable email sender for degradation alerts.
///
/// - Production: [`ResendAlertMailer`] (fires a Tokio task per send).
/// - Tests: [`MockAlertMailer`] (records calls synchronously).
pub trait AlertMailer: Send + Sync + 'static {
    /// Send an alert email. Fire-and-forget; must never panic.
    fn send_alert(&self, to: &str, subject: &str, body: &str);
}

// ---------------------------------------------------------------------------
// Production: ResendAlertMailer
// ---------------------------------------------------------------------------

/// Sends alert emails via the Resend API (`RESEND_API_KEY`).
pub struct ResendAlertMailer {
    api_key: String,
    from: String,
}

impl ResendAlertMailer {
    /// Construct from config + secrets (CO-434). Returns `None` if
    /// `RESEND_API_KEY` is absent.
    pub fn from_secrets(
        config: &crate::CoServerConfig,
        secrets: &dyn crate::infra::secrets::SecretsProvider,
    ) -> Option<Self> {
        let api_key = secrets.get("RESEND_API_KEY")?;
        Some(Self {
            api_key,
            from: config.alert_from.clone(),
        })
    }
}

impl AlertMailer for ResendAlertMailer {
    fn send_alert(&self, to: &str, subject: &str, body: &str) {
        let api_key = self.api_key.clone();
        let from = self.from.clone();
        let to = to.to_string();
        let subject = subject.to_string();
        let body = body.to_string();
        // Fire-and-forget: spawn an async task so the subscriber loop never blocks.
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let payload = serde_json::json!({
                "from": from,
                "to": [to],
                "subject": subject,
                "text": body,
            });
            match client
                .post("https://api.resend.com/emails")
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    info!("degradation alert sent to {to}");
                }
                Ok(resp) => warn!(
                    "degradation alert Resend returned {} — {}",
                    resp.status(),
                    resp.text().await.unwrap_or_default()
                ),
                Err(e) => warn!("degradation alert Resend request failed: {e}"),
            }
        });
    }
}

// ---------------------------------------------------------------------------
// No-op mailer: used when RESEND_API_KEY is absent
// ---------------------------------------------------------------------------

struct NoopAlertMailer;

impl AlertMailer for NoopAlertMailer {
    fn send_alert(&self, _to: &str, subject: &str, _body: &str) {
        debug!("degradation alert suppressed (no RESEND_API_KEY): {subject}");
    }
}

// ---------------------------------------------------------------------------
// Debounce
// ---------------------------------------------------------------------------

struct Debounce {
    last_sent: HashMap<String, DateTime<Utc>>,
    window: ChronoDuration,
}

impl Debounce {
    fn new(hours: i64) -> Self {
        Self {
            last_sent: HashMap::new(),
            window: ChronoDuration::hours(hours),
        }
    }

    /// Returns `true` if an alert should fire for `key` and records the send time.
    fn should_send(&mut self, key: &str) -> bool {
        let now = Utc::now();
        if let Some(last) = self.last_sent.get(key)
            && now - *last < self.window
        {
            return false;
        }
        self.last_sent.insert(key.to_string(), now);
        true
    }
}

// ---------------------------------------------------------------------------
// Email formatting helpers
// ---------------------------------------------------------------------------

/// Build `(subject, body)` for a degradation event.
fn format_email(ev: &Event) -> (String, String) {
    let p = &ev.payload;
    match ev.event_type.as_str() {
        "backup.skipped_low_disk" => {
            let avail_mb = p["available_bytes"].as_u64().unwrap_or(0) / 1_048_576;
            let req_mb = p["required_bytes"].as_u64().unwrap_or(0) / 1_048_576;
            let last_mb = p["last_snapshot_bytes"].as_u64().unwrap_or(0) / 1_048_576;
            let backend = p["backend"].as_str().unwrap_or("unknown");
            let subject = "[CO] Backup pulado — disco cheio".to_string();
            let body = format!(
                "Evento: backup.skipped_low_disk\n\
                 Backend: {backend}\n\
                 Livre: {avail_mb} MB\n\
                 Necessário: {req_mb} MB (2× último snapshot)\n\
                 Último snapshot: {last_mb} MB\n\n\
                 Libere espaço ou expanda o volume para retomar os backups."
            );
            (subject, body)
        }

        "universe.unavailable" => {
            let universe = ev
                .universe_key
                .as_deref()
                .unwrap_or_else(|| p["universe_key"].as_str().unwrap_or("unknown"));
            let reason = p["reason"].as_str().unwrap_or("unknown error");
            let subject = format!("[CO] Universo indisponível: {universe}");
            let body = format!(
                "Evento: universe.unavailable\n\
                 Universo: {universe}\n\
                 Motivo: {reason}\n\n\
                 Verifique /gestao > atividades para mais detalhes."
            );
            (subject, body)
        }

        "system.disk_pressure" => {
            let free_mb = p["free_bytes"].as_u64().unwrap_or(0) / 1_048_576;
            let total_mb = p["total_bytes"].as_u64().unwrap_or(0) / 1_048_576;
            let free_pct = p["free_pct"].as_f64().unwrap_or(0.0);
            let threshold = p["threshold_pct"].as_u64().unwrap_or(15);
            let path = p["path"].as_str().unwrap_or("/data");
            let subject = format!("[CO] Disco acima do limite: {free_pct:.1}% livre");
            let body = format!(
                "Evento: system.disk_pressure\n\
                 Caminho: {path}\n\
                 Livre: {free_mb} MB de {total_mb} MB ({free_pct:.1}%)\n\
                 Limiar: {threshold}%\n\n\
                 Ação recomendada: limpe logs, expanda o volume ou remova snapshots antigos."
            );
            (subject, body)
        }

        other => {
            let subject = format!("[CO] Degradação: {other}");
            let body = format!(
                "Evento de degradação detectado: {other}\n\nPayload: {}",
                ev.payload
            );
            (subject, body)
        }
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Event types the alerter subscribes to.
pub const DEGRADATION_EVENT_TYPES: &[&str] = &[
    "backup.skipped_low_disk",
    "universe.unavailable",
    "system.disk_pressure",
];

// ---------------------------------------------------------------------------
// spawn
// ---------------------------------------------------------------------------

/// Spawn the degradation alerter subscriber.
///
/// - `mailer`: injectable email sender (production or mock).
/// - `alert_to`: recipient address (`CO_ALERT_TO`, default `yuri@artelonga.com.br`).
/// - `debounce_hours`: max 1 email per event type per this many hours.
pub fn spawn(
    bus: Arc<dyn EdaBus>,
    mailer: Arc<dyn AlertMailer>,
    alert_to: String,
    debounce_hours: u64,
) {
    let mut sub = bus.subscribe(Filter {
        event_types: Some(
            DEGRADATION_EVENT_TYPES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        ),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!(
            "EDA: DegradationAlerter started (alert_to={alert_to}, \
             debounce={debounce_hours}h)"
        );
        let mut debounce = Debounce::new(debounce_hours as i64);

        while let Some(ev) = sub.recv().await {
            if !debounce.should_send(&ev.event_type) {
                debug!(
                    "DegradationAlerter: debounce suppressed {} alert",
                    ev.event_type
                );
                continue;
            }
            let (subject, body) = format_email(&ev);
            mailer.send_alert(&alert_to, &subject, &body);
            info!(
                "DegradationAlerter: alert dispatched for {} → {alert_to}",
                ev.event_type
            );
        }
        info!("EDA: DegradationAlerter stopped (bus closed)");
    });
}

/// Build the production mailer from config + secrets (CO-434).
///
/// Returns a `ResendAlertMailer` when `RESEND_API_KEY` is set, or a `NoopAlertMailer`
/// with a one-time WARN when the key is absent.
pub fn mailer_from_secrets(
    config: &crate::CoServerConfig,
    secrets: &dyn crate::infra::secrets::SecretsProvider,
) -> Arc<dyn AlertMailer> {
    match ResendAlertMailer::from_secrets(config, secrets) {
        Some(m) => {
            info!("CO-422: DegradationAlerter using Resend (RESEND_API_KEY present)");
            Arc::new(m)
        }
        None => {
            warn!(
                "CO-422: RESEND_API_KEY absent — degradation alerts will not be emailed. \
                 Set RESEND_API_KEY on the Fly machine to enable email alerts."
            );
            Arc::new(NoopAlertMailer)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::eda::event::Visibility;
    use crate::eda::tokio_bus::TokioBroadcastBus;

    // --- MockAlertMailer -------------------------------------------------------

    pub struct MockAlertMailer {
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl MockAlertMailer {
        pub fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }

        pub fn subjects(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(s, _)| s.clone())
                .collect()
        }
    }

    impl AlertMailer for MockAlertMailer {
        fn send_alert(&self, _to: &str, subject: &str, body: &str) {
            self.calls
                .lock()
                .unwrap()
                .push((subject.to_string(), body.to_string()));
        }
    }

    // --- helpers ---------------------------------------------------------------

    fn make_bus() -> Arc<TokioBroadcastBus> {
        Arc::new(TokioBroadcastBus::new())
    }

    fn backup_event() -> Event {
        Event::new(
            "backup.skipped_low_disk",
            None,
            None,
            serde_json::json!({
                "available_bytes": 100_000_000u64,
                "required_bytes": 536_870_912u64,
                "last_snapshot_bytes": 268_435_456u64,
                "backend": "fly-s3",
            }),
            Visibility::System,
        )
    }

    fn disk_event() -> Event {
        Event::new(
            "system.disk_pressure",
            None,
            None,
            serde_json::json!({
                "free_bytes": 150_000_000u64,
                "total_bytes": 1_000_000_000u64,
                "free_pct": 15.0f64,
                "threshold_pct": 15u64,
                "path": "/data",
            }),
            Visibility::System,
        )
    }

    // --- tests -----------------------------------------------------------------

    #[tokio::test]
    async fn degradation_event_triggers_alert_once() {
        let bus = make_bus();
        let mock = Arc::new(MockAlertMailer::new());

        spawn(
            Arc::clone(&bus) as Arc<dyn EdaBus>,
            Arc::clone(&mock) as Arc<dyn AlertMailer>,
            "ops@example.com".to_string(),
            2,
        );

        bus.publish(backup_event());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(mock.call_count(), 1);
        assert!(mock.subjects()[0].contains("Backup pulado"));
    }

    #[tokio::test]
    async fn debounce_suppresses_second_alert_within_window() {
        let bus = make_bus();
        let mock = Arc::new(MockAlertMailer::new());

        // debounce_hours=2: two events within milliseconds → only 1 email.
        spawn(
            Arc::clone(&bus) as Arc<dyn EdaBus>,
            Arc::clone(&mock) as Arc<dyn AlertMailer>,
            "ops@example.com".to_string(),
            2,
        );

        bus.publish(backup_event());
        bus.publish(backup_event());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(mock.call_count(), 1, "debounce must suppress second alert");
    }

    #[tokio::test]
    async fn different_event_types_each_get_one_alert() {
        let bus = make_bus();
        let mock = Arc::new(MockAlertMailer::new());

        spawn(
            Arc::clone(&bus) as Arc<dyn EdaBus>,
            Arc::clone(&mock) as Arc<dyn AlertMailer>,
            "ops@example.com".to_string(),
            2,
        );

        bus.publish(backup_event());
        bus.publish(disk_event());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(
            mock.call_count(),
            2,
            "each distinct event type gets its own alert"
        );
    }

    #[tokio::test]
    async fn non_degradation_events_are_ignored() {
        let bus = make_bus();
        let mock = Arc::new(MockAlertMailer::new());

        spawn(
            Arc::clone(&bus) as Arc<dyn EdaBus>,
            Arc::clone(&mock) as Arc<dyn AlertMailer>,
            "ops@example.com".to_string(),
            2,
        );

        // Publish an unrelated event — should not trigger the alerter.
        bus.publish(Event::new(
            "entry.created",
            Some("u1".into()),
            None,
            serde_json::json!({"path": "note.md"}),
            Visibility::UniverseMembers,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(mock.call_count(), 0);
    }
}
