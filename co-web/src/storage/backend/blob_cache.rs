//! CO-81: on-disk LRU blob cache for the S3 [`StorageBackend`](super::s3::S3Backend).
//!
//! Cold blobs live in remote object storage (~30 ms per GET). Hot blobs are
//! mirrored to a local, size-bounded directory (`/data/blob-cache/`) so repeat
//! reads stay ~1 ms. The cache is **content-addressed** (same 2-level hash
//! sharding as the backends) and **ephemeral**: it can be wiped at any time
//! without data loss because the durable copy is always in object storage.
//!
//! Eviction is least-recently-used by total bytes: once the cache exceeds
//! `max_bytes`, the least-recently-read blobs are unlinked until it fits. The
//! index lives in memory only — a fresh process starts cold and re-warms on
//! demand.

use std::path::PathBuf;

use lru::LruCache;
use parking_lot::Mutex;

/// Size-bounded, content-addressed LRU cache backed by the local filesystem.
pub struct BlobCache {
    root: PathBuf,
    max_bytes: u64,
    inner: Mutex<CacheInner>,
}

struct CacheInner {
    /// `hash → byte size`, ordered by recency (front = most-recently used).
    entries: LruCache<String, u64>,
    /// Sum of all cached blob sizes; kept in lockstep with `entries`.
    total: u64,
}

impl BlobCache {
    /// Create a cache rooted at `root`, holding at most `max_bytes` of blobs.
    ///
    /// `max_bytes` is treated as a soft cap: a single blob larger than the cap
    /// is written then immediately evicted (never cached), so an oversized blob
    /// can never wedge the cache. The index starts empty (cold) regardless of
    /// any files already on disk.
    pub fn new(root: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            root: root.into(),
            max_bytes,
            // Eviction is byte-driven, not count-driven, so the index itself is
            // unbounded by count; `put`/`get` enforce the byte cap manually.
            inner: Mutex::new(CacheInner {
                entries: LruCache::unbounded(),
                total: 0,
            }),
        }
    }

    /// On-disk path for `hash`: `<root>/<aa>/<bb>/<hash>` (2-level sharding).
    fn cache_path(&self, hash: &str) -> PathBuf {
        let aa = &hash[0..2];
        let bb = &hash[2..4];
        self.root.join(aa).join(bb).join(hash)
    }

    /// Return the cached bytes for `hash`, marking it most-recently used.
    ///
    /// A miss (or a file that vanished underneath us) returns `None`; the stale
    /// index entry, if any, is dropped.
    pub fn get(&self, hash: &str) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock();
        // `LruCache::get` bumps recency; clone the size out so we can drop the
        // borrow before touching the filesystem.
        let known = inner.entries.get(hash).copied();
        if let Some(size) = known {
            match std::fs::read(self.cache_path(hash)) {
                Ok(bytes) => return Some(bytes),
                Err(_) => {
                    // File evicted/corrupted outside our control — forget it.
                    inner.entries.pop(hash);
                    inner.total = inner.total.saturating_sub(size);
                }
            }
        }
        None
    }

    /// Mirror `bytes` into the cache under `hash`, evicting LRU blobs until the
    /// total fits within `max_bytes`. Cache failures are swallowed (best-effort)
    /// — a cache write must never break the caller's read/put path.
    pub fn put(&self, hash: &str, bytes: &[u8]) {
        let path = self.cache_path(hash);
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, bytes).is_err() {
            return;
        }
        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }

        let size = bytes.len() as u64;
        let mut inner = self.inner.lock();
        if let Some(old) = inner.entries.put(hash.to_string(), size) {
            // Replacing an existing entry — discount its old size first.
            inner.total = inner.total.saturating_sub(old);
        }
        inner.total += size;
        self.evict_to_fit(&mut inner);
    }

    /// Drop `hash` from the cache (index + file). No-op if absent.
    pub fn remove(&self, hash: &str) {
        let mut inner = self.inner.lock();
        if let Some(size) = inner.entries.pop(hash) {
            inner.total = inner.total.saturating_sub(size);
        }
        let _ = std::fs::remove_file(self.cache_path(hash));
    }

    /// Current number of cached blobs (testing/observability).
    pub fn len(&self) -> usize {
        self.inner.lock().entries.len()
    }

    /// `true` when the cache holds no blobs.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Current total bytes held by the cache (testing/observability).
    pub fn total_bytes(&self) -> u64 {
        self.inner.lock().total
    }

    /// Evict least-recently-used blobs until `total <= max_bytes`.
    fn evict_to_fit(&self, inner: &mut CacheInner) {
        while inner.total > self.max_bytes {
            match inner.entries.pop_lru() {
                Some((hash, size)) => {
                    inner.total = inner.total.saturating_sub(size);
                    let _ = std::fs::remove_file(self.cache_path(&hash));
                }
                // Index empty but still over budget (shouldn't happen) — stop.
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> String {
        // Distinct 64-char hex keys with distinct 4-char shard prefixes.
        format!("{:02x}{:02x}{}", n, n, "0".repeat(60))
    }

    fn make_cache(max_bytes: u64) -> (tempfile::TempDir, BlobCache) {
        let dir = tempfile::tempdir().unwrap();
        let cache = BlobCache::new(dir.path().join("blob-cache"), max_bytes);
        (dir, cache)
    }

    #[test]
    fn put_then_get_round_trips() {
        let (_dir, cache) = make_cache(1024);
        let k = key(1);
        assert!(cache.get(&k).is_none(), "miss before put");
        cache.put(&k, b"hello cache");
        assert_eq!(cache.get(&k).as_deref(), Some(&b"hello cache"[..]));
    }

    #[test]
    fn shards_by_hash_prefix() {
        let (dir, cache) = make_cache(1024);
        let k = key(0xab);
        cache.put(&k, b"data");
        let expected = dir
            .path()
            .join("blob-cache")
            .join(&k[0..2])
            .join(&k[2..4])
            .join(&k);
        assert!(expected.exists(), "blob must live at the sharded path");
    }

    #[test]
    fn evicts_least_recently_used_when_over_capacity() {
        // Capacity for ~2 blobs of 4 bytes each.
        let (_dir, cache) = make_cache(8);
        let (a, b, c) = (key(1), key(2), key(3));
        cache.put(&a, b"aaaa");
        cache.put(&b, b"bbbb");
        // Touch `a` so `b` becomes the LRU victim.
        assert!(cache.get(&a).is_some());
        cache.put(&c, b"cccc"); // pushes total to 12 > 8 → evict LRU (`b`)

        assert!(cache.get(&a).is_some(), "recently-used survives");
        assert!(cache.get(&c).is_some(), "newest survives");
        assert!(cache.get(&b).is_none(), "LRU was evicted");
        assert!(cache.total_bytes() <= 8);
    }

    #[test]
    fn overwriting_same_key_does_not_double_count() {
        let (_dir, cache) = make_cache(1024);
        let k = key(1);
        cache.put(&k, b"aaaa");
        cache.put(&k, b"aaaa");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_bytes(), 4);
    }

    #[test]
    fn remove_drops_index_and_file() {
        let (_dir, cache) = make_cache(1024);
        let k = key(1);
        cache.put(&k, b"bytes");
        assert!(!cache.is_empty());
        cache.remove(&k);
        assert!(cache.is_empty());
        assert!(cache.get(&k).is_none());
        assert!(!cache.cache_path(&k).exists());
    }

    #[test]
    fn get_recovers_from_externally_deleted_file() {
        let (_dir, cache) = make_cache(1024);
        let k = key(1);
        cache.put(&k, b"bytes");
        // Simulate an external wipe of the cache directory.
        std::fs::remove_file(cache.cache_path(&k)).unwrap();
        assert!(cache.get(&k).is_none(), "missing file → clean miss");
        assert!(cache.is_empty(), "stale index entry dropped");
    }

    #[test]
    fn oversized_blob_is_evicted_immediately() {
        let (_dir, cache) = make_cache(4);
        let k = key(1);
        cache.put(&k, b"way too big for the cap");
        // Larger than the cap → not retained.
        assert!(cache.total_bytes() <= 4);
        assert!(cache.get(&k).is_none());
    }
}
