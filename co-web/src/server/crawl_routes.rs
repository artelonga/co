//! CO-397: Bot/scraper-friendly endpoints.
//!
//! GET /robots.txt   — standard crawl policy
//! GET /sitemap.xml  — auto-generated from public universe list

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::server::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/robots.txt", get(robots_txt))
        .route("/sitemap.xml", get(sitemap_xml))
}

async fn robots_txt() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "User-agent: *\n\
         Allow: /\n\
         Disallow: /api/v1/me/\n\
         Disallow: /gestao/\n\
         Disallow: /_proposals/\n\
         \n\
         Sitemap: https://co-artelonga.fly.dev/sitemap.xml\n",
    )
}

async fn sitemap_xml(headers: HeaderMap, State(state): State<AppState>) -> Response {
    let base = derive_base_url(&headers);

    let keys: Vec<String> = {
        let storage = state.core.storage.lock();
        let conn = storage.conn();
        conn.prepare("SELECT key FROM universes WHERE is_public = 1 ORDER BY key LIMIT 500")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
    };

    let mut xml = String::with_capacity(4096);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    xml.push_str(&format!("  <url><loc>{}/</loc></url>\n", base));
    for key in &keys {
        xml.push_str(&format!(
            "  <url><loc>{}/{}</loc></url>\n",
            base,
            escape_xml(key)
        ));
    }
    xml.push_str("</urlset>\n");

    (
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        xml,
    )
        .into_response()
}

fn derive_base_url(headers: &HeaderMap) -> String {
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("https");
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("co-artelonga.fly.dev");
    format!("{}://{}", proto, host)
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_robots_txt_content() {
        let body = concat!(
            "User-agent: *\n",
            "Allow: /\n",
            "Disallow: /api/v1/me/\n",
            "Disallow: /gestao/\n",
            "Disallow: /_proposals/\n",
            "\n",
            "Sitemap: https://co-artelonga.fly.dev/sitemap.xml\n"
        );
        assert!(body.contains("Disallow: /api/v1/me/"));
        assert!(body.contains("Disallow: /gestao/"));
        assert!(body.contains("Disallow: /_proposals/"));
        assert!(body.contains("Sitemap: https://co-artelonga.fly.dev/sitemap.xml"));
    }

    #[test]
    fn test_escape_xml_special_chars() {
        assert_eq!(escape_xml("a&b"), "a&amp;b");
        assert_eq!(escape_xml("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_xml("it's"), "it&apos;s");
    }

    #[test]
    fn test_derive_base_url_fallback() {
        let headers = HeaderMap::new();
        assert_eq!(derive_base_url(&headers), "https://co-artelonga.fly.dev");
    }

    #[test]
    fn test_derive_base_url_from_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "example.com".parse().unwrap());
        headers.insert("x-forwarded-proto", "http".parse().unwrap());
        assert_eq!(derive_base_url(&headers), "http://example.com");
    }
}
