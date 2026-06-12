//! Reference mapper — Domain ↔ DTO conversions.
//!
//! CO-432: propagates the CO-390 mapper pattern from `entries` to `references`.
//! Mappers are pure functions, easily unit-tested.

use crate::domain::ReferenceDomain;
use crate::dto::references::{ListRefsQuery, OrphanBlob, ReferenceCard};
use crate::repository::reference_repository::{CardFilter, OrphanBlobRecord};

/// Stateless mapper — all methods are associated functions.
pub struct ReferenceMapper;

impl ReferenceMapper {
    /// Convert the domain entity into the card response DTO.
    ///
    /// Wire-identical to the pre-CO-432 `row_to_card` serialization.
    pub fn domain_to_card(domain: ReferenceDomain) -> ReferenceCard {
        ReferenceCard {
            universe_key: domain.universe_key,
            entry_path: domain.entry_path,
            edition_id: domain.edition_id,
            work_id: domain.work_id,
            primary_layer: domain.primary_layer,
            file: domain.file,
            blob_sha256: domain.blob_sha256,
            url: domain.url,
            medium: domain.medium,
            mime: domain.mime,
            size_bytes: domain.size_bytes,
            language: domain.language,
            seed_status: domain.seed_status,
            indexed_at: domain.indexed_at,
            title: domain.title,
        }
    }

    /// Convert the list-endpoint query DTO into the repository filter.
    pub fn query_to_filter(q: ListRefsQuery) -> CardFilter {
        CardFilter {
            medium: q.medium,
            seed_status: q.seed_status,
            work_id: q.work_id,
            primary_layer: q.primary_layer,
            fts: q.q,
        }
    }

    /// Convert an orphan-blob record into its response DTO.
    pub fn orphan_to_dto(record: OrphanBlobRecord) -> OrphanBlob {
        OrphanBlob {
            sha256: record.sha256,
            mime: record.mime,
            size_bytes: record.size_bytes,
            filename: record.filename,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_to_card_preserves_all_fields() {
        let d = ReferenceDomain {
            universe_key: "u".into(),
            entry_path: "refs/X.md".into(),
            edition_id: "default".into(),
            work_id: "X".into(),
            primary_layer: Some(1),
            file: Some("x.pdf".into()),
            blob_sha256: Some("abc".into()),
            url: None,
            medium: "pdf".into(),
            mime: Some("application/pdf".into()),
            size_bytes: Some(42),
            language: Some("pt".into()),
            seed_status: "reviewed".into(),
            indexed_at: "2026-06-12".into(),
            title: Some("X title".into()),
        };
        let c = ReferenceMapper::domain_to_card(d);
        assert_eq!(c.entry_path, "refs/X.md");
        assert_eq!(c.edition_id, "default");
        assert_eq!(c.work_id, "X");
        assert_eq!(c.primary_layer, Some(1));
        assert_eq!(c.file.as_deref(), Some("x.pdf"));
        assert_eq!(c.blob_sha256.as_deref(), Some("abc"));
        assert_eq!(c.medium, "pdf");
        assert_eq!(c.seed_status, "reviewed");
        assert_eq!(c.title.as_deref(), Some("X title"));
    }
}
