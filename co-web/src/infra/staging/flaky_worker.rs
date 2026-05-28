//! `RetryProneWorkerExecutor` — makes a fraction of `enqueue` calls fail.
//!
//! When `enqueue` is chosen to fail (every Nth call at the configured error_rate),
//! it returns `Err` without dispatching the job.  This forces callers to exercise
//! their retry / error-handling paths — the same paths that fire in production
//! when the underlying queue or executor is temporarily unavailable.
//!
//! `run_now`, `cancel`, `status`, and `statuses` are always forwarded unchanged.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

use crate::infra::workers::{Job, JobHandle, JobResult, JobStatus, WorkerExecutor};
use crate::worker_supervisor::WorkerStatus;

/// Wraps any `WorkerExecutor` and makes `error_rate` of `enqueue` calls fail.
pub struct RetryProneWorkerExecutor {
    inner: Arc<dyn WorkerExecutor>,
    fail_every_n: u64,
    call_count: AtomicU64,
}

impl RetryProneWorkerExecutor {
    /// Create a new `RetryProneWorkerExecutor`.
    ///
    /// `error_rate` is clamped to `[0.0, 1.0]`.
    /// Pass `0.0` to make this a transparent pass-through.
    pub fn new(inner: Arc<dyn WorkerExecutor>, error_rate: f64) -> Self {
        let rate = error_rate.clamp(0.0, 1.0);
        let fail_every_n = if rate <= 0.0 {
            0
        } else {
            (1.0 / rate).round() as u64
        };
        Self {
            inner,
            fail_every_n,
            call_count: AtomicU64::new(0),
        }
    }

    fn should_fail(&self) -> bool {
        if self.fail_every_n == 0 {
            return false;
        }
        let n = self.call_count.fetch_add(1, Ordering::Relaxed) + 1;
        n.is_multiple_of(self.fail_every_n)
    }
}

#[async_trait]
impl WorkerExecutor for RetryProneWorkerExecutor {
    async fn enqueue(&self, job: Job) -> anyhow::Result<JobHandle> {
        if self.should_fail() {
            return Err(anyhow::anyhow!(
                "staging: simulated enqueue failure for job '{}' (retry-path coverage)",
                job.name
            ));
        }
        self.inner.enqueue(job).await
    }

    async fn run_now(&self, job: Job) -> anyhow::Result<JobResult> {
        self.inner.run_now(job).await
    }

    async fn cancel(&self, handle: JobHandle) -> anyhow::Result<()> {
        self.inner.cancel(handle).await
    }

    async fn status(&self, handle: JobHandle) -> anyhow::Result<JobStatus> {
        self.inner.status(handle).await
    }

    fn statuses(&self) -> Vec<WorkerStatus> {
        self.inner.statuses()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::workers::InProcessExecutor;

    fn make_inner() -> Arc<dyn WorkerExecutor> {
        InProcessExecutor::new_arc()
    }

    fn noop_job(name: &str) -> Job {
        let name = name.to_string();
        Job::new(name, || async { Ok(()) })
    }

    #[tokio::test]
    async fn rate_zero_never_fails() {
        let inner = make_inner();
        let exec = RetryProneWorkerExecutor::new(inner, 0.0);
        for _ in 0..50 {
            assert!(exec.enqueue(noop_job("noop")).await.is_ok());
        }
    }

    #[tokio::test]
    async fn rate_one_always_fails_enqueue() {
        let inner = make_inner();
        let exec = RetryProneWorkerExecutor::new(inner, 1.0);
        let result = exec.enqueue(noop_job("fail")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("staging:"));
    }

    #[tokio::test]
    async fn rate_05_fails_every_20_enqueues() {
        let inner = make_inner();
        let exec = RetryProneWorkerExecutor::new(inner, 0.05);
        let mut failures = 0usize;
        for _ in 0..100 {
            if exec.enqueue(noop_job("test")).await.is_err() {
                failures += 1;
            }
        }
        assert_eq!(
            failures, 5,
            "expected 5 failures in 100 enqueues at rate=0.05"
        );
    }

    #[tokio::test]
    async fn run_now_always_passes_through() {
        let inner = make_inner();
        let exec = RetryProneWorkerExecutor::new(inner, 1.0);
        let result = exec.run_now(noop_job("inline")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn statuses_delegates_to_inner() {
        let inner = make_inner();
        let exec = RetryProneWorkerExecutor::new(inner, 0.0);
        assert_eq!(exec.statuses().len(), 0);
    }

    #[tokio::test]
    async fn implements_worker_executor_trait() {
        let inner = make_inner();
        let _: Arc<dyn WorkerExecutor> = Arc::new(RetryProneWorkerExecutor::new(inner, 0.0));
    }
}
