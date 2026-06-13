//! CO-366: Conversion + payment wiring — register → paid.
//!
//! The integration choice (Hostinger today, Pix / Stripe tomorrow) lives behind
//! the [`BillingProvider`] trait so we don't lock in. The active provider is
//! selected at request time from `CO_BILLING_PROVIDER` (default `manual`), so
//! swapping providers is an env change, not a redeploy of code.
//!
//! | Impl                    | Status            | File           |
//! |-------------------------|-------------------|----------------|
//! | [`ManualInvoiceProvider`] | full (default)  | `manual.rs`    |
//! | [`HostingerProvider`]   | full (first ship) | `hostinger.rs` |
//! | `PixProvider`           | stub (feature `billing-pix`)    | `pix.rs`    |
//! | `StripeProvider`        | stub (feature `billing-stripe`) | `stripe.rs` |

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::infra::secrets::SecretsProvider;

pub mod hostinger;
pub mod manual;
pub mod routes;
pub mod storage;

#[cfg(feature = "billing-pix")]
pub mod pix;
#[cfg(feature = "billing-stripe")]
pub mod stripe;

pub use hostinger::HostingerProvider;
pub use manual::ManualInvoiceProvider;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    /// Webhook HMAC signature did not match — likely a forged payload.
    #[error("invalid webhook signature")]
    InvalidSignature,
    /// The provider is missing required configuration (API key / secret).
    #[error("billing not configured: {0}")]
    NotConfigured(String),
    /// The operation is not supported by this provider (e.g. a manual provider
    /// has no webhook to verify).
    #[error("unsupported by provider: {0}")]
    Unsupported(String),
    /// The webhook payload was malformed or referenced an unknown event.
    #[error("invalid payload: {0}")]
    InvalidPayload(String),
    /// Any other provider-side failure.
    #[error("provider error: {0}")]
    Provider(String),
}

pub type Result<T> = std::result::Result<T, BillingError>;

// ---------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------

/// Billing plan. Plan *prices* are config (see [`PlanCatalog`]); the set of
/// plan identifiers is fixed in code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    Free,
    Starter,
    Pro,
    Enterprise,
}

impl Plan {
    pub fn as_str(self) -> &'static str {
        match self {
            Plan::Free => "free",
            Plan::Starter => "starter",
            Plan::Pro => "pro",
            Plan::Enterprise => "enterprise",
        }
    }

    /// Parse a plan id; unknown / empty values are rejected.
    pub fn parse(s: &str) -> Option<Plan> {
        match s.trim().to_ascii_lowercase().as_str() {
            "free" => Some(Plan::Free),
            "starter" => Some(Plan::Starter),
            "pro" => Some(Plan::Pro),
            "enterprise" => Some(Plan::Enterprise),
            _ => None,
        }
    }

    /// Default monthly price in cents (BRL). Overridable via
    /// `CO_BILLING_PLAN_PRICES_JSON` (see [`PlanCatalog`]).
    pub fn default_price_cents(self) -> u64 {
        match self {
            Plan::Free => 0,
            Plan::Starter => 900,
            Plan::Pro => 2900,
            Plan::Enterprise => 9900,
        }
    }
}

/// Plan → price map, seeded from defaults and optionally overridden by the
/// `CO_BILLING_PLAN_PRICES_JSON` env var (e.g. `{"starter":1200,"pro":3500}`).
/// Changing prices is therefore an env change, not a code change.
#[derive(Debug, Clone)]
pub struct PlanCatalog {
    starter: u64,
    pro: u64,
    enterprise: u64,
}

impl PlanCatalog {
    pub fn from_secrets(secrets: &dyn SecretsProvider) -> PlanCatalog {
        let mut catalog = PlanCatalog {
            starter: Plan::Starter.default_price_cents(),
            pro: Plan::Pro.default_price_cents(),
            enterprise: Plan::Enterprise.default_price_cents(),
        };
        if let Some(json) = secrets.get_nonempty("CO_BILLING_PLAN_PRICES_JSON")
            && let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, u64>>(&json)
        {
            if let Some(v) = map.get("starter") {
                catalog.starter = *v;
            }
            if let Some(v) = map.get("pro") {
                catalog.pro = *v;
            }
            if let Some(v) = map.get("enterprise") {
                catalog.enterprise = *v;
            }
        }
        catalog
    }

    pub fn price_cents(&self, plan: Plan) -> u64 {
        match plan {
            Plan::Free => 0,
            Plan::Starter => self.starter,
            Plan::Pro => self.pro,
            Plan::Enterprise => self.enterprise,
        }
    }
}

// ---------------------------------------------------------------------------
// Checkout + webhook value types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CheckoutSession {
    pub url: String,
    pub session_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEvent {
    PaymentSucceeded {
        user_id: String,
        plan: Plan,
        amount_cents: u64,
    },
    PaymentFailed {
        user_id: String,
        reason: String,
    },
    SubscriptionCanceled {
        user_id: String,
    },
}

impl WebhookEvent {
    /// The `billing_events.event_type` string this event records.
    pub fn event_type(&self) -> &'static str {
        match self {
            WebhookEvent::PaymentSucceeded { .. } => "payment_succeeded",
            WebhookEvent::PaymentFailed { .. } => "payment_failed",
            WebhookEvent::SubscriptionCanceled { .. } => "subscription_canceled",
        }
    }

    pub fn user_id(&self) -> &str {
        match self {
            WebhookEvent::PaymentSucceeded { user_id, .. }
            | WebhookEvent::PaymentFailed { user_id, .. }
            | WebhookEvent::SubscriptionCanceled { user_id } => user_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait BillingProvider: Send + Sync {
    /// Create a checkout session for a user; returns the URL to redirect to.
    async fn create_checkout(&self, user_id: &str, plan: Plan) -> Result<CheckoutSession>;

    /// Verify a webhook payload signature and decode it into a [`WebhookEvent`].
    fn verify_webhook(&self, payload: &[u8], signature: &str) -> Result<WebhookEvent>;

    /// Provider name for telemetry and the `billing_*` columns.
    fn name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Provider factory
// ---------------------------------------------------------------------------

/// Build the active provider for the given name. Used by the webhook route to
/// pick the provider named in the path (`/billing/webhook/{provider}`), and by
/// [`active_provider`] for the configured default.
pub fn provider_by_name(
    name: &str,
    secrets: &dyn SecretsProvider,
) -> Option<Box<dyn BillingProvider>> {
    match name {
        "hostinger" => Some(Box::new(HostingerProvider::from_secrets(secrets))),
        "manual" => Some(Box::new(ManualInvoiceProvider::from_secrets(secrets))),
        #[cfg(feature = "billing-pix")]
        "pix" => Some(Box::new(pix::PixProvider::from_secrets(secrets))),
        #[cfg(feature = "billing-stripe")]
        "stripe" => Some(Box::new(stripe::StripeProvider::from_secrets(secrets))),
        _ => None,
    }
}

/// The configured default provider (`CO_BILLING_PROVIDER`, default `manual`).
pub fn active_provider(secrets: &dyn SecretsProvider) -> Box<dyn BillingProvider> {
    let name = secrets.get_or("CO_BILLING_PROVIDER", "manual");
    provider_by_name(&name, secrets)
        .unwrap_or_else(|| Box::new(ManualInvoiceProvider::from_secrets(secrets)))
}

// ---------------------------------------------------------------------------
// Shared crypto helpers (HMAC-SHA256 hex + constant-time compare)
// ---------------------------------------------------------------------------

/// Hex-encoded HMAC-SHA256 of `payload` keyed by `secret`.
pub(crate) fn hmac_sha256_hex(secret: &str, payload: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(payload);
    let bytes = mac.finalize().into_bytes();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Length-independent constant-time byte comparison — avoids leaking the
/// position of the first differing byte via timing.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::secrets::StaticSecretsProvider;

    #[test]
    fn plan_parse_roundtrip() {
        for p in [Plan::Free, Plan::Starter, Plan::Pro, Plan::Enterprise] {
            assert_eq!(Plan::parse(p.as_str()), Some(p));
        }
        assert_eq!(Plan::parse("STARTER"), Some(Plan::Starter));
        assert_eq!(Plan::parse(" pro "), Some(Plan::Pro));
        assert_eq!(Plan::parse("nope"), None);
        assert_eq!(Plan::parse(""), None);
    }

    #[test]
    fn catalog_uses_defaults_then_overrides() {
        let empty = StaticSecretsProvider::new(Vec::<(String, String)>::new());
        let cat = PlanCatalog::from_secrets(empty.as_ref());
        assert_eq!(cat.price_cents(Plan::Starter), 900);
        assert_eq!(cat.price_cents(Plan::Pro), 2900);
        assert_eq!(cat.price_cents(Plan::Free), 0);

        let overridden = StaticSecretsProvider::new([(
            "CO_BILLING_PLAN_PRICES_JSON",
            r#"{"starter":1200,"pro":3500}"#,
        )]);
        let cat = PlanCatalog::from_secrets(overridden.as_ref());
        assert_eq!(cat.price_cents(Plan::Starter), 1200);
        assert_eq!(cat.price_cents(Plan::Pro), 3500);
        // not overridden → default
        assert_eq!(cat.price_cents(Plan::Enterprise), 9900);
    }

    #[test]
    fn active_provider_defaults_to_manual() {
        let empty = StaticSecretsProvider::new(Vec::<(String, String)>::new());
        assert_eq!(active_provider(empty.as_ref()).name(), "manual");

        let hostinger = StaticSecretsProvider::new([("CO_BILLING_PROVIDER", "hostinger")]);
        assert_eq!(active_provider(hostinger.as_ref()).name(), "hostinger");

        // Unknown provider falls back to manual rather than panicking.
        let bogus = StaticSecretsProvider::new([("CO_BILLING_PROVIDER", "bogus")]);
        assert_eq!(active_provider(bogus.as_ref()).name(), "manual");
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn hmac_is_deterministic_and_keyed() {
        let a = hmac_sha256_hex("secret", b"payload");
        let b = hmac_sha256_hex("secret", b"payload");
        let c = hmac_sha256_hex("other", b"payload");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64); // 32 bytes hex
    }
}
