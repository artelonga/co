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

use tokio::sync::broadcast;
use tracing::info;

use crate::eda::bus::{EdaBus, Filter};
use crate::eda::event::Event;

/// Capacity of the LiveTimeline fan-out channel.
pub const TIMELINE_CHANNEL_CAPACITY: usize = 256;

/// A shared sender for the LiveTimeline fan-out channel.
pub type TimelineSender = broadcast::Sender<Arc<Event>>;

/// Phase 1: create the fan-out channel.  Safe to call without a Tokio runtime.
pub fn new_channel() -> TimelineSender {
    let (tx, _) = broadcast::channel::<Arc<Event>>(TIMELINE_CHANNEL_CAPACITY);
    tx
}

/// Phase 2: spawn the forward task.  Must be called from a Tokio runtime context.
///
/// The task filters EDA bus events and forwards them to the `TimelineSender`
/// so WebSocket handlers can subscribe via `tx.subscribe()`.
pub fn start(bus: Arc<dyn EdaBus>, tx: TimelineSender) {
    let tx_clone = tx.clone();
    let mut sub = bus.subscribe(Filter::default());

    tokio::spawn(async move {
        info!("EDA: LiveTimeline started");
        while let Some(ev) = sub.recv().await {
            // Never forward System-level events to WebSocket clients.
            if ev.visibility == crate::eda::event::Visibility::System {
                continue;
            }
            // Ignore lag — slow WebSocket clients don't block the bus.
            let _ = tx_clone.send(ev);
        }
        info!("EDA: LiveTimeline stopped");
    });
}

/// Convenience: create channel + spawn task in one call.
/// Only use from async contexts (i.e., server startup, not unit tests).
pub fn spawn(bus: Arc<dyn EdaBus>) -> TimelineSender {
    let tx = new_channel();
    start(bus, tx.clone());
    tx
}
