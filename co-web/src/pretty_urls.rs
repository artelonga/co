//! Pretty-URL slug registry for seeded template pages.
//!
//! `/<slug>` short URLs (e.g. `/seguranca`, `/licensa`) 307-redirect to
//! `/template/<slug>` so the SPA can resolve the entry via its existing
//! universe routing. The redirect itself lives in
//! `crate::server::static_files::serve_co_index` — this module just
//! owns the canonical slug list so adding a new seed page is a
//! single-place edit.
//!
//! Keep `SEED_PAGE_SLUGS` in sync with
//! `reseed_template_content_pages` in `co-web/src/storage/seed.rs`.

const SEED_PAGE_SLUGS: &[&str] = &[
    // Welcome / intro
    "sobre",
    "termos",
    "privacidade",
    "dados-rastreados",
    "linhas-do-tempo",
    "co-plataforma",
    "guia",
    // Security cluster
    "seguranca",
    "seguranca-dependencias",
    "seguranca-cenarios",
    "seguranca-vapid",
    // License + renderers
    "licensa",
    "renderers",
    // Infra catalog
    "infra",
    "infra-co",
    "infra-yggdrasil",
    "infra-quilomboaraucaria",
    "infra-rfq-gateway",
];

/// True when `slug` matches one of the seeded template page slugs.
pub fn is_seed_page_slug(slug: &str) -> bool {
    SEED_PAGE_SLUGS.contains(&slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_slugs_are_lowercase_and_slug_safe() {
        for slug in SEED_PAGE_SLUGS {
            assert!(
                slug.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "slug `{}` contains non-slug characters",
                slug
            );
            assert!(!slug.is_empty());
            assert!(!slug.starts_with('-'));
            assert!(!slug.ends_with('-'));
        }
    }

    #[test]
    fn slug_list_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for slug in SEED_PAGE_SLUGS {
            assert!(seen.insert(slug), "duplicate slug: {}", slug);
        }
    }

    #[test]
    fn is_seed_page_slug_works() {
        assert!(is_seed_page_slug("seguranca"));
        assert!(is_seed_page_slug("licensa"));
        assert!(!is_seed_page_slug("not-a-page"));
        assert!(!is_seed_page_slug(""));
    }
}
