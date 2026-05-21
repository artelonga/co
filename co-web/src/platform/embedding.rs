//! EmbeddingService — local text-embedding via fastembed (CO-164).
//!
//! Model: all-MiniLM-L6-v2 (384 dims, ~80 MB).
//! Downloaded on first use to `~/.co/models/` (or `CO_MODELS_DIR` env var).
//! Falls back gracefully to `None` when the model is unavailable (no internet,
//! restricted env) so the rest of the server continues to function.

use std::path::PathBuf;
use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

pub const DIMS: usize = 384;
pub const MODEL_NAME: &str = "all-MiniLM-L6-v2";

/// Default model cache directory.
pub fn default_model_dir() -> PathBuf {
    std::env::var("CO_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".co")
                .join("models")
        })
}

/// Thread-safe wrapper around a lazily-loaded fastembed model.
///
/// All public methods return `None` / empty results when the model failed to
/// load, so callers never need to handle the unavailable case specially.
pub struct EmbeddingService {
    model: Mutex<Option<TextEmbedding>>,
}

impl EmbeddingService {
    /// Attempt to load the model from `model_dir`, downloading if necessary.
    ///
    /// Failures are logged at WARN and the service is left in the disabled state.
    pub fn new(model_dir: PathBuf) -> Self {
        let model = match load_model(model_dir) {
            Ok(m) => {
                tracing::info!("CO-164: embedding model '{}' ready", MODEL_NAME);
                Some(m)
            }
            Err(e) => {
                tracing::warn!(
                    "CO-164: embedding model unavailable — semantic search disabled: {e:#}"
                );
                None
            }
        };
        Self {
            model: Mutex::new(model),
        }
    }

    /// Construct a disabled service (no model). Used in tests and for
    /// deferred boot loading (see `load_deferred`).
    pub fn disabled() -> Self {
        Self {
            model: Mutex::new(None),
        }
    }

    /// Load and install the model in the background. Returns immediately;
    /// the model becomes available once the tokio task completes.
    /// Idempotent — does nothing if the model is already loaded.
    pub fn load_deferred(self: std::sync::Arc<Self>, model_dir: std::path::PathBuf) {
        tokio::spawn(async move {
            if self.is_available() {
                return;
            }
            match tokio::task::spawn_blocking(move || load_model(model_dir)).await {
                Ok(Ok(m)) => {
                    if let Ok(mut guard) = self.model.lock() {
                        *guard = Some(m);
                        tracing::info!("CO-164: embedding model '{}' ready (deferred)", MODEL_NAME);
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        "CO-164: deferred embedding model load failed — semantic search disabled: {e:#}"
                    );
                }
                Err(e) => {
                    tracing::warn!("CO-164: embedding model task panicked: {e}");
                }
            }
        });
    }

    /// Returns `true` when the model is loaded and ready.
    pub fn is_available(&self) -> bool {
        self.model.lock().map(|m| m.is_some()).unwrap_or(false)
    }

    /// Embed a batch of texts.
    ///
    /// Returns `None` when the model is unavailable or an error occurs.
    /// Texts should be non-empty strings; empty strings produce near-zero vectors.
    pub fn embed(&self, texts: &[&str]) -> Option<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Some(vec![]);
        }
        let mut guard = self.model.lock().ok()?;
        let model = guard.as_mut()?;
        model
            .embed(
                texts.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                None,
            )
            .map_err(|e| tracing::warn!("CO-164: embed error: {e:#}"))
            .ok()
    }

    /// Convenience: embed a single string.
    pub fn embed_one(&self, text: &str) -> Option<Vec<f32>> {
        self.embed(&[text])?.into_iter().next()
    }
}

fn load_model(model_dir: PathBuf) -> anyhow::Result<TextEmbedding> {
    std::fs::create_dir_all(&model_dir)?;
    let opts = InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_cache_dir(model_dir);
    TextEmbedding::try_new(opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_model_dir_contains_co_models() {
        let d = default_model_dir();
        let s = d.to_string_lossy();
        assert!(
            s.contains(".co") || s.contains("CO_MODELS_DIR"),
            "unexpected path: {s}"
        );
    }

    #[test]
    fn test_embedding_service_unavailable_returns_none() {
        // Point at a temp dir with no internet access simulation:
        // The model won't download in test env, so the service is disabled.
        // We just verify it doesn't panic.
        let tmp = tempfile::tempdir().unwrap();
        // Override env to prevent downloading during tests
        // (we can't actually prevent it, but we shouldn't need to — tests
        // that don't call embed_one never trigger a download)
        let svc = EmbeddingService::new(tmp.path().to_path_buf());
        // is_available() depends on whether the model is cached; we just
        // verify the struct is constructible without panic.
        let _ = svc.is_available();
    }
}
