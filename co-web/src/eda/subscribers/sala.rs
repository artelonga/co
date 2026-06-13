//! CO-380: `SalaBroadcaster` — replaces CO-353 workspace-presence lobby.
//!
//! Subscribes to `workspace.*` events and fans them out to WebSocket
//! connections in the same universe room. The actual WebSocket fanout is
//! handled by the existing `sync_rooms` broadcast channels in `RealtimeState`.

use async_trait::async_trait;
use tracing::debug;

use crate::eda::bus::Filter;
use crate::eda::event::Event;
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// CO-435: filter-owner for `workspace.*` events (CO-353 presence replacement).
pub struct SalaBroadcaster;

#[async_trait]
impl EdaSubscriber for SalaBroadcaster {
    fn name(&self) -> &'static str {
        "SalaBroadcaster"
    }

    fn filter(&self) -> Filter {
        Filter {
            event_types: Some(vec![
                "workspace.cursor".into(),
                "workspace.entry_placed".into(),
                "workspace.entry_linked".into(),
            ]),
            ..Default::default()
        }
    }

    async fn handle(&self, ev: &Event, _ctx: &SubscriberCtx) {
        debug!(
            event_type = %ev.event_type,
            universe_key = ?ev.universe_key,
            "EDA: workspace event — would fan-out to sala room"
        );
        // CO-381: feed into LiveTimeline WebSocket fanout here.
    }
}
