//! CO-435: EDA subscriber registry — the extension seam for event subscribers.
//!
//! Before CO-435 the boot sequence spawned ~13 subscribers with explicit
//! `spawn()` calls inline in `server/mod.rs`. Adding a subscriber meant editing
//! the boot. This module turns that into a `registry + trait + impl` seam:
//!
//!   - [`EdaSubscriber`] — the trait every subscriber implements (`name`,
//!     `filter`, async `handle`).
//!   - [`SubscriberRegistry`] — an ordered collection the boot iterates over.
//!   - [`SubscriberCtx`] — the shared dependencies handed to each `handle`.
//!
//! Adding a subscriber is now: `impl EdaSubscriber` + `registry.register(...)`.
//! A test subscriber can register without touching `server/mod.rs` — see the
//! `extra_subscriber_registers_without_touching_boot` test.
//!
//! Behaviour is unchanged: [`default_registry`] registers exactly the same
//! subscribers, with the same filters, that the inline boot used to spawn.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tracing::info;

use crate::eda::bus::{EdaBus, Filter};
use crate::eda::event::Event;
use crate::eda::subscribers::degradation_alerter::AlertMailer;
use crate::eda::subscribers::timeline::TimelineSender;
use crate::storage::Storage;

/// Shared dependencies passed to every subscriber's [`EdaSubscriber::handle`].
///
/// Cloned once per subscriber task. The `Arc`/sender clones are cheap.
#[derive(Clone)]
pub struct SubscriberCtx {
    /// The bus a subscriber may re-publish derived events to.
    pub bus: Arc<dyn EdaBus>,
    /// Shared storage handle. Locked briefly inside `handle`; never across an
    /// `.await` point.
    pub storage: Arc<Mutex<Storage>>,
    /// LiveTimeline fan-out channel (drives the `/api/v1/events` WebSocket).
    pub timeline_tx: TimelineSender,
}

/// A pluggable EDA subscriber.
///
/// The registry drives the receive loop: it subscribes with [`filter`](Self::filter),
/// then calls [`handle`](Self::handle) once per matching event. A subscriber that
/// needs its own state (e.g. debounce timers) holds it inside the implementing
/// struct via interior mutability.
#[async_trait]
pub trait EdaSubscriber: Send + Sync + 'static {
    /// Human-readable name for logging/metrics.
    fn name(&self) -> &'static str;

    /// Server-side filter applied before `handle` is called.
    fn filter(&self) -> Filter;

    /// Handle a single event that passed [`filter`](Self::filter).
    async fn handle(&self, ev: &Event, ctx: &SubscriberCtx);
}

/// An ordered collection of [`EdaSubscriber`]s the boot spawns as Tokio tasks.
#[derive(Default)]
pub struct SubscriberRegistry {
    subscribers: Vec<Arc<dyn EdaSubscriber>>,
}

impl SubscriberRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            subscribers: Vec::new(),
        }
    }

    /// Register a subscriber. Returns `&mut self` for chaining.
    pub fn register(&mut self, subscriber: Arc<dyn EdaSubscriber>) -> &mut Self {
        self.subscribers.push(subscriber);
        self
    }

    /// Number of registered subscribers.
    pub fn len(&self) -> usize {
        self.subscribers.len()
    }

    /// Whether no subscribers are registered.
    pub fn is_empty(&self) -> bool {
        self.subscribers.is_empty()
    }

    /// Names of all registered subscribers (registration order).
    pub fn names(&self) -> Vec<&'static str> {
        self.subscribers.iter().map(|s| s.name()).collect()
    }

    /// Spawn every registered subscriber as an independent background task.
    ///
    /// Each subscriber gets its own filtered [`Subscription`](crate::eda::bus::Subscription)
    /// and receive loop, exactly as the old inline `spawn()` calls did.
    pub fn spawn_all(self, ctx: SubscriberCtx) {
        for subscriber in self.subscribers {
            spawn_subscriber(subscriber, ctx.clone());
        }
    }
}

/// Spawn a single subscriber's receive loop as a Tokio task.
///
/// Exposed within the crate so subscriber unit tests can drive the same loop
/// the registry uses in production.
pub fn spawn_subscriber(subscriber: Arc<dyn EdaSubscriber>, ctx: SubscriberCtx) {
    let mut subscription = ctx.bus.subscribe(subscriber.filter());
    tokio::spawn(async move {
        let name = subscriber.name();
        info!("EDA: {name} started");
        while let Some(ev) = subscription.recv().await {
            subscriber.handle(ev.as_ref(), &ctx).await;
        }
        info!("EDA: {name} stopped (bus closed)");
    });
}

/// Build the default registry — the same set of subscribers the inline boot
/// used to spawn, in the same order, with identical filters and behaviour.
///
/// The degradation alerter needs runtime config (mailer + recipient + debounce
/// window), so those are passed in; every other subscriber is self-contained.
pub fn default_registry(
    alert_mailer: Arc<dyn AlertMailer>,
    alert_to: String,
    alert_debounce_hours: u64,
) -> SubscriberRegistry {
    use crate::eda::subscribers::{
        analytics::AnalyticsAggregator, atividades::AtividadesPersistor, billing::BillingPersistor,
        comunicacao_live::ComunicacaoSalaSubscriber, comunicacao_live::ComunicacaoTermSubscriber,
        degradation_alerter::DegradationAlerter, delivery_pipeline::DeliveryPipelinePersistor,
        kb::KbIndexer, sala::SalaBroadcaster, timeline::TimelineForwarder,
        yggdrasil_notes::YggdrasilNotes,
    };
    use crate::security::subscribers::{
        findings_persistor::FindingsPersistor, pbi_backlogger::PbiBacklogger,
        release_blocker::ReleaseBlocker,
    };

    let mut registry = SubscriberRegistry::new();
    registry
        .register(Arc::new(AtividadesPersistor))
        .register(Arc::new(AnalyticsAggregator))
        .register(Arc::new(BillingPersistor))
        .register(Arc::new(SalaBroadcaster))
        .register(Arc::new(KbIndexer))
        // CO-389: live overlay for Yggdrasil lexicon sala events → comunicacao.
        .register(Arc::new(ComunicacaoTermSubscriber))
        .register(Arc::new(ComunicacaoSalaSubscriber))
        // CO-383: ingest Yggdrasil notes via event bus (no polling).
        .register(Arc::new(YggdrasilNotes))
        // CO-398: persist task status transitions + emit deploy.triggered on done.
        .register(Arc::new(DeliveryPipelinePersistor))
        // LiveTimeline forward task (channel created in CoreState::from_storage_full).
        .register(Arc::new(TimelineForwarder))
        // CO-388: security audit subscribers.
        .register(Arc::new(FindingsPersistor))
        .register(Arc::new(PbiBacklogger))
        .register(Arc::new(ReleaseBlocker))
        // CO-422: in-prod degradation alerter — email on disk/backup/universe events.
        .register(Arc::new(DegradationAlerter::new(
            alert_mailer,
            alert_to,
            alert_debounce_hours,
        )));
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eda::event::Visibility;
    use crate::eda::tokio_bus::TokioBroadcastBus;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A test-only subscriber that counts the events it handles.
    struct CountingSubscriber {
        seen: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EdaSubscriber for CountingSubscriber {
        fn name(&self) -> &'static str {
            "CountingSubscriber"
        }
        fn filter(&self) -> Filter {
            Filter {
                event_types: Some(vec!["test.counted".into()]),
                ..Default::default()
            }
        }
        async fn handle(&self, _ev: &Event, _ctx: &SubscriberCtx) {
            self.seen.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn test_ctx(bus: Arc<dyn EdaBus>) -> SubscriberCtx {
        // Storage is required by the ctx type but unused by CountingSubscriber.
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        SubscriberCtx {
            bus,
            storage: Arc::new(Mutex::new(Storage::new(dir.path()))),
            timeline_tx: crate::eda::subscribers::timeline::new_channel(),
        }
    }

    #[test]
    fn register_appends_in_order() {
        let seen = Arc::new(AtomicUsize::new(0));
        let mut registry = SubscriberRegistry::new();
        assert!(registry.is_empty());
        registry.register(Arc::new(CountingSubscriber { seen }));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.names(), vec!["CountingSubscriber"]);
    }

    #[tokio::test]
    async fn extra_subscriber_registers_without_touching_boot() {
        // A brand-new subscriber type plugs into a registry and receives events
        // without any edit to server/mod.rs — proving the extension seam.
        let bus: Arc<dyn EdaBus> = Arc::new(TokioBroadcastBus::new());
        let seen = Arc::new(AtomicUsize::new(0));

        let mut registry = SubscriberRegistry::new();
        registry.register(Arc::new(CountingSubscriber {
            seen: Arc::clone(&seen),
        }));
        registry.spawn_all(test_ctx(Arc::clone(&bus)));

        // Yield so the spawned task subscribes before we publish.
        tokio::task::yield_now().await;

        bus.publish(Event::new(
            "test.counted",
            None,
            None,
            serde_json::json!({}),
            Visibility::System,
        ));
        // A non-matching event must be ignored by the filter.
        bus.publish(Event::new(
            "test.ignored",
            None,
            None,
            serde_json::json!({}),
            Visibility::System,
        ));

        // Poll until the matching event is handled (deterministic, no fixed sleep).
        for _ in 0..100 {
            if seen.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn default_registry_has_all_subscribers() {
        let mailer: Arc<dyn AlertMailer> =
            Arc::new(crate::eda::subscribers::degradation_alerter::tests::MockAlertMailer::new());
        let registry = default_registry(mailer, "ops@example.com".into(), 2);
        // 13 subscribers: atividades, analytics, billing, sala, kb, comunicacao
        // (term + sala = 2), yggdrasil_notes, delivery_pipeline, timeline,
        // findings_persistor, pbi_backlogger, release_blocker, degradation_alerter.
        assert_eq!(registry.len(), 14);
        assert!(registry.names().contains(&"AtividadesPersistor"));
        assert!(registry.names().contains(&"DegradationAlerter"));
    }
}
