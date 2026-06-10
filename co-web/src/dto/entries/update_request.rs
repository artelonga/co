//! PUT /api/v1/universes/:slug/entries/*path — request body DTO.

use serde::Deserialize;
use serde_json::Value as JsonValue;

/// Input shape for patching an existing entry.
///
/// Both fields are optional; omitting `frontmatter` preserves the existing
/// frontmatter and omitting `body` preserves the existing body.
#[derive(Debug, Deserialize)]
pub struct UpdateEntryRequest {
    /// Partial frontmatter patch (merged over existing).
    pub frontmatter: Option<JsonValue>,
    /// Replacement body (None = keep existing body).
    pub body: Option<String>,
}
