//! Request DTO — body for `POST /references`.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CreateRefBody {
    pub path: String,
    pub frontmatter: serde_json::Value, // FREEFORM: reference card schema (title, work_id, editions, medium) is open and extensible
    #[serde(default)]
    pub body: String,
}
