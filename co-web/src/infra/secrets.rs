use std::collections::HashMap;
use std::sync::Arc;

/// Abstraction for reading runtime secrets.
///
/// Production code uses `EnvSecretsProvider` (reads `std::env::var`).
/// Tests inject `StaticSecretsProvider` to avoid mutating the process
/// environment, which eliminates parallel-test races on `JWT_SECRET` and
/// other secrets.
pub trait SecretsProvider: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
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
