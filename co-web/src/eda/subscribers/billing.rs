//! CO-380: `BillingPersistor` — stub for `billing.*` events.
//!
//! Placeholder for CO-366 billing events. Logs `billing.*` events at info
//! level. The `billing_events` table and full persistence logic will be
//! added in CO-366.

use async_trait::async_trait;
use tracing::warn;

use crate::eda::bus::Filter;
use crate::eda::event::Event;
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// CO-435: stub persistor for `billing.*` events (CO-366 hook).
pub struct BillingPersistor;

#[async_trait]
impl EdaSubscriber for BillingPersistor {
    fn name(&self) -> &'static str {
        "BillingPersistor"
    }

    fn filter(&self) -> Filter {
        Filter {
            event_types: Some(vec!["billing".into()]),
            ..Default::default()
        }
    }

    async fn handle(&self, ev: &Event, _ctx: &SubscriberCtx) {
        // CO-366: replace with INSERT INTO billing_events once that table exists.
        warn!(
            event_type = %ev.event_type,
            universe_key = ?ev.universe_key,
            "EDA: billing event received (stub — no persistence yet, CO-366)"
        );
    }
}
