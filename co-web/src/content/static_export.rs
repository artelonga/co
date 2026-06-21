//! CO-93 Phase 4: static export for `public-static` (and `template`) universes.
//!
//! Public static content has no per-viewer state, so it can be server-rendered
//! to HTML and cached at a CDN edge with a long TTL + stale-while-revalidate. On
//! an entry write we emit a CDN purge for the affected path so the edge refreshes.
//!
//! * `GET /co/{slug}/{*path}` → server-rendered HTML with CDN cache headers.
//! * [`cdn_purge_request`] + [`maybe_purge_cdn`] → best-effort edge purge hook,
//!   wired into the vault write path (env-gated; default no-op).
//!
//! The universe type is *derived* (see [`crate::models::Universe::universe_type`]),
//! so this endpoint never needs a denormalized column. Private universes are
//! never served here (their bodies are encrypted at rest); the endpoint returns
//! 403 so confidential content can't leak through a shared cache.

use axum::{
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::platform::error::AppError;
use crate::server::AppState;

/// CDN cache directive for public-static reads: cache 60s at the edge, then
/// serve stale for up to 24h while revalidating in the background.
pub const PUBLIC_STATIC_CACHE_CONTROL: &str = "public, s-maxage=60, stale-while-revalidate=86400";

/// `GET /co/{slug}/{*path}` — render a public-static entry to cacheable HTML.
pub async fn serve_static_html(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
) -> Result<Response, AppError> {
    if path.contains("..") {
        return Err(AppError::BadRequest("path must not contain ..".into()));
    }
    let entry_path = normalize_entry_path(&path);

    let (utype, title) = {
        let storage = state.core.storage.lock();
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        (universe.universe_type(), Some(universe.name))
    };

    // Only public types are static-exportable. Private content stays off the CDN.
    if !utype.static_exportable() {
        return Err(AppError::Forbidden(
            "Only public-static universes can be served as static HTML".into(),
        ));
    }

    let entry = {
        let uc = {
            let storage = state.core.storage.lock();
            storage.universe_conn(&slug)
        };
        crate::repository::SqliteEntryRepository::new(uc)
            .get(&slug, &entry_path)
            .map_err(|e| AppError::Internal(format!("load entry: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("Entry '{entry_path}' not found")))?
    };

    let page_title = entry
        .title
        .clone()
        .or(title)
        .unwrap_or_else(|| slug.clone());
    let html = render_html(&page_title, &entry.body);

    let mut resp = (StatusCode::OK, axum::response::Html(html)).into_response();
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(PUBLIC_STATIC_CACHE_CONTROL),
    );
    // Cache-Tag lets a Cloudflare-style edge purge by tag on write.
    if let Ok(tag) = HeaderValue::from_str(&cache_tag(&slug, &entry_path)) {
        resp.headers_mut().insert("Cache-Tag", tag);
    }
    Ok(resp)
}

/// Map a request path to a vault entry path: default to `index.md`, and append
/// `.md` when no extension is present.
fn normalize_entry_path(path: &str) -> String {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return "index.md".to_string();
    }
    if std::path::Path::new(p).extension().is_some() {
        p.to_string()
    } else {
        format!("{p}.md")
    }
}

/// The `Cache-Tag` for an entry — purging this tag invalidates exactly this page.
pub fn cache_tag(slug: &str, entry_path: &str) -> String {
    format!("co:{slug}:{entry_path}")
}

/// Minimal, dependency-free markdown → HTML for server-rendered public pages.
///
/// Handles ATX headings (`#`..`######`), blank-line-separated paragraphs, and
/// HTML-escapes everything so user content can never inject markup. This is a
/// safe baseline; richer rendering can layer on a markdown crate later without
/// changing the endpoint contract.
pub fn render_html(title: &str, body_md: &str) -> String {
    let mut out = String::new();
    let mut paragraph: Vec<String> = Vec::new();

    let flush = |out: &mut String, paragraph: &mut Vec<String>| {
        if !paragraph.is_empty() {
            out.push_str("<p>");
            out.push_str(&paragraph.join(" "));
            out.push_str("</p>\n");
            paragraph.clear();
        }
    };

    for line in body_md.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut out, &mut paragraph);
            continue;
        }
        if let Some(level) = heading_level(trimmed) {
            flush(&mut out, &mut paragraph);
            let text = escape_html(trimmed[level..].trim());
            out.push_str(&format!("<h{level}>{text}</h{level}>\n"));
        } else {
            paragraph.push(escape_html(trimmed));
        }
    }
    flush(&mut out, &mut paragraph);

    format!(
        "<!DOCTYPE html>\n<html lang=\"pt-BR\">\n<head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n</head>\n<body>\n<main>\n{}</main>\n</body>\n</html>\n",
        escape_html(title),
        out
    )
}

/// ATX heading level (1..6) if `line` starts with that many `#` then a space.
fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        Some(hashes)
    } else {
        None
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Build the CDN purge request (target URL + cache tag) for a written path.
///
/// Pure so the purge wiring is unit-testable. `base` is the configured purge
/// endpoint (`CO_CDN_PURGE_URL`); returns `None` when no endpoint is set so the
/// hook is a no-op by default.
pub fn cdn_purge_request(
    base: Option<&str>,
    slug: &str,
    entry_path: &str,
) -> Option<(String, String)> {
    let base = base?.trim();
    if base.is_empty() {
        return None;
    }
    Some((base.to_string(), cache_tag(slug, entry_path)))
}

/// Best-effort CDN purge after a vault write to a public-static universe.
///
/// Reads `CO_CDN_PURGE_URL`; default-off (no env ⇒ no-op). When set, it emits a
/// `webhook_dispatch` job carrying the purge target + cache tag so the existing
/// job worker performs the actual HTTP call out-of-band (never blocking the
/// write path). Failures are swallowed — a missed purge only means stale edge
/// content until the 60s TTL lapses.
pub fn maybe_purge_cdn(state: &AppState, slug: &str, entry_path: &str) {
    // Only public-static / template content is ever on the CDN.
    let exportable = {
        let storage = state.core.storage.lock();
        storage
            .get_universe(slug)
            .map(|u| u.universe_type().static_exportable())
            .unwrap_or(false)
    };
    if !exportable {
        return;
    }

    let purge_url = crate::infra::secrets::global().get("CO_CDN_PURGE_URL");
    let Some((url, tag)) = cdn_purge_request(purge_url.as_deref(), slug, entry_path) else {
        return;
    };

    let payload = serde_json::json!({
        "purge_url": url,
        "cache_tag": tag,
        "slug": slug,
        "entry_path": entry_path,
    });
    // Out-of-band: the existing job worker performs the HTTP purge. Failure to
    // enqueue is non-fatal — the edge self-heals after the 60s TTL.
    let storage = state.core.storage.lock();
    let _ = crate::platform::job_queue::enqueue_job(
        storage.conn(),
        slug,
        crate::platform::job_queue::JobKind::WebhookDispatch,
        &payload.to_string(),
        Some(&format!("cdn-purge:{tag}")),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize_entry_path(""), "index.md");
        assert_eq!(normalize_entry_path("/"), "index.md");
        assert_eq!(normalize_entry_path("about"), "about.md");
        assert_eq!(normalize_entry_path("notes/a.md"), "notes/a.md");
        assert_eq!(normalize_entry_path("style.css"), "style.css");
    }

    #[test]
    fn renders_headings_and_paragraphs_escaped() {
        let html = render_html("My <Page>", "# Hello\n\nA <b>line</b> & more.");
        assert!(html.contains("<title>My &lt;Page&gt;</title>"));
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<p>A &lt;b&gt;line&lt;/b&gt; &amp; more.</p>"));
        // No raw user markup survives.
        assert!(!html.contains("<b>line</b>"));
    }

    #[test]
    fn heading_levels() {
        assert_eq!(heading_level("# x"), Some(1));
        assert_eq!(heading_level("###### x"), Some(6));
        assert_eq!(heading_level("####### x"), None); // 7 = too many
        assert_eq!(heading_level("#nospace"), None);
        assert_eq!(heading_level("plain"), None);
    }

    #[test]
    fn cache_control_is_cdn_tuned() {
        assert!(PUBLIC_STATIC_CACHE_CONTROL.contains("s-maxage=60"));
        assert!(PUBLIC_STATIC_CACHE_CONTROL.contains("stale-while-revalidate=86400"));
    }

    #[test]
    fn purge_request_is_none_without_endpoint() {
        assert!(cdn_purge_request(None, "u", "a.md").is_none());
        assert!(cdn_purge_request(Some("   "), "u", "a.md").is_none());
        let (url, tag) = cdn_purge_request(Some("https://cdn/purge"), "u", "a.md").unwrap();
        assert_eq!(url, "https://cdn/purge");
        assert_eq!(tag, "co:u:a.md");
    }
}

/// CO-93 Phase 4: end-to-end route tests — a public-static universe is served as
/// cacheable HTML, while a private universe is refused (403) so encrypted-at-rest
/// content never leaks through a shared cache.
#[cfg(test)]
mod route_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn build_state(dir: &std::path::Path) -> crate::server::AppState {
        // SAFETY: single-threaded test environment.
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let config = crate::config::WebConfig {
            data_dir: dir.to_str().unwrap().to_string(),
            port: 0,
            static_dir: String::new(),
            default_variant: "a".into(),
            experiments: false,
            plugins_dir: String::new(),
            game_db_path: None,
            universo_dir: String::new(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "test".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: true,
        };

        let mut storage = crate::storage::Storage::new(dir);
        // A public-static universe (`public-subscribable` + no proposals).
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "pubuni".into(),
                    name: "Public Universe".into(),
                    description: String::new(),
                },
                "owner",
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "UPDATE universes SET visibility = 'public-subscribable', is_public = 1 \
                 WHERE key = 'pubuni'",
                [],
            )
            .unwrap();
        // A private-static universe (default `private` visibility).
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "privuni".into(),
                    name: "Private Universe".into(),
                    description: String::new(),
                },
                "owner",
            )
            .unwrap();

        let experiment = crate::experiment::ExperimentStore::new(dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        crate::server::AppState::new(crate::server::AppStateInner {
            core: Arc::new(crate::server::CoreState::from_storage(
                storage, config, auth_store,
            )),
            realtime: Arc::new(crate::server::RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
                chat_presence: Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(crate::server::IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(crate::server::IntegrationsState {
                mail,
                geo: Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        })
    }

    /// Public-static universe → 200 with the CDN cache directive + Cache-Tag, and
    /// the rendered (HTML-escaped) body.
    #[tokio::test]
    async fn public_static_is_served_with_cdn_cache_headers() {
        let dir = tempdir().unwrap();
        let state = build_state(dir.path());
        crate::vault_routes::write_vault_entry(
            &state,
            "pubuni",
            "about.md",
            serde_json::json!({ "type": "page", "title": "About" }),
            "# About\n\nHello world.",
        )
        .unwrap();

        let app = crate::server::build_router(state, None);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/co/pubuni/about")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some(super::PUBLIC_STATIC_CACHE_CONTROL),
            "public-static must carry the CDN s-maxage + stale-while-revalidate directive"
        );
        assert_eq!(
            resp.headers()
                .get("Cache-Tag")
                .and_then(|v| v.to_str().ok()),
            Some("co:pubuni:about.md"),
            "Cache-Tag enables per-entry edge purge on write"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(html.contains("<h1>About</h1>"));
        assert!(html.contains("<p>Hello world.</p>"));
    }

    /// Private universe → 403, never served as cacheable HTML.
    #[tokio::test]
    async fn private_universe_is_refused() {
        let dir = tempdir().unwrap();
        let state = build_state(dir.path());
        let app = crate::server::build_router(state, None);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/co/privuni/about")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "private content must never be served through the public CDN route"
        );
    }
}
