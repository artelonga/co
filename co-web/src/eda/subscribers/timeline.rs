//! CO-380: `LiveTimeline` — real-time event stream for CO-381.
//!
//! Subscribes to all non-System events that the SPA's `/api/v1/events`
//! WebSocket may forward to authenticated clients. The WebSocket handler
//! itself lives in `co-web/src/eda/events_ws.rs` (CO-381 paired task).
//!
//! Two-phase design (so `CoreState::from_storage_full` can be called from
//! non-async tests without a Tokio runtime):
//!
//!   Phase 1 (no runtime needed): `new_channel()` creates the broadcast pair.
//!   Phase 2 (runtime required): `start()` spawns the forward task.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::eda::bus::Filter;
use crate::eda::event::{Event, Visibility};
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// Capacity of the LiveTimeline fan-out channel.
pub const TIMELINE_CHANNEL_CAPACITY: usize = 256;

/// A shared sender for the LiveTimeline fan-out channel.
pub type TimelineSender = broadcast::Sender<Arc<Event>>;

/// Create the fan-out channel.  Safe to call without a Tokio runtime.
///
/// The channel is created in `CoreState::from_storage_full` (non-async, used by
/// tests); the forwarding subscriber ([`TimelineForwarder`]) is spawned later by
/// the registry once the runtime is live.
pub fn new_channel() -> TimelineSender {
    let (tx, _) = broadcast::channel::<Arc<Event>>(TIMELINE_CHANNEL_CAPACITY);
    tx
}

/// CO-435: forwards all non-System EDA events to the LiveTimeline channel
/// (`ctx.timeline_tx`) so the `/api/v1/events` WebSocket can fan them out.
pub struct TimelineForwarder;

#[async_trait]
impl EdaSubscriber for TimelineForwarder {
    fn name(&self) -> &'static str {
        "LiveTimeline"
    }

    fn filter(&self) -> Filter {
        Filter::default()
    }

    async fn handle(&self, ev: &Event, ctx: &SubscriberCtx) {
        // Never forward System-level events to WebSocket clients.
        if ev.visibility == Visibility::System {
            return;
        }
        // Ignore lag — slow WebSocket clients don't block the bus.
        let _ = ctx.timeline_tx.send(Arc::new(ev.clone()));
    }
}
