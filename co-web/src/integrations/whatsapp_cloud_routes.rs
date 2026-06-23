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
//! The reply path (forward inbound text to the bot brain, answer via
//! [`crate::notification_providers::CloudApiProvider`]) is a follow-up; this
//! scaffold verifies, parses, and logs inbound messages so the compliant
//! transport is ready the moment a WABA + number exist.

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

    for m in parse_inbound(&body) {
        tracing::info!(from = %m.from, "WhatsApp Cloud inbound: {}", m.text);
        // TODO(CO-480): forward to the bot brain + reply via CloudApiProvider.
    }

    StatusCode::OK.into_response()
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
            let msgs = change
                .get("value")
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
                    out.push(InboundMsg { from, text });
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
          {"value":{"messaging_product":"whatsapp","messages":[
            {"from":"5541999999999","id":"wamid.X","type":"text","text":{"body":"qual minha próxima tarefa?"}}
          ]},"field":"messages"}]}]}"#
            .as_bytes()
    }

    #[test]
    fn parse_inbound_extracts_from_and_text() {
        let msgs = parse_inbound(sample_payload());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from, "5541999999999");
        assert_eq!(msgs[0].text, "qual minha próxima tarefa?");
    }

    #[test]
    fn parse_inbound_ignores_status_only_callbacks() {
        let status =
            br#"{"entry":[{"changes":[{"value":{"statuses":[{"status":"delivered"}]}}]}]}"#;
        assert!(parse_inbound(status).is_empty());
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
