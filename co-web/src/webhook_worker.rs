//! Background worker that drains the `notifications` table and delivers
//! outbound webhooks with HMAC-SHA256 signatures. CO-168.
//! Extended in CO-169 to dispatch email and WhatsApp via direct providers.
//!
//! # Delivery headers (webhook channel)
//! ```text
//! X-CO-Event:          <event_type>
//! X-CO-Delivery:       <notification_id>
//! X-CO-Signature-256:  sha256=<hmac_hex>
//! Content-Type:        application/json
//! ```

use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info, warn};

use crate::notification_providers::{ChannelProvider, EvolutionApiProvider, ResendProvider};
use crate::server::AppState;
use crate::webhook::{
    claim_next_notification, get_webhook_with_secret, mark_notification_failed,
    mark_notification_sent,
};

const POLL_INTERVAL_SECS: u64 = 5;
const HTTP_TIMEOUT_SECS: u64 = 10;

/// Spawn the webhook delivery worker. Polls every 5 s, delivers one
/// notification per tick. Runs until process exit.
///
/// Loads direct providers once at startup:
/// - `ResendProvider::from_env()` → `email` channel
/// - `EvolutionApiProvider::from_env()` → `whatsapp` channel
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

        // Load direct channel providers once at startup.
        let email_provider: Option<Arc<dyn ChannelProvider>> =
            ResendProvider::from_env().map(|p| Arc::new(p) as Arc<dyn ChannelProvider>);
        let whatsapp_provider: Option<Arc<dyn ChannelProvider>> =
            EvolutionApiProvider::from_env().map(|p| Arc::new(p) as Arc<dyn ChannelProvider>);

        if email_provider.is_some() {
            info!("webhook_worker: Resend email provider active");
        }
        if whatsapp_provider.is_some() {
            info!("webhook_worker: Evolution API WhatsApp provider active");
        }

        loop {
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;

            // --- claim one pending notification ---
            let notification = {
                let s = state.storage.lock();
                claim_next_notification(s.conn())
            };

            let Some(notif) = notification else {
                continue;
            };

            let new_attempts = notif.attempts + 1;

            // --- dispatch by channel ---
            match notif.channel.as_deref() {
                Some("email") => {
                    let Some(provider) = &email_provider else {
                        // Provider was not configured at startup; mark dead.
                        {
                            let s = state.storage.lock();
                            mark_notification_failed(
                                s.conn(),
                                &notif.id,
                                3,
                                "email provider not configured",
                            );
                        }
                        continue;
                    };
                    let recipient = notif.recipient.as_deref().unwrap_or_default();
                    info!(
                        notification_id = %notif.id,
                        recipient = %recipient,
                        "webhook_worker: sending email"
                    );
                    match provider.send(&client, recipient, &notif.payload).await {
                        Ok(()) => {
                            let s = state.storage.lock();
                            mark_notification_sent(s.conn(), &notif.id);
                        }
                        Err(e) => {
                            warn!(notification_id = %notif.id, error = %e, "webhook_worker: email failed");
                            {
                                let s = state.storage.lock();
                                mark_notification_failed(s.conn(), &notif.id, new_attempts, &e);
                            }
                        }
                    }
                }
                Some("whatsapp") => {
                    let Some(provider) = &whatsapp_provider else {
                        {
                            let s = state.storage.lock();
                            mark_notification_failed(
                                s.conn(),
                                &notif.id,
                                3,
                                "whatsapp provider not configured",
                            );
                        }
                        continue;
                    };
                    let recipient = notif.recipient.as_deref().unwrap_or_default();
                    info!(
                        notification_id = %notif.id,
                        recipient = %recipient,
                        "webhook_worker: sending WhatsApp"
                    );
                    match provider.send(&client, recipient, &notif.payload).await {
                        Ok(()) => {
                            let s = state.storage.lock();
                            mark_notification_sent(s.conn(), &notif.id);
                        }
                        Err(e) => {
                            warn!(notification_id = %notif.id, error = %e, "webhook_worker: WhatsApp failed");
                            let s = state.storage.lock();
                            mark_notification_failed(s.conn(), &notif.id, new_attempts, &e);
                        }
                    }
                }
                // Legacy webhook channel (or None)
                _ => {
                    // --- look up the webhook config (secret needed for signing) ---
                    let webhook = {
                        let s = state.storage.lock();
                        get_webhook_with_secret(s.conn(), &notif.webhook_id)
                    };

                    let Some(webhook) = webhook else {
                        // Webhook was deleted after the notification was inserted (race).
                        // The ON DELETE CASCADE will clean up the notification eventually;
                        // nothing to deliver.
                        continue;
                    };

                    // Skip sentinel webhook — should not appear here but guard anyway.
                    if webhook.id == "__direct__" {
                        continue;
                    }

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

                    match result {
                        Ok(resp) if resp.status().is_success() => {
                            info!(
                                notification_id = %notif.id,
                                status = %resp.status(),
                                "webhook_worker: delivered"
                            );
                            let s = state.storage.lock();
                            mark_notification_sent(s.conn(), &notif.id);
                        }
                        Ok(resp) => {
                            let err = format!("HTTP {}", resp.status());
                            warn!(
                                notification_id = %notif.id,
                                attempt = new_attempts,
                                error = %err,
                                "webhook_worker: delivery failed"
                            );
                            let s = state.storage.lock();
                            mark_notification_failed(s.conn(), &notif.id, new_attempts, &err);
                        }
                        Err(e) => {
                            let err = e.to_string();
                            warn!(
                                notification_id = %notif.id,
                                attempt = new_attempts,
                                error = %err,
                                "webhook_worker: network error"
                            );
                            let s = state.storage.lock();
                            mark_notification_failed(s.conn(), &notif.id, new_attempts, &err);
                        }
                    }
                }
            }
        }
    });
}

// The worker itself is integration-level (real async HTTP); behaviour is
// validated via the in-process storage tests in webhook.rs.
