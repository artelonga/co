//! In-process caching layer — CO-79.
//!
//! Three independent caches, all L1 in-process LRU (10 K entries each):
//!
//! | Cache      | Key                             | Value               |
//! |------------|---------------------------------|---------------------|
//! | manifest   | universe slug                   | Arc<Manifest>       |
//! | theme_css  | ETag (content hash)             | Arc<String> (CSS)   |
//! | query      | SHA-256(params+slug+mhash)      | Arc<Vec<u8>> (JSON) |
//!
//! Stampede protection: `ManifestCache::get_or_fill` uses a per-slug async
//! mutex so that N concurrent cache-misses for the same slug result in exactly
//! one fill call; the rest wait and share the cached result.
//!
//! In-process invalidation pub/sub: `CacheLayer::invalidate_universe` removes
//! the manifest entry and broadcasts the slug on a `tokio::sync::broadcast`
//! channel, allowing any worker task that subscribed at startup to propagate
//! the invalidation (e.g. a future Redis pub/sub bridge).

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use lru::LruCache;

const L1_CAPACITY: usize = 10_000;

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Per-layer hit / miss / eviction counters (lock-free atomics).
pub struct CacheCounters {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl CacheCounters {
    fn new() -> Self {
        Self {
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }
    fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
    pub fn evictions(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }
    pub fn hit_rate(&self) -> f64 {
        let h = self.hits();
        let total = h + self.misses();
        if total == 0 {
            0.0
        } else {
            h as f64 / total as f64
        }
    }
}

// ---------------------------------------------------------------------------
// Manifest cache (L1 LRU + singleflight)
// ---------------------------------------------------------------------------

/// L1 in-process cache for parsed universe manifests (`_universe.yaml`).
///
/// Singleflight coalescing: when multiple tasks concurrently miss the cache for
/// the same slug, only one executes the fill closure; the rest wait on an async
/// mutex and share the result once the fill completes.
pub struct ManifestCache {
    lru: Mutex<LruCache<String, Arc<co::manifest::Manifest>>>,
    /// Per-slug async mutex used as a singleflight gate.
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub counters: CacheCounters,
}

impl Default for ManifestCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ManifestCache {
    pub fn new() -> Self {
        Self {
            lru: Mutex::new(LruCache::new(
                NonZeroUsize::new(L1_CAPACITY).expect("L1_CAPACITY > 0"),
            )),
            inflight: Mutex::new(HashMap::new()),
            counters: CacheCounters::new(),
        }
    }

    /// Synchronous get — fast path, no singleflight.
    pub fn get(&self, slug: &str) -> Option<Arc<co::manifest::Manifest>> {
        let mut lru = self.lru.lock().unwrap();
        match lru.get(slug) {
            Some(v) => {
                self.counters.record_hit();
                Some(Arc::clone(v))
            }
            None => {
                self.counters.record_miss();
                None
            }
        }
    }

    /// Insert into the LRU, tracking evictions.
    pub fn insert(&self, slug: String, manifest: co::manifest::Manifest) {
        let mut lru = self.lru.lock().unwrap();
        let cap = lru.cap().get();
        let will_evict = lru.len() == cap && !lru.contains(&slug);
        lru.put(slug, Arc::new(manifest));
        if will_evict {
            self.counters.record_eviction();
        }
    }

    /// Remove a slug from the cache (called on universe update / manifest write).
    pub fn invalidate(&self, slug: &str) {
        self.lru.lock().unwrap().pop(slug);
    }

    /// Get or fill with singleflight coalescing.
    ///
    /// - Cache hit → returns immediately (~nanoseconds, no locking beyond the LRU mutex).
    /// - Cache miss, no concurrent fill → calls `fill()`, caches result, wakes waiters.
    /// - Cache miss, concurrent fill in progress → waits for the filler and shares result.
    pub async fn get_or_fill<F, Fut>(
        &self,
        slug: String,
        fill: F,
    ) -> Option<Arc<co::manifest::Manifest>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Option<co::manifest::Manifest>> + Send,
    {
        // Fast path — avoids singleflight overhead on cache hit.
        if let Some(v) = self.get(&slug) {
            return Some(v);
        }

        // Acquire (or create) the per-slug async fill-gate.
        let lock = {
            let mut inflight = self.inflight.lock().unwrap();
            inflight
                .entry(slug.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };

        // Attempt non-blocking acquisition — first caller wins the fill role.
        match lock.clone().try_lock_owned() {
            Ok(guard) => {
                // Double-check: another filler may have finished between the fast
                // path above and now.
                if let Some(v) = self.get(&slug) {
                    drop(guard);
                    self.inflight.lock().unwrap().remove(&slug);
                    return Some(v);
                }

                let manifest_opt = fill().await;

                if let Some(ref m) = manifest_opt {
                    self.insert(slug.clone(), m.clone());
                }

                // Release guard AFTER insertion so waiters see the cached value.
                drop(guard);
                self.inflight.lock().unwrap().remove(&slug);

                // Return from LRU to hand back the Arc that was just inserted.
                manifest_opt.and_then(|_| self.get(&slug))
            }
            Err(_) => {
                // Another task is filling — wait for it to complete.
                let _guard = lock.lock_owned().await;
                drop(_guard);
                // Filler has finished; the inflight entry may already be removed.
                self.get(&slug)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Theme CSS cache (L1 LRU)
// ---------------------------------------------------------------------------

/// L1 in-process cache for generated theme CSS.
///
/// Keyed by ETag (a hash of theme-preset + custom-tokens). When the universe
/// config changes, the ETag changes, so old entries are never served again and
/// will be evicted by LRU over time. No active invalidation needed.
pub struct ThemeCssCache {
    lru: Mutex<LruCache<String, Arc<String>>>,
    pub counters: CacheCounters,
}

impl Default for ThemeCssCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeCssCache {
    pub fn new() -> Self {
        Self {
            lru: Mutex::new(LruCache::new(
                NonZeroUsize::new(L1_CAPACITY).expect("L1_CAPACITY > 0"),
            )),
            counters: CacheCounters::new(),
        }
    }

    pub fn get(&self, etag: &str) -> Option<Arc<String>> {
        let mut lru = self.lru.lock().unwrap();
        match lru.get(etag) {
            Some(v) => {
                self.counters.record_hit();
                Some(Arc::clone(v))
            }
            None => {
                self.counters.record_miss();
                None
            }
        }
    }

    pub fn insert(&self, etag: String, css: String) {
        let mut lru = self.lru.lock().unwrap();
        let cap = lru.cap().get();
        let will_evict = lru.len() == cap && !lru.contains(&etag);
        lru.put(etag, Arc::new(css));
        if will_evict {
            self.counters.record_eviction();
        }
    }
}

// ---------------------------------------------------------------------------
// Query cache (L1 LRU)
// ---------------------------------------------------------------------------

/// L1 query result cache.
///
/// Key: `query_cache_key(params, universe_key, manifest_hash)` — a SHA-256 hex
/// string that uniquely identifies a query against a specific universe schema
/// version. Value: serialized JSON bytes.
///
/// Invalidation: call `invalidate_prefix(slug + ":")` — this removes all keys
/// that were tagged with the universe slug when built via `prefixed_key`.
pub struct QueryCache {
    lru: Mutex<LruCache<String, Arc<Vec<u8>>>>,
    pub counters: CacheCounters,
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryCache {
    pub fn new() -> Self {
        Self {
            lru: Mutex::new(LruCache::new(
                NonZeroUsize::new(L1_CAPACITY).expect("L1_CAPACITY > 0"),
            )),
            counters: CacheCounters::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        let mut lru = self.lru.lock().unwrap();
        match lru.get(key) {
            Some(v) => {
                self.counters.record_hit();
                Some(Arc::clone(v))
            }
            None => {
                self.counters.record_miss();
                None
            }
        }
    }

    pub fn insert(&self, key: String, value: Vec<u8>) {
        let mut lru = self.lru.lock().unwrap();
        let cap = lru.cap().get();
        let will_evict = lru.len() == cap && !lru.contains(&key);
        lru.put(key, Arc::new(value));
        if will_evict {
            self.counters.record_eviction();
        }
    }

    /// Remove all entries whose key starts with `prefix`.
    /// Used for universe-level invalidation: call with `"{slug}:"`.
    pub fn invalidate_prefix(&self, prefix: &str) {
        let mut lru = self.lru.lock().unwrap();
        let keys: Vec<String> = lru
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys {
            lru.pop(&k);
        }
    }
}

// ---------------------------------------------------------------------------
// CacheLayer — combines all three caches
// ---------------------------------------------------------------------------

/// The full CO caching layer, shared via `Arc` in `AppStateInner`.
pub struct CacheLayer {
    pub manifest: ManifestCache,
    pub theme_css: ThemeCssCache,
    pub query: QueryCache,
    /// In-process invalidation broadcast: publishes universe slugs when any
    /// cache entry for that universe is invalidated.
    invalidation_tx: tokio::sync::broadcast::Sender<String>,
}

impl CacheLayer {
    pub fn new() -> Arc<Self> {
        let (invalidation_tx, _) = tokio::sync::broadcast::channel(1024);
        Arc::new(Self {
            manifest: ManifestCache::new(),
            theme_css: ThemeCssCache::new(),
            query: QueryCache::new(),
            invalidation_tx,
        })
    }

    /// Invalidate all cache entries for `slug` and broadcast the event.
    ///
    /// Called from:
    /// - `PUT /api/v1/universes/:slug` (name / description / visibility update)
    /// - `PUT /api/v1/universes/:slug/config` (theme preset / custom tokens)
    /// - Any vault or entry write that modifies `_universe.yaml`
    pub fn invalidate_universe(&self, slug: &str) {
        self.manifest.invalidate(slug);
        self.query.invalidate_prefix(&format!("{slug}:"));
        // theme_css is keyed by ETag, not slug — no active invalidation needed.
        let _ = self.invalidation_tx.send(slug.to_string());
    }

    /// Subscribe to in-process invalidation events.
    ///
    /// Each published value is a universe slug that was just invalidated.
    /// Subscribers can use this to drive cross-worker L2 invalidation (e.g.
    /// sending a Redis `DEL` or pub/sub message for every received slug).
    pub fn subscribe_invalidation(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.invalidation_tx.subscribe()
    }

    /// JSON metrics snapshot (hit rate, eviction rate per layer).
    pub fn stats(&self) -> serde_json::Value {
        serde_json::json!({
            "manifest": {
                "hits": self.manifest.counters.hits(),
                "misses": self.manifest.counters.misses(),
                "evictions": self.manifest.counters.evictions(),
                "hit_rate": self.manifest.counters.hit_rate(),
            },
            "theme_css": {
                "hits": self.theme_css.counters.hits(),
                "misses": self.theme_css.counters.misses(),
                "evictions": self.theme_css.counters.evictions(),
                "hit_rate": self.theme_css.counters.hit_rate(),
            },
            "query": {
                "hits": self.query.counters.hits(),
                "misses": self.query.counters.misses(),
                "evictions": self.query.counters.evictions(),
                "hit_rate": self.query.counters.hit_rate(),
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Query cache key helpers
// ---------------------------------------------------------------------------

/// Build a SHA-256 query cache key prefixed with the universe slug.
///
/// Format: `"{slug}:{hex(SHA-256(params + NUL + universe_key + NUL + manifest_hash))}"`
///
/// The slug prefix allows efficient universe-level invalidation via
/// `QueryCache::invalidate_prefix("{slug}:")`.
pub fn query_cache_key(params: &str, universe_key: &str, manifest_hash: u64) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(params.as_bytes());
    h.update(b"\0");
    h.update(universe_key.as_bytes());
    h.update(b"\0");
    h.update(manifest_hash.to_le_bytes());
    format!("{universe_key}:{:x}", h.finalize())
}

/// Compute a content hash for a manifest (used as part of query cache keys).
///
/// Changes whenever the manifest schema is edited, ensuring that query results
/// cached under the old schema are never served after an update.
pub fn manifest_content_hash(manifest: &co::manifest::Manifest) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{manifest:?}").hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AOrd};

    fn dummy_manifest() -> co::manifest::Manifest {
        co::manifest::default_manifest("test")
    }

    // --- ManifestCache: basic get / insert / invalidate ---

    #[test]
    fn manifest_miss_then_hit() {
        let cache = ManifestCache::new();
        assert!(cache.get("slug").is_none());
        assert_eq!(cache.counters.misses(), 1);

        cache.insert("slug".into(), dummy_manifest());
        assert!(cache.get("slug").is_some());
        assert_eq!(cache.counters.hits(), 1);
    }

    #[test]
    fn manifest_invalidate_clears_entry() {
        let cache = ManifestCache::new();
        cache.insert("slug".into(), dummy_manifest());
        cache.invalidate("slug");
        assert!(cache.get("slug").is_none());
    }

    // --- ManifestCache: repeat read must be sub-millisecond ---

    #[test]
    fn manifest_repeat_read_under_1ms() {
        let cache = ManifestCache::new();
        cache.insert("bench".into(), dummy_manifest());

        let start = std::time::Instant::now();
        for _ in 0..1_000 {
            assert!(cache.get("bench").is_some());
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 1,
            "1 000 LRU reads took {}µs — expected < 1ms total",
            elapsed.as_micros()
        );
    }

    // --- ManifestCache: singleflight — N concurrent misses → 1 fill ---

    #[tokio::test]
    async fn manifest_singleflight_one_fill() {
        let cache = Arc::new(ManifestCache::new());
        let fill_count = Arc::new(AtomicUsize::new(0));

        const CONCURRENCY: usize = 100;
        let handles: Vec<_> = (0..CONCURRENCY)
            .map(|_| {
                let c = Arc::clone(&cache);
                let fc = Arc::clone(&fill_count);
                tokio::spawn(async move {
                    c.get_or_fill("slug".to_string(), || {
                        let fc = Arc::clone(&fc);
                        async move {
                            fc.fetch_add(1, AOrd::SeqCst);
                            // Simulate a ~5ms disk read so others stack up.
                            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                            Some(dummy_manifest())
                        }
                    })
                    .await
                })
            })
            .collect();

        for h in handles {
            assert!(
                h.await.unwrap().is_some(),
                "all tasks must receive a manifest"
            );
        }

        assert_eq!(
            fill_count.load(AOrd::SeqCst),
            1,
            "singleflight must coalesce {CONCURRENCY} concurrent misses to 1 fill"
        );
    }

    // --- ThemeCssCache ---

    #[test]
    fn theme_css_hit_after_insert() {
        let cache = ThemeCssCache::new();
        assert!(cache.get("\"abc\"").is_none());
        cache.insert("\"abc\"".into(), ":root {}".into());
        assert!(cache.get("\"abc\"").is_some());
    }

    // --- QueryCache ---

    #[test]
    fn query_cache_insert_get_invalidate() {
        let cache = QueryCache::new();
        let key = query_cache_key("type=task", "my-universe", 42);
        assert!(
            key.starts_with("my-universe:"),
            "key must be prefixed with slug"
        );

        cache.insert(key.clone(), b"[]".to_vec());
        assert!(cache.get(&key).is_some());

        cache.invalidate_prefix("my-universe:");
        assert!(cache.get(&key).is_none());
    }

    // --- CacheLayer: stats shape ---

    #[test]
    fn cache_layer_stats_has_all_layers() {
        let layer = CacheLayer::new();
        let stats = layer.stats();
        for layer_name in ["manifest", "theme_css", "query"] {
            let obj = stats.get(layer_name).expect("layer present");
            assert!(obj.get("hits").is_some());
            assert!(obj.get("misses").is_some());
            assert!(obj.get("evictions").is_some());
            assert!(obj.get("hit_rate").is_some());
        }
    }

    // --- CacheLayer: invalidation pub/sub ---

    #[tokio::test]
    async fn cache_layer_invalidation_broadcasts_slug() {
        let layer = CacheLayer::new();
        let mut rx = layer.subscribe_invalidation();

        layer.manifest.insert("myslug".into(), dummy_manifest());
        layer.invalidate_universe("myslug");

        let received = rx.try_recv().expect("invalidation event must be broadcast");
        assert_eq!(received, "myslug");
        assert!(
            layer.manifest.get("myslug").is_none(),
            "manifest entry must be removed after invalidation"
        );
    }
}
