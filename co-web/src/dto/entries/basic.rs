//! Minimal entry representation for list views.
//!
//! Used in `GET /entries` list responses where clients need only a subset
//! of fields to render a board card or table row.

use serde::Serialize;

/// Minimal entry DTO for list views.
///
/// Carries only the fields needed for a board-card or table row, avoiding
/// the body (potentially large) and full frontmatter (noisy for lists).
/// The full `EntryInfoDto` is returned by `GET /entries/:path`.
#[derive(Debug, Serialize, Clone)]
pub struct EntryBasicDto {
    pub path: String,
    pub universe_key: String,
    pub entry_type: String,
    pub title: Option<String>,
    pub updated_at: Option<String>,
    /// Semantic search score (only in semantic results).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _score: Option<f32>,
}
