//! CO-366: Manual-invoice billing provider (default).
//!
//! No external dependency: `create_checkout` returns a link to a hosted invoice
//! page, and an admin marks the user paid via
//! `POST /api/v1/gestao/users/{id}/mark-paid`. There is no webhook to verify, so
//! [`verify_webhook`](ManualInvoiceProvider::verify_webhook) returns
//! [`BillingError::Unsupported`].
//!
//! Optional config: `CO_BILLING_MANUAL_INVOICE_BASE`
//! (default `https://co.artelonga.com.br/billing/invoice`).

use async_trait::async_trait;
use chrono::{Duration, Utc};

use super::{BillingError, BillingProvider, CheckoutSession, Plan, Result, WebhookEvent};
use crate::infra::secrets::SecretsProvider;

const DEFAULT_INVOICE_BASE: &str = "https://co.artelonga.com.br/billing/invoice";
/// Manual invoices stay open for 7 days.
const INVOICE_TTL_DAYS: i64 = 7;

pub struct ManualInvoiceProvider {
    invoice_base: String,
}

impl ManualInvoiceProvider {
    pub fn from_secrets(secrets: &dyn SecretsProvider) -> ManualInvoiceProvider {
        ManualInvoiceProvider {
            invoice_base: secrets
                .get_nonempty("CO_BILLING_MANUAL_INVOICE_BASE")
                .unwrap_or_else(|| DEFAULT_INVOICE_BASE.to_string()),
        }
    }
}

#[async_trait]
impl BillingProvider for ManualInvoiceProvider {
    async fn create_checkout(&self, user_id: &str, plan: Plan) -> Result<CheckoutSession> {
        if plan == Plan::Free {
            return Err(BillingError::Unsupported(
                "free plan has no checkout".into(),
            ));
        }
        let session_id = format!("inv_{}", nanoid::nanoid!(16));
        let url = format!(
            "{base}/{session}?user={user}&plan={plan}",
            base = self.invoice_base,
            session = session_id,
            user = user_id,
            plan = plan.as_str(),
        );
        Ok(CheckoutSession {
            url,
            session_id,
            expires_at: Utc::now() + Duration::days(INVOICE_TTL_DAYS),
        })
    }

    fn verify_webhook(&self, _payload: &[u8], _signature: &str) -> Result<WebhookEvent> {
        Err(BillingError::Unsupported(
            "manual provider has no webhook — use /gestao/users/{id}/mark-paid".into(),
        ))
    }

    fn name(&self) -> &'static str {
        "manual"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::secrets::StaticSecretsProvider;

    fn provider() -> ManualInvoiceProvider {
        ManualInvoiceProvider::from_secrets(
            StaticSecretsProvider::new(Vec::<(String, String)>::new()).as_ref(),
        )
    }

    #[tokio::test]
    async fn checkout_returns_invoice_link() {
        let s = provider()
            .create_checkout("usr_abc", Plan::Starter)
            .await
            .unwrap();
        assert!(s.url.contains("/billing/invoice/"));
        assert!(s.url.contains("user=usr_abc"));
        assert!(s.url.contains("plan=starter"));
        assert!(s.session_id.starts_with("inv_"));
    }

    #[test]
    fn webhook_is_unsupported() {
        let err = provider().verify_webhook(b"{}", "sig").unwrap_err();
        assert!(matches!(err, BillingError::Unsupported(_)));
    }
}
