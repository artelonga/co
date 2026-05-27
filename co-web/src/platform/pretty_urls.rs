//! Pretty-URL slug registry for seeded content pages.
//!
//! `/<slug>` short URLs (e.g. `/sobre`, `/seguranca`) redirect to their
//! canonical universe path so the SPA can resolve the entry via its
//! existing universe routing. The redirect itself lives in
//! `crate::server::static_files::serve_co_index` — this module owns the
//! canonical slug lists so adding a new seed page is a single-place edit.
//!
//! Two groups:
//! - `TEMPLATE_SLUGS`: welcome/onboarding pages that live in the `template`
//!   universe → `/<slug>` redirects to `/template/<slug>`.
//! - `CO_PUBLIC_SLUGS`: transparency/security/infra pages that live in
//!   `co::public/*` → `/<slug>` redirects to `/co/public/<slug>`.
//!
//! Keep both lists in sync with the seed constants in
//! `co-web/src/storage/mod.rs` (`reseed_template_content_pages` and
//! `reseed_co_public_pages` respectively).

const TEMPLATE_SLUGS: &[&str] = &[
    // Welcome / intro — live in `template` universe
    "sobre",
    "termos",
    "privacidade",
    "dados-rastreados",
    "linhas-do-tempo",
    "co-plataforma",
    "guia",
];

const CO_PUBLIC_SLUGS: &[&str] = &[
    // Security cluster — live in `co::public/*`
    "seguranca",
    "seguranca-dependencias",
    "seguranca-cenarios",
    "seguranca-vapid",
    "seguranca-criptografia",
    // License + renderers
    "licensa",
    "renderers",
    // Infra catalog — CO-specific pages only
    "infra",
    "infra-co",
    // Conta + mensagens (2.8.1)
    "conta-e-mensagens",
];

/// Returns the redirect target for a slug short URL, or `None` if the
/// slug is not in either list.
///
/// - Template pages → `/template/<slug>`
/// - co/public pages → `/co/public/<slug>`
pub fn slug_redirect_target(slug: &str) -> Option<String> {
    if TEMPLATE_SLUGS.contains(&slug) {
        Some(format!("/template/{slug}"))
    } else if CO_PUBLIC_SLUGS.contains(&slug) {
        Some(format!("/co/public/{slug}"))
    } else {
        None
    }
}

/// True when `slug` matches any known seed page slug (template or co/public).
pub fn is_seed_page_slug(slug: &str) -> bool {
    TEMPLATE_SLUGS.contains(&slug) || CO_PUBLIC_SLUGS.contains(&slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_slugs_are_lowercase_and_slug_safe() {
        for slug in TEMPLATE_SLUGS.iter().chain(CO_PUBLIC_SLUGS.iter()) {
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
        for slug in TEMPLATE_SLUGS.iter().chain(CO_PUBLIC_SLUGS.iter()) {
            assert!(seen.insert(slug), "duplicate slug: {}", slug);
        }
    }

    #[test]
    fn is_seed_page_slug_works() {
        assert!(is_seed_page_slug("seguranca"));
        assert!(is_seed_page_slug("licensa"));
        assert!(is_seed_page_slug("sobre"));
        assert!(!is_seed_page_slug("not-a-page"));
        assert!(!is_seed_page_slug(""));
    }

    #[test]
    fn slug_redirect_target_template() {
        assert_eq!(
            slug_redirect_target("sobre"),
            Some("/template/sobre".to_string())
        );
        assert_eq!(
            slug_redirect_target("termos"),
            Some("/template/termos".to_string())
        );
    }

    #[test]
    fn slug_redirect_target_co_public() {
        assert_eq!(
            slug_redirect_target("seguranca"),
            Some("/co/public/seguranca".to_string())
        );
    }

    #[test]
    fn slug_redirect_target_unknown() {
        assert_eq!(slug_redirect_target("not-a-page"), None);
        assert_eq!(slug_redirect_target(""), None);
    }
}
