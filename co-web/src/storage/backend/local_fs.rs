//! CO-458: Local filesystem [`StorageBackend`] — content-addressed + sharded.
//!
//! Blobs live under `<data_dir>/blobs/<aa>/<bb>/<hash>` (2-level sharding by the
//! hash prefix, the same scheme `infra::blob` and `assets` use). The `blob_refs`
//! ledger in `meta.db` (migration v086) tracks one row per distinct blob with a
//! refcount, so identical bytes are stored once and `delete` only unlinks the
//! file when the last reference drops.
//!
//! All DB + filesystem work is synchronous; the methods are `async` to satisfy
//! the trait (and so an S3 impl can be genuinely async). The `meta.db` lock is a
//! `parking_lot::Mutex` and is never held across an `.await`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension};

use super::{BlobMeta, BlobRef, StorageBackend};
use crate::error::AppError;

const BACKEND_NAME: &str = "local";

/// Minimal lowercase hex encoder (mirrors `content::references::meta::hex_encode`
/// and the asset routes — avoids pulling in a `hex` dependency).
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

pub struct LocalFsBackend {
    /// Root for blob files (`<data_dir>/blobs`).
    root: PathBuf,
    /// Connection to `meta.db` for the `blob_refs` ledger.
    conn: Arc<Mutex<Connection>>,
}

impl LocalFsBackend {
    /// Open a backend rooted at `data_dir` (blobs under `data_dir/blobs`, ledger
    /// in `data_dir/meta.db`). The `blob_refs` table is expected to exist
    /// (created by migration v086 when `Storage` boots).
    pub fn open(data_dir: &Path) -> Result<Self, AppError> {
        let conn = Connection::open(data_dir.join("meta.db"))
            .map_err(|e| AppError::Internal(format!("open meta.db for blob backend: {e}")))?;
        // Match the WAL mode the primary Storage connection uses so concurrent
        // access from both connections is safe.
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        Ok(Self {
            root: data_dir.join("blobs"),
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// On-disk path for `hash`: `<root>/<aa>/<bb>/<hash>` (2-level sharding).
    fn blob_path(&self, hash: &str) -> PathBuf {
        let aa = &hash[0..2];
        let bb = &hash[2..4];
        self.root.join(aa).join(bb).join(hash)
    }
}

#[async_trait]
impl StorageBackend for LocalFsBackend {
    fn name(&self) -> &str {
        BACKEND_NAME
    }

    async fn put(&self, bytes: &[u8], content_type: &str) -> Result<BlobRef, AppError> {
        let hash = sha256_hex(bytes);
        let size = bytes.len() as u64;

        // Hold the ledger lock across the check + (conditional) write + upsert so
        // a concurrent put of the same hash can't double-write the file. No
        // `.await` happens inside this scope.
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
            // Dedupe: identical content already stored — bump refcount, do NOT
            // rewrite the file.
            conn.execute(
                "UPDATE blob_refs SET refcount = refcount + 1 WHERE hash = ?1",
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

        // New blob: write the file atomically (tmp + rename), then record it.
        let path = self.blob_path(&hash);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Internal(format!("create blob dir: {e}")))?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes).map_err(|e| AppError::Internal(format!("write blob: {e}")))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| AppError::Internal(format!("rename blob: {e}")))?;

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

        Ok(BlobRef {
            hash,
            backend: BACKEND_NAME.to_string(),
            size,
            content_type: content_type.to_string(),
        })
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, AppError> {
        let path = self.blob_path(key);
        std::fs::read(&path).map_err(|_| AppError::NotFound(format!("blob not found: {key}")))
    }

    async fn head(&self, key: &str) -> Result<BlobMeta, AppError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT hash, backend, size, content_type, refcount, created_at \
             FROM blob_refs WHERE hash = ?1",
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
            // Missing key → no-op.
            None => Ok(()),
            // Last reference → drop the row and unlink the file.
            Some(n) if n <= 1 => {
                conn.execute("DELETE FROM blob_refs WHERE hash = ?1", [key])
                    .map_err(|e| db_err("delete row", e))?;
                let path = self.blob_path(key);
                if path.exists() {
                    std::fs::remove_file(&path)
                        .map_err(|e| AppError::Internal(format!("remove blob: {e}")))?;
                }
                Ok(())
            }
            // Still referenced elsewhere → just decrement.
            Some(_) => {
                conn.execute(
                    "UPDATE blob_refs SET refcount = refcount - 1 WHERE hash = ?1",
                    [key],
                )
                .map_err(|e| db_err("refcount drop", e))?;
                Ok(())
            }
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, AppError> {
        let conn = self.conn.lock();
        let found: Option<i64> = conn
            .query_row("SELECT 1 FROM blob_refs WHERE hash = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| db_err("exists", e))?;
        Ok(found.is_some())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<BlobRef>, AppError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT hash, backend, size, content_type FROM blob_refs \
                 WHERE hash LIKE ?1 ORDER BY created_at DESC, hash ASC",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    /// Build a backend over a fresh migrated `meta.db` in a temp dir.
    fn make_backend() -> (tempfile::TempDir, LocalFsBackend) {
        let dir = tempfile::tempdir().unwrap();
        // Storage::new runs migrations → creates blob_refs (v086).
        Storage::new(dir.path());
        let backend = LocalFsBackend::open(dir.path()).unwrap();
        (dir, backend)
    }

    #[tokio::test]
    async fn put_get_round_trip() {
        let (_dir, backend) = make_backend();
        let data = b"hello staas keystone";

        let blob = backend.put(data, "text/plain").await.unwrap();
        assert_eq!(blob.size, data.len() as u64);
        assert_eq!(blob.backend, "local");
        assert_eq!(blob.content_type, "text/plain");
        assert_eq!(blob.hash.len(), 64, "sha256 hex key");

        let got = backend.get(&blob.hash).await.unwrap();
        assert_eq!(got, data);
    }

    #[tokio::test]
    async fn content_addressed_shards_by_hash_prefix() {
        let (dir, backend) = make_backend();
        let blob = backend
            .put(b"shard me", "application/octet-stream")
            .await
            .unwrap();

        let aa = &blob.hash[0..2];
        let bb = &blob.hash[2..4];
        let expected = dir.path().join("blobs").join(aa).join(bb).join(&blob.hash);
        assert!(
            expected.exists(),
            "blob must live at the sharded path {}",
            expected.display()
        );
    }

    #[tokio::test]
    async fn dedupe_same_bytes_bumps_refcount_without_rewriting() {
        let (_dir, backend) = make_backend();
        let data = b"dedupe me";

        let first = backend.put(data, "text/plain").await.unwrap();
        let path = backend.blob_path(&first.hash);
        let mtime1 = std::fs::metadata(&path).unwrap().modified().unwrap();

        // Same bytes again → same hash, refcount 2, file untouched.
        let second = backend.put(data, "text/plain").await.unwrap();
        assert_eq!(first.hash, second.hash);

        let meta = backend.head(&first.hash).await.unwrap();
        assert_eq!(meta.refcount, 2, "second put must bump the refcount");

        let mtime2 = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2, "dedupe must NOT rewrite the blob file");
    }

    #[tokio::test]
    async fn delete_decrements_refcount_then_removes_on_zero() {
        let (_dir, backend) = make_backend();
        let data = b"refcounted bytes";

        backend.put(data, "text/plain").await.unwrap();
        let blob = backend.put(data, "text/plain").await.unwrap(); // refcount = 2
        let path = backend.blob_path(&blob.hash);

        // First delete → refcount 1, file still present, still exists.
        backend.delete(&blob.hash).await.unwrap();
        assert_eq!(backend.head(&blob.hash).await.unwrap().refcount, 1);
        assert!(path.exists());
        assert!(backend.exists(&blob.hash).await.unwrap());

        // Second delete → refcount 0, row + file gone.
        backend.delete(&blob.hash).await.unwrap();
        assert!(!backend.exists(&blob.hash).await.unwrap());
        assert!(
            !path.exists(),
            "file must be unlinked when refcount hits zero"
        );
        assert!(matches!(
            backend.head(&blob.hash).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            backend.get(&blob.hash).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn delete_missing_key_is_noop() {
        let (_dir, backend) = make_backend();
        backend
            .delete("0000000000000000000000000000000000000000000000000000000000000000")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_filters_by_prefix() {
        let (_dir, backend) = make_backend();
        let a = backend.put(b"alpha", "text/plain").await.unwrap();
        let b = backend.put(b"beta", "text/plain").await.unwrap();

        // Empty prefix lists everything.
        let all = backend.list("").await.unwrap();
        assert_eq!(all.len(), 2);

        // A prefix narrows to the matching hash.
        let only_a = backend.list(&a.hash[0..8]).await.unwrap();
        assert!(only_a.iter().any(|r| r.hash == a.hash));
        assert!(!only_a.iter().any(|r| r.hash == b.hash) || a.hash[0..8] == b.hash[0..8]);

        // A prefix that matches nothing returns empty.
        let none = backend.list("zzzzzzzz").await.unwrap();
        assert!(none.is_empty());
    }
}
