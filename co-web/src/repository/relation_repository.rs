//! Relation repository — data-access trait + SQLite implementation.
//!
//! CO-432: propagates the CO-390 repository pattern from `entries` to
//! `relations`. The trait hides rusqlite; callers depend on the trait (or the
//! concrete repository), not on `RelationIndex`.
//!
//! Like `SqliteEntryRepository`, the SQLite implementation wraps the
//! per-universe connection (`Arc<std::sync::Mutex<Connection>>` — the type
//! returned by `Storage::universe_conn` / `UniversePool::get_or_open`) and
//! constructs the short-lived `RelationIndex<'_>` internally, so handlers
//! never hold a raw connection guard for relation access.

use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Result, anyhow};
use rusqlite::Connection;

use crate::domain::RelationDomain;
use crate::mapper::relation_mapper::RelationMapper;
use crate::relation_index::RelationIndex;

// ---------------------------------------------------------------------------
// Repository trait
// ---------------------------------------------------------------------------

/// Data-access abstraction for relation edges.
///
/// Implementations may be SQLite (production) or in-memory (tests).
pub trait RelationRepository: Send + Sync {
    /// All relations originating from `from_path` (outbound edges).
    fn outbound(&self, universe_key: &str, from_path: &str) -> Result<Vec<RelationDomain>>;

    /// All same-universe relations pointing to `to_path` (inbound edges).
    fn inbound(&self, universe_key: &str, to_path: &str) -> Result<Vec<RelationDomain>>;

    /// CO-153: edges in THIS universe's DB that point into `target_universe`
    /// at `target_path`. Called once per universe during a cross-universe
    /// inbound scan.
    fn inbound_from_other(
        &self,
        target_universe: &str,
        target_path: &str,
    ) -> Result<Vec<RelationDomain>>;

    /// Replace all outbound relations for `from_path` atomically. Each element
    /// is `(relation_type, to_path, to_universe, link_text)`.
    fn replace_all(
        &self,
        universe_key: &str,
        from_path: &str,
        relations: &[(String, String, Option<String>, Option<String>)],
    ) -> Result<()>;

    /// Remove all outbound relations for `from_path`.
    fn delete_for_entry(&self, universe_key: &str, from_path: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// SQLite implementation
// ---------------------------------------------------------------------------

/// Production repository — wraps `RelationIndex` behind `RelationRepository`.
pub struct SqliteRelationRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteRelationRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow!("universe conn lock poisoned"))
    }

    /// CO-74/CO-363: re-extract and store an entry's typed FK + body wikilink
    /// relations. Wraps the index-level `sync_entry_relations` free function.
    pub fn sync_entry_relations(
        &self,
        universe_key: &str,
        path: &str,
        entry_type: &str,
        frontmatter: &serde_json::Value,
        body: &str,
        manifest: Option<&co::manifest::Manifest>,
    ) -> Result<usize> {
        let guard = self.lock()?;
        crate::relation_index::sync_entry_relations(
            &guard,
            universe_key,
            path,
            entry_type,
            frontmatter,
            body,
            manifest,
        )
    }
}

impl RelationRepository for SqliteRelationRepository {
    fn outbound(&self, universe_key: &str, from_path: &str) -> Result<Vec<RelationDomain>> {
        let guard = self.lock()?;
        let rows = RelationIndex::new(&guard).outbound(universe_key, from_path)?;
        Ok(rows
            .into_iter()
            .map(RelationMapper::row_to_domain)
            .collect())
    }

    fn inbound(&self, universe_key: &str, to_path: &str) -> Result<Vec<RelationDomain>> {
        let guard = self.lock()?;
        let rows = RelationIndex::new(&guard).inbound(universe_key, to_path)?;
        Ok(rows
            .into_iter()
            .map(RelationMapper::row_to_domain)
            .collect())
    }

    fn inbound_from_other(
        &self,
        target_universe: &str,
        target_path: &str,
    ) -> Result<Vec<RelationDomain>> {
        let guard = self.lock()?;
        let rows = RelationIndex::new(&guard).inbound_from_other(target_universe, target_path)?;
        Ok(rows
            .into_iter()
            .map(RelationMapper::row_to_domain)
            .collect())
    }

    fn replace_all(
        &self,
        universe_key: &str,
        from_path: &str,
        relations: &[(String, String, Option<String>, Option<String>)],
    ) -> Result<()> {
        let guard = self.lock()?;
        RelationIndex::new(&guard).replace_all(universe_key, from_path, relations)
    }

    fn delete_for_entry(&self, universe_key: &str, from_path: &str) -> Result<()> {
        let guard = self.lock()?;
        RelationIndex::new(&guard).delete_for_entry(universe_key, from_path)
    }
}

// ---------------------------------------------------------------------------
// In-memory implementation (for unit tests)
// ---------------------------------------------------------------------------

/// Deterministic in-memory repository for unit testing service-layer logic
/// without a SQLite database (the testability payoff of the pattern).
#[cfg(test)]
pub struct InMemoryRelationRepository {
    edges: Mutex<Vec<RelationDomain>>,
}

#[cfg(test)]
impl InMemoryRelationRepository {
    pub fn new(edges: Vec<RelationDomain>) -> Self {
        Self {
            edges: Mutex::new(edges),
        }
    }
}

#[cfg(test)]
impl RelationRepository for InMemoryRelationRepository {
    fn outbound(&self, universe_key: &str, from_path: &str) -> Result<Vec<RelationDomain>> {
        Ok(self
            .edges
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.universe_key == universe_key && e.from_path == from_path)
            .cloned()
            .collect())
    }

    fn inbound(&self, universe_key: &str, to_path: &str) -> Result<Vec<RelationDomain>> {
        Ok(self
            .edges
            .lock()
            .unwrap()
            .iter()
            .filter(|e| {
                e.universe_key == universe_key && e.to_path == to_path && e.to_universe.is_none()
            })
            .cloned()
            .collect())
    }

    fn inbound_from_other(
        &self,
        target_universe: &str,
        target_path: &str,
    ) -> Result<Vec<RelationDomain>> {
        Ok(self
            .edges
            .lock()
            .unwrap()
            .iter()
            .filter(|e| {
                e.to_universe.as_deref() == Some(target_universe) && e.to_path == target_path
            })
            .cloned()
            .collect())
    }

    fn replace_all(
        &self,
        universe_key: &str,
        from_path: &str,
        relations: &[(String, String, Option<String>, Option<String>)],
    ) -> Result<()> {
        let mut edges = self.edges.lock().unwrap();
        edges.retain(|e| !(e.universe_key == universe_key && e.from_path == from_path));
        for (relation_type, to_path, to_universe, link_text) in relations {
            edges.push(RelationDomain {
                universe_key: universe_key.to_string(),
                from_path: from_path.to_string(),
                to_path: to_path.clone(),
                relation_type: relation_type.clone(),
                created_at: String::new(),
                to_universe: to_universe.clone(),
                link_text: link_text.clone(),
            });
        }
        Ok(())
    }

    fn delete_for_entry(&self, universe_key: &str, from_path: &str) -> Result<()> {
        self.edges
            .lock()
            .unwrap()
            .retain(|e| !(e.universe_key == universe_key && e.from_path == from_path));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> InMemoryRelationRepository {
        let r = InMemoryRelationRepository::new(vec![]);
        r.replace_all(
            "alpha",
            "a.md",
            &[
                ("assignee".into(), "b.md".into(), None, None),
                ("wikilink".into(), "c.md".into(), Some("beta".into()), None),
            ],
        )
        .unwrap();
        r
    }

    #[test]
    fn outbound_returns_all_edges_from_path() {
        let r = repo();
        assert_eq!(r.outbound("alpha", "a.md").unwrap().len(), 2);
        assert!(r.outbound("alpha", "b.md").unwrap().is_empty());
    }

    #[test]
    fn inbound_excludes_cross_universe_edges() {
        let r = repo();
        assert_eq!(r.inbound("alpha", "b.md").unwrap().len(), 1);
        // c.md edge points into beta, so it is not same-universe inbound.
        assert!(r.inbound("alpha", "c.md").unwrap().is_empty());
        assert_eq!(r.inbound_from_other("beta", "c.md").unwrap().len(), 1);
    }

    #[test]
    fn replace_all_is_idempotent_and_delete_clears() {
        let r = repo();
        r.replace_all(
            "alpha",
            "a.md",
            &[("assignee".into(), "b.md".into(), None, None)],
        )
        .unwrap();
        assert_eq!(r.outbound("alpha", "a.md").unwrap().len(), 1);
        r.delete_for_entry("alpha", "a.md").unwrap();
        assert!(r.outbound("alpha", "a.md").unwrap().is_empty());
    }
}
