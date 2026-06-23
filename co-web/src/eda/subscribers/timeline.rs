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

/// A single event ready to fan out: the [`Event`] (kept for the per-client
/// visibility check) plus its JSON **serialized exactly once**.
///
/// CO-468: previously the channel carried `Arc<Event>` and each connected
/// `/api/v1/events` socket re-ran `serde_json::to_string` on the same event —
/// N identical serializations for N clients. The JSON is independent of the
/// client's visibility level (that only gates *whether* to send, not the
/// bytes), so we serialize here, once, at the single fan-out point and share
/// the result as an `Arc<str>`.
#[derive(Debug)]
pub struct TimelineFrame {
    /// The event itself — used by the WebSocket handler's `can_see` filter.
    pub event: Event,
    /// The event serialized to JSON once; shared across all subscribers.
    pub json: Arc<str>,
}

/// A shared sender for the LiveTimeline fan-out channel.
pub type TimelineSender = broadcast::Sender<Arc<TimelineFrame>>;

/// Create the fan-out channel.  Safe to call without a Tokio runtime.
///
/// The channel is created in `CoreState::from_storage_full` (non-async, used by
/// tests); the forwarding subscriber ([`TimelineForwarder`]) is spawned later by
/// the registry once the runtime is live.
pub fn new_channel() -> TimelineSender {
    let (tx, _) = broadcast::channel::<Arc<TimelineFrame>>(TIMELINE_CHANNEL_CAPACITY);
    tx
}

/// Build a [`TimelineFrame`] from an event, serializing its JSON once.
/// Returns `None` if the event fails to serialize (logged by the caller).
pub(crate) fn frame_for(ev: &Event) -> Option<TimelineFrame> {
    let json = serde_json::to_string(ev).ok()?;
    Some(TimelineFrame {
        event: ev.clone(),
        json: Arc::from(json),
    })
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
        // CO-468: serialize once here, not once-per-client in the socket loop.
        let Some(frame) = frame_for(ev) else {
            tracing::warn!("EDA: failed to serialize event for LiveTimeline fan-out");
            return;
        };
        // Ignore lag — slow WebSocket clients don't block the bus.
        let _ = ctx.timeline_tx.send(Arc::new(frame));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eda::event::Visibility;
    use chrono::Utc;

    fn ev(vis: Visibility) -> Event {
        Event {
            id: "evt-1".into(),
            event_type: "entry.created".into(),
            universe_key: Some("u1".into()),
            user_id: None,
            payload: serde_json::json!({"path": "content/a.md"}),
            visibility: vis,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn frame_json_matches_single_serialization() {
        let e = ev(Visibility::Public);
        let frame = frame_for(&e).expect("serializes");
        // CO-468: the shared JSON is exactly what a per-client serialize would
        // have produced — so reusing it is byte-for-byte equivalent.
        assert_eq!(frame.json.as_ref(), serde_json::to_string(&e).unwrap());
        assert_eq!(frame.event.id, e.id);
    }

    #[test]
    fn frame_carries_event_for_visibility_filtering() {
        // The event is retained alongside the JSON so the WS handler can still
        // run its per-client `can_see` check without re-parsing the JSON.
        let frame = frame_for(&ev(Visibility::UniverseOwner)).expect("serializes");
        assert_eq!(frame.event.visibility, Visibility::UniverseOwner);
        assert_eq!(frame.event.universe_key.as_deref(), Some("u1"));
    }
}
