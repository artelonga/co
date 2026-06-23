//! CO-81: S3-compatible object-storage [`StorageBackend`].
//!
//! The remote counterpart to [`local_fs::LocalFsBackend`](super::local_fs). Blob
//! bytes live in an S3-compatible bucket (Cloudflare R2 / Tigris / AWS S3) at
//! the content-addressed key `blobs/sha256/<aa>/<bb>/<hash>`; the refcounted
//! `blob_refs` ledger (meta.db, v086) stays the source of truth for dedupe,
//! billing, and the GC cursor (`unreferenced_at`, v090).
//!
//! Three things distinguish it from the local backend:
//!
//! 1. **Content-addressed skip** — before uploading a new blob we `head_object`
//!    the bucket; if the bytes are already there (another universe used the same
//!    image, or a prior migration) we skip the PUT and just record the ledger
//!    row. Identical bytes are stored once, across all universes.
//! 2. **Local LRU cache** — every GET checks [`BlobCache`](super::blob_cache)
//!    first (~1 ms) before falling back to S3 (~30 ms), then warms the cache.
//! 3. **Deferred GC** — `delete` only decrements the refcount; on the zero-drop
//!    it stamps `unreferenced_at` instead of issuing a network DELETE. The
//!    [`gc_unreferenced`](S3Backend::gc_unreferenced) sweep reclaims the bytes
//!    once a blob has been unreferenced past the grace period (30 days).
//!
//! The trait methods are `async`; the `meta.db` lock is a `parking_lot::Mutex`
//! and is **never** held across an `.await` (every network call happens with the
//! lock released).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension};

use super::blob_cache::BlobCache;
use super::{BlobMeta, BlobRef, StorageBackend};
use crate::error::AppError;
use crate::infra::secrets::SecretsProvider;

/// 30-day default grace period before an unreferenced blob is GC-reclaimed.
pub const DEFAULT_GC_GRACE_DAYS: i64 = 30;

const BACKEND_NAME: &str = "s3";

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const ALPH: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(ALPH[(b >> 4) as usize] as char);
        out.push(ALPH[(b & 0xf) as usize] as char);
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(hasher.finalize())
}

fn db_err(ctx: &str, e: rusqlite::Error) -> AppError {
    AppError::Internal(format!("blob_refs {ctx}: {e}"))
}

fn s3_err(ctx: &str, e: co::deploy::DeployAdapterError) -> AppError {
    AppError::Internal(format!("s3 {ctx}: {e}"))
}

pub struct S3Backend {
    s3: Arc<dyn co::deploy::S3Backend>,
    bucket: String,
    /// Optional key prefix inside the bucket (e.g. a tenant namespace). Empty
    /// for the common single-tenant bucket.
    prefix: String,
    /// Connection to `meta.db` for the `blob_refs` ledger.
    conn: Arc<Mutex<Connection>>,
    /// Local hot-blob cache.
    cache: BlobCache,
}

impl S3Backend {
    /// Construct a backend over an arbitrary [`co::deploy::S3Backend`] (the AWS
    /// SDK impl in production, an in-memory fake in tests).
    ///
    /// `data_dir` is the server data root: the `blob_refs` ledger is read from
    /// `data_dir/meta.db` and the LRU cache lives under `data_dir/blob-cache`.
    pub fn new(
        s3: Arc<dyn co::deploy::S3Backend>,
        bucket: impl Into<String>,
        prefix: impl Into<String>,
        data_dir: &Path,
        cache_max_bytes: u64,
    ) -> Result<Self, AppError> {
        let conn = Connection::open(data_dir.join("meta.db"))
            .map_err(|e| AppError::Internal(format!("open meta.db for s3 backend: {e}")))?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        Ok(Self {
            s3,
            bucket: bucket.into(),
            prefix: prefix.into(),
            conn: Arc::new(Mutex::new(conn)),
            cache: BlobCache::new(data_dir.join("blob-cache"), cache_max_bytes),
        })
    }

    /// Content-addressed object key: `[<prefix>/]blobs/sha256/<aa>/<bb>/<hash>`.
    fn object_key(&self, hash: &str) -> String {
        let aa = &hash[0..2];
        let bb = &hash[2..4];
        let path = format!("blobs/sha256/{aa}/{bb}/{hash}");
        if self.prefix.is_empty() {
            path
        } else {
            format!("{}/{path}", self.prefix.trim_end_matches('/'))
        }
    }

    /// Garbage-collect blobs unreferenced for longer than `grace`.
    ///
    /// A row becomes eligible when `delete` drops its refcount to zero (which
    /// stamps `unreferenced_at`) and that stamp is older than `now - grace`. The
    /// physical S3 object is deleted and the ledger row removed. Returns the
    /// number of blobs reclaimed.
    ///
    /// `now` is injected (not read from the clock) so the sweep is deterministic
    /// and testable. A re-`put` of the same bytes before the grace expires
    /// revives the row (refcount back to ≥ 1, `unreferenced_at` cleared), so GC
    /// never races a live reference.
    pub async fn gc_unreferenced(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        grace: chrono::Duration,
    ) -> Result<usize, AppError> {
        let cutoff = (now - grace).to_rfc3339();

        // Collect eligible hashes under the lock, then release it before any
        // network DELETE (never hold the mutex across an `.await`).
        let eligible: Vec<String> = {
            let conn = self.conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT hash FROM blob_refs \
                     WHERE refcount = 0 AND unreferenced_at IS NOT NULL \
                       AND unreferenced_at <= ?1",
                )
                .map_err(|e| db_err("gc prepare", e))?;
            let rows = stmt
                .query_map([&cutoff], |r| r.get::<_, String>(0))
                .map_err(|e| db_err("gc query", e))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| db_err("gc row", e))?);
            }
            out
        };

        let mut reclaimed = 0usize;
        for hash in eligible {
            // Physical delete first; if S3 fails we leave the row so the next
            // sweep retries (no orphaned ledger entries).
            self.s3
                .delete_object(&self.bucket, &self.object_key(&hash))
                .await
                .map_err(|e| s3_err("gc delete_object", e))?;
            {
                let conn = self.conn.lock();
                // Guard against a concurrent re-put reviving the blob between the
                // scan and now: only delete rows still at refcount 0.
                let removed = conn
                    .execute(
                        "DELETE FROM blob_refs WHERE hash = ?1 AND refcount = 0",
                        [&hash],
                    )
                    .map_err(|e| db_err("gc delete row", e))?;
                if removed > 0 {
                    reclaimed += 1;
                }
            }
            self.cache.remove(&hash);
        }
        Ok(reclaimed)
    }

    /// One-shot migration: move every blob currently recorded as `backend =
    /// 'local'` from the on-disk store at `local_blobs_root` into this S3 bucket.
    ///
    /// For each local row: read the file (`<root>/<aa>/<bb>/<hash>`), upload it
    /// (content-addressed skip if already present in the bucket), flip
    /// `blob_refs.backend` to `'s3'`, and unlink the local file. Idempotent —
    /// re-running skips rows already flipped to `'s3'`. Returns the number of
    /// blobs migrated.
    pub async fn import_from_local(&self, local_blobs_root: &Path) -> Result<usize, AppError> {
        let pending: Vec<(String, String)> = {
            let conn = self.conn.lock();
            let mut stmt = conn
                .prepare("SELECT hash, content_type FROM blob_refs WHERE backend = 'local'")
                .map_err(|e| db_err("import select", e))?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| db_err("import query", e))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| db_err("import row", e))?);
            }
            out
        };

        let mut migrated = 0usize;
        for (hash, content_type) in pending {
            let aa = &hash[0..2];
            let bb = &hash[2..4];
            let path = local_blobs_root.join(aa).join(bb).join(&hash);
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                // Ledger row without a backing file — skip; GC/sweep will reconcile.
                Err(_) => continue,
            };

            self.upload_if_absent(&hash, &bytes, &content_type).await?;

            {
                let conn = self.conn.lock();
                conn.execute(
                    "UPDATE blob_refs SET backend = 's3' WHERE hash = ?1",
                    [&hash],
                )
                .map_err(|e| db_err("import flip backend", e))?;
            }
            let _ = std::fs::remove_file(&path);
            migrated += 1;
        }
        Ok(migrated)
    }

    /// Upload `bytes` to the bucket only if the content-addressed object is not
    /// already present (item 2: hash before upload, skip if it exists).
    async fn upload_if_absent(
        &self,
        hash: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(), AppError> {
        let key = self.object_key(hash);
        let exists = self
            .s3
            .head_object(&self.bucket, &key)
            .await
            .map_err(|e| s3_err("head_object", e))?
            .is_some();
        if !exists {
            self.s3
                .put_object(&self.bucket, &key, bytes.to_vec(), content_type)
                .await
                .map_err(|e| s3_err("put_object", e))?;
        }
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for S3Backend {
    fn name(&self) -> &str {
        BACKEND_NAME
    }

    async fn put(&self, bytes: &[u8], content_type: &str) -> Result<BlobRef, AppError> {
        let hash = sha256_hex(bytes);
        let size = bytes.len() as u64;

        // Dedupe / revive: if the ledger already knows this hash, bump the
        // refcount (and clear any GC stamp) without re-uploading.
        {
            let conn = self.conn.lock();
            let existing: Option<(String, i64)> = conn
                .query_row(
                    "SELECT content_type, size FROM blob_refs WHERE hash = ?1",
                    [&hash],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|e| db_err("lookup", e))?;
            if let Some((stored_ct, stored_size)) = existing {
                conn.execute(
                    "UPDATE blob_refs \
                     SET refcount = refcount + 1, unreferenced_at = NULL WHERE hash = ?1",
                    [&hash],
                )
                .map_err(|e| db_err("refcount bump", e))?;
                return Ok(BlobRef {
                    hash,
                    backend: BACKEND_NAME.to_string(),
                    size: stored_size as u64,
                    content_type: stored_ct,
                });
            }
        }

        // New to the ledger: upload (skipping if the bytes already sit in the
        // bucket), then record the row. The lock is released across the network.
        self.upload_if_absent(&hash, bytes, content_type).await?;

        {
            let conn = self.conn.lock();
            // Re-check under the lock: a concurrent put of the same bytes may have
            // inserted the row while we were uploading — bump instead of clashing.
            let now_exists: bool = conn
                .query_row("SELECT 1 FROM blob_refs WHERE hash = ?1", [&hash], |_| {
                    Ok(())
                })
                .optional()
                .map_err(|e| db_err("recheck", e))?
                .is_some();
            if now_exists {
                conn.execute(
                    "UPDATE blob_refs \
                     SET refcount = refcount + 1, unreferenced_at = NULL WHERE hash = ?1",
                    [&hash],
                )
                .map_err(|e| db_err("refcount bump", e))?;
            } else {
                conn.execute(
                    "INSERT INTO blob_refs (hash, backend, size, content_type, refcount, created_at) \
                     VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                    rusqlite::params![
                        hash,
                        BACKEND_NAME,
                        size as i64,
                        content_type,
                        chrono::Utc::now().to_rfc3339(),
                    ],
                )
                .map_err(|e| db_err("insert", e))?;
            }
        }

        // Warm the cache so an immediate read-back is local.
        self.cache.put(&hash, bytes);

        Ok(BlobRef {
            hash,
            backend: BACKEND_NAME.to_string(),
            size,
            content_type: content_type.to_string(),
        })
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, AppError> {
        if let Some(bytes) = self.cache.get(key) {
            return Ok(bytes);
        }
        let bytes = self
            .s3
            .get_object(&self.bucket, &self.object_key(key))
            .await
            .map_err(|e| s3_err("get_object", e))?
            .ok_or_else(|| AppError::NotFound(format!("blob not found: {key}")))?;
        self.cache.put(key, &bytes);
        Ok(bytes)
    }

    async fn head(&self, key: &str) -> Result<BlobMeta, AppError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT hash, backend, size, content_type, refcount, created_at \
             FROM blob_refs WHERE hash = ?1 AND refcount >= 1",
            [key],
            |r| {
                Ok(BlobMeta {
                    hash: r.get(0)?,
                    backend: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                    content_type: r.get(3)?,
                    refcount: r.get::<_, i64>(4)? as u64,
                    created_at: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|e| db_err("head", e))?
        .ok_or_else(|| AppError::NotFound(format!("blob not found: {key}")))
    }

    async fn delete(&self, key: &str) -> Result<(), AppError> {
        {
            let conn = self.conn.lock();
            let refcount: Option<i64> = conn
                .query_row(
                    "SELECT refcount FROM blob_refs WHERE hash = ?1",
                    [key],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| db_err("delete lookup", e))?;

            match refcount {
                // Missing or already-unreferenced → no-op.
                None | Some(0) => return Ok(()),
                // Last reference → stamp for GC, keep the object until the grace
                // period elapses (deferred physical delete).
                Some(n) if n <= 1 => {
                    conn.execute(
                        "UPDATE blob_refs \
                         SET refcount = 0, unreferenced_at = ?2 WHERE hash = ?1",
                        rusqlite::params![key, chrono::Utc::now().to_rfc3339()],
                    )
                    .map_err(|e| db_err("delete stamp", e))?;
                }
                // Still referenced elsewhere → just decrement.
                Some(_) => {
                    conn.execute(
                        "UPDATE blob_refs SET refcount = refcount - 1 WHERE hash = ?1",
                        [key],
                    )
                    .map_err(|e| db_err("refcount drop", e))?;
                }
            }
        }
        // Drop the local mirror immediately; the durable copy stays in S3 until GC.
        self.cache.remove(key);
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool, AppError> {
        let conn = self.conn.lock();
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM blob_refs WHERE hash = ?1 AND refcount >= 1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| db_err("exists", e))?;
        Ok(found.is_some())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<BlobRef>, AppError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT hash, backend, size, content_type FROM blob_refs \
                 WHERE hash LIKE ?1 AND refcount >= 1 ORDER BY created_at DESC, hash ASC",
            )
            .map_err(|e| db_err("list prepare", e))?;
        let pattern = format!("{prefix}%");
        let rows = stmt
            .query_map([pattern], |r| {
                Ok(BlobRef {
                    hash: r.get(0)?,
                    backend: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                    content_type: r.get(3)?,
                })
            })
            .map_err(|e| db_err("list query", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| db_err("list row", e))?);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Build an [`S3Backend`] from secrets (CO-434). Requires the `blob-r2` Cargo
/// feature for the real AWS-SDK client; without it the S3 backend is reported as
/// unavailable so the caller can fall back to `local`.
///
/// Credentials (read through `secrets`):
/// `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, a bucket from
/// `CO_STORAGE_S3_BUCKET` (or `R2_BUCKET`), an optional key prefix
/// `CO_STORAGE_S3_PREFIX`, and a cache budget `CO_BLOB_CACHE_BYTES`
/// (default 10 GiB).
#[cfg(feature = "blob-r2")]
pub async fn from_secrets(
    data_dir: &Path,
    secrets: &dyn SecretsProvider,
) -> Result<S3Backend, AppError> {
    let account_id = secrets.get("R2_ACCOUNT_ID").ok_or_else(|| {
        AppError::BadRequest("CO_STORAGE_BACKEND=s3 requires R2_ACCOUNT_ID".into())
    })?;
    let access_key_id = secrets.get("R2_ACCESS_KEY_ID").ok_or_else(|| {
        AppError::BadRequest("CO_STORAGE_BACKEND=s3 requires R2_ACCESS_KEY_ID".into())
    })?;
    let secret_access_key = secrets.get("R2_SECRET_ACCESS_KEY").ok_or_else(|| {
        AppError::BadRequest("CO_STORAGE_BACKEND=s3 requires R2_SECRET_ACCESS_KEY".into())
    })?;
    let bucket = secrets
        .get("CO_STORAGE_S3_BUCKET")
        .or_else(|| secrets.get("R2_BUCKET"))
        .ok_or_else(|| {
            AppError::BadRequest("CO_STORAGE_BACKEND=s3 requires CO_STORAGE_S3_BUCKET".into())
        })?;
    let prefix = secrets.get_or("CO_STORAGE_S3_PREFIX", "");
    let cache_max_bytes = parse_cache_bytes(&secrets.get_or("CO_BLOB_CACHE_BYTES", ""));

    let s3: Arc<dyn co::deploy::S3Backend> = Arc::new(
        co::deploy::AwsS3Backend::from_r2_credentials(
            &account_id,
            &access_key_id,
            &secret_access_key,
        )
        .await,
    );
    S3Backend::new(s3, bucket, prefix, data_dir, cache_max_bytes)
}

#[cfg(not(feature = "blob-r2"))]
pub async fn from_secrets(
    _data_dir: &Path,
    _secrets: &dyn SecretsProvider,
) -> Result<S3Backend, AppError> {
    Err(AppError::ServiceUnavailable(
        "CO_STORAGE_BACKEND=s3 requires the `blob-r2` Cargo feature; \
         rebuild co-web with --features blob-r2 (or use CO_STORAGE_BACKEND=local)"
            .to_string(),
    ))
}

/// Parse a cache-budget string (raw bytes, e.g. `"10737418240"`). Empty or
/// invalid falls back to 10 GiB — sized to a typical Fly volume.
#[cfg_attr(not(feature = "blob-r2"), allow(dead_code))]
fn parse_cache_bytes(raw: &str) -> u64 {
    const DEFAULT: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
    match raw.trim().parse::<u64>() {
        Ok(n) if n > 0 => n,
        _ => DEFAULT,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    /// In-memory [`co::deploy::S3Backend`] fake — no network, deterministic.
    #[derive(Default)]
    struct InMemoryS3 {
        objects: StdMutex<HashMap<String, Vec<u8>>>,
        /// Count of PUT calls, to assert the content-addressed upload skip.
        puts: StdMutex<usize>,
    }

    #[async_trait]
    impl co::deploy::S3Backend for InMemoryS3 {
        async fn put_object(
            &self,
            _bucket: &str,
            key: &str,
            body: Vec<u8>,
            _ct: &str,
        ) -> Result<(), co::deploy::DeployAdapterError> {
            *self.puts.lock().unwrap() += 1;
            self.objects.lock().unwrap().insert(key.to_string(), body);
            Ok(())
        }
        async fn get_object(
            &self,
            _bucket: &str,
            key: &str,
        ) -> Result<Option<Vec<u8>>, co::deploy::DeployAdapterError> {
            Ok(self.objects.lock().unwrap().get(key).cloned())
        }
        async fn list_objects(
            &self,
            _bucket: &str,
            prefix: &str,
        ) -> Result<Vec<String>, co::deploy::DeployAdapterError> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }
        async fn head_object(
            &self,
            _bucket: &str,
            key: &str,
        ) -> Result<Option<String>, co::deploy::DeployAdapterError> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .get(key)
                .map(|_| "\"etag\"".to_string()))
        }
        async fn delete_object(
            &self,
            _bucket: &str,
            key: &str,
        ) -> Result<(), co::deploy::DeployAdapterError> {
            self.objects.lock().unwrap().remove(key);
            Ok(())
        }
    }

    fn make_backend() -> (tempfile::TempDir, Arc<InMemoryS3>, S3Backend) {
        let dir = tempfile::tempdir().unwrap();
        Storage::new(dir.path()); // migrations → blob_refs + unreferenced_at
        let s3 = Arc::new(InMemoryS3::default());
        let backend = S3Backend::new(
            Arc::clone(&s3) as Arc<dyn co::deploy::S3Backend>,
            "test-bucket",
            "",
            dir.path(),
            1024 * 1024,
        )
        .unwrap();
        (dir, s3, backend)
    }

    #[tokio::test]
    async fn put_get_round_trip_uses_content_addressed_key() {
        let (_dir, s3, backend) = make_backend();
        let data = b"hello object storage";

        let blob = backend.put(data, "text/plain").await.unwrap();
        assert_eq!(blob.backend, "s3");
        assert_eq!(blob.size, data.len() as u64);
        assert_eq!(blob.hash.len(), 64);

        // Object lives at blobs/sha256/<aa>/<bb>/<hash>.
        let aa = &blob.hash[0..2];
        let bb = &blob.hash[2..4];
        let key = format!("blobs/sha256/{aa}/{bb}/{}", blob.hash);
        assert!(s3.objects.lock().unwrap().contains_key(&key));

        let got = backend.get(&blob.hash).await.unwrap();
        assert_eq!(got, data);
    }

    #[tokio::test]
    async fn same_image_to_many_universes_stores_one_blob() {
        // Acceptance: "Same image uploaded to N universes uses only 1 blob row."
        let (_dir, s3, backend) = make_backend();
        let image = b"PNG-bytes-pretend-5MB";

        let mut hash = None;
        for _ in 0..5 {
            let blob = backend.put(image, "image/png").await.unwrap();
            hash = Some(blob.hash);
        }
        let hash = hash.unwrap();

        // Exactly one upload happened; the other four deduped on the ledger.
        assert_eq!(*s3.puts.lock().unwrap(), 1, "content-addressed: one upload");
        // Exactly one ledger row, refcount 5.
        assert_eq!(backend.head(&hash).await.unwrap().refcount, 5);
        assert_eq!(backend.list("").await.unwrap().len(), 1, "one blob row");
    }

    #[tokio::test]
    async fn content_addressed_put_skips_upload_when_object_present() {
        let (_dir, s3, backend) = make_backend();
        let data = b"already in the bucket";
        let hash = sha256_hex(data);

        // Pre-seed the bucket as if a prior migration/other instance uploaded it.
        let key = backend.object_key(&hash);
        s3.objects.lock().unwrap().insert(key, data.to_vec());

        let blob = backend.put(data, "text/plain").await.unwrap();
        assert_eq!(blob.hash, hash);
        assert_eq!(
            *s3.puts.lock().unwrap(),
            0,
            "object already present → upload skipped"
        );
    }

    #[tokio::test]
    async fn get_serves_from_local_cache_after_first_fetch() {
        let (_dir, s3, backend) = make_backend();
        let blob = backend.put(b"cache me", "text/plain").await.unwrap();

        // Wipe the remote object: subsequent gets can only succeed from cache.
        s3.objects.lock().unwrap().clear();
        let got = backend.get(&blob.hash).await.unwrap();
        assert_eq!(got, b"cache me");
    }

    #[tokio::test]
    async fn delete_decrements_then_defers_to_gc_on_zero() {
        let (_dir, s3, backend) = make_backend();
        let data = b"refcounted";

        backend.put(data, "text/plain").await.unwrap();
        let blob = backend.put(data, "text/plain").await.unwrap(); // refcount 2

        backend.delete(&blob.hash).await.unwrap();
        assert_eq!(backend.head(&blob.hash).await.unwrap().refcount, 1);

        // Last reference → logically gone, but object physically retained for GC.
        backend.delete(&blob.hash).await.unwrap();
        assert!(!backend.exists(&blob.hash).await.unwrap());
        assert!(matches!(
            backend.head(&blob.hash).await,
            Err(AppError::NotFound(_))
        ));
        assert!(
            s3.objects
                .lock()
                .unwrap()
                .contains_key(&backend.object_key(&blob.hash)),
            "object stays in S3 until the grace period elapses"
        );
    }

    #[tokio::test]
    async fn delete_missing_key_is_noop() {
        let (_dir, _s3, backend) = make_backend();
        backend
            .delete("0000000000000000000000000000000000000000000000000000000000000000")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn gc_reclaims_only_after_grace_period() {
        // Acceptance: "GC reclaims unreferenced blobs after 30-day grace period."
        let (_dir, s3, backend) = make_backend();
        let blob = backend.put(b"to be collected", "text/plain").await.unwrap();
        backend.delete(&blob.hash).await.unwrap(); // refcount 0, stamped now

        let grace = chrono::Duration::days(DEFAULT_GC_GRACE_DAYS);
        let key = backend.object_key(&blob.hash);

        // 29 days later: still inside the grace window → nothing reclaimed.
        let t29 = chrono::Utc::now() + chrono::Duration::days(29);
        assert_eq!(backend.gc_unreferenced(t29, grace).await.unwrap(), 0);
        assert!(s3.objects.lock().unwrap().contains_key(&key));

        // 31 days later: past the grace window → reclaimed (row + object gone).
        let t31 = chrono::Utc::now() + chrono::Duration::days(31);
        assert_eq!(backend.gc_unreferenced(t31, grace).await.unwrap(), 1);
        assert!(!s3.objects.lock().unwrap().contains_key(&key));
        assert!(matches!(
            backend.get(&blob.hash).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn gc_skips_revived_blob() {
        // A re-put within the grace window must save the blob from GC.
        let (_dir, s3, backend) = make_backend();
        let data = b"revive me";
        let blob = backend.put(data, "text/plain").await.unwrap();
        backend.delete(&blob.hash).await.unwrap(); // refcount 0

        // Re-put before grace expires → refcount back to 1, stamp cleared.
        backend.put(data, "text/plain").await.unwrap();
        assert!(backend.exists(&blob.hash).await.unwrap());

        let grace = chrono::Duration::days(DEFAULT_GC_GRACE_DAYS);
        let t31 = chrono::Utc::now() + chrono::Duration::days(31);
        assert_eq!(
            backend.gc_unreferenced(t31, grace).await.unwrap(),
            0,
            "revived blob must not be collected"
        );
        assert!(
            s3.objects
                .lock()
                .unwrap()
                .contains_key(&backend.object_key(&blob.hash))
        );
    }

    #[tokio::test]
    async fn import_from_local_moves_blobs_to_s3() {
        // Acceptance: migrate existing filesystem blobs to object storage.
        let dir = tempfile::tempdir().unwrap();
        Storage::new(dir.path());

        // Stand up a local backend, write a blob through it.
        let local = super::super::local_fs::LocalFsBackend::open(dir.path()).unwrap();
        let data = b"legacy on-disk blob";
        let blob = local.put(data, "text/plain").await.unwrap();
        let local_path = dir
            .path()
            .join("blobs")
            .join(&blob.hash[0..2])
            .join(&blob.hash[2..4])
            .join(&blob.hash);
        assert!(local_path.exists());

        // Bring up the S3 backend over the same meta.db and migrate.
        let s3 = Arc::new(InMemoryS3::default());
        let backend = S3Backend::new(
            Arc::clone(&s3) as Arc<dyn co::deploy::S3Backend>,
            "test-bucket",
            "",
            dir.path(),
            1024 * 1024,
        )
        .unwrap();

        let moved = backend
            .import_from_local(&dir.path().join("blobs"))
            .await
            .unwrap();
        assert_eq!(moved, 1);

        // Ledger flipped to s3, local file gone, object readable via S3 backend.
        assert_eq!(backend.head(&blob.hash).await.unwrap().backend, "s3");
        assert!(!local_path.exists(), "local file unlinked after migration");
        assert_eq!(backend.get(&blob.hash).await.unwrap(), data);

        // Idempotent: a second run finds nothing left to move.
        assert_eq!(
            backend
                .import_from_local(&dir.path().join("blobs"))
                .await
                .unwrap(),
            0
        );
    }

    #[test]
    fn parse_cache_bytes_defaults_and_overrides() {
        assert_eq!(parse_cache_bytes("2048"), 2048);
        assert_eq!(parse_cache_bytes(""), 10 * 1024 * 1024 * 1024);
        assert_eq!(parse_cache_bytes("nonsense"), 10 * 1024 * 1024 * 1024);
        assert_eq!(parse_cache_bytes("0"), 10 * 1024 * 1024 * 1024);
    }
}
