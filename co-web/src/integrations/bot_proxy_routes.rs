//! CO-483 — authenticated chat proxy to the bot brain.
//!
//! The bot brain (the `whatsapp-bot` bridge `/api/chat`) stays **loopback-only +
//! unauthenticated**. This route is the ONLY network-reachable way to reach it,
//! and it sits behind `require_auth` (JWT session/Bearer) + the global CO-80 rate
//! limiter. The companion app calls THIS instead of the bridge directly, so chat
//! works for remote/guest users without ever exposing the unauthenticated bridge
//! to the LAN/internet.
//!
//!   POST /api/v1/bot/chat   { "text": "..." }  →  { "reply": "...", "intent": "..." }
//!
//! Env: `CO_BOT_BRAIN_URL` (default `http://127.0.0.1:8765/api/chat`).

use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::Deserialize;
use serde_json::json;

use crate::server::AppState;

#[derive(Deserialize)]
struct ChatReq {
    text: String,
    /// CO-489: optional tenant universe, forwarded to the brain so the companion
    /// app routes to the same multi-tenant brain as WhatsApp. Backward-compatible
    /// — omitting it falls back to the brain's default universe.
    #[serde(default)]
    universe: Option<String>,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new().route("/bot/chat", post(chat_handler)).layer(
        axum::middleware::from_fn_with_state(state, crate::auth::require_auth),
    )
}

fn brain_url() -> String {
    crate::infra::secrets::global().get_or("CO_BOT_BRAIN_URL", "http://127.0.0.1:8765/api/chat")
}

/// CO-490 (#5): the loopback authentication header the bot brain checks. Built
/// purely from the `CO_BOT_SHARED_SECRET` value so it is unit-testable without a
/// live socket: `Some(("X-Bot-Secret", secret))` when the secret is set and
/// non-empty, `None` otherwise (header omitted → brain treats it as unset).
pub fn brain_auth_header(secret: Option<String>) -> Option<(&'static str, String)> {
    secret
        .filter(|s| !s.is_empty())
        .map(|s| ("X-Bot-Secret", s))
}

async fn chat_handler(user_id: crate::auth::UserId, Json(req): Json<ChatReq>) -> Response {
    let text = req.text.trim();
    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "empty text"})),
        )
            .into_response();
    }
    // CO-490 (#1): forward the authenticated user so the bot acts AS that user
    // (its BrainRouter keys per-user permissions off `user_id`). `sender` stays
    // "app" so the brain still knows the surface this came from.
    let mut payload = json!({ "text": text, "sender": "app", "user_id": user_id.0 });
    if let Some(u) = req.universe.as_deref().filter(|u| !u.is_empty()) {
        payload["universe"] = json!(u);
    }
    let client = reqwest::Client::new();
    let mut builder = client.post(brain_url()).json(&payload);
    // CO-490 (#5): authenticate co-web → bot over loopback when configured.
    if let Some((name, value)) =
        brain_auth_header(crate::infra::secrets::global().get("CO_BOT_SHARED_SECRET"))
    {
        builder = builder.header(name, value);
    }
    let resp = builder.send().await;
    match resp {
        Ok(r) if r.status().is_success() => {
            let body = r.bytes().await.unwrap_or_default();
            match parse_reply(&body) {
                Some((reply, intent)) => (
                    StatusCode::OK,
                    Json(json!({"reply": reply, "intent": intent})),
                )
                    .into_response(),
                None => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": "bot brain returned no reply"})),
                )
                    .into_response(),
            }
        }
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "bot brain unreachable"})),
        )
            .into_response(),
    }
}

/// `(reply, intent)` from the brain's JSON response; `None` if no non-empty reply.
pub fn parse_reply(body: &[u8]) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let reply = v
        .get("reply")
        .and_then(|r| r.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    let intent = v
        .get("intent")
        .and_then(|i| i.as_str())
        .unwrap_or("chat")
        .to_string();
    Some((reply, intent))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reply_extracts_reply_and_intent() {
        assert_eq!(
            parse_reply(br#"{"reply":"oi","intent":"next_task"}"#),
            Some(("oi".to_string(), "next_task".to_string()))
        );
        // intent defaults to "chat" when absent
        assert_eq!(
            parse_reply(br#"{"reply":"oi"}"#),
            Some(("oi".to_string(), "chat".to_string()))
        );
    }

    #[test]
    fn parse_reply_rejects_empty_or_garbage() {
        assert!(parse_reply(br#"{"reply":""}"#).is_none());
        assert!(parse_reply(br#"{"intent":"chat"}"#).is_none());
        assert!(parse_reply(b"not json").is_none());
    }

    // CO-490 (#5): the loopback secret header is present iff the secret is set.
    #[test]
    fn brain_auth_header_present_only_when_secret_set() {
        assert_eq!(
            brain_auth_header(Some("shh".to_string())),
            Some(("X-Bot-Secret", "shh".to_string()))
        );
        // Unset → no header.
        assert_eq!(brain_auth_header(None), None);
        // Empty → treated as unset.
        assert_eq!(brain_auth_header(Some(String::new())), None);
    }
}
