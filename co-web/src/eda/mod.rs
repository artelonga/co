//! CO-380: Universal EDA spine — event bus, types, and subscribers.
//!
//! Every state-changing route publishes to the `EdaBus`. Subscribers filter
//! events and perform domain-specific side effects (persistence, WebSocket
//! fanout, indexing). Backend is pluggable: default is `TokioBroadcastBus`
//! (single-machine). Future: `RedisBus` (multi-machine) or `NatsBus`.
//!
//! # Backend selection
//!
//! Set `CO_EDA_BACKEND` env var (defaults to `"tokio"`):
//!   - `tokio`  → `TokioBroadcastBus` (default, no extra deps)
//!   - `redis`  → `RedisBus` (requires `--features eda-redis`)
//!   - `nats`   → `NatsBus`  (requires `--features eda-nats`)
//!
//! # Usage
//!
//! ```rust,ignore
//! // Publish
//! state.core.eda_bus.publish(Event::new(
//!     "entry.created",
//!     Some(universe_key),
//!     Some(user_id),
//!     serde_json::json!({ "path": entry_path }),
//!     Visibility::UniverseMembers,
//! ));
//!
//! // Subscribe
//! let mut sub = state.core.eda_bus.subscribe(Filter {
//!     event_types: Some(vec!["entry".into()]),
//!     ..Default::default()
//! });
//! while let Some(ev) = sub.recv().await { /* … */ }
//! ```

pub mod bridge;
pub mod bus;
pub mod event;
pub mod events_ws;
pub mod nats_bus;
pub mod redis_bus;
pub mod subscribers;
pub mod tokio_bus;

pub use bus::{EdaBus, Filter, Subscription};
pub use event::{Event, Visibility, new_ulid};
pub use tokio_bus::TokioBroadcastBus;

// ---------------------------------------------------------------------------
// event_log retention task
// ---------------------------------------------------------------------------

/// Background task: delete `event_log` rows older than 30 days.
///
/// Runs once every 24 hours. Starts immediately on first tick.
pub async fn event_log_retention_task(state: crate::server::AppState) {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(24 * 3600)).await;
        let storage = state.core.storage.lock();
        match storage.prune_event_log_older_than_30_days() {
            Ok(n) if n > 0 => {
                tracing::info!("EDA: event_log retention: pruned {n} row(s) older than 30 days")
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("EDA: event_log retention DELETE failed: {e}"),
        }
    }
}

/// Build the configured `EdaBus` from [`CoServerConfig::eda_backend`].
///
/// Returns `Arc<dyn EdaBus>` backed by `TokioBroadcastBus` for CO-380.
/// The Redis/NATS stubs panic at construction time until implemented.
/// CO-434: backend selection now comes from the boot-time config (injectable
/// in tests) instead of a `std::env::var` read at construction time.
pub fn build_bus(config: &crate::CoServerConfig) -> std::sync::Arc<dyn EdaBus> {
    match config.eda_backend.as_str() {
        "tokio" | "" => {
            tracing::info!("EDA: backend = tokio-broadcast");
            std::sync::Arc::new(TokioBroadcastBus::new())
        }
        other => {
            tracing::error!("EDA: unknown backend '{other}', falling back to tokio-broadcast");
            std::sync::Arc::new(TokioBroadcastBus::new())
        }
    }
}
