use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Abstraction for reading runtime secrets.
///
/// Production code uses `EnvSecretsProvider` (reads `std::env::var`).
/// Tests inject `StaticSecretsProvider` to avoid mutating the process
/// environment, which eliminates parallel-test races on `JWT_SECRET` and
/// other secrets.
pub trait SecretsProvider: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;

    /// Returns `true` when the named secret is present (and, for `get`, would
    /// yield `Some`). Used by "is this integration configured?" checks that
    /// previously called `std::env::var(..).is_ok()`.
    fn is_set(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Returns the secret, or `default` when absent. Empty values are returned
    /// verbatim (matching `std::env::var` semantics) — use
    /// [`get_nonempty`](Self::get_nonempty) to treat empty as absent.
    fn get_or(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or_else(|| default.to_string())
    }

    /// Like [`get`](Self::get) but treats an empty string as absent.
    fn get_nonempty(&self, name: &str) -> Option<String> {
        self.get(name).filter(|v| !v.is_empty())
    }

    /// Read a boolean flag. `default` is returned when the value is absent.
    /// When present, `"1"` and `"true"` (case-insensitive) are true; everything
    /// else is false.
    fn get_bool(&self, name: &str, default: bool) -> bool {
        match self.get(name) {
            Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
            None => default,
        }
    }
}

/// Generic typed accessors for any [`SecretsProvider`] (including `dyn`).
///
/// Kept out of the base trait so `SecretsProvider` stays object-safe (a generic
/// method would otherwise make `dyn SecretsProvider` impossible).
pub trait SecretsProviderExt {
    /// Parse the named value as `T`, falling back to `default` when the value
    /// is absent or fails to parse.
    fn get_parsed<T: std::str::FromStr>(&self, name: &str, default: T) -> T;
}

impl<P: SecretsProvider + ?Sized> SecretsProviderExt for P {
    fn get_parsed<T: std::str::FromStr>(&self, name: &str, default: T) -> T {
        self.get(name)
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(default)
    }
}

// ---------------------------------------------------------------------------
// EnvSecretsProvider — production default
// ---------------------------------------------------------------------------

pub struct EnvSecretsProvider;

impl SecretsProvider for EnvSecretsProvider {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

// ---------------------------------------------------------------------------
// StaticSecretsProvider — test fixture
// ---------------------------------------------------------------------------

pub struct StaticSecretsProvider {
    secrets: HashMap<String, String>,
}

impl StaticSecretsProvider {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<K, V>(pairs: impl IntoIterator<Item = (K, V)>) -> Arc<dyn SecretsProvider>
    where
        K: Into<String>,
        V: Into<String>,
    {
        Arc::new(Self {
            secrets: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        })
    }
}

impl SecretsProvider for StaticSecretsProvider {
    fn get(&self, name: &str) -> Option<String> {
        self.secrets.get(name).cloned()
    }
}

/// A provider that knows no secrets — every lookup returns `None`. Used for
/// [`CoServerConfig::default`](crate::platform::server_config::CoServerConfig::default).
pub struct StaticEmpty;

impl SecretsProvider for StaticEmpty {
    fn get(&self, _name: &str) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// Process-global default provider (CO-434)
// ---------------------------------------------------------------------------
//
// `CoServerConfig` (boot-time) and request handlers (via `CoreState::secrets`)
// are the primary injection seams. But a handful of *stateless* free functions
// and middlewares have no `AppState` to thread a provider through. Rather than
// scatter `std::env::var` back across those call sites, they read through this
// process-global provider — set once at boot from the same provider that builds
// `CoServerConfig`, and defaulting to `EnvSecretsProvider` when uninitialised
// (the case in unit tests, which keeps reading the live process environment so
// existing `set_var("JWT_SECRET", …)` fixtures keep working).
//
// This keeps the literal `std::env::var` call confined to `EnvSecretsProvider`
// above — the single, documented boot seam — instead of 50 files.

static GLOBAL: OnceLock<Arc<dyn SecretsProvider>> = OnceLock::new();

/// Install the process-global provider. Idempotent: the first call wins, later
/// calls are ignored (so tests that boot multiple servers don't panic).
pub fn init_global(provider: Arc<dyn SecretsProvider>) {
    let _ = GLOBAL.set(provider);
}

/// The process-global provider, or a fresh `EnvSecretsProvider` when unset.
pub fn global() -> Arc<dyn SecretsProvider> {
    GLOBAL
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(EnvSecretsProvider))
}
