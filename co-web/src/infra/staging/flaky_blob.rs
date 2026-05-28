//! `FlakyBlobStore` — injects 503-equivalent errors on a fraction of blob ops.
//!
//! Uses a deterministic `AtomicU64` call counter so the failure pattern is
//! reproducible across runs: every `ceil(1 / error_rate)`th call fails.
//! Example: `error_rate = 0.05` → fail on calls 20, 40, 60, … (every 20th).
//!
//! Pass `error_rate = 0.0` to disable fault injection entirely.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::anyhow;
use async_trait::async_trait;

use crate::infra::blob::{BlobMeta, BlobStore};

/// Wraps any `BlobStore` and returns `Err` on approximately `error_rate` of calls.
pub struct FlakyBlobStore {
    inner: Arc<dyn BlobStore>,
    /// 0 means never fail; otherwise fail every Nth call.
    fail_every_n: u64,
    call_count: AtomicU64,
}

impl FlakyBlobStore {
    /// Create a new `FlakyBlobStore`.
    ///
    /// `error_rate` is clamped to `[0.0, 1.0]`.
    /// Pass `0.0` to make this a transparent pass-through.
    pub fn new(inner: Arc<dyn BlobStore>, error_rate: f64) -> Self {
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
impl BlobStore for FlakyBlobStore {
    async fn put(&self, key: &str, bytes: &[u8]) -> anyhow::Result<BlobMeta> {
        if self.should_fail() {
            return Err(anyhow!("staging: simulated 503 on blob put (key={key})"));
        }
        self.inner.put(key, bytes).await
    }

    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        if self.should_fail() {
            return Err(anyhow!("staging: simulated 503 on blob get (key={key})"));
        }
        self.inner.get(key).await
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        if self.should_fail() {
            return Err(anyhow!("staging: simulated 503 on blob delete (key={key})"));
        }
        self.inner.delete(key).await
    }

    async fn presign_url(&self, key: &str, ttl: std::time::Duration) -> anyhow::Result<String> {
        if self.should_fail() {
            return Err(anyhow!("staging: simulated 503 on presign_url (key={key})"));
        }
        self.inner.presign_url(key, ttl).await
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        if self.should_fail() {
            return Err(anyhow!("staging: simulated 503 on blob exists (key={key})"));
        }
        self.inner.exists(key).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::blob::LocalFsBlobStore;
    use tempfile::TempDir;

    // A realistic sha256 hex key (64 chars) for tests.
    const KEY: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    fn make_store() -> (TempDir, Arc<dyn BlobStore>) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(LocalFsBlobStore::new(dir.path().to_path_buf()));
        (dir, store as Arc<dyn BlobStore>)
    }

    #[tokio::test]
    async fn rate_zero_never_fails() {
        let (_dir, inner) = make_store();
        let flaky = FlakyBlobStore::new(inner, 0.0);
        for _ in 0..50 {
            assert!(flaky.put(KEY, b"data").await.is_ok());
        }
    }

    #[tokio::test]
    async fn rate_one_always_fails() {
        let (_dir, inner) = make_store();
        let flaky = FlakyBlobStore::new(inner, 1.0);
        let result = flaky.put(KEY, b"data").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("staging:"));
    }

    #[tokio::test]
    async fn rate_05_fails_exactly_every_20_calls() {
        let (_dir, inner) = make_store();
        // Seed the key so `exists` finds something to check
        inner.put(KEY, b"seed").await.unwrap();
        let flaky = FlakyBlobStore::new(Arc::clone(&inner), 0.05);
        let mut failures = 0usize;
        for _ in 0..100 {
            if flaky.exists(KEY).await.is_err() {
                failures += 1;
            }
        }
        // 100 calls at rate 0.05 → fail every 20th → 5 failures
        assert_eq!(failures, 5);
    }

    #[tokio::test]
    async fn implements_blob_store_trait() {
        let (_dir, inner) = make_store();
        let _: Arc<dyn BlobStore> = Arc::new(FlakyBlobStore::new(inner, 0.0));
    }

    #[tokio::test]
    async fn non_failing_calls_pass_through() {
        let (_dir, inner) = make_store();
        // rate=0.5 → fail every 2nd call; call 1 passes through
        let flaky = FlakyBlobStore::new(Arc::clone(&inner), 0.5);
        let result = flaky.put(KEY, b"hello").await;
        assert!(result.is_ok(), "first call should pass through at rate=0.5");
    }
}
