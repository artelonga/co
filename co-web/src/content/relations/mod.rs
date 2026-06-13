//! Relations — typed FK relation graph (CO-74) + cross-universe queries (CO-153).
//!
//! `index` is the SQLite projection; handlers reach it through
//! `crate::repository::RelationRepository`, not directly.

pub mod extract;
pub mod index;
pub mod routes;
