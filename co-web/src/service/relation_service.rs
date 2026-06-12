//! Relation service — business rules for the relations domain.
//!
//! CO-432: propagates the CO-390 service pattern from `entries` to `relations`.
//! Pure rules only — no HTTP, no database, no async runtime.

use crate::dto::relations::InboundRelation;

/// Relation business-rule service.
///
/// Zero-sized unit struct — all methods are associated functions (no state).
pub struct RelationService;

impl RelationService {
    /// Deterministic ordering for a merged inbound result set.
    ///
    /// Inbound edges are collected per-universe (own DB first, then every
    /// other universe's DB); this rule defines the stable presentation order:
    /// relation_type, then from_universe, then from_path.
    ///
    /// Extracted from `get_inbound_relations` in the relations routes.
    pub fn sort_inbound(edges: &mut [InboundRelation]) {
        edges.sort_by(|a, b| {
            a.relation_type
                .cmp(&b.relation_type)
                .then(a.from_universe.cmp(&b.from_universe))
                .then(a.from_path.cmp(&b.from_path))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(rt: &str, fu: &str, fp: &str) -> InboundRelation {
        InboundRelation {
            from_universe: fu.into(),
            from_path: fp.into(),
            relation_type: rt.into(),
        }
    }

    #[test]
    fn sort_inbound_orders_by_type_then_universe_then_path() {
        let mut edges = vec![
            edge("wikilink", "beta", "z.md"),
            edge("assignee", "beta", "a.md"),
            edge("wikilink", "alpha", "b.md"),
            edge("assignee", "alpha", "c.md"),
        ];
        RelationService::sort_inbound(&mut edges);
        let order: Vec<(String, String, String)> = edges
            .iter()
            .map(|e| {
                (
                    e.relation_type.clone(),
                    e.from_universe.clone(),
                    e.from_path.clone(),
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                ("assignee".into(), "alpha".into(), "c.md".into()),
                ("assignee".into(), "beta".into(), "a.md".into()),
                ("wikilink".into(), "alpha".into(), "b.md".into()),
                ("wikilink".into(), "beta".into(), "z.md".into()),
            ]
        );
    }
}
