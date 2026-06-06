//! CO-380: `RedisBus` stub — multi-machine EDA backend (v3.x, feature-gated).
//!
//! Enable with: `cargo build --features eda-redis`
//! Env var: `CO_EDA_BACKEND=redis`
//!
//! This stub compiles but panics at construction time when the feature is
//! active, signalling that the Redis backend is not yet implemented.
//! Implement by replacing `unimplemented!()` with real Redis PubSub logic.

#[cfg(feature = "eda-redis")]
mod inner {
    use std::sync::Arc;

    use tokio::sync::broadcast;

    use crate::eda::bus::{EdaBus, Filter, Subscription};
    use crate::eda::event::Event;

    pub struct RedisBus;

    impl RedisBus {
        pub fn new(_url: &str) -> Self {
            unimplemented!(
                "RedisBus: not yet implemented — use TokioBroadcastBus (CO_EDA_BACKEND=tokio)"
            )
        }
    }

    impl EdaBus for RedisBus {
        fn publish(&self, _event: Event) {
            unimplemented!("RedisBus::publish")
        }

        fn subscribe(&self, filter: Filter) -> Subscription {
            // Return a receiver that never yields (bus is unimplemented).
            let (tx, rx) = broadcast::channel::<Arc<Event>>(1);
            drop(tx);
            Subscription { rx, filter }
        }

        fn name(&self) -> &'static str {
            "redis"
        }
    }
}

#[cfg(feature = "eda-redis")]
pub use inner::RedisBus;
