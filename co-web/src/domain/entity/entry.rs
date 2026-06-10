//! CO domain entity — Entry
//!
//! Pure business type: no rusqlite, no axum, no HTTP deps.
//! Represents what an entry IS, independent of how it's stored or transported.
//!
//! This type is the proof-of-concept domain entity for the CO-390 layered
//! architecture spike. It mirrors the data in `EntryRow` (which currently
//! doubles as DB row + API response) but lives in a stable inner layer.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// CO domain entity — a content item in a universe.
///
/// Every entry in CO is a markdown file with YAML frontmatter. This domain
/// type represents the entry in business logic — independent of how it's
/// stored in SQLite or serialized over HTTP.
///
/// Invariant: this struct has zero axum or rusqlite dependencies.
/// It may derive serde for mapper convenience but is NOT the wire type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryDomain {
    /// Vault-relative path (e.g. `projects/MP/1.md`).
    pub path: String,
    /// Universe this entry belongs to.
    pub universe_key: String,
    /// Content type from frontmatter `type` field (task, project, event, …).
    pub entry_type: String,
    /// Extracted title from frontmatter `title` field.
    pub title: Option<String>,
    /// Full frontmatter as JSON (enables json_extract queries without reparsing).
    pub frontmatter: JsonValue,
    /// Markdown body (everything after the closing `---`).
    pub body: String,
    /// SHA-256 hex of body for change detection.
    pub body_hash: String,
    /// ISO-8601 creation timestamp (sourced from frontmatter `created` or file stat).
    pub created_at: Option<String>,
    /// ISO-8601 last-update timestamp (sourced from frontmatter `modified` or file stat).
    pub updated_at: Option<String>,
}

impl EntryDomain {
    /// Extract the `title` field from frontmatter, falling back to path stem.
    pub fn title_or_path(&self) -> &str {
        self.title
            .as_deref()
            .or_else(|| self.frontmatter.get("title").and_then(|v| v.as_str()))
            .unwrap_or(&self.path)
    }

    /// Is this entry the manifest file?
    pub fn is_manifest(&self) -> bool {
        self.path == co::manifest::MANIFEST_FILENAME
    }
}
