//! Response DTO — an inbound relation edge.

use serde::Serialize;

/// A single inbound relation edge — who points to this entry and from where.
#[derive(Debug, Serialize)]
pub struct InboundRelation {
    pub from_universe: String,
    pub from_path: String,
    pub relation_type: String,
}
