//! CO-380: `AtividadesPersistor` — appends EDA events to `event_log` table.
//!
//! Subscribes to all events and writes each one to `event_log` for replay,
//! EXCEPT high-frequency transport events (see `TRANSPORT_EVENTS`).
//! The `atividades` audit table continues to be written directly by
//! `log_atividade()` (backward-compat shim).

use async_trait::async_trait;
use tracing::warn;

use crate::eda::bus::Filter;
use crate::eda::event::Event;
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// Transport-layer bridge relay events. The EDA bridge emits one of these for
/// *every* event it relays (~73/sec in prod), so persisting them flooded
/// `event_log` to 38M rows / 18 GB in 6 days (99.1% of all rows) and filled the
/// prod volume. They are pure bus-transport telemetry — not domain events worth
/// durable replay — so they are never written to `event_log`. The in-memory
/// observability ring buffer / live layer still sees them.
const TRANSPORT_EVENTS: &[&str] = &["bridge.event_sent", "bridge.event_received"];

/// Whether an event type should be durably persisted to `event_log`.
fn should_persist(event_type: &str) -> bool {
    !TRANSPORT_EVENTS.contains(&event_type)
}

/// CO-435: appends every (non-transport) EDA event to the `event_log` table for replay.
pub struct AtividadesPersistor;

#[async_trait]
impl EdaSubscriber for AtividadesPersistor {
    fn name(&self) -> &'static str {
        "AtividadesPersistor"
    }

    fn filter(&self) -> Filter {
        Filter::default()
    }

    async fn handle(&self, ev: &Event, ctx: &SubscriberCtx) {
        // Drop high-frequency transport events before taking the storage lock.
        if !should_persist(&ev.event_type) {
            return;
        }
        let storage = ctx.storage.lock();
        let payload_str = ev.payload.to_string();
        let vis = format!("{:?}", ev.visibility);
        let created_at = ev.created_at.to_rfc3339();
        if let Err(e) = storage.insert_event_log(
            &ev.id,
            &ev.event_type,
            ev.universe_key.as_deref(),
            ev.user_id.as_deref(),
            &payload_str,
            &vis,
            &created_at,
        ) {
            warn!("EDA: AtividadesPersistor INSERT failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_persist;

    #[test]
    fn transport_events_are_not_persisted() {
        // The bridge relay events that flooded event_log (99.1% of rows).
        assert!(!should_persist("bridge.event_sent"));
        assert!(!should_persist("bridge.event_received"));
    }

    #[test]
    fn domain_events_are_persisted() {
        for et in [
            "entry.created",
            "entry.updated",
            "atividade.criar",
            "vault.write",
            "bridge.connected",
            "live.timeline_viewed",
        ] {
            assert!(should_persist(et), "{et} should be persisted");
        }
    }
}
