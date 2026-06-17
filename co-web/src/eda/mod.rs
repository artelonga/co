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
pub mod subscriber_registry;
pub mod subscribers;
pub mod tokio_bus;

pub use bus::{EdaBus, Filter, Subscription};
pub use event::{Event, Visibility, new_ulid};
pub use subscriber_registry::{EdaSubscriber, SubscriberCtx, SubscriberRegistry, default_registry};
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

/// One-shot boot maintenance: reclaim the `bridge.*` transport bloat in
/// `event_log` (CO bridge-flood cleanup). Gated by `CO_MAINTENANCE_RECLAIM_EVENT_LOG`.
///
/// **Runs entirely on a blocking thread (`spawn_blocking`)** so it never occupies a
/// tokio worker — the async runtime (and the storage-free `/api/health` Fly check)
/// stays responsive throughout, even on the 2-CPU prod machine. The multi-GB delete
/// runs in batches that **re-acquire the storage lock per batch** (with a short
/// yield between), so request handlers interleave instead of blocking for the whole
/// delete. Only the final `VACUUM` holds the lock continuously (~tens of seconds on
/// the small live remainder); health is unaffected because it touches no storage.
/// Idempotent — a no-op once the bloat is gone, so the flag can be left set.
///
/// (A prior version ran this synchronously on a tokio worker while holding the lock,
/// which starved the 2-worker runtime and failed the health check — hence the
/// `spawn_blocking` + per-batch design here.)
pub async fn event_log_reclaim_boot_task(state: crate::server::AppState) {
    tokio::time::sleep(tokio::time::Duration::from_secs(20)).await;

    const BATCH: usize = 20_000;
    let outcome = tokio::task::spawn_blocking(move || {
        let before = state.core.storage.lock().db_size_bytes();
        if state.core.storage.lock().count_event_log_transport_rows() < BATCH as i64 / 4 {
            return Ok::<_, rusqlite::Error>((0usize, before, before));
        }
        let mut deleted = 0usize;
        loop {
            let n = state
                .core
                .storage
                .lock()
                .delete_event_log_transport_batch(BATCH)?;
            deleted += n;
            if n == 0 {
                break;
            }
            // Lock released here; yield so request handlers (and other contenders)
            // get a turn before the next batch.
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        state.core.storage.lock().vacuum()?;
        let after = state.core.storage.lock().db_size_bytes();
        Ok((deleted, before, after))
    })
    .await;

    match outcome {
        Ok(Ok((0, _, _))) => {
            tracing::info!("maintenance: event_log reclaim — nothing to do (no transport bloat)")
        }
        Ok(Ok((deleted, before, after))) => tracing::warn!(
            "maintenance: event_log reclaim — deleted {deleted} transport rows, \
             meta.db {} MiB → {} MiB (reclaimed {} MiB)",
            before / 1_048_576,
            after / 1_048_576,
            (before - after) / 1_048_576,
        ),
        Ok(Err(e)) => tracing::error!("maintenance: event_log reclaim FAILED (non-fatal): {e}"),
        Err(e) => tracing::error!("maintenance: event_log reclaim task panicked: {e}"),
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
