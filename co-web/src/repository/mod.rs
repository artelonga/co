//! Repository layer — data-access traits and implementations.
//!
//! CO-390 template (propagated by CO-432): traits live here; SQLite
//! implementations wrap the per-entity indexes from `content::*::index`
//! behind `Arc<std::sync::Mutex<Connection>>` (the per-universe connection
//! type), so handlers never construct an index on a raw connection guard.

pub mod entry_repository;
pub mod reference_repository;
pub mod relation_repository;

pub use entry_repository::{EntryRepository, SqliteEntryRepository};
pub use reference_repository::{
    CardFilter, OrphanBlobRecord, ReferenceRepository, SqliteReferenceRepository,
};
pub use relation_repository::{RelationRepository, SqliteRelationRepository};
