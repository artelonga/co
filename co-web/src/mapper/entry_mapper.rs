//! Entry mapper — Domain ↔ DTO conversions.
//!
//! CO-390 spike: proves Pattern 3 + mapper from `docs/spikes/library-manager-case-study.md`.
//!
//! Mappers are pure, deterministic functions — no I/O, no side effects.
//! They translate between the domain entity (`EntryDomain`) and the transport
//! types (DTOs) or the storage type (`co::entry::Entry`).
//!
//! ## Rust vs. Spring
//!
//! Java's MapStruct generates mapper boilerplate at compile time. Rust has no
//! equivalent (without a macro); every mapper is hand-written. For the decision
//! doc: ~80 LOC of explicit mapping code for the entries family.

use chrono::Utc;

use crate::domain::EntryDomain;
use crate::dto::entries::{CreateEntryResponse, EntryBasicDto, EntryInfoDto};
use crate::entry_index::EntryRow;
use crate::relation_index::RelationRow;

/// Stateless mapper — all methods are associated functions.
pub struct EntryMapper;

impl EntryMapper {
    // -----------------------------------------------------------------------
    // EntryRow (DB row) → domain
    // -----------------------------------------------------------------------

    /// Convert a database row into the domain entity.
    ///
    /// `universe_key` is passed explicitly because `EntryRow.universe_key` may
    /// differ from the slug used for routing (for sub-universe queries).
    pub fn row_to_domain(row: EntryRow, universe_key: &str) -> EntryDomain {
        EntryDomain {
            path: row.path,
            universe_key: universe_key.to_string(),
            entry_type: row.entry_type,
            title: row.title,
            frontmatter: row.frontmatter,
            body: row.body,
            body_hash: row.body_hash,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    // -----------------------------------------------------------------------
    // Domain → DTOs
    // -----------------------------------------------------------------------

    /// Convert domain entity into the create-response DTO.
    ///
    /// Wire-identical to `EntryRow` serialization — verified by hypothesis 4.
    pub fn domain_to_create_response(domain: &EntryDomain) -> CreateEntryResponse {
        CreateEntryResponse {
            path: domain.path.clone(),
            universe_key: domain.universe_key.clone(),
            entry_type: domain.entry_type.clone(),
            title: domain.title.clone(),
            frontmatter: domain.frontmatter.clone(),
            body: domain.body.clone(),
            body_hash: domain.body_hash.clone(),
            created_at: domain.created_at.clone(),
            updated_at: domain.updated_at.clone(),
            _score: None,
        }
    }

    /// Convert a DB row into the minimal list-view DTO.
    pub fn row_to_basic(row: EntryRow) -> EntryBasicDto {
        EntryBasicDto {
            path: row.path,
            universe_key: row.universe_key,
            entry_type: row.entry_type,
            title: row.title,
            updated_at: row.updated_at,
            _score: row._score,
        }
    }

    /// Convert a DB row + outbound relations into the full detail DTO.
    pub fn row_to_info(row: EntryRow, relations: Vec<RelationRow>) -> EntryInfoDto {
        EntryInfoDto {
            path: row.path,
            universe_key: row.universe_key,
            entry_type: row.entry_type,
            title: row.title,
            frontmatter: row.frontmatter,
            body: row.body,
            body_hash: row.body_hash,
            created_at: row.created_at,
            updated_at: row.updated_at,
            _score: row._score,
            relations,
        }
    }

    // -----------------------------------------------------------------------
    // Domain → core entry (for file I/O and DB upsert)
    // -----------------------------------------------------------------------

    /// Convert domain entity back to `co::entry::Entry` for file-system writes
    /// and `EntryIndex::upsert` calls.
    ///
    /// ## Note
    ///
    /// `co::entry::Entry::stat` (file-system timestamps + size) is approximated
    /// from the domain's `created_at` / `updated_at` strings.  The real `stat`
    /// comes from `co::write_entry` which sets it from `fs::metadata`; this
    /// reconstruction is only used when the domain entity is built from a DB row
    /// (i.e. the file already exists and its stat is irrelevant for the upsert).
    pub fn domain_to_core_entry(domain: &EntryDomain) -> co::entry::Entry {
        let now = Utc::now();
        let created = domain
            .created_at
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(now);
        let modified = domain
            .updated_at
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(now);

        co::entry::Entry {
            path: domain.path.clone(),
            entry_type: domain.entry_type.clone(),
            frontmatter: domain.frontmatter.clone(),
            body: domain.body.clone(),
            body_hash: co::entry::Entry::hash_body(&domain.body),
            stat: co::entry::FileStat {
                created,
                modified,
                size: domain.body.len() as u64,
            },
        }
    }

    // -----------------------------------------------------------------------
    // EntryRow → EntryRow (identity — for wire-compat diff baseline)
    // -----------------------------------------------------------------------

    /// Build a `CreateEntryResponse` directly from an `EntryRow`, bypassing
    /// the domain layer.  Used in the wire-compat test to prove that
    /// `domain_to_create_response(row_to_domain(row))` == `row_to_response(row)`.
    #[cfg(test)]
    pub fn row_to_create_response(row: EntryRow) -> CreateEntryResponse {
        CreateEntryResponse {
            path: row.path,
            universe_key: row.universe_key,
            entry_type: row.entry_type,
            title: row.title,
            frontmatter: row.frontmatter,
            body: row.body,
            body_hash: row.body_hash,
            created_at: row.created_at,
            updated_at: row.updated_at,
            _score: row._score,
        }
    }
}

// ---------------------------------------------------------------------------
// Wire-compatibility tests — hypothesis 4
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_row() -> EntryRow {
        EntryRow {
            path: "projects/CO/1.md".to_string(),
            universe_key: "test-universe".to_string(),
            entry_type: "task".to_string(),
            title: Some("Sample task".to_string()),
            frontmatter: json!({
                "type": "task",
                "title": "Sample task",
                "id": 1,
            }),
            body: "This is the body.".to_string(),
            body_hash: "abc123".to_string(),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: Some("2026-01-02T00:00:00Z".to_string()),
            _score: None,
        }
    }

    #[test]
    fn round_trip_row_to_domain_to_response_is_wire_identical() {
        let row = sample_row();
        let direct = EntryMapper::row_to_create_response(row.clone());
        let via_domain = {
            let domain = EntryMapper::row_to_domain(row, "test-universe");
            EntryMapper::domain_to_create_response(&domain)
        };
        // Hypothesis 4: wire-identical serialization
        let direct_json = serde_json::to_string(&direct).unwrap();
        let via_domain_json = serde_json::to_string(&via_domain).unwrap();
        assert_eq!(
            direct_json, via_domain_json,
            "CreateEntryResponse must serialize identically whether built via domain or directly"
        );
    }

    #[test]
    fn row_to_basic_carries_key_fields() {
        let row = sample_row();
        let basic = EntryMapper::row_to_basic(row);
        assert_eq!(basic.path, "projects/CO/1.md");
        assert_eq!(basic.entry_type, "task");
        assert_eq!(basic.title, Some("Sample task".to_string()));
        // body is NOT carried — that's the point of basic vs info
    }

    #[test]
    fn row_to_info_carries_relations() {
        let row = sample_row();
        let relations = vec![RelationRow {
            from_path: "projects/CO/1.md".to_string(),
            to_path: "projects/CO/2.md".to_string(),
            relation_type: "blocks".to_string(),
            universe_key: "test-universe".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            to_universe: None,
            link_text: None,
        }];
        let info = EntryMapper::row_to_info(row, relations);
        assert_eq!(info.relations.len(), 1);
        assert_eq!(info.relations[0].relation_type, "blocks");
    }
}
