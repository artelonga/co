#![allow(unused_imports)]
use super::*;

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
) -> Response {
    if looks_like_static_asset(uri.path()) {
        return serve_variant_file(headers, uri, State(state)).await;
    }

    let variant = extract_variant(&headers, &state.config);
    let embed_path = format!("variants/{}/index.html", variant);
    let fs_path = std::path::Path::new(&state.config.static_dir).join(&embed_path);

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
        let state_clone = Arc::clone(&state);
        let uid = visitor_token.clone();
        tokio::spawn(async move {
            let storage = state_clone.storage.lock();
            if let Ok(Some(ab_variant)) = crate::ab::assign(storage.conn(), &uid, "home_v2_layout")
            {
                let _ =
                    crate::ab::expose(storage.conn(), &uid, "home_v2_layout", &ab_variant, None);
            }
        });
    }

    if let Some(contents) = resolve_asset(&embed_path, Some(&fs_path)) {
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

/// CO-150: Serve the asset browser page at `/{slug}/assets`.
pub(super) async fn serve_assets_page(State(state): State<AppState>) -> Response {
    let embed_path = "shared/assets.html";
    let fs_path = std::path::Path::new(&state.config.static_dir).join(embed_path);
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
        let storage = state.storage.lock();
        match storage.create_api_token(&user_id, "co-sync") {
            Ok(t) => t.token,
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
    let variant = extract_variant(&headers, &state.config);
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // CO-160: Serve PDF.js viewer bundle at /pdfjs/...
    if path.starts_with("pdfjs/") {
        let fs_path = std::path::Path::new(&state.config.static_dir).join(path);
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
        let fs_path = std::path::Path::new(&state.config.static_dir).join(&embed_path);
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
    let fs_path = std::path::Path::new(&state.config.static_dir).join(&embed_path);

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
        if extract_variant(&headers, &state.config) == state.config.default_variant {
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
