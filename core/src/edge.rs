//! Edge types for the CO graph
//!
//! Edges represent relationships between nodes.

use serde::{Deserialize, Serialize};

/// The type of relationship between nodes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeType {
    /// Node inherits from another (type hierarchy)
    /// e.g., English inherits from Language
    Inherits,

    /// Node translates to another (same concept, different language)
    /// e.g., "hello" translates_to "olá"
    TranslatesTo,

    /// Node references another (citation, link)
    References,

    /// Node belongs to another (membership)
    /// e.g., Definition belongs_to Domain
    BelongsTo,

    /// Node defines another (definitional relationship)
    Defines,

    /// Node is an instance of another (instantiation)
    InstanceOf,

    /// Custom relationship with label
    Custom(String),
}

/// An edge in the CO graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Type of this edge
    pub edge_type: EdgeType,

    /// Optional weight/strength
    pub weight: Option<f64>,

    /// Additional metadata
    pub metadata: std::collections::HashMap<String, String>,
}

impl Edge {
    /// Create a new edge
    pub fn new(edge_type: EdgeType) -> Self {
        Edge {
            edge_type,
            weight: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Create an inheritance edge
    pub fn inherits() -> Self {
        Self::new(EdgeType::Inherits)
    }

    /// Create a translation edge
    pub fn translates_to() -> Self {
        Self::new(EdgeType::TranslatesTo)
    }

    /// Create a reference edge
    pub fn references() -> Self {
        Self::new(EdgeType::References)
    }

    /// Check if this is an inheritance edge
    pub fn is_inheritance(&self) -> bool {
        matches!(self.edge_type, EdgeType::Inherits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_edge() {
        let edge = Edge::inherits();
        assert!(edge.is_inheritance());
    }
}
