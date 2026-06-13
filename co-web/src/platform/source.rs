//! CO-435: source adapters — the extension seam for content sources.
//!
//! Before CO-435 a universe's content source was hardwired: remote-git sync was
//! a free function in `seed_orchestrator` calling [`crate::vcs`] directly, and
//! the Yggdrasil event-bus sync was a bespoke EDA subscriber. Adding GitLab,
//! Notion, or any new source meant editing those sites.
//!
//! This module turns that into a `registry + trait + impl` seam:
//!
//!   - [`SourceAdapter`] — the trait every source implements (`kind` + `sync`).
//!   - [`GitSourceAdapter`] — the `remote-git` impl (consolidates CO-417/CO-423),
//!     wrapping the shallow clone/pull in [`crate::vcs`].
//!   - [`EventBusSourceAdapter`] — the `event-bus` impl: Yggdrasil's lexicon /
//!     notes sync is push-driven by the `comunicacao_live` / `yggdrasil_notes`
//!     EDA subscribers, so there is no pull step.
//!   - [`SourceRegistry`] — resolves an adapter by its `source_kind`.
//!
//! Adding a source is now: `impl SourceAdapter` + `registry.register(...)`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

/// The result of a single source sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSyncOutcome {
    /// Content was fetched into the destination.
    Synced,
    /// Nothing to pull — this source is push-driven (e.g. the event bus).
    PushDriven,
    /// Skipped because the remote needs credentials none were configured for.
    SkippedNoCredentials,
}

/// A request to sync one universe's source.
pub struct SourceSyncRequest<'a> {
    /// The universe whose content is being synced.
    pub universe_key: &'a str,
    /// The remote location (git URL, API base, …).
    pub remote_url: &'a str,
    /// The ref/branch/tag/SHA to sync (git), or source-specific selector.
    pub ref_name: &'a str,
    /// Local destination directory the content is materialised into.
    pub dest: &'a Path,
}

/// A pluggable content source.
///
/// `kind` is the `source_kind` stored on the universe row; the registry uses it
/// to resolve the adapter. `sync` fetches/refreshes content for one universe.
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    /// The `source_kind` this adapter handles (e.g. `"remote-git"`).
    fn kind(&self) -> &'static str;

    /// Fetch/sync content for the universe described by `req`.
    async fn sync(&self, req: &SourceSyncRequest<'_>) -> anyhow::Result<SourceSyncOutcome>;
}

// ---------------------------------------------------------------------------
// GitSourceAdapter — the `remote-git` default (CO-417/CO-423 + CO-337)
// ---------------------------------------------------------------------------

/// Syncs content from a remote git repository via a shallow clone/pull.
///
/// Consolidates the `co source add github` adapter (CO-417/CO-423) and the
/// remote sister-repo sync (CO-337) onto one seam. The actual git work lives in
/// [`crate::vcs`]; this adapter is the policy/credential wrapper around it.
pub struct GitSourceAdapter;

impl GitSourceAdapter {
    /// Synchronous clone/pull (shells out to `git`).
    ///
    /// Exposed separately from the async trait method because the boot and
    /// 15-min worker seed path (`run_remote_sister_repo_seeds`) run in a
    /// synchronous context. Returns [`SourceSyncOutcome::SkippedNoCredentials`]
    /// (rather than erroring) when a network remote has no configured
    /// credentials — matching the CO-408 quiet-skip behaviour.
    pub fn sync_blocking(
        &self,
        remote_url: &str,
        ref_name: &str,
        dest: &Path,
    ) -> anyhow::Result<SourceSyncOutcome> {
        if crate::vcs::remote_needs_credentials(remote_url) {
            return Ok(SourceSyncOutcome::SkippedNoCredentials);
        }
        crate::vcs::sync_repo(remote_url, ref_name, dest)?;
        Ok(SourceSyncOutcome::Synced)
    }
}

#[async_trait]
impl SourceAdapter for GitSourceAdapter {
    fn kind(&self) -> &'static str {
        "remote-git"
    }

    async fn sync(&self, req: &SourceSyncRequest<'_>) -> anyhow::Result<SourceSyncOutcome> {
        self.sync_blocking(req.remote_url, req.ref_name, req.dest)
    }
}

// ---------------------------------------------------------------------------
// EventBusSourceAdapter — the `event-bus` source (CO-383/CO-389)
// ---------------------------------------------------------------------------

/// Represents a push-driven event-bus source (e.g. Yggdrasil → `comunicacao`).
///
/// There is no pull step: content arrives as EDA events handled by the
/// `comunicacao_live` and `yggdrasil_notes` subscribers (CO-383/CO-389). The
/// adapter exists so this source is a first-class registry entry alongside git.
pub struct EventBusSourceAdapter;

#[async_trait]
impl SourceAdapter for EventBusSourceAdapter {
    fn kind(&self) -> &'static str {
        "event-bus"
    }

    async fn sync(&self, _req: &SourceSyncRequest<'_>) -> anyhow::Result<SourceSyncOutcome> {
        // Push-driven: nothing to pull. The EDA subscribers ingest events live.
        Ok(SourceSyncOutcome::PushDriven)
    }
}

// ---------------------------------------------------------------------------
// SourceRegistry
// ---------------------------------------------------------------------------

/// Resolves a [`SourceAdapter`] by its `source_kind`.
#[derive(Default)]
pub struct SourceRegistry {
    adapters: HashMap<&'static str, Arc<dyn SourceAdapter>>,
}

impl SourceRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    /// Register an adapter under its [`SourceAdapter::kind`].
    pub fn register(&mut self, adapter: Arc<dyn SourceAdapter>) -> &mut Self {
        self.adapters.insert(adapter.kind(), adapter);
        self
    }

    /// Resolve the adapter for `kind`, if registered.
    pub fn get(&self, kind: &str) -> Option<Arc<dyn SourceAdapter>> {
        self.adapters.get(kind).cloned()
    }

    /// All registered source kinds.
    pub fn kinds(&self) -> Vec<&'static str> {
        self.adapters.keys().copied().collect()
    }

    /// The default registry: `remote-git` + `event-bus`.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry
            .register(Arc::new(GitSourceAdapter))
            .register(Arc::new(EventBusSourceAdapter));
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve_git_and_event_bus() {
        let registry = SourceRegistry::with_defaults();
        assert_eq!(registry.get("remote-git").unwrap().kind(), "remote-git");
        assert_eq!(registry.get("event-bus").unwrap().kind(), "event-bus");
        assert!(registry.get("notion").is_none());
        let mut kinds = registry.kinds();
        kinds.sort_unstable();
        assert_eq!(kinds, vec!["event-bus", "remote-git"]);
    }

    #[tokio::test]
    async fn event_bus_source_is_push_driven() {
        let adapter = EventBusSourceAdapter;
        let dir = tempfile::tempdir().unwrap();
        let req = SourceSyncRequest {
            universe_key: "comunicacao",
            remote_url: "wss://yggdrasil.example/api/v1/events",
            ref_name: "",
            dest: dir.path(),
        };
        assert_eq!(
            adapter.sync(&req).await.unwrap(),
            SourceSyncOutcome::PushDriven
        );
    }

    /// Adding a new source = a new `impl SourceAdapter` + `register(...)`.
    /// No edits to the boot or the registry's `with_defaults`.
    #[tokio::test]
    async fn new_source_plugs_in_without_touching_defaults() {
        struct NotionSourceAdapter;

        #[async_trait]
        impl SourceAdapter for NotionSourceAdapter {
            fn kind(&self) -> &'static str {
                "notion"
            }
            async fn sync(
                &self,
                _req: &SourceSyncRequest<'_>,
            ) -> anyhow::Result<SourceSyncOutcome> {
                Ok(SourceSyncOutcome::Synced)
            }
        }

        let mut registry = SourceRegistry::with_defaults();
        registry.register(Arc::new(NotionSourceAdapter));

        let dir = tempfile::tempdir().unwrap();
        let adapter = registry.get("notion").expect("notion registered");
        let req = SourceSyncRequest {
            universe_key: "mse",
            remote_url: "https://api.notion.com/v1/...",
            ref_name: "",
            dest: dir.path(),
        };
        assert_eq!(adapter.sync(&req).await.unwrap(), SourceSyncOutcome::Synced);
    }
}
