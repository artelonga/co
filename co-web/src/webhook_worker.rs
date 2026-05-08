//! Background worker that drains the `notifications` table and delivers
//! outbound webhooks with HMAC-SHA256 signatures. CO-168.
//!
//! # Delivery headers
//! ```text
//! X-CO-Event:          <event_type>
//! X-CO-Delivery:       <notification_id>
//! X-CO-Signature-256:  sha256=<hmac_hex>
//! Content-Type:        application/json
//! ```

use std::time::Duration;

use tracing::{error, info, warn};

use crate::server::AppState;
use crate::webhook::{
    claim_next_notification, get_webhook_with_secret, mark_notification_failed,
    mark_notification_sent,
};

const POLL_INTERVAL_SECS: u64 = 5;
const HTTP_TIMEOUT_SECS: u64 = 10;

/// Spawn the webhook delivery worker. Polls every 5 s, delivers one
/// notification per tick. Runs until process exit.
pub fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        // Build a single reqwest client for connection pooling.
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                error!("webhook_worker: failed to build HTTP client: {e}");
                return;
            }
        };

        loop {
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;

            // --- claim one pending notification ---
            let notification = {
                match state.storage.lock() {
                    Ok(s) => claim_next_notification(s.conn()),
                    Err(e) => {
                        error!("webhook_worker: storage lock poisoned: {e}");
                        continue;
                    }
                }
            };

            let Some(notif) = notification else {
                continue;
            };

            // --- look up the webhook config (secret needed for signing) ---
            let webhook = {
                match state.storage.lock() {
                    Ok(s) => get_webhook_with_secret(s.conn(), &notif.webhook_id),
                    Err(e) => {
                        error!("webhook_worker: storage lock poisoned: {e}");
                        continue;
                    }
                }
            };

            let Some(webhook) = webhook else {
                // Webhook was deleted after the notification was inserted (race).
                // The ON DELETE CASCADE will clean up the notification eventually;
                // nothing to deliver.
                continue;
            };

            let secret = webhook.secret.clone().unwrap_or_default();
            let body = notif.payload.as_bytes().to_vec();
            let signature = crate::webhook::hmac_signature(&secret, &body);

            info!(
                notification_id = %notif.id,
                webhook_id = %notif.webhook_id,
                event_type = %notif.event_type,
                url = %webhook.url,
                attempt = notif.attempts + 1,
                "webhook_worker: delivering"
            );

            let result = client
                .post(&webhook.url)
                .header("Content-Type", "application/json")
                .header("X-CO-Event", &notif.event_type)
                .header("X-CO-Delivery", &notif.id)
                .header("X-CO-Signature-256", &signature)
                .body(body)
                .send()
                .await;

            let new_attempts = notif.attempts + 1;

            match result {
                Ok(resp) if resp.status().is_success() => {
                    info!(
                        notification_id = %notif.id,
                        status = %resp.status(),
                        "webhook_worker: delivered"
                    );
                    if let Ok(s) = state.storage.lock() {
                        mark_notification_sent(s.conn(), &notif.id);
                    }
                }
                Ok(resp) => {
                    let err = format!("HTTP {}", resp.status());
                    warn!(
                        notification_id = %notif.id,
                        attempt = new_attempts,
                        error = %err,
                        "webhook_worker: delivery failed"
                    );
                    if let Ok(s) = state.storage.lock() {
                        mark_notification_failed(s.conn(), &notif.id, new_attempts, &err);
                    }
                }
                Err(e) => {
                    let err = e.to_string();
                    warn!(
                        notification_id = %notif.id,
                        attempt = new_attempts,
                        error = %err,
                        "webhook_worker: network error"
                    );
                    if let Ok(s) = state.storage.lock() {
                        mark_notification_failed(s.conn(), &notif.id, new_attempts, &err);
                    }
                }
            }
        }
    });
}

// The worker itself is integration-level (real async HTTP); behaviour is
// validated via the in-process storage tests in webhook.rs.
