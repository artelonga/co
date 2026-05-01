//! CO-118: Cloudflare Workers Analytics Engine emitter.
//!
//! `co-web` → HTTP POST → WAE Worker proxy → `env.WAE.writeDataPoint()`
//!
//! All writes are fire-and-forget and do not block the request path.
//! When `WAE_ENDPOINT` / `WAE_API_KEY` are absent the emitter is a no-op,
//! so local dev and test runs are unaffected.

use std::sync::Arc;

use serde::Serialize;

// ---------------------------------------------------------------------------
// Privacy limits — load-bearing, tested as a guarantee (CO-118)
// ---------------------------------------------------------------------------

/// Maximum byte length for any string field value in a telemetry payload.
pub const MAX_FIELD_LEN: usize = 256;

/// Field name substrings that are never permitted in telemetry payloads.
/// Regex equivalent: /content|body|text|markdown|prose/i
const BLOCKED_KEY_PATTERNS: &[&str] = &["content", "body", "text", "markdown", "prose"];

// ---------------------------------------------------------------------------
// Event shape (from CO-118 spec)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryEvent {
    /// "page_view" | "ab_exposure" | "error" | "deploy"
    pub event_type: String,
    /// Partition key — use universe slug or "system" for infra events.
    pub universe_id: String,
    /// "anon" | "authed" | "admin"
    pub user_kind: String,
    /// For ab_exposure events only.
    pub flag_key: Option<String>,
    /// For ab_exposure events only.
    pub variant: Option<String>,
    pub duration_ms: Option<u32>,
    pub status: Option<u16>,
}

// ---------------------------------------------------------------------------
// Privacy filter
// ---------------------------------------------------------------------------

/// Structured privacy rejection (returned as JSON on violation).
#[derive(Debug, Serialize)]
pub struct PrivacyError {
    pub field: String,
    pub reason: String,
}

/// Validate a single arbitrary key/value pair from an inbound payload.
///
/// Returns `Some(PrivacyError)` when the field is blocked:
/// - key name matches a banned pattern (`content`, `body`, `text`, `markdown`, `prose`)
/// - value length > 256 bytes
///
/// This is enforced at both the Worker proxy (JS) and here in `co-web` (Rust).
pub fn check_payload_field(key: &str, value: &str) -> Option<PrivacyError> {
    let key_lc = key.to_lowercase();
    for pattern in BLOCKED_KEY_PATTERNS {
        if key_lc.contains(pattern) {
            return Some(PrivacyError {
                field: key.to_string(),
                reason: format!("field name matches blocked pattern '{pattern}'"),
            });
        }
    }
    if value.len() > MAX_FIELD_LEN {
        return Some(PrivacyError {
            field: key.to_string(),
            reason: format!("value exceeds {} bytes", MAX_FIELD_LEN),
        });
    }
    None
}

/// Validate the fixed fields of a `TelemetryEvent` against the privacy rules.
pub fn check_privacy(event: &TelemetryEvent) -> Result<(), PrivacyError> {
    let pairs = [
        ("event_type", event.event_type.as_str()),
        ("universe_id", event.universe_id.as_str()),
        ("user_kind", event.user_kind.as_str()),
    ];
    for (k, v) in pairs {
        if let Some(e) = check_payload_field(k, v) {
            return Err(e);
        }
    }
    if let Some(v) = &event.flag_key
        && let Some(e) = check_payload_field("flag_key", v)
    {
        return Err(e);
    }
    if let Some(v) = &event.variant
        && let Some(e) = check_payload_field("variant", v)
    {
        return Err(e);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Emitter
// ---------------------------------------------------------------------------

pub struct WaeEmitter {
    endpoint: Option<String>,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl WaeEmitter {
    /// Build an emitter. Passes `None` for both fields to get a no-op emitter
    /// (used in tests and when env vars are absent).
    pub fn new(endpoint: Option<String>, api_key: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            endpoint,
            api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        })
    }

    /// Fire-and-forget emit to WAE. Returns immediately; the HTTP request is
    /// sent from a background task. No-op when not configured or when the
    /// event fails the privacy filter (failures are logged, never propagated).
    pub fn emit(self: Arc<Self>, event: TelemetryEvent) {
        if self.endpoint.is_none() {
            return;
        }
        if let Err(e) = check_privacy(&event) {
            tracing::warn!(
                "WAE privacy filter blocked event: field={} reason={}",
                e.field,
                e.reason
            );
            return;
        }
        tokio::spawn(async move {
            self.do_send(&event).await;
        });
    }

    async fn do_send(&self, event: &TelemetryEvent) {
        let (Some(endpoint), Some(api_key)) = (&self.endpoint, &self.api_key) else {
            return;
        };
        if let Err(e) = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(event)
            .send()
            .await
        {
            tracing::warn!("WAE emit failed: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_filter_blocks_body_field() {
        let big_body = "x".repeat(1024); // 1 KB — well above the 256-byte limit
        let err = check_payload_field("body", &big_body);
        assert!(err.is_some(), "1KB 'body' field must be rejected");
        let e = err.unwrap();
        assert_eq!(e.field, "body");
        // Blocked by name pattern first (before size check)
        assert!(e.reason.contains("blocked pattern"), "reason: {}", e.reason);
    }

    #[test]
    fn privacy_filter_blocks_content_field() {
        let err = check_payload_field("content", "some text");
        assert!(err.is_some());
        assert!(err.unwrap().reason.contains("blocked pattern"));
    }

    #[test]
    fn privacy_filter_blocks_text_field() {
        let err = check_payload_field("text", "hello");
        assert!(err.is_some());
        assert!(err.unwrap().reason.contains("blocked pattern"));
    }

    #[test]
    fn privacy_filter_blocks_markdown_field() {
        let err = check_payload_field("markdown_src", "# heading");
        assert!(err.is_some());
    }

    #[test]
    fn privacy_filter_blocks_prose_field() {
        let err = check_payload_field("prose_content", "text");
        assert!(err.is_some());
    }

    #[test]
    fn privacy_filter_blocks_overlong_value() {
        let long_val = "a".repeat(257);
        let err = check_payload_field("some_field", &long_val);
        assert!(err.is_some());
        assert!(err.unwrap().reason.contains("exceeds"));
    }

    #[test]
    fn privacy_filter_allows_exact_limit() {
        let exact = "a".repeat(256);
        assert!(check_payload_field("some_field", &exact).is_none());
    }

    #[test]
    fn privacy_filter_allows_valid_event_fields() {
        assert!(check_payload_field("event_type", "page_view").is_none());
        assert!(check_payload_field("universe_id", "my-universe").is_none());
        assert!(check_payload_field("user_kind", "anon").is_none());
        assert!(check_payload_field("flag_key", "hero-cta").is_none());
        assert!(check_payload_field("variant", "b").is_none());
    }

    #[test]
    fn privacy_filter_case_insensitive_key_match() {
        // "BODY", "Content-Type", "textField" should all be blocked
        assert!(check_payload_field("BODY", "val").is_some());
        assert!(check_payload_field("Content-Type", "application/json").is_some());
        assert!(check_payload_field("textField", "hello").is_some());
    }

    #[test]
    fn check_privacy_valid_event_passes() {
        let ev = TelemetryEvent {
            event_type: "page_view".into(),
            universe_id: "template".into(),
            user_kind: "anon".into(),
            flag_key: None,
            variant: None,
            duration_ms: Some(42),
            status: None,
        };
        assert!(check_privacy(&ev).is_ok());
    }

    #[test]
    fn check_privacy_rejects_overlong_universe_id() {
        let ev = TelemetryEvent {
            event_type: "page_view".into(),
            universe_id: "u".repeat(257),
            user_kind: "anon".into(),
            flag_key: None,
            variant: None,
            duration_ms: None,
            status: None,
        };
        assert!(check_privacy(&ev).is_err());
    }
}
