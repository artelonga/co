//! Request DTO — body for `PUT /references/{*path}`.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct UpdateRefBody {
    pub frontmatter: Option<serde_json::Value>, // FREEFORM: partial patch on open reference card schema
    pub body: Option<String>,
}
