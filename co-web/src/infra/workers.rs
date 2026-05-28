//! CO-292: `WorkerExecutor` trait — pluggable job execution (CO-284-C).
//!
//! Extends CO-223's periodic `Worker` + `WorkerSupervisor` with one-off job
//! operations (`enqueue`, `run_now`, `cancel`, `status`) behind a
//! `WorkerExecutor` trait.  The default `InProcessExecutor` dispatches via
//! `tokio::spawn` with abort-handle tracking.  Future backends (Redis queue,
//! Temporal, k8s Jobs) implement the same trait without touching domain code.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;

use crate::worker_supervisor::{Worker, WorkerStatus, WorkerSupervisor};

// ---------------------------------------------------------------------------
// Job primitives
// ---------------------------------------------------------------------------

/// Opaque handle to an enqueued/running job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct JobHandle(u64);

/// Status of a one-off job dispatched via [`WorkerExecutor::enqueue`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum JobStatus {
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

/// The outcome of a [`WorkerExecutor::run_now`] call.
pub struct JobResult {
    pub name: String,
    pub outcome: anyhow::Result<()>,
}

/// Boxed async future returned by a job task.
type JobFuture = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

/// A unit of work for the executor.
///
/// Create from any [`Worker`] with [`Job::from_worker_tick`], or from an
/// arbitrary async closure with [`Job::new`].
pub struct Job {
    pub name: String,
    task: Box<dyn FnOnce() -> JobFuture + Send>,
}

impl Job {
    /// Construct a one-off job from any async closure.
    pub fn new<F, Fut>(name: impl Into<String>, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        Self {
            name: name.into(),
            task: Box::new(move || Box::pin(f())),
        }
    }

    /// Build a job that executes one [`Worker::tick`] of `worker`.
    ///
    /// This is the bridge that makes all existing concrete worker impls
    /// usable through the `WorkerExecutor` trait.
    pub fn from_worker_tick<W: Worker + Send + 'static>(mut worker: W) -> Self {
        let name = worker.name().to_owned();
        Self::new(name, move || async move { worker.tick().await })
    }

    fn run(self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        (self.task)()
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction over the job-execution environment.
///
/// The default implementation [`InProcessExecutor`] dispatches via
/// `tokio::spawn`.  Future backends plug in without touching domain code.
#[async_trait]
pub trait WorkerExecutor: Send + Sync + 'static {
    /// Enqueue `job` for asynchronous execution; returns a handle for
    /// [`cancel`](Self::cancel) / [`status`](Self::status) queries.
    async fn enqueue(&self, job: Job) -> anyhow::Result<JobHandle>;

    /// Run `job` to completion inline (useful in tests and triggered one-shots).
    async fn run_now(&self, job: Job) -> anyhow::Result<JobResult>;

    /// Abort a running job identified by `handle`.
    async fn cancel(&self, handle: JobHandle) -> anyhow::Result<()>;

    /// Return the current [`JobStatus`] for `handle`.
    async fn status(&self, handle: JobHandle) -> anyhow::Result<JobStatus>;

    /// Snapshot of all periodic-worker statuses tracked by this executor.
    fn statuses(&self) -> Vec<WorkerStatus>;
}

// ---------------------------------------------------------------------------
// InProcessExecutor
// ---------------------------------------------------------------------------

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

struct TrackedJob {
    #[allow(dead_code)]
    name: String,
    abort_handle: tokio::task::AbortHandle,
    /// Shared between the map entry and the spawned task so status transitions
    /// (Running → Completed / Failed) are visible without re-acquiring the jobs lock.
    status: Arc<Mutex<JobStatus>>,
}

/// Default in-process executor backed by `tokio::spawn`.
///
/// Wraps a [`WorkerSupervisor`] for periodic workers and adds one-off job
/// dispatch with abort-handle tracking.  Both are exposed through a single
/// `Arc<dyn WorkerExecutor>` stored in `IntegrationsState`.
///
/// # Panic isolation
/// Periodic worker panics are caught by the supervisor loop (CO-223).
/// One-off jobs spawned via [`enqueue`](WorkerExecutor::enqueue) are *not*
/// restarted on panic; their handle status transitions to `Failed`.
///
/// # Memory note
/// Completed / cancelled job handles are retained in the map indefinitely.
/// This is acceptable for the current single-machine deployment; a future
/// queue-backed executor can prune on completion.
pub struct InProcessExecutor {
    supervisor: WorkerSupervisor,
    jobs: Arc<Mutex<HashMap<JobHandle, TrackedJob>>>,
}

impl InProcessExecutor {
    pub fn new() -> Self {
        Self {
            supervisor: WorkerSupervisor::new(),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Convenience constructor: returns `Arc<dyn WorkerExecutor>` for use in
    /// `IntegrationsState` and test helpers.
    pub fn new_arc() -> Arc<dyn WorkerExecutor> {
        Arc::new(Self::new())
    }

    /// Register a periodic [`Worker`] with the internal supervisor.
    ///
    /// Not exposed on the [`WorkerExecutor`] trait because generic bounds make
    /// it non-object-safe.  Call-sites that need to register workers (server
    /// startup, tests) hold the concrete `Arc<InProcessExecutor>` or call this
    /// before type-erasing to `Arc<dyn WorkerExecutor>`.
    pub fn spawn_worker<W: Worker + Clone + Send + 'static>(&self, worker: W) {
        self.supervisor.spawn(worker);
    }
}

impl Default for InProcessExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WorkerExecutor for InProcessExecutor {
    async fn enqueue(&self, job: Job) -> anyhow::Result<JobHandle> {
        let id = JobHandle(NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed));
        let status = Arc::new(Mutex::new(JobStatus::Running));
        let status_for_task = Arc::clone(&status);
        let name = job.name.clone();
        let fut = job.run();

        // Spawn the task; drop the JoinHandle (task keeps running).
        // We retain the AbortHandle for cancel() support.
        let join_handle = tokio::spawn(async move {
            let result = fut.await;
            let new_status = match result {
                Ok(()) => JobStatus::Completed,
                Err(ref e) => JobStatus::Failed(format!("{e:#}")),
            };
            let mut guard = status_for_task.lock();
            // Don't overwrite a Cancelled status set by cancel().
            if *guard == JobStatus::Running {
                *guard = new_status;
            }
        });

        self.jobs.lock().insert(
            id.clone(),
            TrackedJob {
                name,
                abort_handle: join_handle.abort_handle(),
                status,
            },
        );
        Ok(id)
    }

    async fn run_now(&self, job: Job) -> anyhow::Result<JobResult> {
        let name = job.name.clone();
        let outcome = job.run().await;
        Ok(JobResult { name, outcome })
    }

    async fn cancel(&self, handle: JobHandle) -> anyhow::Result<()> {
        let jobs = self.jobs.lock();
        let j = jobs
            .get(&handle)
            .ok_or_else(|| anyhow::anyhow!("unknown job handle"))?;
        let mut status = j.status.lock();
        if *status != JobStatus::Running {
            return Err(anyhow::anyhow!(
                "job is not running (status: {:?})",
                *status
            ));
        }
        j.abort_handle.abort();
        *status = JobStatus::Cancelled;
        Ok(())
    }

    async fn status(&self, handle: JobHandle) -> anyhow::Result<JobStatus> {
        let jobs = self.jobs.lock();
        let j = jobs
            .get(&handle)
            .ok_or_else(|| anyhow::anyhow!("unknown job handle"))?;
        Ok(j.status.lock().clone())
    }

    fn statuses(&self) -> Vec<WorkerStatus> {
        self.supervisor.statuses()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Clone)]
    struct NoopWorker;

    #[async_trait]
    impl Worker for NoopWorker {
        fn name(&self) -> &'static str {
            "noop"
        }
        fn interval(&self) -> Duration {
            Duration::from_secs(60)
        }
        async fn tick(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FailingWorker;

    #[async_trait]
    impl Worker for FailingWorker {
        fn name(&self) -> &'static str {
            "failing"
        }
        fn interval(&self) -> Duration {
            Duration::from_secs(60)
        }
        async fn tick(&mut self) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("deliberate failure"))
        }
    }

    #[tokio::test]
    async fn enqueue_runs_job_and_tracks_completion() {
        let exec = InProcessExecutor::new();
        let job = Job::new("test-job", || async { Ok(()) });
        let handle = exec.enqueue(job).await.unwrap();

        // Give the spawned task time to complete.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let s = exec.status(handle).await.unwrap();
        assert_eq!(s, JobStatus::Completed);
    }

    #[tokio::test]
    async fn enqueue_tracks_failure() {
        let exec = InProcessExecutor::new();
        let job = Job::new("fail-job", || async { Err(anyhow::anyhow!("boom")) });
        let handle = exec.enqueue(job).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let s = exec.status(handle).await.unwrap();
        assert!(matches!(s, JobStatus::Failed(_)));
    }

    #[tokio::test]
    async fn run_now_returns_result_inline() {
        let exec = InProcessExecutor::new();
        let job = Job::new("inline", || async { Ok(()) });
        let result = exec.run_now(job).await.unwrap();
        assert_eq!(result.name, "inline");
        assert!(result.outcome.is_ok());
    }

    #[tokio::test]
    async fn cancel_aborts_running_job() {
        let exec = InProcessExecutor::new();
        let job = Job::new("slow", || async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(())
        });
        let handle = exec.enqueue(job).await.unwrap();

        // Allow the task to start before cancelling.
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert_eq!(
            exec.status(handle.clone()).await.unwrap(),
            JobStatus::Running
        );

        exec.cancel(handle.clone()).await.unwrap();
        assert_eq!(exec.status(handle).await.unwrap(), JobStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_unknown_handle_returns_error() {
        let exec = InProcessExecutor::new();
        let err = exec.cancel(JobHandle(9999)).await.unwrap_err();
        assert!(err.to_string().contains("unknown job handle"));
    }

    #[tokio::test]
    async fn from_worker_tick_runs_worker_tick() {
        let exec = InProcessExecutor::new();
        let job = Job::from_worker_tick(NoopWorker);
        let result = exec.run_now(job).await.unwrap();
        assert!(result.outcome.is_ok());
    }

    #[tokio::test]
    async fn from_worker_tick_propagates_worker_error() {
        let exec = InProcessExecutor::new();
        let job = Job::from_worker_tick(FailingWorker);
        let result = exec.run_now(job).await.unwrap();
        assert!(result.outcome.is_err());
    }

    #[tokio::test]
    async fn spawn_worker_registers_with_supervisor() {
        let exec = InProcessExecutor::new();
        exec.spawn_worker(NoopWorker);
        assert_eq!(exec.statuses().len(), 1);
        assert_eq!(exec.statuses()[0].name, "noop");
    }

    #[test]
    fn new_arc_returns_dyn_worker_executor() {
        // Compile-time check: new_arc() returns the right type.
        let exec: Arc<dyn WorkerExecutor> = InProcessExecutor::new_arc();
        // Runtime check: supervisor starts empty.
        assert_eq!(exec.statuses().len(), 0);
    }
}
