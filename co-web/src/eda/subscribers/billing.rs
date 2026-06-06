//! CO-380: `BillingPersistor` — stub for `billing.*` events.
//!
//! Placeholder for CO-366 billing events. Logs `billing.*` events at info
//! level. The `billing_events` table and full persistence logic will be
//! added in CO-366.

use std::sync::Arc;

use tracing::{info, warn};

use crate::eda::bus::{EdaBus, Filter};

/// Spawn the `BillingPersistor` subscriber.
pub fn spawn(bus: Arc<dyn EdaBus>) {
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec!["billing".into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: BillingPersistor started");
        while let Some(ev) = sub.recv().await {
            // CO-366: replace with INSERT INTO billing_events once that table exists.
            warn!(
                event_type = %ev.event_type,
                universe_key = ?ev.universe_key,
                "EDA: billing event received (stub — no persistence yet, CO-366)"
            );
        }
        info!("EDA: BillingPersistor stopped");
    });
}
