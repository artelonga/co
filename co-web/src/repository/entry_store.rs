//! Entry store — per-universe content seam (CO-433).
//!
//! `EntryStore` is the backend-agnostic, **per-universe** view of entry CRUD:
//! each instance is bound to a single universe, so callers never thread a
//! `universe_key` through every call. It is the seam the `UniversePool`
//! factory hands out (`UniversePool::entry_store`) so a universe's content
//! reads/writes go through that universe's own connection — never the global
//! `Mutex<Storage>`.
//!
//! SQLite is the default implementation ([`SqliteEntryStore`]); it reuses the
//! existing [`SqliteEntryRepository`] (CO-390) under the hood, so there is no
//! duplicated SQL. A future S3/Postgres backend implements this trait without
//! touching any handler — that is the seam this module leaves open. No backend
//! other than SQLite is implemented here (per CO-433 scope).

use std::sync::{Arc, Mutex};

use anyhow::Result;
use rusqlite::Connection;

use crate::domain::EntryDomain;
use crate::repository::{EntryRepository, SqliteEntryRepository};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Per-universe entry CRUD. One instance is scoped to a single universe.
///
/// Held as `Arc<dyn EntryStore>`; produced by the `UniversePool` factory so
/// each universe's writes are serialized only by that universe's own lock.
pub trait EntryStore: Send + Sync {
    /// Fetch an entry by path. `Ok(None)` if absent.
    fn get(&self, path: &str) -> Result<Option<EntryDomain>>;

    /// Insert or overwrite an entry.
    fn upsert(&self, entry: &EntryDomain) -> Result<()>;

    /// Delete an entry by path (no-op if absent).
    fn delete(&self, path: &str) -> Result<()>;

    /// List entries, optionally filtered by type and frontmatter filter.
    fn list(
        &self,
        entry_type: &str,
        filter: &serde_json::Value,
        limit: Option<usize>,
    ) -> Result<Vec<EntryDomain>>;
}

// ---------------------------------------------------------------------------
// SQLite implementation (default backend)
// ---------------------------------------------------------------------------

/// SQLite-backed [`EntryStore`] over one universe's `data.db` connection.
///
/// Wraps [`SqliteEntryRepository`] and binds it to a fixed `universe_key`, so
/// the universe scoping lives here instead of at every call site.
pub struct SqliteEntryStore {
    universe_key: String,
    repo: SqliteEntryRepository,
}

impl SqliteEntryStore {
    /// Build a store bound to `universe_key`, operating on `conn` (the
    /// per-universe connection handed out by the pool).
    pub fn new(universe_key: impl Into<String>, conn: Arc<Mutex<Connection>>) -> Self {
        Self {
            universe_key: universe_key.into(),
            repo: SqliteEntryRepository::new(conn),
        }
    }

    /// The universe this store is bound to.
    pub fn universe_key(&self) -> &str {
        &self.universe_key
    }
}

impl EntryStore for SqliteEntryStore {
    fn get(&self, path: &str) -> Result<Option<EntryDomain>> {
        self.repo.find(&self.universe_key, path)
    }

    fn upsert(&self, entry: &EntryDomain) -> Result<()> {
        self.repo.upsert(&self.universe_key, entry)
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.repo.delete(&self.universe_key, path)
    }

    fn list(
        &self,
        entry_type: &str,
        filter: &serde_json::Value,
        limit: Option<usize>,
    ) -> Result<Vec<EntryDomain>> {
        self.repo.list(&self.universe_key, entry_type, filter, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universe_pool::UniversePool;
    use tempfile::tempdir;

    fn sample_entry(universe_key: &str, path: &str) -> EntryDomain {
        EntryDomain {
            path: path.to_string(),
            universe_key: universe_key.to_string(),
            entry_type: "note".to_string(),
            title: Some(path.to_string()),
            frontmatter: serde_json::json!({"title": path}),
            body: "body".to_string(),
            body_hash: String::new(),
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn sqlite_entry_store_roundtrip_is_universe_scoped() {
        let dir = tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);

        let store: Arc<dyn EntryStore> = pool.entry_store("uni-a").unwrap();
        assert!(store.get("note.md").unwrap().is_none());

        store.upsert(&sample_entry("uni-a", "note.md")).unwrap();
        assert!(store.get("note.md").unwrap().is_some());

        // A different universe's store does not see uni-a's entry.
        let other: Arc<dyn EntryStore> = pool.entry_store("uni-b").unwrap();
        assert!(other.get("note.md").unwrap().is_none());

        store.delete("note.md").unwrap();
        assert!(store.get("note.md").unwrap().is_none());
    }
}
