//! CO-89: hourly incremental git-sync worker.
//!
//! Every tick, each universe with a `git_source` configured is fetched and its
//! new commits ingested (incremental from `git_last_synced_sha`). Runs the
//! blocking git work on `spawn_blocking` and never holds the storage lock across
//! it — the brief locks only fetch config + the per-universe store handle and
//! later advance the cursor. Per-universe failures are logged, never fatal (one
//! unreachable remote must not stall the others or the worker).

use std::time::Duration;

use async_trait::async_trait;

use crate::platform::worker_supervisor::Worker;
use crate::server::AppState;
use crate::telemetry::{CrudEvent, emit_crud_event};

/// Worker that incrementally syncs every git-backed universe once an hour.
#[derive(Clone)]
pub struct GitSyncWorker {
    state: AppState,
}

impl GitSyncWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Sync every git-backed universe once. Returns the number successfully
    /// synced. Exposed for the boot-time first run + tests.
    pub async fn sync_all(&self) -> usize {
        let targets = {
            let storage = self.state.0.core.storage.lock();
            storage.list_git_backed_universes()
        };
        let mut ok = 0;
        for (key, source, branch, since_sha) in targets {
            match self.sync_one(&key, &source, &branch, since_sha).await {
                Ok(()) => ok += 1,
                Err(e) => tracing::warn!("CO-89 git-sync failed for '{key}': {e}"),
            }
        }
        ok
    }

    async fn sync_one(
        &self,
        key: &str,
        source: &str,
        branch: &str,
        since_sha: Option<String>,
    ) -> anyhow::Result<()> {
        if crate::vcs::remote_needs_credentials(source) {
            anyhow::bail!("'{source}' needs credentials (CO_GIT_TOKEN / CO_GIT_SSH_KEY_PATH)");
        }
        let (store, dest) = {
            let storage = self.state.0.core.storage.lock();
            let store = storage.entry_store(key)?;
            let dest = storage.data_dir.join("git_sources").join(key);
            (store, dest)
        };
        let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let key_owned = key.to_string();
        let source_owned = source.to_string();
        let branch_owned = branch.to_string();
        let stats = tokio::task::spawn_blocking(move || {
            super::sync_universe_git(
                store.as_ref(),
                &key_owned,
                &source_owned,
                &branch_owned,
                &dest,
                since_sha.as_deref(),
                now_ns,
            )
        })
        .await??;

        if let Some(head) = &stats.head_sha {
            let now_iso = chrono::Utc::now().to_rfc3339();
            let storage = self.state.0.core.storage.lock();
            let _ = storage.set_universe_git_synced(key, head, &now_iso);
        }
        emit_crud_event(
            &self.state,
            CrudEvent {
                kind: "pipeline.git_sync",
                universe: key.to_string(),
                list: Some("commit".into()),
                key: stats.head_sha.clone(),
                actor: None,
                session_id: None,
                extra: Some(serde_json::json!({
                    "commits_ingested": stats.commits_ingested,
                    "profiles_written": stats.profiles_written,
                    "trigger": "hourly_worker",
                })),
            },
        );
        Ok(())
    }
}

#[async_trait]
impl Worker for GitSyncWorker {
    fn name(&self) -> &'static str {
        "git_sync"
    }

    fn interval(&self) -> Duration {
        // Hourly incremental sync (overridable later via config if needed).
        Duration::from_secs(3600)
    }

    fn initial_delay(&self) -> Duration {
        // Keep git work out of the boot path (it can clone full history on the
        // first run). First tick fires 2 minutes after start.
        Duration::from_secs(120)
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        let n = self.sync_all().await;
        if n > 0 {
            tracing::info!("CO-89: hourly git-sync completed for {n} universe(s)");
        }
        Ok(())
    }
}
