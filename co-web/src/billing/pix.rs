//! CO-366: Pix billing provider — **stub** (feature `billing-pix`, v3.1 target).
//!
//! Pix is Brazil's zero-fee instant-payment rail. The full implementation
//! (charge creation + QR Code + webhook reconciliation) lands in v3.1; this
//! stub keeps the trait wired so the provider factory and feature flag compile.

use async_trait::async_trait;

use super::{BillingError, BillingProvider, CheckoutSession, Plan, Result, WebhookEvent};
use crate::infra::secrets::SecretsProvider;

pub struct PixProvider;

impl PixProvider {
    pub fn from_secrets(_secrets: &dyn SecretsProvider) -> PixProvider {
        PixProvider
    }
}

#[async_trait]
impl BillingProvider for PixProvider {
    async fn create_checkout(&self, _user_id: &str, _plan: Plan) -> Result<CheckoutSession> {
        Err(BillingError::Unsupported(
            "pix provider is a v3.1 stub".into(),
        ))
    }

    fn verify_webhook(&self, _payload: &[u8], _signature: &str) -> Result<WebhookEvent> {
        Err(BillingError::Unsupported(
            "pix provider is a v3.1 stub".into(),
        ))
    }

    fn name(&self) -> &'static str {
        "pix"
    }
}
