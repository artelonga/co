//! Full entry detail with outbound relations — detail view DTO.
//!
//! Returned by `GET /entries/:path`. Includes the full frontmatter, body,
//! and all outbound FK relations declared in the universe manifest (CO-74).

use serde::Serialize;
use serde_json::Value as JsonValue;

/// Full entry DTO for detail views.
///
/// The wire shape matches `EntryWithRelations` from entry_routes.rs.
/// Includes the complete frontmatter + body + outbound relations.
#[derive(Debug, Serialize)]
pub struct EntryInfoDto {
    // Entry fields
    pub path: String,
    pub universe_key: String,
    pub entry_type: String,
    pub title: Option<String>,
    pub frontmatter: JsonValue,
    pub body: String,
    pub body_hash: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _score: Option<f32>,
    // CO-74: outbound FK relations (relation_type → to_path)
    pub relations: Vec<crate::relation_index::RelationRow>,
}
