//! Entries — the core content entity (markdown + frontmatter).
//!
//! `index` is the SQLite projection; handlers reach it through
//! `crate::repository::EntryRepository`, not directly.

pub mod index;
pub mod query_dsl;
pub mod routes;
