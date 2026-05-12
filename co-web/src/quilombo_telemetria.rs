//! Telemetry middleware for the Quilombo community platform.
//!
//! Tracks page views, filters bots and scanner probes, manages visitor cookies.
//! Ported from quilombo-blog's hooks.server.ts.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, Response, header};
use axum::middleware::Next;

use crate::quilombo_storage;
use crate::server::AppState;

// --- Bot detection ---

const BOT_PATTERNS: &[&str] = &[
    "bot",
    "crawl",
    "spider",
    "slurp",
    "googlebot",
    "bingbot",
    "baiduspider",
    "yandexbot",
    "duckduckbot",
    "facebookexternalhit",
    "twitterbot",
    "linkedinbot",
    "whatsapp",
    "applebot",
    "discordbot",
    "telegrambot",
    "pinterestbot",
    "slackbot",
    "embedly",
    "quora link preview",
    "redditbot",
    "sogou",
    "exabot",
    "semrushbot",
    "ahrefsbot",
    "mj12bot",
    "dotbot",
    "petalbot",
    "uptimerobot",
    "pingdom",
    "statuscake",
    "curl",
    "wget",
    "python-requests",
    "go-http-client",
    "java/",
    "apache-httpclient",
    "okhttp",
    "postmanruntime",
    "insomnia",
    "axios",
    "node-fetch",
    "undici",
    "fly-healthcheck",
    "meta-externalagent",
    "ccbot",
    "amazonbot",
    "anthropic-ai",
    "cohere-ai",
    "gptbot",
    "claudebot",
    "consul health check",
    "headlesschrome",
    "phantomjs",
    "lighthouse",
];

const SCANNER_PREFIXES: &[&str] = &[
    "/wp-",
    "/.env",
    "/.git",
    "/phpinfo",
    "/_profiler",
    "/phpmyadmin",
    "/xmlrpc",
    "/administrator",
    "/debug/",
    "/cgi-bin",
];

const SCANNER_PATHS: &[&str] = &[
    "/signup",
    "/pricing",
    "/donate",
    "/dashboard",
    "/cart",
    "/subscribe",
    "/shop",
    "/payment",
    "/order",
    "/checkout",
    "/account",
    "/register",
    "/billing",
    "/blog-verify",
];

fn is_bot(user_agent: &str) -> bool {
    let ua_lower = user_agent.to_lowercase();
    BOT_PATTERNS
        .iter()
        .any(|pattern| ua_lower.contains(pattern))
}

fn is_scanner_path(path: &str) -> bool {
    SCANNER_PREFIXES.iter().any(|p| path.starts_with(p)) || SCANNER_PATHS.contains(&path)
}

/// Hash IP using xxhash (fast, non-cryptographic — sufficient for analytics).
fn hash_ip(ip: &str) -> String {
    let hash = xxhash_rust::xxh3::xxh3_64(ip.as_bytes());
    format!("{hash:016x}")
}

fn extract_ip(req: &Request<Body>) -> String {
    // Try Fly.io / proxy headers first
    if let Some(first) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|xff| xff.split(',').next())
    {
        return first.trim().to_string();
    }
    "unknown".to_string()
}

/// CO-97 Option A: read `al_vid` (marketing apex cookie) first so both
/// surfaces share one canonical visitor token.  Falls back to `visitante_id`
/// for visitors who hit Co before the marketing site.
fn extract_visitor_token(req: &Request<Body>) -> Option<String> {
    let cookies = req.headers().get(header::COOKIE)?.to_str().ok()?;
    let mut visitante_id = None;
    for cookie in cookies.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix("al_vid=") {
            return Some(value.to_string()); // marketing token wins
        }
        if let Some(value) = cookie.strip_prefix("visitante_id=") {
            visitante_id = Some(value.to_string());
        }
    }
    visitante_id
}

/// Telemetry middleware — tracks page views, filters bots.
pub async fn telemetry_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response<Body> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // Skip API requests, static assets, and health checks
    if path.starts_with("/api/")
        || path.starts_with("/_app/")
        || path.starts_with("/static/")
        || path.contains('.')
        || path == "/api/health"
    {
        return next.run(req).await;
    }

    // Only track GET requests
    if method != axum::http::Method::GET {
        return next.run(req).await;
    }

    let user_agent = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Filter bots
    if is_bot(&user_agent) {
        return next.run(req).await;
    }

    // Filter scanner probes
    if is_scanner_path(&path) {
        return next.run(req).await;
    }

    let ip_hash = hash_ip(&extract_ip(&req));
    let visitor_token = extract_visitor_token(&req);
    let referrer = req
        .headers()
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let start = Instant::now();

    let mut response = next.run(req).await;

    let duration_ms = start.elapsed().as_millis() as i64;

    // Generate visitor token if not present
    let token = visitor_token.unwrap_or_else(|| nanoid::nanoid!(24));

    // CO-97: emit al_vid at apex scope so marketing site JS can read the same
    // token.  HttpOnly is intentionally dropped — this token is analytics-only
    // (no auth role); XSS worst-case is skewed attribution, not account takeover.
    // Documented in docs/decisions/001-visitor-token-unification.md.
    let cookie = format!(
        "al_vid={token}; Domain=.artelonga.com.br; Path=/; SameSite=Lax; Secure; Max-Age=31536000"
    );
    if let Ok(cookie_val) = cookie.parse() {
        response
            .headers_mut()
            .append(header::SET_COOKIE, cookie_val);
    }

    // Record visit asynchronously (don't block response)
    let state = Arc::clone(&state);
    let path_clone = path;
    let user_agent_clone = user_agent;
    tokio::spawn(async move {
        let storage = state.storage.lock();
        // Filter internal referrers
        let filtered_referrer = referrer.as_deref().and_then(|r| {
            if r.contains("quilomboaraucaria.org")
                || r.contains("quilombo-araucaria")
                || r.contains("localhost")
            {
                None
            } else {
                Some(r)
            }
        });

        quilombo_storage::registrar_visita(
            storage.conn(),
            Some(&token),
            None, // user_id extracted at request level, not here
            &path_clone,
            filtered_referrer,
            Some(&user_agent_clone),
            Some(&ip_hash),
            Some(duration_ms),
        );
    });

    response
}

/// Canonical host redirect middleware.
///
/// Redirects `*.fly.dev` and `www.quilomboaraucaria.org` to the canonical host.
/// Configured via `CANONICAL_HOST` env var.
pub async fn canonical_host_middleware(req: Request<Body>, next: Next) -> Response<Body> {
    let canonical = match std::env::var("CANONICAL_HOST") {
        Ok(host) if !host.is_empty() => host,
        _ => return next.run(req).await, // no canonical host set, skip
    };

    let request_host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Redirect if host doesn't match canonical
    if !request_host.is_empty()
        && request_host != canonical
        && (request_host.ends_with(".fly.dev") || request_host.starts_with("www."))
    {
        let path_and_query = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");
        let target = format!("https://{canonical}{path_and_query}");

        return Response::builder()
            .status(301)
            .header(header::LOCATION, target)
            .body(Body::empty())
            .unwrap_or_else(|_| Response::builder().status(500).body(Body::empty()).unwrap());
    }

    next.run(req).await
}

/// CSRF protection middleware.
///
/// Rejects non-safe HTTP methods (POST, PUT, DELETE) if the Origin header
/// doesn't match allowed origins. Configured via `ALLOWED_ORIGINS` env var.
pub async fn csrf_middleware(req: Request<Body>, next: Next) -> Response<Body> {
    let method = req.method().clone();

    // Safe methods are exempt
    if method == axum::http::Method::GET
        || method == axum::http::Method::HEAD
        || method == axum::http::Method::OPTIONS
    {
        return next.run(req).await;
    }

    // Get allowed origins from env (comma-separated)
    let allowed = std::env::var("ALLOWED_ORIGINS").unwrap_or_default();
    let allowed_origins: Vec<&str> = allowed.split(',').map(|s| s.trim()).collect();

    // Also always allow the canonical host
    let canonical = std::env::var("CANONICAL_HOST").unwrap_or_default();

    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // If no Origin header, allow (e.g., curl, server-to-server)
    if origin.is_empty() {
        return next.run(req).await;
    }

    // Same-origin check: if origin matches the request Host, always allow
    let request_host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Hardcoded trust list mirrors the CORS layer's allowlist (CO-205).
    // Origins legitimately allowed to POST cross-domain to this server.
    // Without this, CSRF rejected logout/onboard calls from
    // artelonga.com.br even though CORS preflight succeeded (2026-05-12
    // bug — "sair on artelonga doesn't work").
    const TRUSTED_HOSTS: &[&str] = &[
        "artelonga.com.br",
        "co.artelonga.com.br",
        "yggdrasil.artelonga.com.br",
        "quilomboaraucaria.com.br",
        "quilomboaraucaria.org",
    ];

    let is_allowed = origin.contains(request_host) && !request_host.is_empty()
        || allowed_origins
            .iter()
            .any(|o| !o.is_empty() && origin.contains(o))
        || (!canonical.is_empty() && origin.contains(&canonical))
        || TRUSTED_HOSTS.iter().any(|h| origin.contains(h))
        || origin.contains("localhost")
        || origin.contains("127.0.0.1");

    if !is_allowed {
        return Response::builder()
            .status(403)
            .body(Body::from("CSRF: Origin not allowed"))
            .unwrap_or_else(|_| Response::builder().status(403).body(Body::empty()).unwrap());
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_bot() {
        assert!(is_bot("Mozilla/5.0 (compatible; Googlebot/2.1)"));
        assert!(is_bot("Consul Health Check"));
        assert!(is_bot("meta-externalagent/1.0"));
        assert!(is_bot("python-requests/2.28.0"));
        assert!(!is_bot(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
        ));
    }

    #[test]
    fn test_is_scanner_path() {
        assert!(is_scanner_path("/wp-admin"));
        assert!(is_scanner_path("/wp-login.php"));
        assert!(is_scanner_path("/.env"));
        assert!(is_scanner_path("/.git/config"));
        assert!(is_scanner_path("/signup"));
        assert!(!is_scanner_path("/"));
        assert!(!is_scanner_path("/blog"));
        assert!(!is_scanner_path("/encontros"));
    }

    #[test]
    fn test_hash_ip() {
        let hash1 = hash_ip("192.168.1.1");
        let hash2 = hash_ip("192.168.1.2");
        assert_eq!(hash1.len(), 16);
        assert_ne!(hash1, hash2);
        // Deterministic
        assert_eq!(hash1, hash_ip("192.168.1.1"));
    }
}
