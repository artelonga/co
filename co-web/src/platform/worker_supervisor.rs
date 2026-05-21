//! CO-223: Shared `Worker` trait + `WorkerSupervisor`.
//!
//! Every background worker implements `Worker`, which provides a `name`, a
//! tick `interval`, and a single unit of work (`tick`). The `WorkerSupervisor`
//! owns all running tasks: it catches panics in one worker so siblings are
//! never brought down, tracks `last_tick_at`, and exposes a status snapshot
//! for `/api/v1/admin/workers/status`.
//!
//! # Panic isolation
//! Each worker runs in its own child `tokio::spawn`. A panic surfaces as a
//! `JoinError::Panic` to the outer supervision loop, which logs it, increments
//! `panic_count`, and restarts the worker (from a fresh `Clone`) after a
//! 5-second back-off (doubling, capped at 60 s). Other workers are unaffected.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::{error, warn};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A background periodic worker.
///
/// Implement this for every long-running background job so the
/// `WorkerSupervisor` can manage its lifecycle uniformly.
#[async_trait]
pub trait Worker: Send + 'static {
    /// Short, unique identifier shown in the status endpoint.
    fn name(&self) -> &'static str;

    /// How long to wait between consecutive ticks.
    fn interval(&self) -> Duration;

    /// Perform one unit of work.
    ///
    /// Errors are logged and counted but do not restart the loop.
    /// Panics propagate to the supervisor, which restarts the worker.
    async fn tick(&mut self) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// Status snapshot
// ---------------------------------------------------------------------------

/// Runtime snapshot of one worker — serialised for the status endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct WorkerStatus {
    pub name: String,
    pub last_tick_at: Option<DateTime<Utc>>,
    pub tick_count: u64,
    pub error_count: u64,
    pub panic_count: u64,
    pub running: bool,
}

type StatusTable = Arc<Mutex<Vec<WorkerStatus>>>;

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

/// Owns all background worker tasks.
///
/// `Clone` is cheap — cloning shares the underlying status table.
#[derive(Clone)]
pub struct WorkerSupervisor {
    statuses: StatusTable,
}

impl WorkerSupervisor {
    pub fn new() -> Self {
        Self {
            statuses: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Register `worker` and start its supervision loop.
    ///
    /// The inner tick loop runs inside a child `tokio::spawn` so panics in
    /// `tick` become `JoinError::Panic` rather than propagating to siblings.
    /// On panic the outer loop restarts from `worker.clone()` after back-off.
    pub fn spawn<W: Worker + Clone>(&self, worker: W) {
        let idx = self.register(&worker);
        let statuses = Arc::clone(&self.statuses);
        tokio::spawn(run_supervised(worker, idx, statuses));
    }

    /// Snapshot of all worker statuses (cheap clone of the status vector).
    pub fn statuses(&self) -> Vec<WorkerStatus> {
        self.statuses.lock().unwrap().clone()
    }

    fn register<W: Worker>(&self, worker: &W) -> usize {
        let mut s = self.statuses.lock().unwrap();
        let idx = s.len();
        s.push(WorkerStatus {
            name: worker.name().to_owned(),
            last_tick_at: None,
            tick_count: 0,
            error_count: 0,
            panic_count: 0,
            running: true,
        });
        idx
    }
}

impl Default for WorkerSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal: supervision loop
// ---------------------------------------------------------------------------

async fn run_supervised<W: Worker + Clone>(template: W, idx: usize, statuses: StatusTable) {
    let name = template.name();
    let mut backoff = Duration::from_secs(5);

    loop {
        let worker = template.clone();
        let statuses2 = Arc::clone(&statuses);

        let result = tokio::spawn(run_worker_loop(worker, idx, statuses2)).await;

        match result {
            Ok(()) => break, // clean exit — impossible for an infinite loop; treat as shutdown
            Err(e) if e.is_panic() => {
                error!(
                    "worker '{name}' panicked — restarting in {}s",
                    backoff.as_secs()
                );
                {
                    let mut s = statuses.lock().unwrap();
                    if let Some(ws) = s.get_mut(idx) {
                        ws.panic_count += 1;
                        ws.running = false;
                    }
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
                {
                    let mut s = statuses.lock().unwrap();
                    if let Some(ws) = s.get_mut(idx) {
                        ws.running = true;
                    }
                }
            }
            Err(_) => break, // task cancelled (graceful shutdown)
        }
    }
}

async fn run_worker_loop<W: Worker>(mut worker: W, idx: usize, statuses: StatusTable) {
    let name = worker.name();
    let mut ticker = tokio::time::interval(worker.interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        match worker.tick().await {
            Ok(()) => {
                let mut s = statuses.lock().unwrap();
                if let Some(ws) = s.get_mut(idx) {
                    ws.last_tick_at = Some(Utc::now());
                    ws.tick_count += 1;
                }
            }
            Err(e) => {
                warn!("worker '{name}' error: {e:#}");
                let mut s = statuses.lock().unwrap();
                if let Some(ws) = s.get_mut(idx) {
                    ws.error_count += 1;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Clone)]
    struct CountingWorker {
        counter: Arc<AtomicU64>,
    }

    #[async_trait]
    impl Worker for CountingWorker {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn interval(&self) -> Duration {
            Duration::from_millis(10)
        }
        async fn tick(&mut self) -> anyhow::Result<()> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct ErrorWorker;

    #[async_trait]
    impl Worker for ErrorWorker {
        fn name(&self) -> &'static str {
            "error"
        }
        fn interval(&self) -> Duration {
            Duration::from_millis(10)
        }
        async fn tick(&mut self) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("deliberate error"))
        }
    }

    #[derive(Clone)]
    struct PanickingWorker;

    #[async_trait]
    impl Worker for PanickingWorker {
        fn name(&self) -> &'static str {
            "panicking"
        }
        fn interval(&self) -> Duration {
            Duration::from_millis(10)
        }
        async fn tick(&mut self) -> anyhow::Result<()> {
            panic!("deliberate panic in test");
        }
    }

    #[tokio::test]
    async fn supervisor_tracks_ticks() {
        let sup = WorkerSupervisor::new();
        let counter = Arc::new(AtomicU64::new(0));
        sup.spawn(CountingWorker {
            counter: Arc::clone(&counter),
        });

        tokio::time::sleep(Duration::from_millis(120)).await;

        let statuses = sup.statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].name, "counting");
        assert!(statuses[0].tick_count > 0, "should have ticked");
        assert!(statuses[0].last_tick_at.is_some());
        assert!(statuses[0].running);
        assert_eq!(statuses[0].error_count, 0);
        assert_eq!(statuses[0].panic_count, 0);
    }

    #[tokio::test]
    async fn errors_are_counted_not_fatal() {
        let sup = WorkerSupervisor::new();
        sup.spawn(ErrorWorker);

        tokio::time::sleep(Duration::from_millis(120)).await;

        let s = sup.statuses();
        assert!(s[0].error_count > 0, "errors should be counted");
        assert!(s[0].running, "errors must not kill the worker");
        assert_eq!(s[0].panic_count, 0);
    }

    #[tokio::test]
    async fn panic_in_one_worker_does_not_affect_sibling() {
        let sup = WorkerSupervisor::new();

        let counter = Arc::new(AtomicU64::new(0));
        sup.spawn(CountingWorker {
            counter: Arc::clone(&counter),
        });
        sup.spawn(PanickingWorker);

        // Allow good worker to tick; bad worker panics immediately.
        tokio::time::sleep(Duration::from_millis(80)).await;

        let statuses = sup.statuses();
        assert_eq!(statuses.len(), 2);

        let good = statuses.iter().find(|s| s.name == "counting").unwrap();
        assert!(good.tick_count > 0, "good worker must keep running");

        let bad = statuses.iter().find(|s| s.name == "panicking").unwrap();
        assert!(bad.panic_count > 0, "panic should be caught and counted");
    }

    #[test]
    fn supervisor_is_clone_and_shares_state() {
        let sup = WorkerSupervisor::new();
        let sup2 = sup.clone();
        // Both point at the same status table.
        assert!(Arc::ptr_eq(&sup.statuses, &sup2.statuses));
    }
}
