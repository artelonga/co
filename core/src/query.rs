//! Query DSL for graph traversal

use serde::{Deserialize, Serialize};

/// A query against the graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    /// Filter expressions
    pub filters: Vec<Filter>,

    /// Sort field
    pub sort: Option<Sort>,

    /// Maximum results
    pub limit: Option<usize>,

    /// Skip first N results
    pub offset: Option<usize>,
}

/// A filter expression
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Filter {
    /// Field equals value
    Eq(String, String),

    /// Field not equals value
    Ne(String, String),

    /// Field less than value
    Lt(String, String),

    /// Field greater than value
    Gt(String, String),

    /// Field contains value (for arrays)
    Contains(String, String),

    /// Logical AND of filters
    And(Vec<Filter>),

    /// Logical OR of filters
    Or(Vec<Filter>),

    /// Logical NOT of filter
    Not(Box<Filter>),
}

/// Sort specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sort {
    /// Field to sort by
    pub field: String,

    /// Sort direction
    pub descending: bool,
}

impl Query {
    /// Create a new empty query
    pub fn new() -> Self {
        Query {
            filters: Vec::new(),
            sort: None,
            limit: None,
            offset: None,
        }
    }

    /// Add a filter
    pub fn filter(mut self, f: Filter) -> Self {
        self.filters.push(f);
        self
    }

    /// Set sort
    pub fn sort_by(mut self, field: &str, descending: bool) -> Self {
        self.sort = Some(Sort {
            field: field.to_string(),
            descending,
        });
        self
    }

    /// Set limit
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Set offset
    pub fn offset(mut self, n: usize) -> Self {
        self.offset = Some(n);
        self
    }

    /// Parse a query string like "status:active AND priority:high"
    pub fn parse(_query_str: &str) -> Result<Self, String> {
        // TODO: Implement query parser
        Ok(Query::new())
    }
}

impl Default for Query {
    fn default() -> Self {
        Self::new()
    }
}
