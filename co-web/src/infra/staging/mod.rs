//! CO-298: Staging-mode simulation decorators.
//!
//! Each plug wraps a production infra trait impl and adds configurable fault
//! or latency simulation to drive pre-deploy verification:
//!
//! - [`LatencyInjectedStorage`] — adds delay to every `Storage` call
//! - [`FlakyBlobStore`] — injects 503s on a fraction of blob ops
//! - [`EvictingCache`] — simulates eviction pressure on any `Cache<K,V>`
//! - [`RetryProneWorkerExecutor`] — makes a fraction of `enqueue` calls fail
//!
//! Enable all four with `co serve --staging`, or mix-and-match per plug:
//!   `CO_STAGING_LATENCY=false`        to disable latency injection
//!   `CO_STAGING_FAULT_INJECTION=false` to disable blob fault injection
//!   `CO_STAGING_EVICTION=false`        to disable cache eviction pressure
//!   `CO_STAGING_WORKER_FAILURE=false`  to disable worker failure injection

pub mod evicting_cache;
pub mod flaky_blob;
pub mod flaky_worker;
pub mod latency;

pub use evicting_cache::EvictingCache;
pub use flaky_blob::FlakyBlobStore;
pub use flaky_worker::RetryProneWorkerExecutor;
pub use latency::LatencyInjectedStorage;

/// Configuration for staging-mode decorators.
///
/// Build from CLI values with [`StagingConfig::from_cli`]; opt out individual
/// plugs by setting the corresponding env var to `"false"` or `"0"`.
#[derive(Clone, Debug)]
pub struct StagingConfig {
    /// Delay added to every storage call and HTTP request (milliseconds).
    pub latency_ms: u64,
    /// Fraction of blob ops that return an error (0.0 = never, 1.0 = always).
    pub error_rate: f64,
    /// Enable latency injection on the Storage plug.
    pub latency_enabled: bool,
    /// Enable fault injection on the BlobStore plug.
    pub fault_enabled: bool,
    /// Enable eviction pressure on the Cache plug.
    pub eviction_enabled: bool,
    /// Enable worker-failure injection on the WorkerExecutor plug.
    pub worker_failure_enabled: bool,
}

impl StagingConfig {
    /// Construct from CLI latency / error-rate values.
    ///
    /// Each plug is **enabled by default** when `--staging` is active.
    /// Set the corresponding env var to `"false"` or `"0"` to opt out:
    ///
    /// | Env var                       | Plug                           |
    /// |-------------------------------|--------------------------------|
    /// | `CO_STAGING_LATENCY`          | [`LatencyInjectedStorage`]     |
    /// | `CO_STAGING_FAULT_INJECTION`  | [`FlakyBlobStore`]             |
    /// | `CO_STAGING_EVICTION`         | [`EvictingCache`]              |
    /// | `CO_STAGING_WORKER_FAILURE`   | [`RetryProneWorkerExecutor`]   |
    pub fn from_cli(latency_ms: u64, error_rate: f64) -> Self {
        fn plug_enabled(var: &str) -> bool {
            std::env::var(var)
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true)
        }
        Self {
            latency_ms,
            error_rate,
            latency_enabled: plug_enabled("CO_STAGING_LATENCY"),
            fault_enabled: plug_enabled("CO_STAGING_FAULT_INJECTION"),
            eviction_enabled: plug_enabled("CO_STAGING_EVICTION"),
            worker_failure_enabled: plug_enabled("CO_STAGING_WORKER_FAILURE"),
        }
    }

    /// Build from `WebConfig` — used by `start_server_inner`.
    ///
    /// When `staging_enabled` is true (i.e. `--staging` was passed), all plugs
    /// are on by default and can be opted out via env vars (opt-out semantics).
    ///
    /// When `staging_enabled` is false, individual plugs can still be activated
    /// by setting the corresponding env var to `"true"` or `"1"` (opt-in semantics):
    ///
    /// | Env var                      | Plug                          |
    /// |------------------------------|-------------------------------|
    /// | `CO_STAGING_LATENCY=true`    | [`LatencyInjectedStorage`]    |
    /// | `CO_STAGING_FAULT_INJECTION=true` | [`FlakyBlobStore`]       |
    /// | `CO_STAGING_EVICTION=true`   | [`EvictingCache`]             |
    /// | `CO_STAGING_WORKER_FAILURE=true` | [`RetryProneWorkerExecutor`] |
    pub fn from_config(staging_enabled: bool, latency_ms: u64, error_rate: f64) -> Self {
        if staging_enabled {
            // Opt-out: all plugs enabled unless explicitly set to "false"/"0".
            return Self::from_cli(latency_ms, error_rate);
        }

        // Opt-in: each plug disabled unless explicitly set to "true"/"1".
        fn plug_on(var: &str) -> bool {
            std::env::var(var)
                .ok()
                .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
        }

        Self {
            latency_ms,
            error_rate,
            latency_enabled: plug_on("CO_STAGING_LATENCY"),
            fault_enabled: plug_on("CO_STAGING_FAULT_INJECTION"),
            eviction_enabled: plug_on("CO_STAGING_EVICTION"),
            worker_failure_enabled: plug_on("CO_STAGING_WORKER_FAILURE"),
        }
    }

    /// Returns `true` if any simulation plug is active.
    pub fn is_active(&self) -> bool {
        self.latency_enabled
            || self.fault_enabled
            || self.eviction_enabled
            || self.worker_failure_enabled
    }
}

impl Default for StagingConfig {
    fn default() -> Self {
        Self {
            latency_ms: 50,
            error_rate: 0.05,
            latency_enabled: true,
            fault_enabled: true,
            eviction_enabled: true,
            worker_failure_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_all_plugs_enabled() {
        let cfg = StagingConfig::default();
        assert!(cfg.latency_enabled);
        assert!(cfg.fault_enabled);
        assert!(cfg.eviction_enabled);
        assert!(cfg.worker_failure_enabled);
        assert_eq!(cfg.error_rate, 0.05);
        assert_eq!(cfg.latency_ms, 50);
    }

    #[test]
    fn from_cli_passes_values() {
        let cfg = StagingConfig::from_cli(30, 0.10);
        assert_eq!(cfg.latency_ms, 30);
        assert!((cfg.error_rate - 0.10).abs() < f64::EPSILON);
    }
}
