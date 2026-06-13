//! CO-380: `AnalyticsAggregator` — aggregates `analytics.*` events.
//!
//! CO-340 analytics rollups continue to work through the existing
//! `/api/v1/analytics/public/rollups` ingest path. This subscriber is the
//! future hook for in-process aggregation when rollups are computed
//! server-side (v3.x).

use async_trait::async_trait;
use tracing::debug;

use crate::eda::bus::Filter;
use crate::eda::event::Event;
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// CO-435: no-op aggregator for `analytics.*` events (CO-340 rollup hook).
pub struct AnalyticsAggregator;

#[async_trait]
impl EdaSubscriber for AnalyticsAggregator {
    fn name(&self) -> &'static str {
        "AnalyticsAggregator"
    }

    fn filter(&self) -> Filter {
        Filter {
            event_types: Some(vec!["analytics".into()]),
            ..Default::default()
        }
    }

    async fn handle(&self, ev: &Event, _ctx: &SubscriberCtx) {
        debug!(
            event_type = %ev.event_type,
            universe_key = ?ev.universe_key,
            "EDA: analytics event received"
        );
    }
}
