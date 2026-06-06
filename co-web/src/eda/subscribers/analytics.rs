//! CO-380: `AnalyticsAggregator` — aggregates `analytics.*` events.
//!
//! CO-340 analytics rollups continue to work through the existing
//! `/api/v1/analytics/public/rollups` ingest path. This subscriber is the
//! future hook for in-process aggregation when rollups are computed
//! server-side (v3.x).

use std::sync::Arc;

use tracing::{debug, info};

use crate::eda::bus::{EdaBus, Filter};

/// Spawn the `AnalyticsAggregator` subscriber.
///
/// Currently a no-op aggregator — logs `analytics.*` events at debug level.
/// CO-340 rollup path is unchanged; this subscriber is wired for v3.x.
pub fn spawn(bus: Arc<dyn EdaBus>) {
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec!["analytics".into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: AnalyticsAggregator started");
        while let Some(ev) = sub.recv().await {
            debug!(
                event_type = %ev.event_type,
                universe_key = ?ev.universe_key,
                "EDA: analytics event received"
            );
        }
        info!("EDA: AnalyticsAggregator stopped");
    });
}
