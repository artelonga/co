//! Direct notification provider adapters — CO-169.
//!
//! Implements two channel providers on top of the existing notification queue:
//! - [`ResendProvider`]: sends transactional email via Resend API.
//! - [`EvolutionApiProvider`]: sends WhatsApp text via Evolution API.
//!
//! Both providers are constructed via `from_env()` which returns `None` when
//! the required env vars are absent. Email/WhatsApp rows are only enqueued when
//! the corresponding provider would be built.
//!
//! # Template rendering
//! Templates are loaded from env vars with the pattern:
//! - `CO_TPL_{PREFIX}_EMAIL_SUBJECT`
//! - `CO_TPL_{PREFIX}_EMAIL_BODY`
//! - `CO_TPL_{PREFIX}_WHATSAPP`
//!
//! Where `{PREFIX}` is derived from the event type by uppercasing and replacing
//! `.` with `_`. E.g. `quilombo.evento.criado` → `QUILOMBO_EVENTO_CRIADO`.
//!
//! Substitution uses `{{key}}` placeholders resolved from the event payload.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A channel-specific notification provider.
#[async_trait]
pub trait ChannelProvider: Send + Sync {
    /// Unique channel name: `"email"` | `"whatsapp"`.
    fn name(&self) -> &'static str;

    /// Send a single notification. Returns `Ok(())` on success.
    async fn send(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        payload: &str,
    ) -> Result<(), String>;

    /// CO-496: send a **business-initiated** message, honoring any provider-specific
    /// 24h customer-service window. The default impl just sends free-form `text` —
    /// correct for providers with no window constraint (Evolution self-host links
    /// your own WhatsApp; email has no window). [`CloudApiProvider`] overrides this:
    /// outside Meta's 24h window, free-form text is rejected, so it falls back to a
    /// pre-approved `template` (erroring with an actionable message when none is
    /// configured, rather than silently sending text Meta will reject).
    async fn send_business_initiated(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        text: &str,
        _template: Option<&OutboundTemplate>,
    ) -> Result<(), String> {
        self.send(client, recipient, text).await
    }
}

/// A pre-approved WhatsApp template plus the positional body params that fill its
/// `{{1}}`, `{{2}}`, … placeholders — the only message deliverable **outside** the
/// 24h customer-service window (CO-496). Loaded from env via [`template_from_env`].
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundTemplate {
    pub name: String,
    pub lang: String,
    pub params: Vec<String>,
}

impl OutboundTemplate {
    /// Clone the template (name + lang) with fresh per-recipient `params` (e.g. the
    /// greeted user's name, or the OTP code).
    pub fn with_params(&self, params: Vec<String>) -> Self {
        Self {
            name: self.name.clone(),
            lang: self.lang.clone(),
            params,
        }
    }
}

/// Load an [`OutboundTemplate`] from env: `name_var` holds the approved template
/// name (required → `None` when unset/empty so the caller can log an actionable
/// error), `lang_var` the BCP-47 language code (default `pt_BR`). `params` are the
/// caller-supplied body substitutions.
pub fn template_from_env(
    name_var: &str,
    lang_var: &str,
    params: Vec<String>,
) -> Option<OutboundTemplate> {
    let secrets = crate::infra::secrets::global();
    let name = secrets.get(name_var).filter(|s| !s.is_empty())?;
    let lang = secrets.get_or(lang_var, "pt_BR");
    Some(OutboundTemplate { name, lang, params })
}

// ---------------------------------------------------------------------------
// Template rendering
// ---------------------------------------------------------------------------

/// Derive the env var prefix from an event type.
/// `quilombo.evento.criado` → `QUILOMBO_EVENTO_CRIADO`
pub fn event_type_to_prefix(event_type: &str) -> String {
    event_type.to_uppercase().replace('.', "_")
}

/// Render `{{key}}` placeholders in a template string using the provided JSON payload.
pub fn render_template(template: &str, payload: &Value) -> String {
    let mut result = template.to_string();
    if let Some(obj) = payload.as_object() {
        for (key, value) in obj {
            let placeholder = format!("{{{{{}}}}}", key);
            let replacement = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => value.to_string(),
            };
            result = result.replace(&placeholder, &replacement);
        }
    }
    result
}

/// Resolve a template for the given event type and channel slot.
/// Checks env var first; falls back to built-in default if available.
pub fn get_template(event_type: &str, slot: &str) -> Option<String> {
    let prefix = event_type_to_prefix(event_type);
    let var_name = format!("CO_TPL_{prefix}_{slot}");
    if let Some(val) = crate::infra::secrets::global().get(&var_name) {
        return Some(val);
    }
    // Built-in defaults for known events
    match (event_type, slot) {
        ("quilombo.evento.criado", "EMAIL_SUBJECT") => {
            Some("Novo evento no Quilombo: {{titulo}}".to_string())
        }
        ("quilombo.evento.criado", "EMAIL_BODY") => Some(
            "Olá {{nome}},\n\nHá um novo evento: {{titulo}}.\n\nVeja em https://quilomboaraucaria.com.br".to_string()
        ),
        ("quilombo.evento.criado", "WHATSAPP") => {
            Some("🌿 Novo evento no Quilombo!\n*{{titulo}}*".to_string())
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// ResendProvider
// ---------------------------------------------------------------------------

/// Sends transactional email via api.resend.com.
/// Requires env vars: `RESEND_API_KEY`, `RESEND_FROM`.
pub struct ResendProvider {
    api_key: String,
    from: String,
}

impl ResendProvider {
    /// Construct from env vars. Returns `None` if `RESEND_API_KEY` is absent.
    pub fn from_env() -> Option<Self> {
        let secrets = crate::infra::secrets::global();
        let api_key = secrets.get("RESEND_API_KEY")?;
        let from = secrets.get_or("RESEND_FROM", "CO <noreply@quilomboaraucaria.com.br>");
        Some(Self { api_key, from })
    }
}

#[async_trait]
impl ChannelProvider for ResendProvider {
    fn name(&self) -> &'static str {
        "email"
    }

    async fn send(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        payload: &str,
    ) -> Result<(), String> {
        // payload encodes: subject\n---\nbody (simple separator convention)
        let (subject, body) = if let Some(sep) = payload.find("\n---\n") {
            (&payload[..sep], &payload[sep + 5..])
        } else {
            (payload, payload)
        };

        let request_body = serde_json::json!({
            "from": self.from,
            "to": [recipient],
            "subject": subject,
            "text": body,
        });

        let resp = client
            .post("https://api.resend.com/emails")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Resend request failed: {e}"))?;

        if resp.status().is_success() {
            info!(recipient = %recipient, "ResendProvider: email delivered");
            Ok(())
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(recipient = %recipient, status = %status, body = %text, "ResendProvider: delivery failed");
            Err(format!("Resend API {status}: {text}"))
        }
    }
}

// ---------------------------------------------------------------------------
// EvolutionApiProvider
// ---------------------------------------------------------------------------

/// Sends WhatsApp text via Evolution API.
/// Requires env vars: `EVOLUTION_API_URL`, `EVOLUTION_API_KEY`, `EVOLUTION_INSTANCE`.
pub struct EvolutionApiProvider {
    api_url: String,
    api_key: String,
    instance: String,
}

impl EvolutionApiProvider {
    /// Construct from env vars. Returns `None` if `EVOLUTION_API_KEY` is absent.
    pub fn from_env() -> Option<Self> {
        let secrets = crate::infra::secrets::global();
        let api_key = secrets.get("EVOLUTION_API_KEY")?;
        // CO-497: default to a LOCAL Evolution (self-host) — never the public SaaS.
        // A self-hoster who sets only EVOLUTION_API_KEY must not have their WhatsApp
        // silently routed through a third party; set EVOLUTION_API_URL explicitly
        // to target a remote instance.
        let api_url = secrets.get_or("EVOLUTION_API_URL", "http://localhost:8080");
        let instance = secrets.get_or("EVOLUTION_INSTANCE", "default");
        Some(Self {
            api_url,
            api_key,
            instance,
        })
    }
}

#[async_trait]
impl ChannelProvider for EvolutionApiProvider {
    fn name(&self) -> &'static str {
        "whatsapp"
    }

    async fn send(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        payload: &str,
    ) -> Result<(), String> {
        let url = format!(
            "{}/message/sendText/{}",
            self.api_url.trim_end_matches('/'),
            self.instance
        );

        let request_body = serde_json::json!({
            "number": recipient,
            "text": payload,
        });

        let resp = client
            .post(&url)
            .header("apikey", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Evolution API request failed: {e}"))?;

        if resp.status().is_success() {
            info!(recipient = %recipient, "EvolutionApiProvider: WhatsApp message delivered");
            Ok(())
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(recipient = %recipient, status = %status, body = %text, "EvolutionApiProvider: delivery failed");
            Err(format!("Evolution API {status}: {text}"))
        }
    }
}

// ---------------------------------------------------------------------------
// CO-496 — Cloud API at scale: 24h window, template fallback, 429 backoff
// ---------------------------------------------------------------------------
//
// Meta's WhatsApp Cloud API has two scaling constraints the tier-3 review called
// out:
//   1. The 24-hour customer-service window: free-form `text` is only deliverable
//      within 24h of the recipient's last inbound message. Business-initiated
//      messages outside it MUST use a pre-approved **template**.
//   2. Rate limits: the API replies `429` (with a `Retry-After`) when the number
//      is sending faster than its messaging tier (250 → 1k → 10k → 100k/day)
//      allows. Hammering through a 429 gets the number throttled harder.
// These helpers track the window per recipient and compute a polite backoff.

/// The customer-service window: free-form text is deliverable for this long after
/// the recipient's last inbound message. 24h, in seconds.
pub const SERVICE_WINDOW_SECS: u64 = 24 * 60 * 60;

/// Max attempts (initial + retries) for a Cloud API send before giving up on 429.
const MAX_SEND_ATTEMPTS: u32 = 3;
/// Ceiling on any single backoff sleep, so one throttled worker can't park for an
/// unbounded `Retry-After`.
const MAX_BACKOFF_SECS: u64 = 30;

/// Is the 24h customer-service window still open? Pure → unit-testable. Saturating
/// so a clock skew (now < last) is treated as "just received" (open).
pub fn within_service_window(last_inbound_unix: u64, now_unix: u64) -> bool {
    now_unix.saturating_sub(last_inbound_unix) < SERVICE_WINDOW_SECS
}

/// Parse a `Retry-After` header value. Supports the delta-seconds form (the only
/// form Meta emits). Returns `None` for absent/malformed values. Pure → testable.
pub fn parse_retry_after(value: Option<&str>) -> Option<Duration> {
    value
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// The delay before retrying a 429: honor `Retry-After` when present, else
/// exponential backoff (1s, 2s, 4s, …), always capped at [`MAX_BACKOFF_SECS`].
/// Pure → unit-testable.
pub fn backoff_delay(retry_after: Option<Duration>, attempt: u32) -> Duration {
    let secs = match retry_after {
        Some(d) => d.as_secs(),
        None => 1u64 << attempt.min(6), // 1,2,4,…,64 before the cap
    };
    Duration::from_secs(secs.min(MAX_BACKOFF_SECS))
}

/// Build the Cloud API payload for an **approved template** message — the only
/// thing deliverable outside the 24h window. `params` fill the body component's
/// positional `{{1}}`, `{{2}}`, … placeholders. Pure → unit-testable.
pub fn build_template_payload(
    recipient: &str,
    template_name: &str,
    lang_code: &str,
    params: &[String],
) -> Value {
    let components = if params.is_empty() {
        serde_json::json!([])
    } else {
        let parameters: Vec<Value> = params
            .iter()
            .map(|p| serde_json::json!({ "type": "text", "text": p }))
            .collect();
        serde_json::json!([{ "type": "body", "parameters": parameters }])
    };
    serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": recipient,
        "type": "template",
        "template": {
            "name": template_name,
            "language": { "code": lang_code },
            "components": components,
        },
    })
}

/// Build the Cloud API payload for a free-form `text` message — valid only inside
/// the 24h customer-service window. Pure → unit-testable.
pub fn build_text_payload(recipient: &str, text: &str) -> Value {
    serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": recipient,
        "type": "text",
        "text": { "preview_url": false, "body": text },
    })
}

/// CO-496: choose the payload for a **business-initiated** Cloud API message.
/// Inside the window (`window_open`) free-form `text` is deliverable; outside it,
/// only an approved `template` is. Returns `Err` (so the caller logs + aborts)
/// when the window is closed and no template is configured — refusing to send
/// free-form text Meta would reject. Pure → asserts the ACTUAL request shape.
pub fn build_business_payload(
    recipient: &str,
    text: &str,
    window_open: bool,
    template: Option<&OutboundTemplate>,
) -> Result<Value, String> {
    if window_open {
        return Ok(build_text_payload(recipient, text));
    }
    match template {
        Some(t) => Ok(build_template_payload(
            recipient, &t.name, &t.lang, &t.params,
        )),
        None => Err(
            "CO-496: recipient is outside the 24h WhatsApp customer-service window and no \
             approved template is configured — refusing to send free-form text (Meta would \
             reject it). Set the template env var (e.g. CO_WHATSAPP_OTP_TEMPLATE / \
             CO_WHATSAPP_BIRTHDAY_TEMPLATE) to send business-initiated messages."
                .to_string(),
        ),
    }
}

/// Process-wide tracker of each recipient's last inbound-message time (unix secs).
/// Drives the 24h-window decision. In-memory by design: it is a soft heuristic
/// (a stale/empty entry just means "assume the window is closed → prefer a
/// template"), so it need not survive a restart.
fn recipient_windows() -> &'static Mutex<HashMap<String, u64>> {
    static W: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record that `recipient` sent us an inbound message at `now_unix` (opens/refreshes
/// their 24h window). Prunes entries older than the window so the map stays bounded
/// by the count of *recently-active* recipients.
pub fn note_inbound_at(recipient: &str, now_unix: u64) {
    let mut map = recipient_windows()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.retain(|_, &mut t| within_service_window(t, now_unix));
    map.insert(recipient.to_string(), now_unix);
}

/// Is `recipient`'s 24h free-form window currently open? `false` when we have no
/// record of a recent inbound (→ caller should fall back to an approved template).
pub fn is_window_open_at(recipient: &str, now_unix: u64) -> bool {
    let map = recipient_windows()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.get(recipient)
        .map(|&t| within_service_window(t, now_unix))
        .unwrap_or(false)
}

/// Current unix time in seconds.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Public entry used by the inbound webhook to open/refresh a recipient's window.
pub fn note_inbound(recipient: &str) {
    note_inbound_at(recipient, now_unix());
}

/// Public entry: is free-form text deliverable to `recipient` right now?
pub fn is_window_open(recipient: &str) -> bool {
    is_window_open_at(recipient, now_unix())
}

// ---------------------------------------------------------------------------
// CloudApiProvider (CO-479) — official WhatsApp Business Platform (Cloud API)
// ---------------------------------------------------------------------------

/// Sends WhatsApp text via Meta's **official** WhatsApp Cloud API (Graph API).
///
/// This is the enterprise-compliant channel, in contrast to the unofficial
/// [`EvolutionApiProvider`] (Baileys/WhatsApp-Web, against WhatsApp ToS). It
/// requires a verified WhatsApp Business Account + a Graph API access token.
///
/// Requires env vars:
/// - `WHATSAPP_CLOUD_TOKEN`     — System User access token (sent as `Bearer`).
/// - `WHATSAPP_PHONE_NUMBER_ID` — the WABA phone-number id.
///
/// Optional:
/// - `WHATSAPP_GRAPH_VERSION`   — Graph API version (default `v21.0`).
///
/// NOTE: free-form `text` is only valid inside the 24-hour customer-service
/// window. Business-initiated messages outside it must use **approved templates**
/// (tracked as a follow-up).
pub struct CloudApiProvider {
    token: String,
    phone_number_id: String,
    graph_version: String,
    /// Graph API base origin (default `https://graph.facebook.com`). Overridable
    /// via `WHATSAPP_GRAPH_BASE` so the send path can be pointed at a local capture
    /// server in tests (and at a proxy in prod if ever needed).
    graph_base: String,
}

impl CloudApiProvider {
    /// Construct from env. Returns `None` if the token or phone-number id is absent.
    pub fn from_env() -> Option<Self> {
        let secrets = crate::infra::secrets::global();
        let token = secrets.get("WHATSAPP_CLOUD_TOKEN")?;
        let phone_number_id = secrets.get("WHATSAPP_PHONE_NUMBER_ID")?;
        let graph_version = secrets.get_or("WHATSAPP_GRAPH_VERSION", "v21.0");
        let graph_base = secrets.get_or("WHATSAPP_GRAPH_BASE", "https://graph.facebook.com");
        Some(Self {
            token,
            phone_number_id,
            graph_version,
            graph_base,
        })
    }

    /// The Graph API messages endpoint for this WABA phone number.
    pub fn messages_url(&self) -> String {
        format!(
            "{}/{}/{}/messages",
            self.graph_base.trim_end_matches('/'),
            self.graph_version,
            self.phone_number_id
        )
    }

    /// POST an already-built JSON payload to the messages endpoint, honoring
    /// `429 Too Many Requests` with a bounded `Retry-After`/exponential backoff
    /// (CO-496). Shared by free-form [`ChannelProvider::send`] and
    /// [`send_template`](Self::send_template).
    async fn post_payload(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        payload: &Value,
    ) -> Result<(), String> {
        for attempt in 0..MAX_SEND_ATTEMPTS {
            let resp = client
                .post(self.messages_url())
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .json(payload)
                .send()
                .await
                .map_err(|e| format!("WhatsApp Cloud API request failed: {e}"))?;

            let status = resp.status();
            if status.is_success() {
                info!(recipient = %recipient, "CloudApiProvider: WhatsApp message delivered");
                return Ok(());
            }

            // CO-496: honor 429 backoff — Retry-After if present, else capped
            // exponential. Retrying past a 429 without backing off gets the number
            // throttled harder, so this is mandatory at scale.
            if status.as_u16() == 429 && attempt + 1 < MAX_SEND_ATTEMPTS {
                let retry_after = parse_retry_after(
                    resp.headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok()),
                );
                let delay = backoff_delay(retry_after, attempt);
                warn!(
                    recipient = %recipient,
                    attempt,
                    delay_secs = delay.as_secs(),
                    "CloudApiProvider: 429 rate-limited — backing off"
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            let text = resp.text().await.unwrap_or_default();
            warn!(recipient = %recipient, status = %status, body = %text, "CloudApiProvider: delivery failed");
            return Err(format!("WhatsApp Cloud API {status}: {text}"));
        }
        Err("WhatsApp Cloud API: exhausted retries after repeated 429".to_string())
    }

    /// Send an **approved template** message — the channel for business-initiated
    /// messages outside the 24h customer-service window (CO-496). `params` fill the
    /// template body's positional placeholders. Same 429 backoff as `send`.
    pub async fn send_template(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        template_name: &str,
        lang_code: &str,
        params: &[String],
    ) -> Result<(), String> {
        let payload = build_template_payload(recipient, template_name, lang_code, params);
        self.post_payload(client, recipient, &payload).await
    }

    /// CO-496: send a business-initiated message respecting the live 24h window for
    /// `recipient`. Inside the window → free-form text; outside → the approved
    /// `template` (or an actionable error when none is configured). This is the
    /// production wiring of [`is_window_open`] + [`build_template_payload`].
    pub async fn send_business_initiated_now(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        text: &str,
        template: Option<&OutboundTemplate>,
    ) -> Result<(), String> {
        let window_open = is_window_open(recipient);
        let payload =
            build_business_payload(recipient, text, window_open, template).inspect_err(|e| {
                warn!(recipient = %recipient, "CloudApiProvider: {e}");
            })?;
        self.post_payload(client, recipient, &payload).await
    }
}

#[async_trait]
impl ChannelProvider for CloudApiProvider {
    fn name(&self) -> &'static str {
        "whatsapp"
    }

    async fn send(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        payload: &str,
    ) -> Result<(), String> {
        // Free-form text — valid only inside the 24h window (CO-496). Callers
        // sending business-initiated messages outside it must use
        // `send_business_initiated` (which falls back to an approved template).
        let request_body = build_text_payload(recipient, payload);
        self.post_payload(client, recipient, &request_body).await
    }

    /// CO-496: business-initiated override — outside the 24h window, fall back to an
    /// approved template instead of free-form text Meta would reject.
    async fn send_business_initiated(
        &self,
        client: &reqwest::Client,
        recipient: &str,
        text: &str,
        template: Option<&OutboundTemplate>,
    ) -> Result<(), String> {
        self.send_business_initiated_now(client, recipient, text, template)
            .await
    }
}

// ---------------------------------------------------------------------------
// Payload encoding helpers (used by emit_event)
// ---------------------------------------------------------------------------

/// Encode an email notification payload as `subject\n---\nbody`.
pub fn encode_email_payload(subject: &str, body: &str) -> String {
    format!("{subject}\n---\n{body}")
}

/// CO-497: the WhatsApp send cascade — **the self-host/managed invariant in one
/// place**. Prefer the official Cloud API (managed/public deploys with a Meta app
/// and token), else fall back to Evolution (self-host: QR-links your own WhatsApp,
/// outbound-only, no Meta app and no public webhook — survives residential CGNAT).
/// `None` ⇒ no channel configured (callers log/no-op). Every WhatsApp send (OTP,
/// reply, greeting) goes through this so a self-hosted deploy with only Evolution
/// works identically to a managed Cloud deploy.
pub fn whatsapp_provider_cascade() -> Option<Box<dyn ChannelProvider>> {
    CloudApiProvider::from_env()
        .map(|p| Box::new(p) as Box<dyn ChannelProvider>)
        .or_else(|| {
            EvolutionApiProvider::from_env().map(|p| Box::new(p) as Box<dyn ChannelProvider>)
        })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // --- CO-479: WhatsApp Cloud API provider ---

    #[test]
    fn cloud_api_messages_url_uses_version_and_phone_id() {
        let p = CloudApiProvider {
            token: "tok".into(),
            phone_number_id: "123456".into(),
            graph_version: "v21.0".into(),
            graph_base: "https://graph.facebook.com".into(),
        };
        assert_eq!(
            p.messages_url(),
            "https://graph.facebook.com/v21.0/123456/messages"
        );
        // Same channel name as Evolution → it is a drop-in compliant replacement.
        assert_eq!(p.name(), "whatsapp");
    }

    // --- CO-496: Cloud API at scale ---

    #[test]
    fn within_service_window_boundary() {
        // Just received → open. 23h59m → open. Exactly 24h / beyond → closed.
        assert!(within_service_window(1000, 1000));
        assert!(within_service_window(1000, 1000 + SERVICE_WINDOW_SECS - 1));
        assert!(!within_service_window(1000, 1000 + SERVICE_WINDOW_SECS));
        assert!(!within_service_window(
            1000,
            1000 + SERVICE_WINDOW_SECS + 100
        ));
        // Clock skew (now < last) is treated as open (saturating).
        assert!(within_service_window(2000, 1000));
    }

    #[test]
    fn parse_retry_after_handles_seconds_and_garbage() {
        assert_eq!(parse_retry_after(Some("5")), Some(Duration::from_secs(5)));
        assert_eq!(
            parse_retry_after(Some("  12 ")),
            Some(Duration::from_secs(12))
        );
        assert_eq!(parse_retry_after(None), None);
        assert_eq!(parse_retry_after(Some("")), None);
        assert_eq!(parse_retry_after(Some("soon")), None); // HTTP-date form unsupported → None
    }

    #[test]
    fn backoff_delay_honors_retry_after_then_exponential_capped() {
        // Retry-After wins, capped at MAX_BACKOFF_SECS.
        assert_eq!(
            backoff_delay(Some(Duration::from_secs(7)), 0),
            Duration::from_secs(7)
        );
        assert_eq!(
            backoff_delay(Some(Duration::from_secs(900)), 0),
            Duration::from_secs(MAX_BACKOFF_SECS)
        );
        // No header → exponential 1,2,4… capped.
        assert_eq!(backoff_delay(None, 0), Duration::from_secs(1));
        assert_eq!(backoff_delay(None, 2), Duration::from_secs(4));
        assert_eq!(
            backoff_delay(None, 20),
            Duration::from_secs(MAX_BACKOFF_SECS)
        );
    }

    #[test]
    fn build_template_payload_with_and_without_params() {
        let p = build_template_payload("5541999999999", "lembrete", "pt_BR", &["Festa".into()]);
        assert_eq!(p["type"], "template");
        assert_eq!(p["to"], "5541999999999");
        assert_eq!(p["template"]["name"], "lembrete");
        assert_eq!(p["template"]["language"]["code"], "pt_BR");
        assert_eq!(p["template"]["components"][0]["type"], "body");
        assert_eq!(
            p["template"]["components"][0]["parameters"][0]["text"],
            "Festa"
        );

        // No params → empty components array (a template with no variables).
        let p0 = build_template_payload("55", "ola", "pt_BR", &[]);
        assert_eq!(p0["template"]["components"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn window_tracker_opens_and_expires() {
        let r = "co496-test-recipient-unique";
        // No record yet → closed (prefer template).
        assert!(!is_window_open_at(r, 100_000));
        // Inbound opens it.
        note_inbound_at(r, 100_000);
        assert!(is_window_open_at(r, 100_000 + 10));
        // Past 24h → closed again.
        assert!(!is_window_open_at(r, 100_000 + SERVICE_WINDOW_SECS + 1));
    }

    // --- CO-496: business-initiated window/template fallback ---

    #[test]
    fn business_payload_is_text_inside_window_template_outside() {
        let tpl = OutboundTemplate {
            name: "lembrete".into(),
            lang: "pt_BR".into(),
            params: vec!["Yuri".into()],
        };

        // Inside the window → free-form TEXT (the actual wire shape).
        let inside = build_business_payload("55", "oi", true, Some(&tpl)).unwrap();
        assert_eq!(inside["type"], "text");
        assert_eq!(inside["text"]["body"], "oi");

        // Outside the window → the approved TEMPLATE, never free-form text.
        let outside = build_business_payload("55", "oi", false, Some(&tpl)).unwrap();
        assert_eq!(outside["type"], "template");
        assert_eq!(outside["template"]["name"], "lembrete");
        assert_eq!(
            outside["template"]["components"][0]["parameters"][0]["text"],
            "Yuri"
        );
    }

    #[test]
    fn business_payload_errors_outside_window_without_template() {
        // The actionable-error path: no template configured + window closed → refuse
        // (do NOT silently build free-form text that Meta rejects).
        let err = build_business_payload("55", "oi", false, None).unwrap_err();
        assert!(
            err.contains("template"),
            "error must name the missing config"
        );
    }

    /// CONTRACT/INTEGRATION (CO-496): drive the REAL `CloudApiProvider` send path
    /// against a local capture server and assert the bytes on the wire are a
    /// `template` when the recipient is outside the 24h window. This catches a
    /// regression to free-form text — the exact prod-rejection the fix prevents.
    #[tokio::test]
    async fn cloud_api_business_initiated_puts_template_on_the_wire_outside_window() {
        use axum::{Json, Router, extract::State, routing::post};
        use std::sync::Arc as StdArc;

        let captured: StdArc<Mutex<Option<Value>>> = StdArc::new(Mutex::new(None));
        let app = Router::new()
            .route(
                "/{*rest}",
                post(
                    |State(c): State<StdArc<Mutex<Option<Value>>>>, Json(body): Json<Value>| async move {
                        *c.lock().unwrap() = Some(body);
                        Json(serde_json::json!({"messages":[{"id":"wamid.OK"}]}))
                    },
                ),
            )
            .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let provider = CloudApiProvider {
            token: "tok".into(),
            phone_number_id: "PN".into(),
            graph_version: "v21.0".into(),
            graph_base: format!("http://{addr}"),
        };
        let tpl = OutboundTemplate {
            name: "verificacao".into(),
            lang: "pt_BR".into(),
            params: vec!["123456".into()],
        };
        // A fresh recipient we NEVER note_inbound for → window closed.
        let recipient = "5541900000496";
        provider
            .send_business_initiated_now(
                shared_client_for_test(),
                recipient,
                "free-form would be rejected",
                Some(&tpl),
            )
            .await
            .unwrap();

        let body = captured
            .lock()
            .unwrap()
            .clone()
            .expect("server got a request");
        assert_eq!(
            body["type"], "template",
            "outside the 24h window the wire payload MUST be a template, not free-form text"
        );
        assert_eq!(body["template"]["name"], "verificacao");
        assert_eq!(body["to"], recipient);
    }

    /// Companion to the above: inside the window the SAME path puts free-form text
    /// on the wire (so the template fallback is genuinely window-gated, not always-on).
    #[tokio::test]
    async fn cloud_api_business_initiated_puts_text_on_the_wire_inside_window() {
        use axum::{Json, Router, extract::State, routing::post};
        use std::sync::Arc as StdArc;

        let captured: StdArc<Mutex<Option<Value>>> = StdArc::new(Mutex::new(None));
        let app = Router::new()
            .route(
                "/{*rest}",
                post(
                    |State(c): State<StdArc<Mutex<Option<Value>>>>, Json(body): Json<Value>| async move {
                        *c.lock().unwrap() = Some(body);
                        Json(serde_json::json!({"messages":[{"id":"wamid.OK"}]}))
                    },
                ),
            )
            .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let provider = CloudApiProvider {
            token: "tok".into(),
            phone_number_id: "PN".into(),
            graph_version: "v21.0".into(),
            graph_base: format!("http://{addr}"),
        };
        let recipient = "5541900000497";
        // Open the window for this recipient → free-form is allowed.
        note_inbound(recipient);
        provider
            .send_business_initiated_now(shared_client_for_test(), recipient, "olá!", None)
            .await
            .unwrap();

        let body = captured
            .lock()
            .unwrap()
            .clone()
            .expect("server got a request");
        assert_eq!(body["type"], "text");
        assert_eq!(body["text"]["body"], "olá!");
    }

    /// A process-wide reqwest client for the integration tests above.
    fn shared_client_for_test() -> &'static reqwest::Client {
        static C: OnceLock<reqwest::Client> = OnceLock::new();
        C.get_or_init(reqwest::Client::new)
    }

    // --- template rendering ---

    #[test]
    fn render_template_substitutes_string_fields() {
        let tpl = "Olá {{nome}}, evento: {{titulo}}";
        let payload = serde_json::json!({"nome": "Yuri", "titulo": "Festa"});
        let result = render_template(tpl, &payload);
        assert_eq!(result, "Olá Yuri, evento: Festa");
    }

    #[test]
    fn render_template_substitutes_number_fields() {
        let tpl = "ID: {{id}}";
        let payload = serde_json::json!({"id": 42});
        let result = render_template(tpl, &payload);
        assert_eq!(result, "ID: 42");
    }

    #[test]
    fn render_template_leaves_unknown_placeholders() {
        let tpl = "Hello {{name}} {{unknown}}";
        let payload = serde_json::json!({"name": "World"});
        let result = render_template(tpl, &payload);
        assert_eq!(result, "Hello World {{unknown}}");
    }

    #[test]
    fn event_type_to_prefix_converts_dots_to_underscores() {
        assert_eq!(
            event_type_to_prefix("quilombo.evento.criado"),
            "QUILOMBO_EVENTO_CRIADO"
        );
        assert_eq!(
            event_type_to_prefix("co.universe.criado"),
            "CO_UNIVERSE_CRIADO"
        );
    }

    #[test]
    fn get_template_returns_default_for_evento_criado_email_subject() {
        let tpl = get_template("quilombo.evento.criado", "EMAIL_SUBJECT");
        assert!(tpl.is_some());
        assert!(tpl.unwrap().contains("{{titulo}}"));
    }

    #[test]
    fn get_template_returns_default_for_evento_criado_email_body() {
        let tpl = get_template("quilombo.evento.criado", "EMAIL_BODY");
        assert!(tpl.is_some());
        let body = tpl.unwrap();
        assert!(body.contains("{{nome}}"));
        assert!(body.contains("{{titulo}}"));
    }

    #[test]
    fn get_template_returns_default_for_evento_criado_whatsapp() {
        let tpl = get_template("quilombo.evento.criado", "WHATSAPP");
        assert!(tpl.is_some());
        assert!(tpl.unwrap().contains("{{titulo}}"));
    }

    #[test]
    fn get_template_returns_none_for_unknown_event() {
        let tpl = get_template("unknown.event", "EMAIL_SUBJECT");
        assert!(tpl.is_none());
    }

    // --- env var override for templates ---

    // CO-477: env-var-dependent tests serialize on the single process-wide lock
    // in crate::test_support (acquired in each test below), excluding env-mutating
    // tests in other modules too — a per-file mutex did not.
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
    fn get_template_reads_env_var_override() {
        let _env = crate::test_support::env_lock_blocking();
        let _guard = EnvGuard::set(
            "CO_TPL_QUILOMBO_EVENTO_CRIADO_EMAIL_SUBJECT",
            "Override: {{titulo}}",
        );
        let tpl = get_template("quilombo.evento.criado", "EMAIL_SUBJECT");
        assert_eq!(tpl.unwrap(), "Override: {{titulo}}");
    }

    // --- ResendProvider ---

    #[test]
    fn resend_provider_from_env_returns_none_without_api_key() {
        let _env = crate::test_support::env_lock_blocking();
        let _guard = EnvGuard::remove("RESEND_API_KEY");
        let provider = ResendProvider::from_env();
        assert!(provider.is_none());
    }

    #[test]
    fn resend_provider_name_is_email() {
        let provider = ResendProvider {
            api_key: "test".into(),
            from: "test@example.com".into(),
        };
        assert_eq!(provider.name(), "email");
    }

    // --- EvolutionApiProvider ---

    #[test]
    fn evolution_provider_from_env_returns_none_without_api_key() {
        let _env = crate::test_support::env_lock_blocking();
        let _guard = EnvGuard::remove("EVOLUTION_API_KEY");
        let provider = EvolutionApiProvider::from_env();
        assert!(provider.is_none());
    }

    #[test]
    fn evolution_provider_name_is_whatsapp() {
        let provider = EvolutionApiProvider {
            api_key: "test".into(),
            api_url: "https://api.example.com".into(),
            instance: "default".into(),
        };
        assert_eq!(provider.name(), "whatsapp");
    }

    // --- MockChannelProvider (used in dispatch tests) ---

    struct MockChannelProvider {
        channel_name: &'static str,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl MockChannelProvider {
        fn new(channel_name: &'static str) -> (Self, Arc<Mutex<Vec<(String, String)>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let provider = Self {
                channel_name,
                calls: calls.clone(),
            };
            (provider, calls)
        }
    }

    #[async_trait]
    impl ChannelProvider for MockChannelProvider {
        fn name(&self) -> &'static str {
            self.channel_name
        }

        async fn send(
            &self,
            _client: &reqwest::Client,
            recipient: &str,
            payload: &str,
        ) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push((recipient.to_string(), payload.to_string()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn mock_provider_records_send_calls() {
        let (provider, calls) = MockChannelProvider::new("email");
        let client = reqwest::Client::new();
        provider
            .send(&client, "test@example.com", "subject\n---\nbody")
            .await
            .unwrap();
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "test@example.com");
    }

    // --- encode_email_payload ---

    #[test]
    fn encode_email_payload_format() {
        let encoded = encode_email_payload("Subject here", "Body here");
        assert_eq!(encoded, "Subject here\n---\nBody here");
    }

    #[test]
    fn render_full_email_template_for_evento() {
        let payload = serde_json::json!({
            "titulo": "Festa Julina",
            "nome": "Yuri",
        });
        let subject_tpl = get_template("quilombo.evento.criado", "EMAIL_SUBJECT").unwrap();
        let body_tpl = get_template("quilombo.evento.criado", "EMAIL_BODY").unwrap();
        let subject = render_template(&subject_tpl, &payload);
        let body = render_template(&body_tpl, &payload);
        assert!(subject.contains("Festa Julina"));
        assert!(body.contains("Yuri"));
        assert!(body.contains("Festa Julina"));
    }
}
