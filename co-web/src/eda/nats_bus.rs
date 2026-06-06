//! CO-380: `NatsBus` stub — federated EDA backend (v3.x+, feature-gated).
//!
//! Enable with: `cargo build --features eda-nats`
//! Env var: `CO_EDA_BACKEND=nats`
//!
//! This stub compiles but panics at construction time when the feature is
//! active, signalling that the NATS backend is not yet implemented.

#[cfg(feature = "eda-nats")]
mod inner {
    use std::sync::Arc;

    use tokio::sync::broadcast;

    use crate::eda::bus::{EdaBus, Filter, Subscription};
    use crate::eda::event::Event;

    pub struct NatsBus;

    impl NatsBus {
        pub fn new(_url: &str) -> Self {
            unimplemented!(
                "NatsBus: not yet implemented — use TokioBroadcastBus (CO_EDA_BACKEND=tokio)"
            )
        }
    }

    impl EdaBus for NatsBus {
        fn publish(&self, _event: Event) {
            unimplemented!("NatsBus::publish")
        }

        fn subscribe(&self, filter: Filter) -> Subscription {
            let (tx, rx) = broadcast::channel::<Arc<Event>>(1);
            drop(tx);
            Subscription { rx, filter }
        }

        fn name(&self) -> &'static str {
            "nats"
        }
    }
}

#[cfg(feature = "eda-nats")]
pub use inner::NatsBus;
