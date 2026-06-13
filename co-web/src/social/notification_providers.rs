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
        let api_url = secrets.get_or("EVOLUTION_API_URL", "https://api.evolution-api.com");
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
// Payload encoding helpers (used by emit_event)
// ---------------------------------------------------------------------------

/// Encode an email notification payload as `subject\n---\nbody`.
pub fn encode_email_payload(subject: &str, body: &str) -> String {
    format!("{subject}\n---\n{body}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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

    /// Mutex to serialize env-var-dependent tests.
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        key: String,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: test-only, guarded by ENV_MUTEX
            unsafe { std::env::set_var(key, value) };
            Self {
                key: key.to_string(),
                original,
            }
        }

        fn remove(key: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: test-only, guarded by ENV_MUTEX
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
        let _lock = ENV_MUTEX.lock().unwrap();
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
        let _lock = ENV_MUTEX.lock().unwrap();
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
        let _lock = ENV_MUTEX.lock().unwrap();
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
