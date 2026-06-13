//! DTO layer — transport types per API endpoint family.
//!
//! CO-390 spike: proof-of-concept layered architecture.
//! DTOs live here; domain entities live in `crate::domain`.
//! Mappers between the two live in `crate::mapper`.

pub mod entries;
pub mod references;
pub mod relations;
