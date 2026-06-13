//! CO-366: Hostinger billing provider (first ship).
//!
//! Hostinger offers a quick payment integration. For the MVP, `create_checkout`
//! builds a deterministic checkout URL embedding the user reference and plan
//! (no outbound API call — keeps the hot path and tests free of network), and
//! `verify_webhook` authenticates the inbound webhook with an HMAC-SHA256
//! signature over the raw body keyed by `CO_BILLING_HOSTINGER_WEBHOOK_SECRET`.
//!
//! Activate with `CO_BILLING_PROVIDER=hostinger`. Required secrets:
//! `CO_BILLING_HOSTINGER_API_KEY` and `CO_BILLING_HOSTINGER_WEBHOOK_SECRET`.
//! Optional: `CO_BILLING_HOSTINGER_CHECKOUT_BASE`
//! (default `https://www.hostinger.com/checkout`).

use async_trait::async_trait;
use chrono::{Duration, Utc};

use super::{
    BillingError, BillingProvider, CheckoutSession, Plan, Result, WebhookEvent, constant_time_eq,
    hmac_sha256_hex,
};
use crate::infra::secrets::SecretsProvider;

const DEFAULT_CHECKOUT_BASE: &str = "https://www.hostinger.com/checkout";
/// Checkout sessions are valid for 30 minutes.
const CHECKOUT_TTL_MINUTES: i64 = 30;

pub struct HostingerProvider {
    api_key: Option<String>,
    webhook_secret: Option<String>,
    checkout_base: String,
}

impl HostingerProvider {
    pub fn from_secrets(secrets: &dyn SecretsProvider) -> HostingerProvider {
        HostingerProvider {
            api_key: secrets.get_nonempty("CO_BILLING_HOSTINGER_API_KEY"),
            webhook_secret: secrets.get_nonempty("CO_BILLING_HOSTINGER_WEBHOOK_SECRET"),
            checkout_base: secrets
                .get_nonempty("CO_BILLING_HOSTINGER_CHECKOUT_BASE")
                .unwrap_or_else(|| DEFAULT_CHECKOUT_BASE.to_string()),
        }
    }
}

#[async_trait]
impl BillingProvider for HostingerProvider {
    async fn create_checkout(&self, user_id: &str, plan: Plan) -> Result<CheckoutSession> {
        // The API key gates checkout creation; without it the integration is
        // not configured.
        if self.api_key.is_none() {
            return Err(BillingError::NotConfigured(
                "CO_BILLING_HOSTINGER_API_KEY".into(),
            ));
        }
        if plan == Plan::Free {
            return Err(BillingError::Unsupported(
                "free plan has no checkout".into(),
            ));
        }
        let session_id = format!("hs_{}", nanoid::nanoid!(16));
        let url = format!(
            "{base}?plan={plan}&client_reference_id={user}&session={session}",
            base = self.checkout_base,
            plan = plan.as_str(),
            user = user_id,
            session = session_id,
        );
        Ok(CheckoutSession {
            url,
            session_id,
            expires_at: Utc::now() + Duration::minutes(CHECKOUT_TTL_MINUTES),
        })
    }

    fn verify_webhook(&self, payload: &[u8], signature: &str) -> Result<WebhookEvent> {
        let secret = self.webhook_secret.as_deref().ok_or_else(|| {
            BillingError::NotConfigured("CO_BILLING_HOSTINGER_WEBHOOK_SECRET".into())
        })?;
        let expected = hmac_sha256_hex(secret, payload);
        if !constant_time_eq(expected.as_bytes(), signature.trim().as_bytes()) {
            return Err(BillingError::InvalidSignature);
        }

        let v: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| BillingError::InvalidPayload(format!("not JSON: {e}")))?;

        let user_id = v
            .get("user_id")
            .or_else(|| v.get("client_reference_id"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| BillingError::InvalidPayload("missing user_id".into()))?
            .to_string();
        let event = v
            .get("event")
            .and_then(|x| x.as_str())
            .ok_or_else(|| BillingError::InvalidPayload("missing event".into()))?;

        match event {
            "payment_succeeded" | "payment.succeeded" => {
                let plan = v
                    .get("plan")
                    .and_then(|x| x.as_str())
                    .and_then(Plan::parse)
                    .ok_or_else(|| BillingError::InvalidPayload("missing/unknown plan".into()))?;
                let amount_cents = v.get("amount_cents").and_then(|x| x.as_u64()).unwrap_or(0);
                Ok(WebhookEvent::PaymentSucceeded {
                    user_id,
                    plan,
                    amount_cents,
                })
            }
            "payment_failed" | "payment.failed" => {
                let reason = v
                    .get("reason")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                Ok(WebhookEvent::PaymentFailed { user_id, reason })
            }
            "subscription_canceled" | "subscription.canceled" => {
                Ok(WebhookEvent::SubscriptionCanceled { user_id })
            }
            other => Err(BillingError::InvalidPayload(format!(
                "unknown event '{other}'"
            ))),
        }
    }

    fn name(&self) -> &'static str {
        "hostinger"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billing::hmac_sha256_hex;
    use crate::infra::secrets::StaticSecretsProvider;

    fn provider() -> HostingerProvider {
        HostingerProvider::from_secrets(
            StaticSecretsProvider::new([
                ("CO_BILLING_HOSTINGER_API_KEY", "key-123"),
                ("CO_BILLING_HOSTINGER_WEBHOOK_SECRET", "whsec"),
            ])
            .as_ref(),
        )
    }

    #[tokio::test]
    async fn checkout_returns_url_with_plan_and_user() {
        let p = provider();
        let s = p.create_checkout("usr_abc", Plan::Starter).await.unwrap();
        assert!(s.url.contains("plan=starter"));
        assert!(s.url.contains("client_reference_id=usr_abc"));
        assert!(s.url.starts_with("https://www.hostinger.com/checkout"));
        assert!(!s.session_id.is_empty());
        assert!(s.expires_at > Utc::now());
    }

    #[tokio::test]
    async fn checkout_requires_api_key() {
        let p = HostingerProvider::from_secrets(
            StaticSecretsProvider::new(Vec::<(String, String)>::new()).as_ref(),
        );
        let err = p.create_checkout("usr_abc", Plan::Pro).await.unwrap_err();
        assert!(matches!(err, BillingError::NotConfigured(_)));
    }

    #[test]
    fn webhook_accepts_valid_signature_and_decodes_payment() {
        let p = provider();
        let body =
            br#"{"event":"payment_succeeded","user_id":"usr_x","plan":"pro","amount_cents":2900}"#;
        let sig = hmac_sha256_hex("whsec", body);
        let ev = p.verify_webhook(body, &sig).unwrap();
        assert_eq!(
            ev,
            WebhookEvent::PaymentSucceeded {
                user_id: "usr_x".into(),
                plan: Plan::Pro,
                amount_cents: 2900,
            }
        );
    }

    #[test]
    fn webhook_rejects_forged_signature() {
        let p = provider();
        let body = br#"{"event":"payment_succeeded","user_id":"usr_x","plan":"pro"}"#;
        let err = p.verify_webhook(body, "deadbeef").unwrap_err();
        assert!(matches!(err, BillingError::InvalidSignature));
    }

    #[test]
    fn webhook_rejects_tampered_payload() {
        let p = provider();
        let signed = br#"{"event":"payment_succeeded","user_id":"usr_x","plan":"pro"}"#;
        let sig = hmac_sha256_hex("whsec", signed);
        // Same signature, different body → must fail.
        let tampered =
            br#"{"event":"payment_succeeded","user_id":"usr_attacker","plan":"enterprise"}"#;
        let err = p.verify_webhook(tampered, &sig).unwrap_err();
        assert!(matches!(err, BillingError::InvalidSignature));
    }

    #[test]
    fn webhook_decodes_failed_and_canceled() {
        let p = provider();
        let body = br#"{"event":"payment_failed","user_id":"usr_x","reason":"card_declined"}"#;
        let sig = hmac_sha256_hex("whsec", body);
        assert_eq!(
            p.verify_webhook(body, &sig).unwrap(),
            WebhookEvent::PaymentFailed {
                user_id: "usr_x".into(),
                reason: "card_declined".into(),
            }
        );

        let body = br#"{"event":"subscription_canceled","user_id":"usr_x"}"#;
        let sig = hmac_sha256_hex("whsec", body);
        assert_eq!(
            p.verify_webhook(body, &sig).unwrap(),
            WebhookEvent::SubscriptionCanceled {
                user_id: "usr_x".into(),
            }
        );
    }
}
