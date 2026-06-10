//! Repository layer — data-access traits and implementations.
//!
//! CO-390 spike: proof-of-concept layered architecture.
//! Traits live here; SQLite implementations wrap `EntryIndex` from `content::entry_index`.

pub mod entry_repository;

pub use entry_repository::{EntryRepository, SqliteEntryRepository};
