//! Cache layer — short-term memoization, keyed by `CoFile.path`.

use std::num::NonZeroUsize;
use std::sync::Mutex;

use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

/// In-process, non-shared LRU cache. Sits *above* storage/transport so it is
/// closer to the consumer than the durable backing.
///
/// As a composed [`Layer`] it write-throughs and read-throughs transparently:
/// every observed `CoFile` is promoted into the cache, and the value is passed
/// along unchanged. Its standalone [`get`](LruCache::get) / [`put`](LruCache::put)
/// API exercises the cache directly (used by callers that want a real
/// short-circuit before hitting the layer below).
pub struct LruCache {
    inner: Mutex<lru::LruCache<String, CoFile>>,
}

impl LruCache {
    /// Create a cache holding at most `capacity` entries (min 1).
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Mutex::new(lru::LruCache::new(cap)),
        }
    }

    /// Promote `value` under `path` (eviction on capacity).
    pub fn put(&self, path: &str, value: CoFile) {
        self.inner.lock().unwrap().put(path.to_string(), value);
    }

    /// Fetch a cached `CoFile`, promoting it to most-recently-used.
    pub fn get(&self, path: &str) -> Option<CoFile> {
        self.inner.lock().unwrap().get(path).cloned()
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Whether the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Layer for LruCache {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        // Read flows bottom→top: the layer below already produced the value;
        // promote it and pass it along.
        self.put(&input.path, input.clone());
        Ok(input)
    }

    fn write(&self, input: CoFile) -> Result<CoFile, LayerError> {
        // Write flows top→bottom: cache the (plaintext) value, pass it down.
        self.put(&input.path, input.clone());
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cofile(path: &str, body: &[u8]) -> CoFile {
        CoFile {
            path: path.into(),
            content: body.to_vec(),
            sha256: String::new(),
            mime_type: "text/markdown".into(),
            modified_ns: 0,
        }
    }

    #[test]
    fn put_get_round_trips() {
        let c = LruCache::new(4);
        assert!(c.is_empty());
        c.put("a.md", cofile("a.md", b"alpha"));
        assert_eq!(c.get("a.md").unwrap().content, b"alpha");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn evicts_least_recently_used() {
        let c = LruCache::new(2);
        c.put("a", cofile("a", b"1"));
        c.put("b", cofile("b", b"2"));
        let _ = c.get("a"); // touch a → b is now LRU
        c.put("c", cofile("c", b"3")); // evicts b
        assert!(c.get("b").is_none());
        assert!(c.get("a").is_some());
        assert!(c.get("c").is_some());
    }

    #[test]
    fn layer_write_and_read_populate_cache() {
        let c = LruCache::new(8);
        c.write(cofile("x.md", b"body")).unwrap();
        assert_eq!(c.get("x.md").unwrap().content, b"body");
        let out = c.read(cofile("y.md", b"other")).unwrap();
        assert_eq!(out.content, b"other");
        assert_eq!(c.get("y.md").unwrap().content, b"other");
    }
}
