//! CO-294 — Blob store trait + implementations.
//!
//! `BlobStore` is a thin async abstraction over content-addressable binary
//! storage. The default implementation (`LocalFsBlobStore`) writes to the
//! local filesystem using the same 2-level sharding scheme that
//! `asset_routes.rs` has always used. An optional R2 implementation
//! (`R2BlobStore`) is gated behind `#[cfg(feature = "blob-r2")]` and
//! forwards to the `S3Backend` trait from `co::deploy`.
//!
//! A `BlobBackend` selector reads `CO_BLOB_BACKEND` at boot and returns an
//! `Arc<dyn BlobStore>` scoped to a specific universe via
//! `BlobBackend::for_universe`.

use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// BlobMeta
// ---------------------------------------------------------------------------

/// Metadata returned by a successful `put`.
#[derive(Debug)]
pub struct BlobMeta {
    /// Content key (sha256 hex or arbitrary client-supplied key).
    pub key: String,
    /// Number of bytes stored (after any envelope; same as input for local).
    pub size: usize,
}

// ---------------------------------------------------------------------------
// BlobStore trait
// ---------------------------------------------------------------------------

/// Async content-addressable blob storage.
///
/// Keys are typically sha256 hex strings (64 chars). All methods are `async`
/// and return `anyhow::Result` so implementations can propagate any I/O or
/// SDK error without re-wrapping.
#[async_trait::async_trait]
pub trait BlobStore: Send + Sync {
    /// Store `bytes` under `key`. If the key already exists the write is
    /// idempotent (same bytes → same result, no error).
    async fn put(&self, key: &str, bytes: &[u8]) -> anyhow::Result<BlobMeta>;

    /// Retrieve the bytes stored under `key`.
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>>;

    /// Remove the blob identified by `key`. Missing key is not an error.
    async fn delete(&self, key: &str) -> anyhow::Result<()>;

    /// Return a pre-signed URL valid for `ttl`. Returns an empty string for
    /// backends that do not support pre-signed URLs (local, R2 in this impl).
    async fn presign_url(&self, key: &str, ttl: std::time::Duration) -> anyhow::Result<String>;

    /// Return `true` if a blob with the given key exists.
    async fn exists(&self, key: &str) -> anyhow::Result<bool>;
}

// ---------------------------------------------------------------------------
// LocalFsBlobStore
// ---------------------------------------------------------------------------

/// `BlobStore` backed by the local filesystem.
///
/// Files are stored under `<base_dir>/<key[0..2]>/<key[2..4]>/<key>` — the
/// same 2-level sharding used by the original `blob_path` helper in
/// `asset_routes.rs`. Writes are atomic (write to `<key>.tmp`, then rename).
pub struct LocalFsBlobStore {
    base_dir: PathBuf,
}

impl LocalFsBlobStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Compute the on-disk path for `key` (2-level sharding).
    ///
    /// Panics if `key.len() < 4` — callers must supply at least 4-char keys
    /// (sha256 hex keys are always 64 chars).
    fn key_path(&self, key: &str) -> PathBuf {
        let aa = &key[0..2];
        let bb = &key[2..4];
        self.base_dir.join(aa).join(bb).join(key)
    }
}

#[async_trait::async_trait]
impl BlobStore for LocalFsBlobStore {
    async fn put(&self, key: &str, bytes: &[u8]) -> anyhow::Result<BlobMeta> {
        let path = self.key_path(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(BlobMeta {
            key: key.to_string(),
            size: bytes.len(),
        })
    }

    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        let path = self.key_path(key);
        Ok(std::fs::read(&path)?)
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        let path = self.key_path(key);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    async fn presign_url(&self, _key: &str, _ttl: std::time::Duration) -> anyhow::Result<String> {
        Ok(String::new())
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        Ok(self.key_path(key).exists())
    }
}

// ---------------------------------------------------------------------------
// R2BlobStore (gated behind blob-r2 feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "blob-r2")]
pub struct R2BlobStore {
    s3: Arc<dyn co::deploy::S3Backend>,
    bucket: String,
    prefix: String,
}

#[cfg(feature = "blob-r2")]
impl R2BlobStore {
    fn full_key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{}", self.prefix, key)
        }
    }
}

#[cfg(feature = "blob-r2")]
#[async_trait::async_trait]
impl BlobStore for R2BlobStore {
    async fn put(&self, key: &str, bytes: &[u8]) -> anyhow::Result<BlobMeta> {
        let full = self.full_key(key);
        self.s3
            .put_object(
                &self.bucket,
                &full,
                bytes.to_vec(),
                "application/octet-stream",
            )
            .await
            .map_err(|e| anyhow::anyhow!("R2 put_object: {e}"))?;
        Ok(BlobMeta {
            key: key.to_string(),
            size: bytes.len(),
        })
    }

    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        let full = self.full_key(key);
        self.s3
            .get_object(&self.bucket, &full)
            .await
            .map_err(|e| anyhow::anyhow!("R2 get_object: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("blob not found: {key}"))
    }

    async fn delete(&self, _key: &str) -> anyhow::Result<()> {
        // S3Backend has no delete_object; blob cleanup is a separate ops task.
        Ok(())
    }

    async fn presign_url(&self, _key: &str, _ttl: std::time::Duration) -> anyhow::Result<String> {
        Ok(String::new())
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        let full = self.full_key(key);
        let result = self
            .s3
            .head_object(&self.bucket, &full)
            .await
            .map_err(|e| anyhow::anyhow!("R2 head_object: {e}"))?;
        Ok(result.is_some())
    }
}

// ---------------------------------------------------------------------------
// BlobBackend selector
// ---------------------------------------------------------------------------

enum BlobBackendKind {
    Local,
    #[cfg(feature = "blob-r2")]
    R2 {
        s3: Arc<dyn co::deploy::S3Backend>,
        bucket: String,
    },
}

/// Top-level blob backend selector.
///
/// Call `for_universe` to obtain an `Arc<dyn BlobStore>` scoped to a
/// specific universe directory / key. The `Local` variant stores blobs under
/// `<universe_dir>/blobs/…`; the `R2` variant uses the universe key as a
/// prefix in the bucket.
///
/// When `staging_error_rate` is set via [`BlobBackend::with_staging_error_rate`],
/// `for_universe` wraps the returned store with a
/// [`crate::infra::staging::FlakyBlobStore`] that injects errors at that rate.
pub struct BlobBackend {
    kind: BlobBackendKind,
    /// CO-298: when Some, for_universe wraps the store with FlakyBlobStore.
    staging_error_rate: Option<f64>,
}

impl BlobBackend {
    /// Local filesystem backend (default; no env vars required).
    pub fn local() -> Self {
        Self {
            kind: BlobBackendKind::Local,
            staging_error_rate: None,
        }
    }

    /// Enable staging fault injection: `for_universe` will wrap the store with
    /// [`crate::infra::staging::FlakyBlobStore`] at the given error rate.
    pub fn with_staging_error_rate(mut self, rate: f64) -> Self {
        self.staging_error_rate = Some(rate);
        self
    }

    /// Cloudflare R2 backend (requires `blob-r2` feature).
    #[cfg(feature = "blob-r2")]
    pub fn r2(s3: Arc<dyn co::deploy::S3Backend>, bucket: String) -> Self {
        Self {
            kind: BlobBackendKind::R2 { s3, bucket },
            staging_error_rate: None,
        }
    }

    /// Return a `BlobStore` scoped to a single universe.
    ///
    /// When [`BlobBackend::with_staging_error_rate`] was called, the returned
    /// store is wrapped with [`crate::infra::staging::FlakyBlobStore`].
    pub fn for_universe(
        &self,
        universe_dir: &std::path::Path,
        #[cfg_attr(not(feature = "blob-r2"), allow(unused_variables))] universe_key: &str,
    ) -> Arc<dyn BlobStore> {
        let store: Arc<dyn BlobStore> = match &self.kind {
            BlobBackendKind::Local => Arc::new(LocalFsBlobStore::new(universe_dir.join("blobs"))),
            #[cfg(feature = "blob-r2")]
            BlobBackendKind::R2 { s3, bucket } => Arc::new(R2BlobStore {
                s3: Arc::clone(s3),
                bucket: bucket.clone(),
                prefix: universe_key.to_string(),
            }),
        };
        if let Some(rate) = self.staging_error_rate {
            Arc::new(crate::infra::staging::FlakyBlobStore::new(store, rate))
        } else {
            store
        }
    }
}

// ---------------------------------------------------------------------------
// Factory: blob_backend_from_env
// ---------------------------------------------------------------------------

/// Construct a `BlobBackend` by reading `CO_BLOB_BACKEND` (default: `"local"`).
///
/// | Value                      | Behaviour                                  |
/// |----------------------------|--------------------------------------------|
/// | `"local"` (or missing)     | `BlobBackend::local()`                     |
/// | `"local_then_r2_fallback"` | `BlobBackend::local()` (fallback not yet)  |
/// | `"r2"`                     | `BlobBackend::r2(…)` if feature enabled    |
/// | anything else              | `BlobBackend::local()`                     |
pub async fn blob_backend_from_env() -> BlobBackend {
    let backend = std::env::var("CO_BLOB_BACKEND").unwrap_or_else(|_| "local".to_string());
    match backend.as_str() {
        "r2" => {
            #[cfg(feature = "blob-r2")]
            {
                let account_id = std::env::var("R2_ACCOUNT_ID")
                    .expect("CO_BLOB_BACKEND=r2 requires R2_ACCOUNT_ID");
                let access_key_id = std::env::var("R2_ACCESS_KEY_ID")
                    .expect("CO_BLOB_BACKEND=r2 requires R2_ACCESS_KEY_ID");
                let secret_access_key = std::env::var("R2_SECRET_ACCESS_KEY")
                    .expect("CO_BLOB_BACKEND=r2 requires R2_SECRET_ACCESS_KEY");
                let bucket =
                    std::env::var("R2_BUCKET").expect("CO_BLOB_BACKEND=r2 requires R2_BUCKET");
                let s3: Arc<dyn co::deploy::S3Backend> = Arc::new(
                    co::deploy::AwsS3Backend::from_r2_credentials(
                        &account_id,
                        &access_key_id,
                        &secret_access_key,
                    )
                    .await,
                );
                BlobBackend::r2(s3, bucket)
            }
            #[cfg(not(feature = "blob-r2"))]
            {
                panic!(
                    "CO_BLOB_BACKEND=r2 requires the `blob-r2` Cargo feature. \
                     Rebuild co-web with --features blob-r2."
                );
            }
        }
        // "local", "local_then_r2_fallback", or anything unknown → local
        _ => BlobBackend::local(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (TempDir, LocalFsBlobStore) {
        let dir = TempDir::new().unwrap();
        let store = LocalFsBlobStore::new(dir.path().to_path_buf());
        (dir, store)
    }

    // A realistic sha256 hex key (64 chars).
    const SHA256_KEY: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    #[tokio::test]
    async fn local_put_get_round_trip() {
        let (_dir, store) = make_store();
        let data = b"hello co blob store";

        let meta = store.put(SHA256_KEY, data).await.unwrap();
        assert_eq!(meta.key, SHA256_KEY);
        assert_eq!(meta.size, data.len());

        let retrieved = store.get(SHA256_KEY).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn local_exists_and_delete() {
        let (_dir, store) = make_store();
        let data = b"some bytes";

        // Does not exist before put.
        assert!(!store.exists(SHA256_KEY).await.unwrap());

        store.put(SHA256_KEY, data).await.unwrap();
        assert!(store.exists(SHA256_KEY).await.unwrap());

        store.delete(SHA256_KEY).await.unwrap();
        assert!(!store.exists(SHA256_KEY).await.unwrap());
    }

    #[tokio::test]
    async fn local_shards_by_key_prefix() {
        let (dir, store) = make_store();

        store.put(SHA256_KEY, b"data").await.unwrap();

        // Expected path: <base>/<aa>/<bb>/<key>
        let aa = &SHA256_KEY[0..2]; // "ab"
        let bb = &SHA256_KEY[2..4]; // "cd"
        let expected = dir.path().join(aa).join(bb).join(SHA256_KEY);
        assert!(
            expected.exists(),
            "blob not found at expected sharded path: {}",
            expected.display()
        );
    }

    #[tokio::test]
    async fn local_put_is_idempotent() {
        let (_dir, store) = make_store();
        let data = b"idempotent bytes";

        store.put(SHA256_KEY, data).await.unwrap();
        // Second put overwrites atomically — should not error.
        store.put(SHA256_KEY, data).await.unwrap();

        let retrieved = store.get(SHA256_KEY).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn local_delete_missing_key_is_noop() {
        let (_dir, store) = make_store();
        // Deleting a non-existent key must not return an error.
        store.delete(SHA256_KEY).await.unwrap();
    }

    #[tokio::test]
    async fn local_presign_returns_empty_string() {
        let (_dir, store) = make_store();
        let url = store
            .presign_url(SHA256_KEY, std::time::Duration::from_secs(300))
            .await
            .unwrap();
        assert_eq!(url, "");
    }
}
