//! GET /api/v1/universes/:slug/entries — query filter DTO.

use serde::Deserialize;

/// Typed query parameters for `GET /entries`.
///
/// Wire-compatible with the existing `EntryListQuery`.
#[derive(Debug, Deserialize)]
pub struct EntryFilter {
    /// Filter by content type (e.g. `task`, `project`).
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    /// JSON-encoded frontmatter filter (e.g. `{"project":"MP"}`).
    pub filter: Option<String>,
    /// Full-text search query.
    pub q: Option<String>,
    /// CO-73: date semantic to filter by (e.g. `event_at`, `due_at`).
    pub date_semantic: Option<String>,
    /// CO-73: inclusive ISO-8601 start of date range.
    pub from: Option<String>,
    /// CO-73: inclusive ISO-8601 end of date range.
    pub to: Option<String>,
    /// Max entries to return. Defaults to 5000; hard-capped at 50000.
    pub limit: Option<usize>,
    /// 1.62.0: rewind view — `states/...md` path to filter by historical state.
    pub as_of: Option<String>,
    /// CO-164: semantic similarity query text.
    pub semantic: Option<String>,
    /// CO-164: number of top-K results for semantic/similar queries (default 10).
    pub k: Option<usize>,
    /// CO-264: filter entries by path prefix (e.g. `public/` → all `public/*`).
    pub path_prefix: Option<String>,
}
