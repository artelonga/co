//! Generic async `Cache<K, V>` trait + in-process LRU default implementation.
//!
//! # Design
//!
//! The trait is async so that a future Redis or Memcached backend can plug in
//! without changing call sites. The in-process default (`InProcessLruCache`)
//! uses a `parking_lot::Mutex` — fast, non-blocking for short critical sections,
//! and compatible with both sync and async callers without holding the lock
//! across an `.await` point.
//!
//! # Per-cache configuration
//!
//! Each `InProcessLruCache` carries a `name`. Capacity can be set via environment
//! variable `CO_CACHE_<NAME>_MAX_ENTRIES` (uppercase, hyphens → underscores).
//! Construct with [`InProcessLruCache::from_env`] to pick up the override at
//! startup; fall back to the hard-coded `default_capacity` when the variable is
//! absent or invalid.

use async_trait::async_trait;
use lru::LruCache;
use parking_lot::Mutex;
use std::borrow::Borrow;
use std::hash::Hash;
use std::num::NonZeroUsize;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Async cache abstraction — foundation for pluggable backends (Redis, Memcached).
///
/// The in-process default is [`InProcessLruCache`].
#[async_trait]
pub trait Cache<K, V>: Send + Sync
where
    K: Send + Sync,
    V: Send + Sync + Clone,
{
    async fn get(&self, key: &K) -> Option<V>;
    async fn put(&self, key: K, value: V);
    async fn invalidate(&self, key: &K);
    async fn invalidate_all(&self);
}

// ---------------------------------------------------------------------------
// InProcessLruCache
// ---------------------------------------------------------------------------

/// In-process LRU cache backed by `parking_lot::Mutex<lru::LruCache<K, V>>`.
///
/// Implements [`Cache<K, V>`] and additionally exposes synchronous convenience
/// methods (`get_sync`, `put_and_detect_eviction`, `remove_where`, etc.) for
/// callers that cannot use `.await`.
pub struct InProcessLruCache<K, V> {
    inner: Mutex<LruCache<K, V>>,
    pub name: &'static str,
    capacity: usize,
}

impl<K, V> InProcessLruCache<K, V>
where
    K: Hash + Eq,
{
    pub fn new(name: &'static str, capacity: usize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).expect("capacity must be > 0"),
            )),
            name,
            capacity,
        }
    }

    /// Build from environment variable `CO_CACHE_<NAME>_MAX_ENTRIES`, falling
    /// back to `default_capacity` when the variable is absent or unparseable.
    ///
    /// `name` is used both as the human-readable label and to derive the env
    /// key (uppercase, hyphens replaced with underscores).
    pub fn from_env(name: &'static str, default_capacity: usize) -> Self {
        let env_key = format!(
            "CO_CACHE_{}_MAX_ENTRIES",
            name.to_uppercase().replace('-', "_")
        );
        let capacity = crate::infra::secrets::global()
            .get(&env_key)
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(default_capacity);
        Self::new(name, capacity)
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    // --- Synchronous helpers (not part of the trait) -----------------------

    /// Synchronous get — compatible with non-async callers.
    pub fn get_sync<Q>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
        V: Clone,
    {
        self.inner.lock().get(key).cloned()
    }

    /// Synchronous put.
    pub fn put_sync(&self, key: K, value: V) {
        self.inner.lock().put(key, value);
    }

    /// Put `value` under `key` and return `true` if an LRU eviction occurred.
    pub fn put_and_detect_eviction(&self, key: K, value: V) -> bool {
        let mut guard = self.inner.lock();
        let cap = guard.cap().get();
        let will_evict = guard.len() == cap && !guard.contains(&key);
        guard.put(key, value);
        will_evict
    }

    /// Synchronous key removal.
    pub fn invalidate_sync<Q>(&self, key: &Q)
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.lock().pop(key);
    }

    /// Synchronous cache clear.
    pub fn invalidate_all_sync(&self) {
        self.inner.lock().clear();
    }

    /// Remove all entries for which `pred(&key)` returns `true`.
    pub fn remove_where<F>(&self, mut pred: F)
    where
        F: FnMut(&K) -> bool,
        K: Clone,
    {
        let mut guard = self.inner.lock();
        let to_remove: Vec<K> = guard
            .iter()
            .filter(|(k, _)| pred(k))
            .map(|(k, _)| k.clone())
            .collect();
        for k in to_remove {
            guard.pop(&k);
        }
    }
}

// ---------------------------------------------------------------------------
// Cache<K, V> impl
// ---------------------------------------------------------------------------

#[async_trait]
impl<K, V> Cache<K, V> for InProcessLruCache<K, V>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    async fn get(&self, key: &K) -> Option<V> {
        self.get_sync(key)
    }

    async fn put(&self, key: K, value: V) {
        self.put_sync(key, value);
    }

    async fn invalidate(&self, key: &K) {
        self.invalidate_sync(key);
    }

    async fn invalidate_all(&self) {
        self.invalidate_all_sync();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make<K: Hash + Eq, V>() -> InProcessLruCache<K, V> {
        InProcessLruCache::new("test", 10)
    }

    #[tokio::test]
    async fn get_miss_then_hit() {
        let c: InProcessLruCache<String, String> = make();
        assert!(c.get(&"k".to_string()).await.is_none());
        c.put("k".to_string(), "v".to_string()).await;
        assert_eq!(c.get(&"k".to_string()).await.as_deref(), Some("v"));
    }

    #[tokio::test]
    async fn invalidate_removes_entry() {
        let c: InProcessLruCache<String, u32> = make();
        c.put("k".to_string(), 1u32).await;
        c.invalidate(&"k".to_string()).await;
        assert!(c.get(&"k".to_string()).await.is_none());
    }

    #[tokio::test]
    async fn invalidate_all_clears_everything() {
        let c: InProcessLruCache<String, u32> = make();
        c.put("a".to_string(), 1u32).await;
        c.put("b".to_string(), 2u32).await;
        c.invalidate_all().await;
        assert!(c.get(&"a".to_string()).await.is_none());
        assert!(c.get(&"b".to_string()).await.is_none());
    }

    #[test]
    fn put_and_detect_eviction_fires_at_capacity() {
        let c: InProcessLruCache<String, u32> = InProcessLruCache::new("test", 2);
        assert!(!c.put_and_detect_eviction("a".to_string(), 1));
        assert!(!c.put_and_detect_eviction("b".to_string(), 2)); // fills capacity
        assert!(c.put_and_detect_eviction("c".to_string(), 3)); // evicts LRU
    }

    #[test]
    fn remove_where_removes_matching_keys() {
        let c: InProcessLruCache<String, u32> = InProcessLruCache::new("test", 10);
        c.put_sync("prefix:a".to_string(), 1);
        c.put_sync("prefix:b".to_string(), 2);
        c.put_sync("other:c".to_string(), 3);
        c.remove_where(|k| k.starts_with("prefix:"));
        assert!(c.get_sync("prefix:a").is_none());
        assert!(c.get_sync("prefix:b").is_none());
        assert!(c.get_sync("other:c").is_some());
    }

    #[test]
    fn get_sync_accepts_str_key_for_string_cache() {
        let c: InProcessLruCache<String, u32> = make();
        c.put_sync("hello".to_string(), 42);
        assert_eq!(c.get_sync("hello"), Some(42)); // &str, not &String
    }

    #[tokio::test]
    async fn trait_object_works() {
        let c: Box<dyn Cache<String, String>> = Box::new(InProcessLruCache::new("test", 5));
        c.put("key".to_string(), "val".to_string()).await;
        assert_eq!(c.get(&"key".to_string()).await.as_deref(), Some("val"));
    }

    #[test]
    fn from_env_uses_default_when_var_absent() {
        // Safety: single-threaded test; no other thread reads this var concurrently.
        unsafe { std::env::remove_var("CO_CACHE_TEST_CACHE_MAX_ENTRIES") };
        let c: InProcessLruCache<String, u32> = InProcessLruCache::from_env("test-cache", 77);
        assert_eq!(c.capacity(), 77);
    }
}
