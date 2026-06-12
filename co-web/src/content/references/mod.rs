//! References — reference cards (CO-156) and the references index (CO-154).
//!
//! `index` is the SQLite projection; handlers reach it through
//! `crate::repository::ReferenceRepository`, not directly.

pub mod index;
pub mod meta;
pub mod routes;
