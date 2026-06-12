//! Relation mapper — Domain ↔ DTO conversions.
//!
//! CO-432: propagates the CO-390 mapper pattern from `entries` to `relations`.
//! Mappers are pure functions, easily unit-tested.

use crate::domain::RelationDomain;
use crate::dto::relations::{InboundRelation, OutboundRelation};
use crate::relation_index::RelationRow;

/// Stateless mapper — all methods are associated functions.
pub struct RelationMapper;

impl RelationMapper {
    /// Convert a database row into the domain entity.
    pub fn row_to_domain(row: RelationRow) -> RelationDomain {
        RelationDomain {
            universe_key: row.universe_key,
            from_path: row.from_path,
            to_path: row.to_path,
            relation_type: row.relation_type,
            created_at: row.created_at,
            to_universe: row.to_universe,
            link_text: row.link_text,
        }
    }

    /// Convert a domain edge back into the row shape — used where the wire
    /// format is the full `RelationRow` serialization (entry detail view).
    pub fn domain_to_row(domain: &RelationDomain) -> RelationRow {
        RelationRow {
            universe_key: domain.universe_key.clone(),
            from_path: domain.from_path.clone(),
            to_path: domain.to_path.clone(),
            relation_type: domain.relation_type.clone(),
            created_at: domain.created_at.clone(),
            to_universe: domain.to_universe.clone(),
            link_text: domain.link_text.clone(),
        }
    }

    /// Project a domain edge onto the inbound wire shape (who points here).
    pub fn domain_to_inbound(domain: &RelationDomain) -> InboundRelation {
        InboundRelation {
            from_universe: domain.universe_key.clone(),
            from_path: domain.from_path.clone(),
            relation_type: domain.relation_type.clone(),
        }
    }

    /// Project a domain edge onto the outbound wire shape (where this points).
    pub fn domain_to_outbound(domain: &RelationDomain) -> OutboundRelation {
        OutboundRelation {
            to_universe: domain.to_universe.clone(),
            to_path: domain.to_path.clone(),
            relation_type: domain.relation_type.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge() -> RelationDomain {
        RelationDomain {
            universe_key: "alpha".into(),
            from_path: "a.md".into(),
            to_path: "b.md".into(),
            relation_type: "assignee".into(),
            created_at: "2026-06-12T00:00:00Z".into(),
            to_universe: Some("beta".into()),
            link_text: None,
        }
    }

    #[test]
    fn row_to_domain_preserves_all_fields() {
        let row = RelationRow {
            universe_key: "alpha".into(),
            from_path: "a.md".into(),
            to_path: "b.md".into(),
            relation_type: "assignee".into(),
            created_at: "2026-06-12T00:00:00Z".into(),
            to_universe: Some("beta".into()),
            link_text: Some("label".into()),
        };
        let d = RelationMapper::row_to_domain(row);
        assert_eq!(d.universe_key, "alpha");
        assert_eq!(d.from_path, "a.md");
        assert_eq!(d.to_path, "b.md");
        assert_eq!(d.relation_type, "assignee");
        assert_eq!(d.to_universe.as_deref(), Some("beta"));
        assert_eq!(d.link_text.as_deref(), Some("label"));
        assert!(d.is_cross_universe());
    }

    #[test]
    fn inbound_projection_uses_from_side() {
        let dto = RelationMapper::domain_to_inbound(&edge());
        assert_eq!(dto.from_universe, "alpha");
        assert_eq!(dto.from_path, "a.md");
        assert_eq!(dto.relation_type, "assignee");
    }

    #[test]
    fn outbound_projection_uses_to_side() {
        let dto = RelationMapper::domain_to_outbound(&edge());
        assert_eq!(dto.to_universe.as_deref(), Some("beta"));
        assert_eq!(dto.to_path, "b.md");
        assert_eq!(dto.relation_type, "assignee");
    }
}
