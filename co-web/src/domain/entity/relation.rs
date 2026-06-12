//! CO domain entity — Relation edge.
//!
//! Pure business type: no rusqlite, no axum, no HTTP deps.
//! Represents one directed edge of the typed relation graph (CO-74):
//! `from_path --[relation_type]--> to_path`, optionally crossing into
//! another universe (CO-153).
//!
//! CO-432: propagates the CO-390 layering template from `entries` to the
//! `relations` entity.

/// A directed relation edge between two entries.
///
/// Invariant: this struct has zero axum or rusqlite dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationDomain {
    /// Universe whose database stores this edge (the from-side universe).
    pub universe_key: String,
    /// Path of the entry the edge originates from.
    pub from_path: String,
    /// Path of the entry the edge points to.
    pub to_path: String,
    /// Manifest field name that declared the relationship (e.g. `assignee`),
    /// or `wikilink` for body links.
    pub relation_type: String,
    /// ISO-8601 timestamp of when the edge was indexed.
    pub created_at: String,
    /// Target universe key — `None` means same universe as `universe_key`.
    pub to_universe: Option<String>,
    /// Optional alias label from `[[target|label]]` wikilinks (CO-363).
    pub link_text: Option<String>,
}

impl RelationDomain {
    /// Does this edge point into another universe?
    pub fn is_cross_universe(&self) -> bool {
        self.to_universe.is_some()
    }
}
