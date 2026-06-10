//! Entry repository — data-access trait + SQLite implementation.
//!
//! CO-390 spike: proves Pattern 4 from `docs/spikes/library-manager-case-study.md`.
//! The trait hides rusqlite; callers depend on the trait, not the impl.
//!
//! ## Rust ergonomic note (for the decision document)
//!
//! Unlike `JpaRepository<Book, Long>` in Spring, Rust requires explicit trait
//! definitions and manual SQLite connection management.  `EntryIndex<'a>` uses
//! `&'a Connection` with lifetime constraints that don't compose cleanly with
//! `Box<dyn EntryRepository>`.  The spike works around this by embedding an
//! `Arc<parking_lot::Mutex<Connection>>` in `SqliteEntryRepository` — the same
//! pattern used for universe connections in `co-web/src/platform/universe_pool.rs`.
//! This adds one allocation per repository but avoids lifetime leakage.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::Connection;

use crate::domain::EntryDomain;
use crate::mapper::entry_mapper::EntryMapper;

// ---------------------------------------------------------------------------
// Repository trait
// ---------------------------------------------------------------------------

/// Data-access abstraction for entries.
///
/// Implementations may be SQLite (production) or in-memory (tests).
/// The controller and service layers depend on this trait, not `EntryIndex`.
pub trait EntryRepository: Send + Sync {
    /// Find an entry by (universe_key, path). Returns `None` if not found.
    fn find(&self, universe_key: &str, path: &str) -> Result<Option<EntryDomain>>;

    /// List entries, optionally filtered by type and frontmatter filter.
    fn list(
        &self,
        universe_key: &str,
        entry_type: &str,
        filter: &serde_json::Value,
        limit: Option<usize>,
    ) -> Result<Vec<EntryDomain>>;

    /// Upsert an entry (insert or overwrite).
    fn upsert(&self, universe_key: &str, entry: &EntryDomain) -> Result<()>;

    /// Delete an entry by path.
    fn delete(&self, universe_key: &str, path: &str) -> Result<()>;

    /// Count entries in a universe, optionally filtered by type.
    fn count(&self, universe_key: &str, entry_type: Option<&str>) -> i64;
}

// ---------------------------------------------------------------------------
// SQLite implementation
// ---------------------------------------------------------------------------

/// Production repository — wraps `EntryIndex` behind the `EntryRepository` trait.
pub struct SqliteEntryRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteEntryRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

impl EntryRepository for SqliteEntryRepository {
    fn find(&self, universe_key: &str, path: &str) -> Result<Option<EntryDomain>> {
        let conn = self.conn.lock();
        let index = crate::entry_index::EntryIndex::new(&conn);
        let row = index.get(universe_key, path)?;
        Ok(row.map(|r| EntryMapper::row_to_domain(r, universe_key)))
    }

    fn list(
        &self,
        universe_key: &str,
        entry_type: &str,
        filter: &serde_json::Value,
        limit: Option<usize>,
    ) -> Result<Vec<EntryDomain>> {
        let conn = self.conn.lock();
        let index = crate::entry_index::EntryIndex::new(&conn);
        let rows = index.query_with_limit(universe_key, entry_type, filter, limit)?;
        Ok(rows
            .into_iter()
            .map(|r| EntryMapper::row_to_domain(r, universe_key))
            .collect())
    }

    fn upsert(&self, universe_key: &str, entry: &EntryDomain) -> Result<()> {
        let conn = self.conn.lock();
        let index = crate::entry_index::EntryIndex::new(&conn);
        let core_entry = EntryMapper::domain_to_core_entry(entry);
        index.upsert(universe_key, &core_entry)?;
        Ok(())
    }

    fn delete(&self, universe_key: &str, path: &str) -> Result<()> {
        let conn = self.conn.lock();
        let index = crate::entry_index::EntryIndex::new(&conn);
        index.remove(universe_key, path)?;
        Ok(())
    }

    fn count(&self, universe_key: &str, entry_type: Option<&str>) -> i64 {
        let conn = self.conn.lock();
        let index = crate::entry_index::EntryIndex::new(&conn);
        index.count(universe_key, entry_type)
    }
}

// ---------------------------------------------------------------------------
// In-memory implementation (for unit tests)
// ---------------------------------------------------------------------------

/// Spike-only: deterministic in-memory repository for unit testing the service layer.
///
/// This is the testability payoff from Pattern 4: service tests can use
/// `InMemoryEntryRepository` instead of a real SQLite database.
#[cfg(test)]
pub struct InMemoryEntryRepository {
    entries: Mutex<Vec<EntryDomain>>,
}

#[cfg(test)]
impl InMemoryEntryRepository {
    pub fn new(entries: Vec<EntryDomain>) -> Self {
        Self {
            entries: Mutex::new(entries),
        }
    }
}

#[cfg(test)]
impl EntryRepository for InMemoryEntryRepository {
    fn find(&self, universe_key: &str, path: &str) -> Result<Option<EntryDomain>> {
        Ok(self
            .entries
            .lock()
            .iter()
            .find(|e| e.universe_key == universe_key && e.path == path)
            .cloned())
    }

    fn list(
        &self,
        universe_key: &str,
        entry_type: &str,
        _filter: &serde_json::Value,
        limit: Option<usize>,
    ) -> Result<Vec<EntryDomain>> {
        let entries = self.entries.lock();
        let results: Vec<EntryDomain> = entries
            .iter()
            .filter(|e| {
                e.universe_key == universe_key
                    && (entry_type.is_empty() || e.entry_type == entry_type)
            })
            .cloned()
            .collect();
        Ok(match limit {
            Some(n) => results.into_iter().take(n).collect(),
            None => results,
        })
    }

    fn upsert(&self, _universe_key: &str, entry: &EntryDomain) -> Result<()> {
        let mut entries = self.entries.lock();
        entries.retain(|e| !(e.universe_key == entry.universe_key && e.path == entry.path));
        entries.push(entry.clone());
        Ok(())
    }

    fn delete(&self, universe_key: &str, path: &str) -> Result<()> {
        let mut entries = self.entries.lock();
        entries.retain(|e| !(e.universe_key == universe_key && e.path == path));
        Ok(())
    }

    fn count(&self, universe_key: &str, entry_type: Option<&str>) -> i64 {
        self.entries
            .lock()
            .iter()
            .filter(|e| {
                e.universe_key == universe_key && entry_type.map_or(true, |t| e.entry_type == t)
            })
            .count() as i64
    }
}
