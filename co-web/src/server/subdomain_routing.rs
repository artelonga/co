use axum::body::Body;
use axum::http::{Request, Response, header};
use axum::middleware::Next;

/// Universe key extracted from a user-controlled subdomain.
///
/// Set in request extensions by [`subdomain_routing_middleware`] when the
/// incoming `Host` header matches `<slug>.artelonga.com.br`.
/// Downstream handlers (e.g. `serve_co_index`) read this to inject the
/// `window.__CO_SUBDOMAIN_UNIVERSE__` bootstrap script.
#[derive(Clone, Debug)]
pub struct SubdomainUniverse(pub String);

/// Middleware that detects requests arriving via a universe subdomain
/// (`<slug>.artelonga.com.br`) and stores the resolved universe key in the
/// request extensions for use by SPA-serving handlers.
///
/// Reserved subdomain prefixes (`co`, `www`) are ignored so the main app
/// domains pass through unchanged.
pub async fn subdomain_routing_middleware(mut req: Request<Body>, next: Next) -> Response<Body> {
    if let Some(key) = extract_subdomain_universe(&req) {
        req.extensions_mut().insert(SubdomainUniverse(key));
    }
    next.run(req).await
}

fn extract_subdomain_universe(req: &Request<Body>) -> Option<String> {
    let host = req.headers().get(header::HOST)?.to_str().ok()?;
    // Strip port if present (e.g. "yuri.artelonga.com.br:8080" → "yuri.artelonga.com.br")
    let host = host.split(':').next()?;
    // Match *.artelonga.com.br only
    let slug = host.strip_suffix(".artelonga.com.br")?;
    // Skip reserved prefixes and empty strings
    const RESERVED: &[&str] = &["co", "www", ""];
    if RESERVED.contains(&slug) {
        return None;
    }
    // Validate: lowercase alphanumeric + hyphens, no leading/trailing hyphens
    if !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
    {
        Some(slug.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;

    fn req_with_host(host: &str) -> Request<Body> {
        Request::builder()
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap()
    }

    #[test]
    fn extracts_yuri_subdomain() {
        let req = req_with_host("yuri.artelonga.com.br");
        assert_eq!(extract_subdomain_universe(&req).as_deref(), Some("yuri"));
    }

    #[test]
    fn extracts_subdomain_with_port() {
        let req = req_with_host("alice.artelonga.com.br:8080");
        assert_eq!(extract_subdomain_universe(&req).as_deref(), Some("alice"));
    }

    #[test]
    fn ignores_co_subdomain() {
        let req = req_with_host("co.artelonga.com.br");
        assert!(extract_subdomain_universe(&req).is_none());
    }

    #[test]
    fn ignores_www_subdomain() {
        let req = req_with_host("www.artelonga.com.br");
        assert!(extract_subdomain_universe(&req).is_none());
    }

    #[test]
    fn ignores_fly_dev_host() {
        let req = req_with_host("co-artelonga.fly.dev");
        assert!(extract_subdomain_universe(&req).is_none());
    }

    #[test]
    fn ignores_bare_artelonga() {
        let req = req_with_host("artelonga.com.br");
        assert!(extract_subdomain_universe(&req).is_none());
    }

    #[test]
    fn ignores_invalid_slug_with_dots() {
        let req = req_with_host("sub.sub.artelonga.com.br");
        // "sub.sub" contains a dot — not a valid universe slug
        assert!(extract_subdomain_universe(&req).is_none());
    }

    #[test]
    fn extracts_hyphenated_slug() {
        let req = req_with_host("meu-universo.artelonga.com.br");
        assert_eq!(
            extract_subdomain_universe(&req).as_deref(),
            Some("meu-universo")
        );
    }

    #[test]
    fn ignores_leading_hyphen() {
        let req = req_with_host("-bad.artelonga.com.br");
        assert!(extract_subdomain_universe(&req).is_none());
    }
}
