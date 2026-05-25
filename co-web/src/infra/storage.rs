//! Storage trait abstraction for CO — CO-290.
//!
//! `Storage` decouples route handlers from the rusqlite-backed implementation.
//! `SqliteStorage` is the production impl; future impls (Postgres, in-memory
//! test doubles) can slot in without touching any route code.
//!
//! All methods take `&self` so the trait object can be held behind `Arc<dyn Storage>`.
//! Write methods achieve interior mutability via the `parking_lot::Mutex` inside
//! `SqliteStorage`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::entry_index::{EntryRow, TagCount, TreeNode};
use crate::models::Universe;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Backend-agnostic storage interface.
///
/// All methods take `&self` — implementors use interior mutability for writes.
/// Hold as `Arc<dyn Storage>` in `AppState`.
pub trait Storage: Send + Sync {
    // --- Universe metadata ---
    fn get_universe(&self, universe_key: &str) -> Option<Universe>;
    fn universe_root(&self, universe_key: &str) -> PathBuf;
    fn get_blob(&self, hash: &str) -> Option<Vec<u8>>;

    // --- Entry reads ---
    fn get_entry(&self, universe_key: &str, path: &str) -> anyhow::Result<Option<EntryRow>>;

    fn list_entries(
        &self,
        universe_key: &str,
        entry_type: &str,
        filter: &serde_json::Value,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntryRow>>;

    fn search_entries(&self, universe_key: &str, query: &str) -> anyhow::Result<Vec<EntryRow>>;

    fn list_entries_by_date(
        &self,
        universe_key: &str,
        date_semantic: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> anyhow::Result<Vec<EntryRow>>;

    fn list_entries_by_prefix(
        &self,
        universe_key: &str,
        prefix: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntryRow>>;

    fn list_entry_tags(&self, universe_key: &str) -> anyhow::Result<Vec<TagCount>>;

    fn entry_tree(&self, universe_key: &str, entry_type: &str) -> anyhow::Result<Vec<TreeNode>>;

    // --- Entry writes ---
    fn put_entry(&self, universe_key: &str, entry: &co::entry::Entry) -> anyhow::Result<()>;
    fn delete_entry(&self, universe_key: &str, path: &str) -> anyhow::Result<()>;

    // --- Escape hatch for paths that still need the raw connection ---
    fn universe_conn(&self, universe_key: &str) -> Arc<std::sync::Mutex<rusqlite::Connection>>;
}

// ---------------------------------------------------------------------------
// SqliteStorage
// ---------------------------------------------------------------------------

/// Production `Storage` implementation backed by the existing rusqlite `Storage`.
///
/// Wraps `Arc<parking_lot::Mutex<crate::storage::Storage>>` so it can be
/// shared cheaply between `CoreState.storage` (for legacy call sites that
/// haven't migrated yet) and `CoreState.storage_trait` (the new trait path).
pub struct SqliteStorage {
    inner: Arc<parking_lot::Mutex<crate::storage::Storage>>,
}

impl SqliteStorage {
    /// Construct from a shared Arc — caller shares the same mutex with
    /// `CoreState.storage` so both paths operate on the same connection.
    pub fn from_arc(inner: Arc<parking_lot::Mutex<crate::storage::Storage>>) -> Self {
        Self { inner }
    }

    fn uc(&self, universe_key: &str) -> Arc<std::sync::Mutex<rusqlite::Connection>> {
        self.inner.lock().universe_conn(universe_key)
    }
}

impl Storage for SqliteStorage {
    fn get_universe(&self, universe_key: &str) -> Option<Universe> {
        self.inner.lock().get_universe(universe_key)
    }

    fn universe_root(&self, universe_key: &str) -> PathBuf {
        self.inner.lock().universe_root(universe_key)
    }

    fn get_blob(&self, hash: &str) -> Option<Vec<u8>> {
        self.inner.lock().get_blob(hash)
    }

    fn get_entry(&self, universe_key: &str, path: &str) -> anyhow::Result<Option<EntryRow>> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).get(universe_key, path)
    }

    fn list_entries(
        &self,
        universe_key: &str,
        entry_type: &str,
        filter: &serde_json::Value,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntryRow>> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).query_with_limit(
            universe_key,
            entry_type,
            filter,
            limit,
        )
    }

    fn search_entries(&self, universe_key: &str, query: &str) -> anyhow::Result<Vec<EntryRow>> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).search(universe_key, query)
    }

    fn list_entries_by_date(
        &self,
        universe_key: &str,
        date_semantic: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> anyhow::Result<Vec<EntryRow>> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).query_by_date(
            universe_key,
            date_semantic,
            from,
            to,
        )
    }

    fn list_entries_by_prefix(
        &self,
        universe_key: &str,
        prefix: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntryRow>> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).query_by_path_prefix(
            universe_key,
            prefix,
            limit,
        )
    }

    fn list_entry_tags(&self, universe_key: &str) -> anyhow::Result<Vec<TagCount>> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).tags(universe_key)
    }

    fn entry_tree(&self, universe_key: &str, entry_type: &str) -> anyhow::Result<Vec<TreeNode>> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).tree(universe_key, entry_type)
    }

    fn put_entry(&self, universe_key: &str, entry: &co::entry::Entry) -> anyhow::Result<()> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).upsert(universe_key, entry)
    }

    fn delete_entry(&self, universe_key: &str, path: &str) -> anyhow::Result<()> {
        let uc = self.uc(universe_key);
        let guard = uc
            .lock()
            .map_err(|_| anyhow::anyhow!("universe conn lock poisoned"))?;
        crate::entry_index::EntryIndex::new(&guard).remove(universe_key, path)
    }

    fn universe_conn(&self, universe_key: &str) -> Arc<std::sync::Mutex<rusqlite::Connection>> {
        self.inner.lock().universe_conn(universe_key)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_storage(dir: &std::path::Path) -> Arc<parking_lot::Mutex<crate::storage::Storage>> {
        Arc::new(parking_lot::Mutex::new(crate::storage::Storage::new(dir)))
    }

    #[test]
    fn sqlite_storage_get_entry_missing_returns_none() {
        let dir = tempdir().unwrap();
        let inner = make_storage(dir.path());
        let st = SqliteStorage::from_arc(inner);

        let result = st.get_entry("no-such-universe", "no/such/path.md");
        assert!(
            result.is_ok(),
            "get_entry on missing path should return Ok(None), not Err"
        );
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn sqlite_storage_list_entries_empty_universe() {
        let dir = tempdir().unwrap();
        let inner = make_storage(dir.path());
        let st = SqliteStorage::from_arc(inner);

        let result = st.list_entries("ghost", "", &serde_json::json!({}), None);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn sqlite_storage_implements_storage_trait() {
        let dir = tempdir().unwrap();
        let inner = make_storage(dir.path());
        let st: Arc<dyn Storage> = Arc::new(SqliteStorage::from_arc(inner));

        // Trait object is Send + Sync — verify it compiles and basic calls work.
        assert!(st.get_universe("nonexistent").is_none());
    }
}
