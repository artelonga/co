//! CO-380: `KbIndexer` — stub for CO-367 knowledge-base ingest.
//!
//! Subscribes to `entry.created` + `entry.updated` events and will trigger
//! KB indexing when CO-367 is implemented. Currently logs at debug level.

use async_trait::async_trait;
use tracing::debug;

use crate::eda::bus::Filter;
use crate::eda::event::Event;
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// CO-435: stub indexer for `entry.{created,updated}` events (CO-367 hook).
pub struct KbIndexer;

#[async_trait]
impl EdaSubscriber for KbIndexer {
    fn name(&self) -> &'static str {
        "KbIndexer"
    }

    fn filter(&self) -> Filter {
        Filter {
            event_types: Some(vec!["entry.created".into(), "entry.updated".into()]),
            ..Default::default()
        }
    }

    async fn handle(&self, ev: &Event, _ctx: &SubscriberCtx) {
        debug!(
            event_type = %ev.event_type,
            universe_key = ?ev.universe_key,
            "EDA: entry event — would push to KB index (CO-367)"
        );
    }
}
