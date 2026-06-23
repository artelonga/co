//! CO-458: Pluggable `StorageBackend` trait — the **StaaS keystone**.
//!
//! This is the single abstraction point for content-addressed blob storage.
//! Today there is exactly one implementation, [`local_fs::LocalFsBackend`]
//! (sharded local filesystem under `/data/blobs/` + a refcounted `blob_refs`
//! ledger for dedupe). S3 (CO-81) and partner backends become alternate impls
//! of this same trait — **without touching call-sites**.
//!
//! Selection is config-driven (CO-434 pattern): `CO_STORAGE_BACKEND=local|s3`
//! (default `local`). Only `local` is wired here; `s3` is reserved for CO-81.
//!
//! ```text
//!   put(bytes, ct) ─┐                         ┌─ /data/blobs/<aa>/<bb>/<hash>
//!                   ├─ sha256 → hash (key) ───┤
//!   get/head/delete ┘                         └─ blob_refs(hash, refcount, …)
//! ```
//!
//! Distinct from the per-universe `infra::blob::BlobStore` (CO-294) and the
//! whole-DB `storage::backup::BackupBackend` (CO-365): this is the *central*
//! ledger that StaaS billing, S3, and partner tenants converge on.

use std::path::Path;

use async_trait::async_trait;

use crate::error::AppError;
use crate::infra::secrets::SecretsProvider;

pub mod blob_cache;
pub mod local_fs;
pub mod s3;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A reference to a stored blob. Returned by `put` and `list`.
///
/// `hash` is the content key (sha256 hex, 64 chars). Because storage is
/// content-addressed, identical bytes always map to the same `hash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRef {
    /// sha256 hex of the stored bytes — the content key.
    pub hash: String,
    /// Which backend stored it (`local`, later `s3`/partner).
    pub backend: String,
    /// Number of bytes stored.
    pub size: u64,
    /// MIME type of the stored bytes.
    pub content_type: String,
}

/// Full ledger metadata for a blob. Returned by `head`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMeta {
    pub hash: String,
    pub backend: String,
    pub size: u64,
    pub content_type: String,
    /// How many logical references point at this blob (dedupe counter).
    pub refcount: u64,
    /// RFC3339 timestamp of first insert.
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Content-addressed blob storage with refcounted dedupe.
///
/// All methods are `async` and `Send + Sync`; errors surface as [`AppError`].
/// Keys are sha256 hex strings produced by [`put`](StorageBackend::put).
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Short backend identifier recorded in `blob_refs.backend` (`local`/`s3`/…).
    fn name(&self) -> &str;

    /// Store `bytes`, returning a content-addressed [`BlobRef`].
    ///
    /// Content-addressed + deduped: the key is `sha256(bytes)`. Re-`put`ting
    /// identical bytes does **not** rewrite the blob — it bumps the refcount and
    /// returns the existing [`BlobRef`].
    async fn put(&self, bytes: &[u8], content_type: &str) -> Result<BlobRef, AppError>;

    /// Retrieve the bytes stored under `key`. `NotFound` if absent.
    async fn get(&self, key: &str) -> Result<Vec<u8>, AppError>;

    /// Ledger metadata for `key` without reading the bytes. `NotFound` if absent.
    async fn head(&self, key: &str) -> Result<BlobMeta, AppError>;

    /// Drop one reference to `key`. The underlying bytes are removed only when
    /// the refcount reaches zero. Deleting a missing key is a no-op.
    async fn delete(&self, key: &str) -> Result<(), AppError>;

    /// `true` when a blob with `key` exists (refcount ≥ 1).
    async fn exists(&self, key: &str) -> Result<bool, AppError>;

    /// All blobs whose hash starts with `prefix`, newest first. An empty prefix
    /// lists everything. (Pagination lands with S3 in CO-81; the local impl
    /// returns the full set.)
    async fn list(&self, prefix: &str) -> Result<Vec<BlobRef>, AppError>;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Construct a [`StorageBackend`] from `CO_STORAGE_BACKEND` (default `local`),
/// reading any backend credentials through `secrets` (CO-434).
///
/// | Value               | Behaviour                                   |
/// |---------------------|---------------------------------------------|
/// | `local` (or missing)| [`local_fs::LocalFsBackend`] under `data_dir/blobs` |
/// | `s3`                | [`s3::S3Backend`] (CO-81; needs the `blob-r2` feature + R2/S3 secrets) |
/// | anything else       | error                                        |
///
/// `data_dir` is the server data root (e.g. `/data`); blobs live under
/// `data_dir/blobs/` and the `blob_refs` ledger lives in `data_dir/meta.db`.
///
/// `async` because the S3 client is built asynchronously; the `local` path does
/// no I/O beyond opening the ledger connection.
pub async fn from_config(
    data_dir: &Path,
    secrets: &dyn SecretsProvider,
) -> Result<Box<dyn StorageBackend>, AppError> {
    let kind = secrets.get_or("CO_STORAGE_BACKEND", "local");
    match kind.as_str() {
        "local" => Ok(Box::new(local_fs::LocalFsBackend::open(data_dir)?)),
        "s3" => Ok(Box::new(s3::from_secrets(data_dir, secrets).await?)),
        other => Err(AppError::BadRequest(format!(
            "unknown CO_STORAGE_BACKEND={other:?} (expected `local` or `s3`)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::secrets::StaticSecretsProvider;

    #[tokio::test]
    async fn from_config_defaults_to_local() {
        let dir = tempfile::tempdir().unwrap();
        crate::storage::Storage::new(dir.path()); // run migrations (creates blob_refs)
        let secrets = StaticSecretsProvider::new(Vec::<(&str, &str)>::new());
        let backend = from_config(dir.path(), secrets.as_ref()).await.unwrap();
        assert_eq!(backend.name(), "local");
    }

    /// Without the `blob-r2` feature the S3 backend is reported unavailable so
    /// the caller can fall back to `local`. (With the feature, missing
    /// credentials surface as `BadRequest` instead — see `s3::from_secrets`.)
    #[cfg(not(feature = "blob-r2"))]
    #[tokio::test]
    async fn from_config_s3_requires_feature() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = StaticSecretsProvider::new([("CO_STORAGE_BACKEND", "s3")]);
        let result = from_config(dir.path(), secrets.as_ref()).await;
        assert!(matches!(result, Err(AppError::ServiceUnavailable(_))));
    }

    #[tokio::test]
    async fn from_config_rejects_unknown_backend() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = StaticSecretsProvider::new([("CO_STORAGE_BACKEND", "ftp")]);
        let result = from_config(dir.path(), secrets.as_ref()).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
