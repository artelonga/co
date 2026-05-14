//! Pretty-URL redirects for known template seed pages.
//!
//! Maps `GET /<slug>` to `GET /template/<slug>` via 302 redirect when
//! `slug` is one of the seeded template pages. The browser ends up on
//! the canonical `/template/<slug>` URL and the SPA resolves the entry
//! via its existing routing.
//!
//! The slug list is hardcoded to keep this layer fast and predictable
//! and to avoid shadowing real universe slugs (`co`, `template`,
//! `yggdrasil`, etc.) on routes that happen to share a name.

use axum::{
    Router,
    extract::Path,
    response::{IntoResponse, Redirect},
    routing::get,
};

use crate::server::AppState;

/// Seed page slugs eligible for pretty-URL redirect.
///
/// Keep in sync with `reseed_template_content_pages` in
/// `co-web/src/storage/seed.rs`.
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

async fn maybe_redirect(Path(slug): Path<String>) -> impl IntoResponse {
    if SEED_PAGE_SLUGS.contains(&slug.as_str()) {
        Redirect::temporary(&format!("/template/{}", slug)).into_response()
    } else {
        // Not a known pretty URL — fall through to the SPA fallback
        // by returning 404 here; axum will hand off to the fallback
        // chain. Returning Redirect on every miss would clash with
        // existing universe routes.
        axum::http::StatusCode::NOT_FOUND.into_response()
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/{slug}", get(maybe_redirect))
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
}
