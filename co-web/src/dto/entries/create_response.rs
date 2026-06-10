//! POST /api/v1/universes/:slug/entries — 201 Created response body DTO.
//!
//! Wire-identical to the current `EntryRow` serialization.  The field names
//! and serde attributes are kept in sync so that before/after JSON diffs are
//! empty (hypothesis 4 in CO-390).

use serde::Serialize;
use serde_json::Value as JsonValue;

/// Response body returned after a successful `POST /entries`.
///
/// Identical JSON shape to `EntryRow` — kept as a distinct type so the
/// compiler enforces that the create response is an intentional contract,
/// not an accidental re-use of the DB row type.
#[derive(Debug, Serialize)]
pub struct CreateEntryResponse {
    pub path: String,
    pub universe_key: String,
    pub entry_type: String,
    pub title: Option<String>,
    pub frontmatter: JsonValue,
    pub body: String,
    pub body_hash: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Semantic search score — only present in search results; omitted here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _score: Option<f32>,
}
