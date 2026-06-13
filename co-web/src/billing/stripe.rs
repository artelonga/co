//! CO-366: Stripe billing provider — **stub** (feature `billing-stripe`, v3.1 target).
//!
//! Stripe covers international cards. The full implementation (Checkout Session
//! API + signed webhook handling) lands in v3.1; this stub keeps the trait
//! wired so the provider factory and feature flag compile.

use async_trait::async_trait;

use super::{BillingError, BillingProvider, CheckoutSession, Plan, Result, WebhookEvent};
use crate::infra::secrets::SecretsProvider;

pub struct StripeProvider;

impl StripeProvider {
    pub fn from_secrets(_secrets: &dyn SecretsProvider) -> StripeProvider {
        StripeProvider
    }
}

#[async_trait]
impl BillingProvider for StripeProvider {
    async fn create_checkout(&self, _user_id: &str, _plan: Plan) -> Result<CheckoutSession> {
        Err(BillingError::Unsupported(
            "stripe provider is a v3.1 stub".into(),
        ))
    }

    fn verify_webhook(&self, _payload: &[u8], _signature: &str) -> Result<WebhookEvent> {
        Err(BillingError::Unsupported(
            "stripe provider is a v3.1 stub".into(),
        ))
    }

    fn name(&self) -> &'static str {
        "stripe"
    }
}
