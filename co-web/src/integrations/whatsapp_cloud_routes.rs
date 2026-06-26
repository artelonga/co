//! CO-479 — inbound webhook for the official WhatsApp Cloud API.
//!
//! Meta calls this endpoint directly (no auth gate — security is the HMAC
//! signature, mirroring `billing::routes::webhook_router`):
//! - `GET  /api/v1/whatsapp/webhook` — verification handshake; echoes
//!   `hub.challenge` when `hub.verify_token` matches `WHATSAPP_VERIFY_TOKEN`.
//! - `POST /api/v1/whatsapp/webhook` — message events, signed with
//!   `X-Hub-Signature-256: sha256=<hmac>` keyed by `WHATSAPP_APP_SECRET`.
//!
//! Env vars:
//! - `WHATSAPP_VERIFY_TOKEN` — shared secret echoed during GET verification.
//! - `WHATSAPP_APP_SECRET`   — Meta app secret; verifies POST signatures.
//!
//! CO-480: inbound messages are forwarded to the bot brain (`CO_BOT_BRAIN_URL`,
//! default the `whatsapp-bot` bridge) and the reply is sent via
//! [`crate::notification_providers::CloudApiProvider`] — **asynchronously**, so
//! Meta still gets its fast 200 ack while the model call runs in the background.
//!
//! CO-489: the inbound `phone_number_id` (which WhatsApp business number was
//! messaged) is forwarded to the brain so a single bot deploy can route to the
//! right tenant universe (the brain maps `phone_number_id → universe`).

use std::collections::HashMap;

use axum::{
    Router,
    body::Bytes,
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::server::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/whatsapp/webhook", get(verify_handler).post(event_handler))
}

// ---------------------------------------------------------------------------
// GET — Meta verification handshake
// ---------------------------------------------------------------------------

async fn verify_handler(Query(params): Query<HashMap<String, String>>) -> Response {
    let expected = crate::infra::secrets::global()
        .get("WHATSAPP_VERIFY_TOKEN")
        .unwrap_or_default();
    let mode = params.get("hub.mode").map(String::as_str).unwrap_or("");
    let token = params
        .get("hub.verify_token")
        .map(String::as_str)
        .unwrap_or("");
    let challenge = params.get("hub.challenge").cloned().unwrap_or_default();

    if mode == "subscribe" && !expected.is_empty() && token == expected {
        (StatusCode::OK, challenge).into_response()
    } else {
        tracing::warn!("WhatsApp Cloud webhook: verification failed");
        (StatusCode::FORBIDDEN, "verification failed").into_response()
    }
}

// ---------------------------------------------------------------------------
// POST — signed message events
// ---------------------------------------------------------------------------

async fn event_handler(headers: HeaderMap, body: Bytes) -> Response {
    let app_secret = crate::infra::secrets::global()
        .get("WHATSAPP_APP_SECRET")
        .unwrap_or_default();

    // Meta requires a fast 200. If the secret is unset (not yet configured),
    // accept-and-ignore so the handshake/subscription works during setup.
    if app_secret.is_empty() {
        tracing::warn!("WHATSAPP_APP_SECRET unset — skipping signature verification");
        return StatusCode::OK.into_response();
    }

    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_signature(&app_secret, &body, sig) {
        tracing::warn!("WhatsApp Cloud webhook: invalid signature");
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let msgs = parse_inbound(&body);
    if !msgs.is_empty() {
        // CO-480: reply ASYNCHRONOUSLY. Meta requires a fast 200 (it retries on
        // delay), and the brain/model call takes seconds — so never block the ack
        // on it. Spawn the reply loop and return 200 immediately.
        tokio::spawn(async move {
            for m in msgs {
                tracing::info!(from = %m.from, "WhatsApp Cloud inbound: {}", m.text);
                if let Err(e) = reply_to(&m).await {
                    tracing::warn!(from = %m.from, "WhatsApp Cloud reply failed: {e}");
                }
            }
        });
    }

    StatusCode::OK.into_response()
}

// ---------------------------------------------------------------------------
// CO-480 — inbound → bot brain → reply (via the official Cloud API)
// ---------------------------------------------------------------------------

/// The bot brain endpoint (the `whatsapp-bot` bridge's `/api/chat`, or any
/// `{text}→{reply}` JSON service). Same brain the companion app uses.
fn brain_url() -> String {
    crate::infra::secrets::global().get_or("CO_BOT_BRAIN_URL", "http://localhost:8765/api/chat")
}

/// Inbound message → brain reply → send via [`CloudApiProvider`]. No-op (logged)
/// when the Cloud API isn't configured; falls back to a fixed ack if the brain
/// is unreachable so the user always gets a response.
async fn reply_to(msg: &InboundMsg) -> Result<(), String> {
    use crate::notification_providers::{ChannelProvider, CloudApiProvider};
    let provider = CloudApiProvider::from_env()
        .ok_or("CloudApiProvider not configured (WHATSAPP_CLOUD_TOKEN/PHONE_NUMBER_ID)")?;
    let client = reqwest::Client::new();
    let reply = fetch_brain_reply(&client, &brain_url(), &msg.text, &msg.phone_number_id)
        .await
        .unwrap_or_else(|| "Recebi sua mensagem — já te respondo! 🙂".to_string());
    provider.send(&client, &msg.from, &reply).await
}

/// POST `{text, phone_number_id}` to the brain, return its `reply`. The
/// `phone_number_id` selects the tenant universe brain-side (CO-489). `None` on
/// any transport/parse failure (caller substitutes a fallback).
async fn fetch_brain_reply(
    client: &reqwest::Client,
    url: &str,
    text: &str,
    phone_number_id: &str,
) -> Option<String> {
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "text": text,
            "sender": "whatsapp",
            "phone_number_id": phone_number_id,
        }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.bytes().await.ok()?;
    parse_brain_reply(&body)
}

/// Extract the non-empty `reply` string from the brain's JSON response.
pub fn parse_brain_reply(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("reply")
        .and_then(|r| r.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a server)
// ---------------------------------------------------------------------------

/// Verify Meta's `X-Hub-Signature-256: sha256=<hex>` against the raw body.
pub fn verify_signature(app_secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hexsig) = header.strip_prefix("sha256=") else {
        return false;
    };
    let expected = crate::billing::hmac_sha256_hex(app_secret, body);
    crate::billing::constant_time_eq(expected.as_bytes(), hexsig.as_bytes())
}

/// A single inbound text message lifted from a Cloud API webhook payload.
#[derive(Debug, PartialEq)]
pub struct InboundMsg {
    pub from: String,
    pub text: String,
    /// Which business number was messaged — selects the tenant brain-side (CO-489).
    pub phone_number_id: String,
}

/// Extract text messages from a Cloud API webhook body. Status-only callbacks
/// (delivery receipts, read markers) yield an empty vec.
pub fn parse_inbound(body: &[u8]) -> Vec<InboundMsg> {
    let mut out = Vec::new();
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return out;
    };
    let entries = v.get("entry").and_then(|e| e.as_array());
    for entry in entries.into_iter().flatten() {
        let changes = entry.get("changes").and_then(|c| c.as_array());
        for change in changes.into_iter().flatten() {
            let value = change.get("value");
            // CO-489: the business number id lives once per change, in metadata.
            let phone_number_id = value
                .and_then(|val| val.get("metadata"))
                .and_then(|md| md.get("phone_number_id"))
                .and_then(|p| p.as_str())
                .unwrap_or_default()
                .to_string();
            let msgs = value
                .and_then(|val| val.get("messages"))
                .and_then(|m| m.as_array());
            for m in msgs.into_iter().flatten() {
                let from = m
                    .get("from")
                    .and_then(|f| f.as_str())
                    .unwrap_or_default()
                    .to_string();
                let text = m
                    .get("text")
                    .and_then(|t| t.get("body"))
                    .and_then(|b| b.as_str())
                    .unwrap_or_default()
                    .to_string();
                if !from.is_empty() && !text.is_empty() {
                    out.push(InboundMsg {
                        from,
                        text,
                        phone_number_id: phone_number_id.clone(),
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> &'static [u8] {
        // Raw STRING (not byte string) so the UTF-8 accent is allowed.
        r#"{"object":"whatsapp_business_account","entry":[{"id":"WABA","changes":[
          {"value":{"messaging_product":"whatsapp",
            "metadata":{"display_phone_number":"5541999999999","phone_number_id":"1238594552665575"},
            "messages":[
            {"from":"5541999999999","id":"wamid.X","type":"text","text":{"body":"qual minha próxima tarefa?"}}
          ]},"field":"messages"}]}]}"#
            .as_bytes()
    }

    #[test]
    fn parse_inbound_extracts_from_text_and_phone_number_id() {
        let msgs = parse_inbound(sample_payload());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from, "5541999999999");
        assert_eq!(msgs[0].text, "qual minha próxima tarefa?");
        // CO-489: the tenant key rides through to the brain.
        assert_eq!(msgs[0].phone_number_id, "1238594552665575");
    }

    #[test]
    fn parse_inbound_tolerates_missing_metadata() {
        let no_meta = r#"{"entry":[{"changes":[{"value":{"messages":[
            {"from":"55","type":"text","text":{"body":"oi"}}]}}]}]}"#
            .as_bytes();
        let msgs = parse_inbound(no_meta);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].phone_number_id, ""); // empty → brain falls back to default
    }

    #[test]
    fn parse_inbound_ignores_status_only_callbacks() {
        let status =
            br#"{"entry":[{"changes":[{"value":{"statuses":[{"status":"delivered"}]}}]}]}"#;
        assert!(parse_inbound(status).is_empty());
    }

    #[test]
    fn parse_brain_reply_extracts_reply() {
        assert_eq!(
            parse_brain_reply(
                r#"{"reply":"Tua próxima tarefa é X","intent":"next_task"}"#.as_bytes()
            ),
            Some("Tua próxima tarefa é X".to_string())
        );
        assert_eq!(parse_brain_reply(br#"{"reply":""}"#), None); // empty → fallback
        assert_eq!(parse_brain_reply(br#"{"intent":"chat"}"#), None); // missing
        assert_eq!(parse_brain_reply(b"not json"), None);
    }

    #[test]
    fn verify_signature_accepts_valid_and_rejects_forged() {
        let secret = "app-secret";
        let body = sample_payload();
        let good = format!("sha256={}", crate::billing::hmac_sha256_hex(secret, body));
        assert!(verify_signature(secret, body, &good));
        assert!(!verify_signature(secret, body, "sha256=deadbeef"));
        assert!(!verify_signature(secret, body, "deadbeef")); // missing prefix
        assert!(!verify_signature("wrong-secret", body, &good));
    }
}
