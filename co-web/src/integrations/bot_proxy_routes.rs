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

async fn chat_handler(Json(req): Json<ChatReq>) -> Response {
    let text = req.text.trim();
    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "empty text"})),
        )
            .into_response();
    }
    let mut payload = json!({ "text": text, "sender": "app" });
    if let Some(u) = req.universe.as_deref().filter(|u| !u.is_empty()) {
        payload["universe"] = json!(u);
    }
    let client = reqwest::Client::new();
    let resp = client.post(brain_url()).json(&payload).send().await;
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
}
