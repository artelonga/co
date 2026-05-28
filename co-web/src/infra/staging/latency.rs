//! `LatencyInjectedStorage` — adds configurable per-call latency to any `Storage` impl.
//!
//! Uses `std::thread::sleep` because the `Storage` trait is synchronous.
//! In staging mode the blocking is intentional — it simulates production-grade
//! storage latency on every call so realistic timing shows up in traces.
//!
//! Set `latency_ms = 0` to make this a transparent pass-through.

use std::path::PathBuf;
use std::sync::Arc;

use crate::entry_index::{EntryRow, TagCount, TreeNode};
use crate::infra::storage::Storage;
use crate::models::Universe;

/// Wraps any `Storage` impl and sleeps `latency_ms` before forwarding each call.
pub struct LatencyInjectedStorage {
    inner: Arc<dyn Storage>,
    latency_ms: u64,
}

impl LatencyInjectedStorage {
    pub fn new(inner: Arc<dyn Storage>, latency_ms: u64) -> Self {
        Self { inner, latency_ms }
    }

    fn inject(&self) {
        if self.latency_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(self.latency_ms));
        }
    }
}

impl Storage for LatencyInjectedStorage {
    fn get_universe(&self, universe_key: &str) -> Option<Universe> {
        self.inject();
        self.inner.get_universe(universe_key)
    }

    fn universe_root(&self, universe_key: &str) -> PathBuf {
        self.inject();
        self.inner.universe_root(universe_key)
    }

    fn get_blob(&self, hash: &str) -> Option<Vec<u8>> {
        self.inject();
        self.inner.get_blob(hash)
    }

    fn get_entry(&self, universe_key: &str, path: &str) -> anyhow::Result<Option<EntryRow>> {
        self.inject();
        self.inner.get_entry(universe_key, path)
    }

    fn list_entries(
        &self,
        universe_key: &str,
        entry_type: &str,
        filter: &serde_json::Value,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntryRow>> {
        self.inject();
        self.inner
            .list_entries(universe_key, entry_type, filter, limit)
    }

    fn search_entries(&self, universe_key: &str, query: &str) -> anyhow::Result<Vec<EntryRow>> {
        self.inject();
        self.inner.search_entries(universe_key, query)
    }

    fn list_entries_by_date(
        &self,
        universe_key: &str,
        date_semantic: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> anyhow::Result<Vec<EntryRow>> {
        self.inject();
        self.inner
            .list_entries_by_date(universe_key, date_semantic, from, to)
    }

    fn list_entries_by_prefix(
        &self,
        universe_key: &str,
        prefix: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntryRow>> {
        self.inject();
        self.inner
            .list_entries_by_prefix(universe_key, prefix, limit)
    }

    fn list_entry_tags(&self, universe_key: &str) -> anyhow::Result<Vec<TagCount>> {
        self.inject();
        self.inner.list_entry_tags(universe_key)
    }

    fn entry_tree(&self, universe_key: &str, entry_type: &str) -> anyhow::Result<Vec<TreeNode>> {
        self.inject();
        self.inner.entry_tree(universe_key, entry_type)
    }

    fn put_entry(&self, universe_key: &str, entry: &co::entry::Entry) -> anyhow::Result<()> {
        self.inject();
        self.inner.put_entry(universe_key, entry)
    }

    fn delete_entry(&self, universe_key: &str, path: &str) -> anyhow::Result<()> {
        self.inject();
        self.inner.delete_entry(universe_key, path)
    }

    fn universe_conn(&self, universe_key: &str) -> Arc<std::sync::Mutex<rusqlite::Connection>> {
        self.inner.universe_conn(universe_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::storage::SqliteStorage;
    use tempfile::tempdir;

    fn make_inner() -> Arc<dyn Storage> {
        let dir = tempdir().unwrap();
        let storage = Arc::new(parking_lot::Mutex::new(crate::storage::Storage::new(
            dir.path(),
        )));
        Arc::new(SqliteStorage::from_arc(storage))
    }

    #[test]
    fn zero_latency_is_transparent() {
        let inner = make_inner();
        let wrapped: Arc<dyn Storage> = Arc::new(LatencyInjectedStorage::new(inner, 0));
        assert!(wrapped.get_universe("nonexistent").is_none());
    }

    #[test]
    fn implements_storage_trait_as_arc() {
        let inner = make_inner();
        let _: Arc<dyn Storage> = Arc::new(LatencyInjectedStorage::new(inner, 0));
    }

    #[test]
    fn get_entry_missing_returns_none() {
        let inner = make_inner();
        let wrapped = LatencyInjectedStorage::new(inner, 0);
        let result = wrapped.get_entry("ghost", "no/path.md");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn list_entries_empty_universe() {
        let inner = make_inner();
        let wrapped = LatencyInjectedStorage::new(inner, 0);
        let result = wrapped.list_entries("ghost", "", &serde_json::json!({}), None);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
