//! CO domain layer — pure business types, no framework dependencies.
//!
//! CO-390 spike: proof-of-concept for layered architecture.
//! Domain types represent business entities; they are NOT tied to HTTP or SQLite.

pub mod entity;

pub use entity::{EntryDomain, ReferenceDomain, RelationDomain};
