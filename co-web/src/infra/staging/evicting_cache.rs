//! `EvictingCache` — simulates cache eviction pressure on any `Cache<K,V>`.
//!
//! On every Nth `put` (where N = ceil(1/eviction_rate)), the just-inserted entry
//! is immediately invalidated — simulating a cache eviction under memory pressure.
//! Subsequent `get` calls for that key return `None`, forcing the caller to
//! re-fetch from storage (the typical production cache-miss path).
//!
//! Pass `eviction_rate = 0.0` to disable eviction pressure entirely.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

use crate::infra::cache::Cache;

/// Wraps any `Cache<K, V>` and evicts entries at a configurable rate.
pub struct EvictingCache<K, V> {
    inner: Arc<dyn Cache<K, V>>,
    /// 0 means never evict; otherwise evict every Nth put.
    evict_every_n: u64,
    put_count: AtomicU64,
}

impl<K, V> EvictingCache<K, V>
where
    K: Send + Sync,
    V: Send + Sync + Clone,
{
    /// Create a new `EvictingCache`.
    ///
    /// `eviction_rate` is clamped to `[0.0, 1.0]`.
    /// Pass `0.0` to make this a transparent pass-through.
    pub fn new(inner: Arc<dyn Cache<K, V>>, eviction_rate: f64) -> Self {
        let rate = eviction_rate.clamp(0.0, 1.0);
        let evict_every_n = if rate <= 0.0 {
            0
        } else {
            (1.0 / rate).round() as u64
        };
        Self {
            inner,
            evict_every_n,
            put_count: AtomicU64::new(0),
        }
    }

    fn should_evict(&self) -> bool {
        if self.evict_every_n == 0 {
            return false;
        }
        let n = self.put_count.fetch_add(1, Ordering::Relaxed) + 1;
        n.is_multiple_of(self.evict_every_n)
    }
}

#[async_trait]
impl<K, V> Cache<K, V> for EvictingCache<K, V>
where
    K: Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    async fn get(&self, key: &K) -> Option<V> {
        self.inner.get(key).await
    }

    async fn put(&self, key: K, value: V) {
        if self.should_evict() {
            // Insert then immediately evict: simulates cache pressure.
            self.inner.put(key.clone(), value).await;
            self.inner.invalidate(&key).await;
        } else {
            self.inner.put(key, value).await;
        }
    }

    async fn invalidate(&self, key: &K) {
        self.inner.invalidate(key).await;
    }

    async fn invalidate_all(&self) {
        self.inner.invalidate_all().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::cache::InProcessLruCache;

    fn make_inner(cap: usize) -> Arc<dyn Cache<String, u32>> {
        Arc::new(InProcessLruCache::new("evicting-cache-test", cap))
    }

    #[tokio::test]
    async fn rate_zero_never_evicts() {
        let inner = make_inner(100);
        let cache = EvictingCache::new(inner, 0.0);
        for i in 0..10u32 {
            cache.put(format!("k{i}"), i).await;
        }
        for i in 0..10u32 {
            assert_eq!(
                cache.get(&format!("k{i}")).await,
                Some(i),
                "key k{i} should still be present at rate=0.0"
            );
        }
    }

    #[tokio::test]
    async fn rate_one_evicts_every_put() {
        let inner = make_inner(100);
        let cache = EvictingCache::new(inner, 1.0);
        cache.put("key".to_string(), 42u32).await;
        // rate=1.0 → evict_every_n=1 → first put immediately evicted
        assert!(cache.get(&"key".to_string()).await.is_none());
    }

    #[tokio::test]
    async fn rate_05_evicts_every_20_puts() {
        let inner = make_inner(1000);
        let cache = EvictingCache::new(inner, 0.05);
        let mut evictions = 0usize;
        for i in 0..100u32 {
            let key = format!("k{i}");
            cache.put(key.clone(), i).await;
            if cache.get(&key).await.is_none() {
                evictions += 1;
            }
        }
        assert_eq!(
            evictions, 5,
            "expected 5 evictions in 100 puts at rate=0.05"
        );
    }

    #[tokio::test]
    async fn implements_cache_trait_as_arc() {
        let inner = make_inner(10);
        let _: Arc<dyn Cache<String, u32>> = Arc::new(EvictingCache::new(inner, 0.0));
    }

    #[tokio::test]
    async fn invalidate_and_invalidate_all_delegate_to_inner() {
        let inner = make_inner(10);
        let cache = EvictingCache::new(Arc::clone(&inner), 0.0);
        cache.put("a".to_string(), 1u32).await;
        cache.put("b".to_string(), 2u32).await;

        cache.invalidate(&"a".to_string()).await;
        assert!(cache.get(&"a".to_string()).await.is_none());
        assert_eq!(cache.get(&"b".to_string()).await, Some(2));

        cache.invalidate_all().await;
        assert!(cache.get(&"b".to_string()).await.is_none());
    }
}
