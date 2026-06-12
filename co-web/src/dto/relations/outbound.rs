//! Response DTO — an outbound relation edge.

use serde::Serialize;

/// A single outbound relation edge — where this entry points.
#[derive(Debug, Serialize)]
pub struct OutboundRelation {
    /// `None` means same universe.
    pub to_universe: Option<String>,
    pub to_path: String,
    pub relation_type: String,
}
