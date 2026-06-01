#![allow(unused_imports)]
use super::*;

use axum::Extension;

use crate::server::subdomain_routing::SubdomainUniverse;

/// CO-323: inject `<script>window.__CO_SUBDOMAIN_UNIVERSE__='<slug>';</script>`
/// just before `</body>` so the SPA boot sequence can detect single-universe mode
/// and hide the multi-universe sidebar.
fn inject_subdomain_script(html: Vec<u8>, universe_key: &str) -> Vec<u8> {
    // Sanitize: universe keys are already validated as [a-z0-9-] in the middleware,
    // but be explicit here to prevent any XSS via unexpected extension values.
    let safe_key: String = universe_key
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect();
    if safe_key.is_empty() {
        return html;
    }
    let script = format!("<script>window.__CO_SUBDOMAIN_UNIVERSE__='{safe_key}';</script>");
    // Find the last occurrence of `</body>` (case-insensitive) and insert before it.
    let needle = b"</body>";
    if let Some(pos) = html
        .windows(needle.len())
        .rposition(|w| w.eq_ignore_ascii_case(needle))
    {
        let mut out = Vec::with_capacity(html.len() + script.len());
        out.extend_from_slice(&html[..pos]);
        out.extend_from_slice(script.as_bytes());
        out.extend_from_slice(&html[pos..]);
        out
    } else {
        // Fallback: append if no </body> found (shouldn't happen with our template).
        let mut out = html;
        out.extend_from_slice(script.as_bytes());
        out
    }
}

/// Returns `true` when the URL path looks like a static asset request
/// (has a file extension somewhere, or starts with a known asset prefix).
/// Used to keep `/{slug}` from swallowing `/style.css`, `/shared/foo.css`,
/// `/variants/a/app.js`, etc. after the v1.43 URL refactor.
pub(super) fn looks_like_static_asset(path: &str) -> bool {
    const ASSET_PREFIXES: &[&str] = &["shared/", "variants/", "pdfjs/", "games/", "icons/"];
    let stripped = path.trim_start_matches('/');
    if ASSET_PREFIXES.iter().any(|p| stripped.starts_with(p)) {
        return true;
    }
    // Last path segment contains a `.` → treat as filename.
    stripped
        .rsplit('/')
        .next()
        .map(|seg| seg.contains('.'))
        .unwrap_or(false)
}

/// Serve `index.html` for `/`, `/{slug}`, and deep SPA paths. The hub
/// (`/`) and any universe view (`/{slug}/...`) all return the same SPA
/// shell; the client-side router resolves the path.
///
/// After the v1.43 URL refactor `/{slug}` matches every top-level path,
/// including filename-like ones (`/style.css`, `/app.js`, `/manifest.json`)
/// and asset-prefix paths (`/shared/production.css`). [`looks_like_static_asset`]
/// detects these and delegates to the static-file handler.
pub(super) async fn serve_co_index(
    headers: HeaderMap,
    uri: Uri,
    State(state): State<AppState>,
    subdomain: Option<Extension<SubdomainUniverse>>,
) -> Response {
    if looks_like_static_asset(uri.path()) {
        return serve_variant_file(headers, uri, State(state)).await;
    }

    // Pretty URLs: `/<slug>` → 307 → canonical universe path.
    // Template pages (sobre, termos, …) → `/template/<slug>`.
    // co/public pages (seguranca, licensa, …) → `/co/public/<slug>`.
    // Falls through to the regular SPA serve when the slug isn't on either list.
    if let Some(slug) = uri.path().strip_prefix('/').and_then(|p| {
        if p.is_empty() || p.contains('/') {
            None
        } else {
            Some(p)
        }
    }) && let Some(target) = crate::pretty_urls::slug_redirect_target(slug)
    {
        return axum::response::Redirect::temporary(&target).into_response();
    }

    let variant = extract_variant(&headers, &state.core.config);
    let embed_path = format!("variants/{}/index.html", variant);
    let fs_path = std::path::Path::new(&state.core.config.static_dir).join(&embed_path);

    // CO-121: assign + expose home_v2_layout for each visitor (fire-and-forget).
    // CO-97: prefer al_vid (apex marketing cookie) for unified visitor identity.
    let visitor_token = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            let mut visitante = None;
            for part in cookies.split(';') {
                let part = part.trim();
                if let Some(v) = part.strip_prefix("al_vid=") {
                    return Some(v.to_string());
                }
                if let Some(v) = part.strip_prefix("visitante_id=") {
                    visitante = Some(v.to_string());
                }
            }
            visitante
        })
        .unwrap_or_else(|| nanoid::nanoid!(24));
    {
        let state_clone = state.clone();
        let uid = visitor_token.clone();
        tokio::spawn(async move {
            let storage = state_clone.core.storage.lock();
            if let Ok(Some(ab_variant)) = crate::ab::assign(storage.conn(), &uid, "home_v2_layout")
            {
                let _ =
                    crate::ab::expose(storage.conn(), &uid, "home_v2_layout", &ab_variant, None);
            }
        });
    }

    if let Some(contents) = resolve_asset(&embed_path, Some(&fs_path)) {
        // CO-323: inject subdomain universe script before serving the SPA shell.
        let contents = if let Some(Extension(sub)) = subdomain {
            inject_subdomain_script(contents, &sub.0)
        } else {
            contents
        };

        let mut response = (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
            contents,
        )
            .into_response();

        if extract_lang_cookie(&headers).is_none() {
            let lang = detect_lang_from_accept(&headers);
            if let Ok(v) =
                format!("co_lang={}; Path=/; SameSite=Lax; Max-Age=31536000", lang).parse()
            {
                response.headers_mut().append(header::SET_COOKIE, v);
            }
        }

        return response;
    }

    (StatusCode::NOT_FOUND, "Not found").into_response()
}

/// Universe-existence check separated from entry lookup so `serve_deep_link` can
/// distinguish "not a universe slug at all" (SPA route like `/entrar/`) from
/// "universe exists but entry doesn't" (true 404).
fn universe_exists(state: &AppState, universe_slug: &str) -> bool {
    let storage = state.core.storage.lock();
    storage.get_universe(universe_slug).is_some()
}

/// Check whether any of the SPA's candidate entry paths exist for a given subpath.
///
/// Mirrors `maybeOpenEntryFromUrl` candidates in `app.js`. Returns `false` when
/// the universe does not exist, when the per-universe connection cannot be acquired,
/// or when none of the candidate paths are present in the entry index.
///
/// CO-264: also checks well-known file aliases (CHANGELOG.md for `changelog`,
/// README.md for `readme`, LICENSE.md for `license`) and folder-level index.md
/// when the subpath ends with a trailing slash.
fn entry_exists_for_subpath(state: &AppState, universe_slug: &str, subpath: &str) -> bool {
    let uc = {
        let storage = state.core.storage.lock();
        if storage.get_universe(universe_slug).is_none() {
            return false;
        }
        storage.universe_conn(universe_slug)
    };
    let Ok(uc_guard) = uc.lock() else {
        return false;
    };
    let index = crate::entry_index::EntryIndex::new(&uc_guard);

    let mut candidates: Vec<String> = vec![
        format!("{subpath}.md"),
        subpath.to_string(),
        format!("content/{subpath}.md"),
        format!("content/{subpath}"),
    ];

    // CO-264: folder-level index.md for trailing-slash paths (e.g. `public/`).
    if subpath.ends_with('/') {
        candidates.push(format!("{subpath}index.md"));
        candidates.push(format!("{subpath}index"));
    }

    // CO-264: well-known file aliases (case-insensitive subpath match).
    let lower = subpath.to_lowercase();
    match lower.as_str() {
        "changelog" => {
            candidates.push("CHANGELOG.md".to_string());
            candidates.push("changelog.md".to_string());
        }
        "readme" => {
            candidates.push("README.md".to_string());
            candidates.push("readme.md".to_string());
        }
        "license" => {
            candidates.push("LICENSE.md".to_string());
            candidates.push("LICENSE".to_string());
        }
        _ => {}
    }

    candidates
        .iter()
        .any(|p| index.get(universe_slug, p).ok().flatten().is_some())
}

/// GET `/{universe}/{*subpath}` — SPA deep-link handler (CO-232).
///
/// Returns HTTP 200 when the entry exists (or is indeterminate) and HTTP 404
/// when it is provably absent. In both cases the SPA shell is served so the
/// client-side router can render the appropriate view. The SPA's own
/// `maybeOpenEntryFromUrl()` also shows a 404 view on failure, so status code
/// and rendered view always agree.
pub(super) async fn serve_deep_link(
    Path((universe_slug, subpath)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    State(state): State<AppState>,
    subdomain: Option<Extension<SubdomainUniverse>>,
) -> Response {
    // 2.13.3 hotfix: serve_deep_link previously served the SPA shell unconditionally
    // for `/{slug}/{*subpath}`. That matches `/variants/a/app.js`, `/shared/style.css`,
    // `/pdfjs/build/pdf.js` etc. — all static assets — returning HTML instead of JS/CSS.
    // The browser then fails to parse the SPA bundle (MIME error), no JS runs, the
    // app shows "Carregando..." forever. Delegate to the static-file handler when
    // the path looks like an asset (matches `looks_like_static_asset`).
    if looks_like_static_asset(uri.path()) {
        return serve_variant_file(headers, uri, State(state)).await;
    }

    let variant = extract_variant(&headers, &state.core.config);
    let embed_path = format!("variants/{}/index.html", variant);
    let fs_path = std::path::Path::new(&state.core.config.static_dir).join(&embed_path);

    let Some(contents) = resolve_asset(&embed_path, Some(&fs_path)) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };

    // CO-323: inject subdomain universe script when request came via a subdomain.
    let contents = if let Some(Extension(sub)) = subdomain {
        inject_subdomain_script(contents, &sub.0)
    } else {
        contents
    };

    // CO-232 hotfix: when the slug is not a known universe, treat the URL as a
    // pure SPA route (e.g. `/entrar/`, `/sobre/`, `/termos/`) and serve 200 so
    // the client-side router renders the page. Only return 404 when the
    // universe exists but the entry within it does not.
    let status = if !universe_exists(&state, &universe_slug)
        || entry_exists_for_subpath(&state, &universe_slug, &subpath)
    {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };

    let mut response = (
        status,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        contents,
    )
        .into_response();

    if extract_lang_cookie(&headers).is_none() {
        let lang = detect_lang_from_accept(&headers);
        if let Ok(v) = format!("co_lang={}; Path=/; SameSite=Lax; Max-Age=31536000", lang).parse() {
            response.headers_mut().append(header::SET_COOKIE, v);
        }
    }

    response
}

/// CO-150: Serve the asset browser page at `/{slug}/assets`.
pub(super) async fn serve_assets_page(State(state): State<AppState>) -> Response {
    let embed_path = "shared/assets.html";
    let fs_path = std::path::Path::new(&state.core.config.static_dir).join(embed_path);
    if let Some(contents) = resolve_asset(embed_path, Some(&fs_path)) {
        return (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
            contents,
        )
            .into_response();
    }
    (StatusCode::NOT_FOUND, "Asset browser page not found").into_response()
}

/// GET /settings/sync — shows API token. Paths live in co-universes.yaml locally.
/// Auth required.
pub(super) async fn serve_sync_settings(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    let user_id = match crate::auth::resolve_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::FOUND,
                [(header::LOCATION, HeaderValue::from_static("/"))],
                (),
            )
                .into_response();
        }
    };

    let token = {
        let storage = state.core.storage.lock();
        match storage.create_api_token(&user_id, "co-sync") {
            Ok(t) => t.token.unwrap_or_default(),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Token error: {e}"),
                )
                    .into_response();
            }
        }
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>CO — Sync</title>
<style>
  body{{font-family:system-ui,sans-serif;max-width:560px;margin:60px auto;padding:0 24px;color:#111;}}
  h1{{font-size:1.4rem;margin-bottom:6px;}}
  p{{color:#555;line-height:1.6;margin:0 0 16px;}}
  .cmd{{background:#f4f4f5;border:1px solid #e5e7eb;border-radius:8px;padding:14px 16px;
        font-family:monospace;font-size:.95rem;position:relative;user-select:all;}}
  .copy{{position:absolute;top:8px;right:8px;background:#fff;border:1px solid #ddd;
         border-radius:4px;padding:4px 12px;cursor:pointer;font-size:.8rem;}}
  .copy:active{{background:#eee;}}
  .step{{font-size:.8rem;color:#888;margin:4px 0 12px;}}
  hr{{border:none;border-top:1px solid #eee;margin:28px 0;}}
  code{{background:#f4f4f5;padding:2px 6px;border-radius:4px;font-size:.85rem;}}
  a{{color:#2563eb;}}
</style>
</head>
<body>
<h1>Sync</h1>
<p>Universe paths are declared in <code>co-universes.yaml</code> — no configuration needed here.
Copy your token and run once from the CO repo directory.</p>

<div class="cmd" id="cmd">co-sync {token}<button class="copy" onclick="copy()">Copy</button></div>
<p class="step">Reads <code>co-universes.yaml</code>, syncs all universes, watches for changes.</p>

<hr>
<p style="font-size:.85rem;">
  <strong>Install</strong> (from the CO repo):<br>
  <code>cargo install --path co-agent --bin co-sync</code>
</p>
<p style="font-size:.85rem;">
  <strong>Auto-start at login</strong><br>
  Add <code>co-sync</code> to System Settings → General → Login Items.
</p>

<script>
function copy() {{
  navigator.clipboard.writeText(document.getElementById('cmd').textContent.replace('Copy','').trim()).then(() => {{
    document.querySelector('.copy').textContent = 'Copied!';
    setTimeout(() => document.querySelector('.copy').textContent = 'Copy', 2000);
  }});
}}
</script>
</body>
</html>"#,
        token = token,
    );
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        html,
    )
        .into_response()
}

pub(super) async fn serve_variant_file(
    headers: HeaderMap,
    uri: Uri,
    State(state): State<AppState>,
) -> Response {
    let variant = extract_variant(&headers, &state.core.config);
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // CO-160: Serve PDF.js viewer bundle at /pdfjs/...
    if path.starts_with("pdfjs/") {
        let fs_path = std::path::Path::new(&state.core.config.static_dir).join(path);
        if let Some(contents) = resolve_asset(path, Some(&fs_path)) {
            let content_type = guess_content_type(path);
            let cache_header = cache_control_for(path);
            return (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                    (header::CACHE_CONTROL, cache_header),
                ],
                contents,
            )
                .into_response();
        }
    }

    // 2.13.4: paths that already start with `variants/` (e.g. `variants/a/app.js`
    // from the SPA's relative module imports) need to be served as-is, not
    // re-prefixed with another `variants/<variant>/`.
    if path.starts_with("variants/") {
        let fs_path = std::path::Path::new(&state.core.config.static_dir).join(path);
        if let Some(contents) = resolve_asset(path, Some(&fs_path)) {
            let content_type = guess_content_type(path);
            let cache_header = cache_control_for(path);
            return (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                    (header::CACHE_CONTROL, cache_header),
                ],
                contents,
            )
                .into_response();
        }
    }

    // Try shared/ first (for experiment.js, experiment.css)
    if path.starts_with("shared/") || path == "manifest.json" || path == "sw.js" {
        let embed_path = if path.starts_with("shared/") {
            path.to_string()
        } else {
            format!("shared/{}", path)
        };
        let fs_path = std::path::Path::new(&state.core.config.static_dir).join(&embed_path);
        if let Some(contents) = resolve_asset(&embed_path, Some(&fs_path)) {
            let content_type = guess_content_type(path);
            let cache_header = cache_control_for(path);
            return (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                    (header::CACHE_CONTROL, cache_header),
                ],
                contents,
            )
                .into_response();
        }
    }

    // Try variant-specific file
    let embed_path = format!("variants/{}/{}", variant, path);
    let fs_path = std::path::Path::new(&state.core.config.static_dir).join(&embed_path);

    if let Some(contents) = resolve_asset(&embed_path, Some(&fs_path)) {
        let content_type = guess_content_type(path);
        let cache_header = cache_control_for(path);
        let mut response = (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                (header::CACHE_CONTROL, cache_header),
            ],
            contents,
        )
            .into_response();

        // Set variant cookie if not present
        if extract_variant(&headers, &state.core.config) == state.core.config.default_variant {
            response.headers_mut().insert(
                header::SET_COOKIE,
                format!(
                    "co_variant={}; Path=/; SameSite=Lax; HttpOnly; Max-Age=31536000",
                    variant
                )
                .parse()
                .unwrap(),
            );
        }

        // Set participant cookie if not present
        if extract_participant(&headers).is_none() {
            let participant_id = uuid::Uuid::new_v4().to_string();
            response.headers_mut().append(
                header::SET_COOKIE,
                format!(
                    "co_participant={}; Path=/; SameSite=Lax; HttpOnly; Max-Age=31536000",
                    participant_id
                )
                .parse()
                .unwrap(),
            );
        }

        // Set co_lang cookie for HTML responses when not already set.
        // co_lang cookie overrides Accept-Language on subsequent loads.
        if (path.ends_with(".html") || path == "index.html")
            && extract_lang_cookie(&headers).is_none()
        {
            let lang = detect_lang_from_accept(&headers);
            response.headers_mut().append(
                header::SET_COOKIE,
                format!("co_lang={}; Path=/; SameSite=Lax; Max-Age=31536000", lang)
                    .parse()
                    .unwrap(),
            );
        }

        return response;
    }

    (StatusCode::NOT_FOUND, "Not found").into_response()
}

pub(super) fn guess_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

pub(super) fn cache_control_for(path: &str) -> HeaderValue {
    match path.rsplit('.').next() {
        Some("html") => HeaderValue::from_static("no-cache"),
        Some("css") | Some("js") => HeaderValue::from_static("public, max-age=60, must-revalidate"),
        Some("png") | Some("svg") | Some("ico") | Some("woff2") => {
            HeaderValue::from_static("public, max-age=31536000, immutable")
        }
        _ => HeaderValue::from_static("no-cache"),
    }
}
