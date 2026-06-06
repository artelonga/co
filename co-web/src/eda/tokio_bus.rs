//! CO-380: `TokioBroadcastBus` — default single-machine EDA backend.
//!
//! Uses `tokio::sync::broadcast` internally. Capacity is 4096 events.
//! If a subscriber is too slow and lags, it skips events with a warning.
//! The producer is NEVER blocked by a slow subscriber (CO-380 guarantee).

use std::sync::Arc;

use tokio::sync::broadcast;

use super::bus::{EdaBus, Filter, Subscription};
use super::event::Event;

const BUS_CAPACITY: usize = 4_096;

/// In-process fan-out bus backed by `tokio::sync::broadcast`.
///
/// `Clone` is cheap — internally holds an `Arc` over the sender half.
#[derive(Clone)]
pub struct TokioBroadcastBus {
    tx: broadcast::Sender<Arc<Event>>,
}

impl TokioBroadcastBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }
}

impl Default for TokioBroadcastBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EdaBus for TokioBroadcastBus {
    fn publish(&self, event: Event) {
        // `send` returns Err when no receivers are active — that's fine.
        let _ = self.tx.send(Arc::new(event));
    }

    fn subscribe(&self, filter: Filter) -> Subscription {
        Subscription {
            rx: self.tx.subscribe(),
            filter,
        }
    }

    fn name(&self) -> &'static str {
        "tokio-broadcast"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::eda::event::Visibility;

    fn sample_event(etype: &str) -> Event {
        Event::new(
            etype,
            Some("u1".into()),
            None,
            serde_json::json!({}),
            Visibility::Public,
        )
    }

    #[tokio::test]
    async fn publish_received_by_subscriber() {
        let bus = TokioBroadcastBus::new();
        let mut sub = bus.subscribe(Filter::default());

        bus.publish(sample_event("entry.created"));

        let ev = tokio::time::timeout(Duration::from_millis(100), sub.recv())
            .await
            .expect("timed out")
            .expect("bus closed");
        assert_eq!(ev.event_type, "entry.created");
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let bus = TokioBroadcastBus::new();
        let mut s1 = bus.subscribe(Filter::default());
        let mut s2 = bus.subscribe(Filter::default());

        bus.publish(sample_event("analytics.visit"));

        let e1 = tokio::time::timeout(Duration::from_millis(100), s1.recv())
            .await
            .unwrap()
            .unwrap();
        let e2 = tokio::time::timeout(Duration::from_millis(100), s2.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(e1.event_type, "analytics.visit");
        assert_eq!(e2.event_type, "analytics.visit");
    }

    #[tokio::test]
    async fn no_subscriber_publish_does_not_panic() {
        let bus = TokioBroadcastBus::new();
        // No subscribers — must not panic.
        bus.publish(sample_event("system.deploy"));
    }

    #[tokio::test]
    async fn filter_blocks_non_matching_event() {
        let bus = TokioBroadcastBus::new();
        let mut sub = bus.subscribe(Filter {
            event_types: Some(vec!["billing".into()]),
            ..Default::default()
        });

        bus.publish(sample_event("entry.created"));
        bus.publish(sample_event("billing.paid"));

        // Only billing.paid should arrive.
        let ev = tokio::time::timeout(Duration::from_millis(100), sub.recv())
            .await
            .expect("timed out")
            .expect("closed");
        assert_eq!(ev.event_type, "billing.paid");
    }

    #[tokio::test]
    async fn dropped_subscriber_does_not_block_producer() {
        let bus = TokioBroadcastBus::new();
        {
            let _sub = bus.subscribe(Filter::default());
            // sub drops here
        }
        // Producer must still work.
        for _ in 0..10 {
            bus.publish(sample_event("entry.created"));
        }
    }

    #[test]
    fn name_is_tokio_broadcast() {
        let bus = TokioBroadcastBus::new();
        assert_eq!(bus.name(), "tokio-broadcast");
    }
}
