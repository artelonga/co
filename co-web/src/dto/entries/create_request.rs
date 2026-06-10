//! POST /api/v1/universes/:slug/entries — request body DTO.

use serde::Deserialize;
use serde_json::Value as JsonValue;

/// Input shape for creating a new entry.
///
/// Wire-compatible with the existing `CreateEntryBody` in entry_routes.rs.
/// This DTO owns the request boundary; `EntryDomain` owns business logic.
#[derive(Debug, Deserialize)]
pub struct CreateEntryRequest {
    /// Vault-relative path for the new entry (e.g. `projects/MP/1.md`).
    pub path: String,
    /// Frontmatter key-value pairs; validated against the universe manifest.
    pub frontmatter: JsonValue,
    /// Markdown body (optional; defaults to empty string).
    #[serde(default)]
    pub body: String,
}
