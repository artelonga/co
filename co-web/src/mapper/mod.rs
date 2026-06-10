//! Mapper layer — Domain ↔ DTO conversions.
//!
//! CO-390 spike: proof-of-concept layered architecture.
//! Mappers are pure functions, easily unit-tested.

pub mod entry_mapper;

pub use entry_mapper::EntryMapper;
