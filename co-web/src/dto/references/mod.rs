//! Reference DTO family — transport types for the references API (CO-156).
//!
//! CO-432: propagates the CO-390 DTO pattern from `entries` to `references`.
//! Type names are kept identical to the pre-432 route-local definitions so
//! the wire format and call sites are unchanged.

pub mod broken_card;
pub mod card;
pub mod create_request;
pub mod list_query;
pub mod orphan_blob;
pub mod update_request;

pub use broken_card::BrokenCard;
pub use card::ReferenceCard;
pub use create_request::CreateRefBody;
pub use list_query::ListRefsQuery;
pub use orphan_blob::OrphanBlob;
pub use update_request::UpdateRefBody;
