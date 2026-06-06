//! CO-380: `KbIndexer` — stub for CO-367 knowledge-base ingest.
//!
//! Subscribes to `entry.created` + `entry.updated` events and will trigger
//! KB indexing when CO-367 is implemented. Currently logs at debug level.

use std::sync::Arc;

use tracing::{debug, info};

use crate::eda::bus::{EdaBus, Filter};

/// Spawn the `KbIndexer` subscriber.
pub fn spawn(bus: Arc<dyn EdaBus>) {
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec!["entry.created".into(), "entry.updated".into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: KbIndexer started (CO-367 stub)");
        while let Some(ev) = sub.recv().await {
            debug!(
                event_type = %ev.event_type,
                universe_key = ?ev.universe_key,
                "EDA: entry event — would push to KB index (CO-367)"
            );
        }
        info!("EDA: KbIndexer stopped");
    });
}
