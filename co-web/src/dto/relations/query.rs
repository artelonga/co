//! Request DTO — query parameters for the relations endpoints.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RelationQuery {
    pub path: String,
}
