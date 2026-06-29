//! CO-210: Segurança + Dependências + Licença docs SPA routes.
//!
//! Serves the markdown documentation already living in `docs/` as browsable
//! HTML pages under stable, indexable URLs (`/seguranca`, `/licensa`,
//! `/renderers`, …) with a collapsible left-rail (desktop) / dropdown (mobile)
//! navigation shared across every page.
//!
//! Design notes:
//! - **Renderer**: a tiny CommonMark + GFM-table subset (`markdown_to_html`).
//!   Every byte of doc content is HTML-escaped before tag emission, so no raw
//!   HTML — and in particular no inline `<script>` — survives into the page.
//!   That keeps the rendered output CSP-safe (the page's only script is the
//!   external `/shared/docs-viewer.js`).
//! - **Cross-links**: `[texto](vapid-security.md)` style links between docs are
//!   rewritten to their canonical route (`/seguranca/vapid`) at render time
//!   (`rewrite_doc_link`). External URLs / anchors are left untouched.
//! - **Telemetry**: each render emits a server-side `page_view` event and a
//!   404 attempt emits `404_route`, both written to `telemetry_events`. The
//!   reader opt-out (`localStorage["co_viewer_telemetry"] = "0"`, mirrored to a
//!   `co_viewer_telemetry=0` cookie by `docs-viewer.js`) suppresses emission.
//!   Real client-measured load time is sent as `page_render` from the browser.
//!
//! Routes are merged into the main router *before* the `/{slug}` catch-all so
//! these literal paths win over universe-slug resolution.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;

use crate::server::AppState;
use crate::telemetry::{EventRow, insert_event};

// ---------------------------------------------------------------------------
// Route table — path → embedded markdown
// ---------------------------------------------------------------------------

/// A documentation page: its public route, the source file basename (used for
/// cross-link rewriting), a display title, and the embedded markdown body.
pub struct DocRoute {
    /// Canonical public path, e.g. `/seguranca/vapid`.
    pub path: &'static str,
    /// Source markdown basename, e.g. `vapid-security.md`. Used to map
    /// inter-doc links to routes.
    pub source: &'static str,
    /// Page title (also the `<title>` and document `<h1>` fallback).
    pub title: &'static str,
    /// Embedded markdown content.
    pub markdown: &'static str,
}

/// All markdown-backed doc routes. Paths are matched verbatim by [`resolve`].
pub const DOC_ROUTES: &[DocRoute] = &[
    DocRoute {
        path: "/seguranca",
        source: "SECURITY.md",
        title: "Segurança",
        markdown: include_str!("../../../docs/security/SECURITY.md"),
    },
    DocRoute {
        path: "/seguranca/dependencias",
        source: "dependencies.md",
        title: "Dependências",
        markdown: include_str!("../../../docs/security/dependencies.md"),
    },
    DocRoute {
        path: "/seguranca/dependencias/decisoes",
        source: "dependency-decisions.md",
        title: "Decisões de dependências",
        markdown: include_str!("../../../docs/security/dependency-decisions.md"),
    },
    DocRoute {
        path: "/seguranca/red-team",
        source: "red-team-scenarios.md",
        title: "Red Team — cenários",
        markdown: include_str!("../../../docs/security/red-team-scenarios.md"),
    },
    DocRoute {
        path: "/seguranca/red-team/playbook",
        source: "incident-playbook.md",
        title: "Playbook de incidentes",
        markdown: include_str!("../../../docs/security/incident-playbook.md"),
    },
    DocRoute {
        path: "/seguranca/vapid",
        source: "vapid-security.md",
        title: "VAPID — segurança de push",
        markdown: include_str!("../../../docs/vapid-security.md"),
    },
    DocRoute {
        path: "/licensa",
        source: "licensa.md",
        title: "Licença",
        markdown: include_str!("../../../docs/licensa.md"),
    },
    DocRoute {
        path: "/renderers",
        source: "markdown-renderers.md",
        title: "Markdown renderers",
        markdown: include_str!("../../../docs/markdown-renderers.md"),
    },
    DocRoute {
        path: "/e2e-walkthrough",
        source: "e2e-walkthrough.md",
        title: "E2E walkthrough",
        markdown: include_str!("../../../docs/e2e-walkthrough.md"),
    },
];

/// A single navigation entry. `depth` controls the rail indent (0 = top level,
/// 1 = nested under the previous top-level item).
struct NavItem {
    label: &'static str,
    path: &'static str,
    depth: u8,
}

/// Ordered navigation rail shared by every doc page (mirrors the spec layout).
const NAV: &[NavItem] = &[
    NavItem {
        label: "Visão geral",
        path: "/seguranca",
        depth: 0,
    },
    NavItem {
        label: "Dependências",
        path: "/seguranca/dependencias",
        depth: 0,
    },
    NavItem {
        label: "Decisões",
        path: "/seguranca/dependencias/decisoes",
        depth: 1,
    },
    NavItem {
        label: "Red Team",
        path: "/seguranca/red-team",
        depth: 0,
    },
    NavItem {
        label: "Playbook",
        path: "/seguranca/red-team/playbook",
        depth: 1,
    },
    NavItem {
        label: "VAPID",
        path: "/seguranca/vapid",
        depth: 0,
    },
    NavItem {
        label: "Licença",
        path: "/licensa",
        depth: 0,
    },
    NavItem {
        label: "Renderers",
        path: "/renderers",
        depth: 0,
    },
    NavItem {
        label: "E2E flow",
        path: "/e2e-walkthrough",
        depth: 0,
    },
    // CO-452: discoverable entry point to the interactive API explorer.
    NavItem {
        label: "API docs",
        path: "/api/docs",
        depth: 0,
    },
];

/// Resolve a request path to its [`DocRoute`], or `None` if no doc serves it.
pub fn resolve(path: &str) -> Option<&'static DocRoute> {
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    DOC_ROUTES.iter().find(|d| d.path == trimmed)
}

/// Map an inter-doc markdown link target to its canonical route, or `None`
/// when the basename isn't a known doc.
fn route_for_source(basename: &str) -> Option<&'static str> {
    DOC_ROUTES
        .iter()
        .find(|d| d.source == basename)
        .map(|d| d.path)
}

// ---------------------------------------------------------------------------
// Markdown → HTML (tiny CommonMark + GFM-table subset, CSP-safe)
// ---------------------------------------------------------------------------

/// Escape the five HTML-significant characters. Applied to every span of raw
/// document text so no markup (including `<script>`) is ever passed through.
fn esc(s: &str) -> String {
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

/// Rewrite a markdown link href: inter-doc `.md` links become canonical routes;
/// external URLs, in-page anchors, and absolute paths are returned unchanged.
fn rewrite_doc_link(href: &str) -> String {
    let h = href.trim();
    // Leave absolute URLs, protocol-relative, mailto, anchors, and site-absolute
    // paths untouched.
    if h.starts_with("http://")
        || h.starts_with("https://")
        || h.starts_with("//")
        || h.starts_with("mailto:")
        || h.starts_with('#')
        || h.starts_with('/')
    {
        return h.to_string();
    }

    // Split off a trailing #anchor so it can be preserved on the rewritten URL.
    let (link, anchor) = match h.split_once('#') {
        Some((l, a)) => (l, Some(a)),
        None => (h, None),
    };

    // Reduce `../some/dir/vapid-security.md` to `vapid-security.md`.
    let basename = link.rsplit('/').next().unwrap_or(link);

    if let Some(route) = route_for_source(basename) {
        match anchor {
            Some(a) => format!("{route}#{a}"),
            None => route.to_string(),
        }
    } else {
        h.to_string()
    }
}

/// Render inline markdown (code spans, links, bold, italic) within already
/// line-level text. Output is HTML-escaped and CSP-safe.
fn render_inline(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;

    while i < n {
        let c = chars[i];
        match c {
            // Inline code span: `code` — verbatim, escaped, no nested formatting.
            '`' => {
                if let Some(close) = (i + 1..n).find(|&j| chars[j] == '`') {
                    let code: String = chars[i + 1..close].iter().collect();
                    out.push_str("<code>");
                    out.push_str(&esc(&code));
                    out.push_str("</code>");
                    i = close + 1;
                } else {
                    out.push_str("&#96;");
                    i += 1;
                }
            }
            // Link: [text](href)
            '[' => {
                if let Some((text, href, next)) = parse_link(&chars, i) {
                    let safe_href = esc(&rewrite_doc_link(&href));
                    out.push_str("<a href=\"");
                    out.push_str(&safe_href);
                    // External links open safely in a new tab.
                    if href.starts_with("http://") || href.starts_with("https://") {
                        out.push_str("\" target=\"_blank\" rel=\"noopener noreferrer\">");
                    } else {
                        out.push_str("\">");
                    }
                    out.push_str(&render_inline(&text));
                    out.push_str("</a>");
                    i = next;
                } else {
                    out.push_str("&#91;");
                    i += 1;
                }
            }
            // Bold (**text**) and italic (*text* / _text_).
            '*' | '_' => {
                let (tag, marker_len) = if c == '*' && i + 1 < n && chars[i + 1] == '*' {
                    ("strong", 2)
                } else {
                    ("em", 1)
                };
                let marker: String = chars[i..i + marker_len].iter().collect();
                if let Some(close) = find_marker(&chars, i + marker_len, &marker) {
                    let inner: String = chars[i + marker_len..close].iter().collect();
                    out.push('<');
                    out.push_str(tag);
                    out.push('>');
                    out.push_str(&render_inline(&inner));
                    out.push_str("</");
                    out.push_str(tag);
                    out.push('>');
                    i = close + marker_len;
                } else {
                    out.push_str(&esc(&c.to_string()));
                    i += 1;
                }
            }
            _ => {
                out.push_str(&esc(&c.to_string()));
                i += 1;
            }
        }
    }

    out
}

/// Parse a `[text](href)` link starting at `chars[start] == '['`. Returns
/// `(text, href, index_after_link)` on success.
fn parse_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let n = chars.len();
    let text_close = (start + 1..n).find(|&j| chars[j] == ']')?;
    if text_close + 1 >= n || chars[text_close + 1] != '(' {
        return None;
    }
    let href_close = (text_close + 2..n).find(|&j| chars[j] == ')')?;
    let text: String = chars[start + 1..text_close].iter().collect();
    let href: String = chars[text_close + 2..href_close].iter().collect();
    Some((text, href, href_close + 1))
}

/// Find the next occurrence of `marker` (e.g. `**` or `*`) at or after `from`.
fn find_marker(chars: &[char], from: usize, marker: &str) -> Option<usize> {
    let m: Vec<char> = marker.chars().collect();
    let n = chars.len();
    let mut i = from;
    while i + m.len() <= n {
        if chars[i..i + m.len()] == m[..] {
            // For italic `*`, don't match the first char of a `**` bold marker.
            if m.len() == 1 && i + 1 < n && chars[i + 1] == m[0] {
                i += 1;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

/// CO-338: convert markdown to HTML after rewriting `[[key::path]]` surface
/// wikilinks to resolved deployment links. The shared render entry point for
/// content + surfaces that want live cross-universe `<a href>`s: the wikilinks
/// become `[label](url)` (via [`co::surface::rewrite_surface_links`]) and then
/// ordinary `<a href="url">label</a>` anchors through [`markdown_to_html`].
pub fn markdown_to_html_with_surfaces(md: &str, registry: &co::surface::SurfaceRegistry) -> String {
    markdown_to_html(&co::surface::rewrite_surface_links(md, registry))
}

/// Convert a markdown document to CSP-safe HTML.
pub fn markdown_to_html(md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let mut html = String::with_capacity(md.len() + 512);
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_end();

        // Blank line — skip (block boundaries are handled per-block).
        if trimmed.trim().is_empty() {
            i += 1;
            continue;
        }

        // Fenced code block.
        if let Some(rest) = trimmed.trim_start().strip_prefix("```") {
            let lang = rest.trim();
            let mut code = String::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            i += 1; // consume closing fence (if present)
            if lang.is_empty() {
                html.push_str("<pre><code>");
            } else {
                html.push_str(&format!("<pre><code class=\"language-{}\">", esc(lang)));
            }
            html.push_str(&esc(&code));
            html.push_str("</code></pre>\n");
            continue;
        }

        // Horizontal rule.
        let hr = trimmed.trim();
        if hr.len() >= 3 && (hr.chars().all(|c| c == '-') || hr.chars().all(|c| c == '*')) {
            html.push_str("<hr>\n");
            i += 1;
            continue;
        }

        // ATX heading.
        if let Some(level) = heading_level(trimmed) {
            let text = trimmed.trim_start().trim_start_matches('#').trim();
            let id = slugify(text);
            html.push_str(&format!(
                "<h{lvl} id=\"{id}\">{body}</h{lvl}>\n",
                lvl = level,
                id = id,
                body = render_inline(text)
            ));
            i += 1;
            continue;
        }

        // GFM table: header row followed by a separator row of dashes/pipes.
        if is_table_row(trimmed) && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            let mut rows: Vec<&str> = Vec::new();
            while i < lines.len() && is_table_row(lines[i].trim_end()) {
                rows.push(lines[i].trim_end());
                i += 1;
            }
            html.push_str(&render_table(&rows));
            continue;
        }

        // Blockquote (consecutive `>` lines).
        if trimmed.trim_start().starts_with('>') {
            let mut quote = String::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let content = lines[i].trim_start().trim_start_matches('>').trim_start();
                quote.push_str(content);
                quote.push('\n');
                i += 1;
            }
            html.push_str("<blockquote>");
            html.push_str(&markdown_to_html(quote.trim_end()));
            html.push_str("</blockquote>\n");
            continue;
        }

        // Unordered list.
        if is_ul_item(trimmed) {
            html.push_str("<ul>\n");
            while i < lines.len() && is_ul_item(lines[i].trim_end()) {
                let item = list_item_text(lines[i].trim_end());
                html.push_str(&format!("<li>{}</li>\n", render_inline(item)));
                i += 1;
            }
            html.push_str("</ul>\n");
            continue;
        }

        // Ordered list.
        if is_ol_item(trimmed) {
            html.push_str("<ol>\n");
            while i < lines.len() && is_ol_item(lines[i].trim_end()) {
                let item = ol_item_text(lines[i].trim_end());
                html.push_str(&format!("<li>{}</li>\n", render_inline(item)));
                i += 1;
            }
            html.push_str("</ol>\n");
            continue;
        }

        // Paragraph: gather consecutive non-blank, non-special lines.
        let mut para = String::new();
        while i < lines.len() {
            let l = lines[i].trim_end();
            if l.trim().is_empty()
                || heading_level(l).is_some()
                || is_ul_item(l)
                || is_ol_item(l)
                || l.trim_start().starts_with('>')
                || l.trim_start().starts_with("```")
                || (is_table_row(l) && i + 1 < lines.len() && is_table_separator(lines[i + 1]))
            {
                break;
            }
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(l.trim());
            i += 1;
        }
        if !para.is_empty() {
            html.push_str(&format!("<p>{}</p>\n", render_inline(&para)));
        }
    }

    html
}

fn heading_level(line: &str) -> Option<u8> {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && t.chars().nth(hashes) == Some(' ') {
        Some(hashes as u8)
    } else {
        None
    }
}

fn is_ul_item(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ")
}

fn list_item_text(line: &str) -> &str {
    let t = line.trim_start();
    t[2..].trim_start()
}

fn is_ol_item(line: &str) -> bool {
    let t = line.trim_start();
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    digits > 0 && t[digits..].starts_with(". ")
}

fn ol_item_text(line: &str) -> &str {
    let t = line.trim_start();
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    t[digits + 2..].trim_start()
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.matches('|').count() >= 2
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    if !t.starts_with('|') {
        return false;
    }
    t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) && t.contains('-')
}

fn table_cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim())
        .collect()
}

fn render_table(rows: &[&str]) -> String {
    if rows.len() < 2 {
        return String::new();
    }
    let mut out = String::from("<table>\n<thead><tr>");
    for cell in table_cells(rows[0]) {
        out.push_str(&format!("<th>{}</th>", render_inline(cell)));
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    // rows[1] is the separator; data starts at rows[2].
    for row in &rows[2..] {
        out.push_str("<tr>");
        for cell in table_cells(row) {
            out.push_str(&format!("<td>{}</td>", render_inline(cell)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n");
    out
}

/// Lowercase, dash-joined, alnum-only anchor slug for heading IDs.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

// ---------------------------------------------------------------------------
// Page shell
// ---------------------------------------------------------------------------

/// Render the navigation rail (+ mobile dropdown) for `active_path`.
fn render_nav(active_path: &str) -> String {
    let mut rail = String::from("<nav class=\"docs-rail\" aria-label=\"Documentação\">\n<ul>\n");
    // Mobile dropdown: a <select> wired by docs-viewer.js (no inline handlers).
    let mut select =
        String::from("<select class=\"docs-nav-select\" aria-label=\"Navegar documentação\">\n");
    for item in NAV {
        let active = if item.path == active_path {
            " class=\"active\""
        } else {
            ""
        };
        rail.push_str(&format!(
            "<li data-depth=\"{depth}\"><a href=\"{path}\"{active}>{label}</a></li>\n",
            depth = item.depth,
            path = item.path,
            active = active,
            label = esc(item.label),
        ));
        let sel = if item.path == active_path {
            " selected"
        } else {
            ""
        };
        let indent = if item.depth > 0 { "↳ " } else { "" };
        select.push_str(&format!(
            "<option value=\"{path}\"{sel}>{indent}{label}</option>\n",
            path = item.path,
            sel = sel,
            indent = indent,
            label = esc(item.label),
        ));
    }
    rail.push_str("</ul>\n</nav>\n");
    select.push_str("</select>\n");
    format!("{select}{rail}")
}

/// Build the full HTML page for a rendered doc body.
fn render_page(
    title: &str,
    active_path: &str,
    body_html: &str,
    status: u16,
    render_ms: i64,
) -> String {
    let nav = render_nav(active_path);
    format!(
        "<!DOCTYPE html>\n\
<html lang=\"pt-BR\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>CO · {title}</title>\n\
<link rel=\"stylesheet\" href=\"/shared/docs-viewer.css\">\n\
</head>\n\
<body class=\"docs-body\">\n\
<div id=\"docs-meta\" data-doc-path=\"{path}\" data-status=\"{status}\" data-render-ms=\"{render_ms}\" hidden></div>\n\
<header class=\"docs-header\">\n\
<a class=\"docs-home\" href=\"/seguranca\">CO · Segurança</a>\n\
<button class=\"docs-nav-toggle\" aria-label=\"Menu\" aria-expanded=\"false\">☰</button>\n\
</header>\n\
<div class=\"docs-shell\">\n\
<aside class=\"docs-sidebar\">{nav}</aside>\n\
<main class=\"docs-content\">{body}</main>\n\
</div>\n\
<footer class=\"docs-footer\">\n\
<label class=\"docs-telemetry-toggle\">\n\
<input type=\"checkbox\" id=\"docs-telemetry-optout\"> Desativar telemetria de leitura\n\
</label>\n\
</footer>\n\
<script src=\"/shared/docs-viewer.js\" defer></script>\n\
</body>\n\
</html>\n",
        title = esc(title),
        path = esc(active_path),
        status = status,
        render_ms = render_ms,
        nav = nav,
        body = body_html,
    )
}

// ---------------------------------------------------------------------------
// Telemetry
// ---------------------------------------------------------------------------

/// True when the reader has opted out via the `co_viewer_telemetry=0` cookie
/// (mirrored from `localStorage` by `docs-viewer.js`).
fn telemetry_opted_out(headers: &HeaderMap) -> bool {
    cookie_value(headers, "co_viewer_telemetry").as_deref() == Some("0")
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookies.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&format!("{name}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn visitor_token(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, "al_vid").or_else(|| cookie_value(headers, "visitante_id"))
}

/// Strip the query string from a referrer (privacy: no query params).
fn strip_query(referrer: &str) -> String {
    referrer
        .split(['?', '#'])
        .next()
        .unwrap_or(referrer)
        .to_string()
}

/// Emit a docs telemetry event synchronously (best-effort) unless opted out.
fn emit_event(
    state: &AppState,
    headers: &HeaderMap,
    event_name: &str,
    event_type: &str,
    path: &str,
    duration_ms: Option<i64>,
) {
    if telemetry_opted_out(headers) {
        return;
    }
    let referrer = headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .map(strip_query);
    let row = EventRow {
        timestamp: chrono::Utc::now().to_rfc3339(),
        visitor_token: visitor_token(headers),
        user_id: None,
        session_id: crate::telemetry::extract_session_id(headers),
        event_type: event_type.to_string(),
        event_name: event_name.to_string(),
        universe_key: None,
        path: Some(path.to_string()),
        properties: referrer.map(|r| serde_json::json!({ "referrer": r })),
        duration_ms,
        ip_hash: None,
        ua_device: None,
        ua_browser: None,
        ua_os: None,
        country: None,
        city: None,
    };
    let storage = state.core.storage.lock();
    insert_event(storage.conn(), &row);
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Serve any markdown-backed doc route. Resolves by request path.
async fn serve_doc(State(state): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    let path = uri.path();
    let Some(doc) = resolve(path) else {
        return serve_not_found(State(state), headers, uri).await;
    };

    let start = std::time::Instant::now();
    let body = markdown_to_html(doc.markdown);
    let render_ms = start.elapsed().as_millis() as i64;
    let html = render_page(doc.title, doc.path, &body, 200, render_ms);

    emit_event(
        &state,
        &headers,
        "page_view",
        "pageview",
        doc.path,
        Some(render_ms),
    );

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            // Short cache: docs are static-ish but iterate often (no hashed name).
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        html,
    )
        .into_response()
}

/// `/dependencias` → `/seguranca/dependencias`.
async fn serve_dependencias_redirect() -> Response {
    Redirect::temporary("/seguranca/dependencias").into_response()
}

/// Catch-all for unknown `/seguranca/...` paths: returns a 404 doc page and
/// emits a `404_route` telemetry event with the attempted path.
async fn serve_not_found(State(state): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    let attempted = uri.path();
    emit_event(&state, &headers, "404_route", "error", attempted, None);

    let body = format!(
        "<h1>Página não encontrada</h1>\n<p>O caminho <code>{}</code> não corresponde a \
         nenhum documento. Use a navegação ao lado.</p>\n",
        esc(attempted)
    );
    let html = render_page("Não encontrado", "/seguranca", &body, 404, 0);
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// All CO-210 doc routes. Merge into the main router *before* the `/{slug}`
/// catch-all so these literal paths take precedence.
pub fn router() -> Router<AppState> {
    let mut r = Router::new();
    for doc in DOC_ROUTES {
        r = r.route(doc.path, get(serve_doc));
    }
    r.route("/dependencias", get(serve_dependencias_redirect))
        // Catch unknown `/seguranca/...` sub-paths (literal routes above win).
        .route("/seguranca/{*rest}", get(serve_not_found))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::Request;
    use tempfile::tempdir;
    use tower::ServiceExt;

    // --- unit: route resolution ---------------------------------------------

    #[test]
    fn surface_wikilinks_render_to_resolved_anchors() {
        let reg = co::surface::SurfaceRegistry::new(vec![
            co::surface::SurfaceNode::new(
                "yggdrasil",
                None::<String>,
                Some("yggdrasil.artelonga.com.br"),
            ),
            co::surface::SurfaceNode::new("comunicacao", Some("yggdrasil"), None::<String>),
            co::surface::SurfaceNode::new("mbya", Some("comunicacao"), None::<String>),
        ]);
        let html = markdown_to_html_with_surfaces("Veja [[mbya::]] aqui.", &reg);
        assert!(
            html.contains(
                "<a href=\"https://yggdrasil.artelonga.com.br/comunicacao/mbya\" \
                 target=\"_blank\" rel=\"noopener noreferrer\">mbya::</a>"
            ),
            "surface wikilink must render to a resolved anchor; got: {html}"
        );
    }

    #[test]
    fn resolve_maps_all_nine_routes() {
        let paths = [
            "/seguranca",
            "/seguranca/dependencias",
            "/seguranca/dependencias/decisoes",
            "/seguranca/red-team",
            "/seguranca/red-team/playbook",
            "/seguranca/vapid",
            "/licensa",
            "/renderers",
            "/e2e-walkthrough",
        ];
        assert_eq!(paths.len(), 9);
        for p in paths {
            assert!(resolve(p).is_some(), "route {p} should resolve");
        }
    }

    #[test]
    fn resolve_tolerates_trailing_slash_and_rejects_unknown() {
        assert!(resolve("/seguranca/").is_some());
        assert!(resolve("/seguranca/nope").is_none());
        assert!(resolve("/").is_none());
    }

    // --- unit: markdown rendering -------------------------------------------

    #[test]
    fn renders_sample_doc_blocks() {
        let md = "# Título\n\nUm **forte** e _ênfase_ com `code`.\n\n- a\n- b\n\n1. um\n2. dois\n";
        let html = markdown_to_html(md);
        assert!(html.contains("<h1 id=\"título\">Título</h1>"));
        assert!(html.contains("<strong>forte</strong>"));
        assert!(html.contains("<em>ênfase</em>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<ul>") && html.contains("<li>a</li>"));
        assert!(html.contains("<ol>") && html.contains("<li>um</li>"));
    }

    #[test]
    fn renders_table_and_code_fence() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet x = 1;\n```\n";
        let html = markdown_to_html(md);
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>A</th>"));
        assert!(html.contains("<td>1</td>"));
        assert!(html.contains("<pre><code class=\"language-rust\">"));
        assert!(html.contains("let x = 1;"));
    }

    #[test]
    fn rendering_is_csp_safe_escapes_raw_html() {
        let md = "Texto <script>alert(1)</script> e <b>tag</b>.";
        let html = markdown_to_html(md);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<b>tag</b>"));
    }

    // --- unit: cross-link rewriting -----------------------------------------

    #[test]
    fn cross_links_rewrite_to_routes() {
        assert_eq!(rewrite_doc_link("vapid-security.md"), "/seguranca/vapid");
        assert_eq!(rewrite_doc_link("../vapid-security.md"), "/seguranca/vapid");
        assert_eq!(rewrite_doc_link("../licensa.md"), "/licensa");
        assert_eq!(
            rewrite_doc_link("SECURITY.md#portabilidade"),
            "/seguranca#portabilidade"
        );
        // External + anchors + unknown docs untouched.
        assert_eq!(
            rewrite_doc_link("https://obsidian.md/"),
            "https://obsidian.md/"
        );
        assert_eq!(rewrite_doc_link("#secao"), "#secao");
        assert_eq!(rewrite_doc_link("unknown.md"), "unknown.md");
    }

    #[test]
    fn cross_link_renders_anchor_with_rewritten_href() {
        let html = markdown_to_html("Veja [vapid](vapid-security.md).");
        assert!(html.contains("<a href=\"/seguranca/vapid\">vapid</a>"));
    }

    // --- integration: routing + telemetry -----------------------------------

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        // CO-477: share the canonical "test-jwt-secret" with every other co-web
        // test so concurrent JWT_SECRET writes can never produce a value
        // mismatch (no test removes JWT_SECRET, so identical writes are safe).
        unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
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
            bypass_rate_limit: true,
        };
        let storage = crate::storage::Storage::new(dir);
        let experiment = crate::experiment::ExperimentStore::new(dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let state = crate::server::AppState::new(crate::server::AppStateInner {
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
        });
        crate::server::build_router(state, None)
    }

    async fn get(app: &axum::Router, uri: &str) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn count_events(dir: &std::path::Path, event_name: &str) -> i64 {
        let storage = crate::storage::Storage::new(dir);
        storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM telemetry_events WHERE event_name = ?1",
                [event_name],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn all_nine_routes_serve_html_content() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        for doc in DOC_ROUTES {
            let resp = get(&app, doc.path).await;
            assert_eq!(resp.status(), StatusCode::OK, "route {} not 200", doc.path);
            let ct = resp
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap();
            assert!(ct.contains("text/html"));
            let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
                .await
                .unwrap();
            let body = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(body.contains("docs-rail"), "route {} missing nav", doc.path);
            assert!(body.contains("docs-viewer.js"));
        }
    }

    #[tokio::test]
    async fn page_view_telemetry_is_recorded() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = get(&app, "/seguranca").await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(count_events(dir.path(), "page_view"), 1);
    }

    #[tokio::test]
    async fn opt_out_cookie_suppresses_telemetry() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/licensa")
                    .header(header::COOKIE, "co_viewer_telemetry=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(count_events(dir.path(), "page_view"), 0);
    }

    #[tokio::test]
    async fn unknown_seguranca_path_404s_and_emits_404_route() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = get(&app, "/seguranca/does-not-exist").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(count_events(dir.path(), "404_route"), 1);
    }

    #[tokio::test]
    async fn dependencias_redirects() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = get(&app, "/dependencias").await;
        assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            resp.headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "/seguranca/dependencias"
        );
    }

    #[test]
    fn slow_pages_detectable_via_duration_ms() {
        // The page_render path carries a client-measured duration; verify the
        // schema + query can isolate slow (>500ms) renders.
        let dir = tempdir().unwrap();
        let storage = crate::storage::Storage::new(dir.path());
        let fast = EventRow {
            timestamp: chrono::Utc::now().to_rfc3339(),
            visitor_token: None,
            user_id: None,
            session_id: None,
            event_type: "performance".into(),
            event_name: "page_render".into(),
            universe_key: None,
            path: Some("/seguranca".into()),
            properties: None,
            duration_ms: Some(120),
            ip_hash: None,
            ua_device: None,
            ua_browser: None,
            ua_os: None,
            country: None,
            city: None,
        };
        let slow = EventRow {
            duration_ms: Some(620),
            ..fast_clone(&fast)
        };
        insert_event(storage.conn(), &fast);
        insert_event(storage.conn(), &slow);
        let slow_count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM telemetry_events \
                 WHERE event_name = 'page_render' AND duration_ms > 500",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(slow_count, 1);
    }

    // Small helper: EventRow isn't Clone, so rebuild for the slow variant.
    fn fast_clone(r: &EventRow) -> EventRow {
        EventRow {
            timestamp: r.timestamp.clone(),
            visitor_token: None,
            user_id: None,
            session_id: None,
            event_type: r.event_type.clone(),
            event_name: r.event_name.clone(),
            universe_key: None,
            path: r.path.clone(),
            properties: None,
            duration_ms: r.duration_ms,
            ip_hash: None,
            ua_device: None,
            ua_browser: None,
            ua_os: None,
            country: None,
            city: None,
        }
    }
}
