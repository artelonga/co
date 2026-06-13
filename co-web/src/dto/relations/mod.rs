//! Relation DTO family — transport types for the relations API (CO-153).
//!
//! CO-432: propagates the CO-390 DTO pattern from `entries` to `relations`.
//! Type names are kept identical to the pre-432 route-local definitions so
//! the wire format and call sites are unchanged.

pub mod inbound;
pub mod outbound;
pub mod query;

pub use inbound::InboundRelation;
pub use outbound::OutboundRelation;
pub use query::RelationQuery;
