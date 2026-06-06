//! CO-380: `EdaBus` trait + `Filter` + `Subscription`.
//!
//! Implementations:
//!  - `TokioBroadcastBus` — default, single-machine (see `tokio_bus.rs`)
//!  - `RedisBus`          — stub, feature-gated `eda-redis` (see `redis_bus.rs`)
//!  - `NatsBus`           — stub, feature-gated `eda-nats`  (see `nats_bus.rs`)
//!
//! The concrete impl is chosen by `CO_EDA_BACKEND` env var at startup.

use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::warn;

use super::event::{Event, Visibility};

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

/// Server-side filter applied before a subscriber receives an event.
///
/// All `Some` fields must match; `None` means "any".
/// `min_visibility` gates whether the event's visibility is observable.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Accept only these `event_type` prefixes. `None` = all types.
    pub event_types: Option<Vec<String>>,
    /// Accept only events from these universes. `None` = all universes.
    pub universe_keys: Option<Vec<String>>,
    /// Accept only events from these user ids. `None` = all users.
    pub user_ids: Option<Vec<String>>,
    /// The minimum visibility level the subscriber may observe.
    pub min_visibility: Option<Visibility>,
}

impl Filter {
    /// `true` iff `event` passes all configured constraints.
    pub fn matches(&self, event: &Event) -> bool {
        // event_type prefix filter
        if self.event_types.as_ref().is_some_and(|types| {
            !types
                .iter()
                .any(|t| event.event_type == *t || event.event_type.starts_with(&format!("{t}.")))
        }) {
            return false;
        }
        // universe_key filter
        if let Some(universes) = &self.universe_keys {
            match &event.universe_key {
                Some(k) if universes.contains(k) => {}
                _ => return false,
            }
        }
        // user_id filter
        if let Some(users) = &self.user_ids {
            match &event.user_id {
                Some(u) if users.contains(u) => {}
                _ => return false,
            }
        }
        // visibility gate
        if self
            .min_visibility
            .as_ref()
            .is_some_and(|min_vis| &event.visibility < min_vis)
        {
            return false;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

/// A filtered receiver returned by `EdaBus::subscribe`.
///
/// Backed by a `tokio::sync::broadcast::Receiver` so pub→sub latency is
/// bounded only by the Tokio scheduler — well within 50ms p99 in-process.
pub struct Subscription {
    pub(super) rx: broadcast::Receiver<Arc<Event>>,
    pub(super) filter: Filter,
}

impl Subscription {
    /// Await the next event that passes the filter.
    ///
    /// Returns `None` when the bus is dropped (sender closed).
    /// Lagged messages are silently skipped with a warning.
    pub async fn recv(&mut self) -> Option<Arc<Event>> {
        loop {
            match self.rx.recv().await {
                Ok(ev) if self.filter.matches(&ev) => return Some(ev),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("EDA subscriber lagged, skipped {n} event(s)");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// EdaBus trait
// ---------------------------------------------------------------------------

/// Pluggable universal event bus.
///
/// Default impl: `TokioBroadcastBus` (single-machine, in-process).
/// Future impls: `RedisBus` (multi-machine) and `NatsBus` (federated).
pub trait EdaBus: Send + Sync + 'static {
    /// Publish an event to all active subscribers.
    ///
    /// Fire-and-forget: silently drops when no subscribers are listening.
    /// Must not block; must not panic.
    fn publish(&self, event: Event);

    /// Return a filtered subscription.
    ///
    /// The subscriber's `Subscription::recv()` loop must be driven by a
    /// `tokio::spawn`-ed task so a slow subscriber never blocks the bus.
    fn subscribe(&self, filter: Filter) -> Subscription;

    /// Human-readable backend name for logging/metrics.
    fn name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eda::event::Visibility;

    fn make_event(etype: &str, universe_key: Option<&str>, vis: Visibility) -> Event {
        Event {
            id: "01J0000000000000000000000A".into(),
            event_type: etype.into(),
            universe_key: universe_key.map(Into::into),
            user_id: None,
            payload: serde_json::json!({}),
            visibility: vis,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn filter_all_passes_any_event() {
        let f = Filter::default();
        assert!(f.matches(&make_event("entry.created", Some("u1"), Visibility::Public)));
    }

    #[test]
    fn filter_event_type_exact_match() {
        let f = Filter {
            event_types: Some(vec!["entry.created".into()]),
            ..Default::default()
        };
        assert!(f.matches(&make_event("entry.created", None, Visibility::Public)));
        assert!(!f.matches(&make_event("entry.deleted", None, Visibility::Public)));
    }

    #[test]
    fn filter_event_type_prefix_match() {
        let f = Filter {
            event_types: Some(vec!["analytics".into()]),
            ..Default::default()
        };
        assert!(f.matches(&make_event("analytics.visit", None, Visibility::Public)));
        assert!(!f.matches(&make_event("entry.created", None, Visibility::Public)));
    }

    #[test]
    fn filter_universe_key() {
        let f = Filter {
            universe_keys: Some(vec!["my-universe".into()]),
            ..Default::default()
        };
        assert!(f.matches(&make_event(
            "entry.created",
            Some("my-universe"),
            Visibility::Public
        )));
        assert!(!f.matches(&make_event(
            "entry.created",
            Some("other"),
            Visibility::Public
        )));
        assert!(!f.matches(&make_event("entry.created", None, Visibility::Public)));
    }

    #[test]
    fn filter_min_visibility_blocks_public_for_system_gate() {
        let f = Filter {
            min_visibility: Some(Visibility::System),
            ..Default::default()
        };
        assert!(!f.matches(&make_event("entry.created", None, Visibility::Public)));
        assert!(f.matches(&make_event("system.deploy", None, Visibility::System)));
    }

    #[test]
    fn filter_min_visibility_allows_equal_or_higher() {
        let f = Filter {
            min_visibility: Some(Visibility::UniverseMembers),
            ..Default::default()
        };
        assert!(!f.matches(&make_event("x", None, Visibility::Public)));
        assert!(f.matches(&make_event("x", None, Visibility::UniverseMembers)));
        assert!(f.matches(&make_event("x", None, Visibility::UniverseOwner)));
        assert!(f.matches(&make_event("x", None, Visibility::System)));
    }
}
