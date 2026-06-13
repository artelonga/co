//! CO-223: Concrete `Worker` implementations for all five background workers.
//!
//! Each struct wraps an `AppState` (or a subset of it) and delegates to the
//! per-module tick functions. The `WorkerSupervisor` in `AppStateInner` owns
//! these and manages their lifecycle.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::notification_providers::ChannelProvider;
use crate::server::AppState;
use crate::worker_supervisor::Worker;

// ---------------------------------------------------------------------------
// 1. EmbeddingWorker
//    The actual CPU-bound inference runs on a dedicated OS thread (see
//    `embedding_worker::spawn`). This async worker monitors the channel: a
//    `Probe` job confirms the OS thread is still alive and draining.
// ---------------------------------------------------------------------------

/// Health-monitor for the OS-thread embedding worker.
#[derive(Clone)]
pub struct EmbeddingWorker {
    tx: crate::embedding_worker::EmbeddingSender,
}

impl EmbeddingWorker {
    pub fn new(tx: crate::embedding_worker::EmbeddingSender) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl Worker for EmbeddingWorker {
    fn name(&self) -> &'static str {
        "embedding"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        use std::sync::mpsc::TrySendError;
        match self
            .tx
            .try_send(crate::embedding_worker::EmbeddingJob::Probe)
        {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => {
                Err(anyhow::anyhow!("embedding OS thread disconnected"))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2. EmailWorker
//    Drives the 60-second email digest loop (CO-200).
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct EmailWorker {
    state: AppState,
    failure_counts: HashMap<String, u32>,
}

impl EmailWorker {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            failure_counts: HashMap::new(),
        }
    }
}

#[async_trait]
impl Worker for EmailWorker {
    fn name(&self) -> &'static str {
        "notification_email"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        crate::notification_email_worker::tick(&self.state, &mut self.failure_counts).await
    }
}

// ---------------------------------------------------------------------------
// 3. PushWorker
//    Drives the 10-second web-push delivery loop (CO-201).
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct PushWorker {
    state: AppState,
}

impl PushWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Worker for PushWorker {
    fn name(&self) -> &'static str {
        "notification_push"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        crate::notification_push_worker::tick(&self.state, Utc::now()).await
    }
}

// ---------------------------------------------------------------------------
// 4. WebhookWorker
//    Drives the 5-second webhook/email/WhatsApp delivery loop (CO-168/169).
//    The reqwest client and channel providers are initialised once in `new`
//    and reused across ticks for connection pooling.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct WebhookWorker {
    state: AppState,
    client: reqwest::Client,
    email_provider: Option<Arc<dyn ChannelProvider>>,
    whatsapp_provider: Option<Arc<dyn ChannelProvider>>,
}

impl WebhookWorker {
    pub fn new(state: AppState) -> anyhow::Result<Self> {
        let (client, email_provider, whatsapp_provider) =
            crate::webhook_worker::build_client_and_providers()?;
        Ok(Self {
            state,
            client,
            email_provider,
            whatsapp_provider,
        })
    }
}

#[async_trait]
impl Worker for WebhookWorker {
    fn name(&self) -> &'static str {
        "webhook"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(5)
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        crate::webhook_worker::tick(
            &self.state,
            &self.client,
            self.email_provider.as_ref(),
            self.whatsapp_provider.as_ref(),
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// 5. JobQueueWorker
//    Drives the 3-second doc-gen job queue loop (CO-72/78).
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct JobQueueWorker {
    state: AppState,
}

impl JobQueueWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Worker for JobQueueWorker {
    fn name(&self) -> &'static str {
        "job_queue"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(crate::job_queue::POLL_INTERVAL_SECS)
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        crate::job_queue::tick(&self.state).await
    }
}

// ---------------------------------------------------------------------------
// 6. DeploymentSnapshotWorker
//    Probes each deployable unit's Fly.io machines API and /api/health every
//    5 minutes and persists the result to `deployment_snapshots` (CO-273).
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DeploymentSnapshotWorker {
    state: AppState,
}

impl DeploymentSnapshotWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Worker for DeploymentSnapshotWorker {
    fn name(&self) -> &'static str {
        "deployment_snapshot"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(300) // 5 minutes
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        crate::deployment_snapshot_worker::tick(&self.state).await
    }
}

// ---------------------------------------------------------------------------
// 7. ReleaseNotesWorker
//    Parses sister-repo CHANGELOG.md files every 5 minutes and upserts the
//    parsed releases into `release_notes` (CO-334).
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ReleaseNotesWorker {
    state: AppState,
}

impl ReleaseNotesWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Worker for ReleaseNotesWorker {
    fn name(&self) -> &'static str {
        "release_notes_refresh"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(300) // 5 minutes
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        crate::changelog_routes::run_release_notes_refresh(&self.state).await
    }
}

// ---------------------------------------------------------------------------
// 8. BackupWorker
//    Runs every CO_BACKUP_INTERVAL_HOURS (default 24h). Creates a snapshot
//    tarball, calls backend.put(), then prunes old snapshots (CO-365).
//    CO-405: first tick is deferred CO_BACKUP_BOOT_DELAY_SECS (default 10 min)
//    so the snapshot never sits in the boot path — on 2026-06-11 it blocked
//    the HTTP bind >6 min on a 1-vCPU machine while health checks failed.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct BackupWorker {
    state: AppState,
    interval: std::time::Duration,
    boot_delay: std::time::Duration,
}

impl BackupWorker {
    pub fn new(state: AppState) -> Self {
        let interval_hours = state.core.server_config.backup_interval_hours;
        let boot_delay_secs = state.core.server_config.backup_boot_delay_secs;
        Self {
            state,
            interval: std::time::Duration::from_secs(interval_hours * 3600),
            boot_delay: std::time::Duration::from_secs(boot_delay_secs),
        }
    }
}

#[async_trait]
impl Worker for BackupWorker {
    fn name(&self) -> &'static str {
        "backup"
    }

    fn interval(&self) -> std::time::Duration {
        self.interval
    }

    fn initial_delay(&self) -> std::time::Duration {
        self.boot_delay
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        crate::storage::backup::worker::run_backup_tick(&self.state).await
    }
}

// ---------------------------------------------------------------------------
// 9. RemoteSisterRepoWorker
//    Clones/pulls remote sister repos and reseeds their content (CO-337).
//    Default interval: 15 min. Override with CO_REMOTE_SYNC_INTERVAL_SECS.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct RemoteSisterRepoWorker {
    config: crate::config::WebConfig,
    interval_secs: u64,
}

impl RemoteSisterRepoWorker {
    pub fn new(config: crate::config::WebConfig) -> Self {
        use crate::infra::secrets::SecretsProviderExt;
        let interval_secs =
            crate::infra::secrets::global().get_parsed("CO_REMOTE_SYNC_INTERVAL_SECS", 900); // 15 minutes
        Self {
            config,
            interval_secs,
        }
    }
}

#[async_trait]
impl Worker for RemoteSisterRepoWorker {
    fn name(&self) -> &'static str {
        "remote_sister_repo_sync"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        crate::server::seed_orchestrator::run_remote_sister_repo_seeds(&self.config);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 9. StagingTestSweepWorker
//    CO-379: weekly Sunday 03:00 BRT sweep of u-test-* universes older than
//    7 days. Ticks hourly; skips if the schedule doesn't match or the sweep
//    already ran today. Only registered when CO_ENV=staging.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct StagingTestSweepWorker {
    config: crate::config::WebConfig,
    last_sweep_date: Option<chrono::NaiveDate>,
}

impl StagingTestSweepWorker {
    pub fn new(config: crate::config::WebConfig) -> Self {
        Self {
            config,
            last_sweep_date: None,
        }
    }
}

#[async_trait]
impl Worker for StagingTestSweepWorker {
    fn name(&self) -> &'static str {
        "staging_test_sweep"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(3600) // check every hour
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        use chrono::{Datelike, Timelike, Utc, Weekday};

        let now = Utc::now();
        let today = now.date_naive();

        // Sunday 06:00 UTC = 03:00 BRT (UTC-3). Skip if it's not the window.
        if now.weekday() != Weekday::Sun || now.hour() != 6 {
            return Ok(());
        }

        // Only sweep once per calendar day even if the worker restarts.
        if self.last_sweep_date == Some(today) {
            return Ok(());
        }

        let mut storage = crate::storage::Storage::new(&self.config.data_dir);
        let n = storage.sweep_test_namespaces(7, 100);
        tracing::info!(
            "CO-379: staging test namespace sweep: deleted {} u-test-* universe(s)",
            n
        );

        self.last_sweep_date = Some(today);
        Ok(())
    }
}
