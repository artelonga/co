//! OpenAPI spec routes — `/api/openapi.json` and `/api/docs`
//!
//! Serves the formal v1 Universe Content API specification as machine-readable
//! JSON and as a human-readable Swagger UI page.

use axum::{
    Router,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};

// Embed the spec at compile time so the binary is self-contained.
//
// CO-452: serve the **catalog-generated** spec (`co-web/openapi.yaml`) — the
// single source of truth checked by `npm run openapi:check`. This avoids drift
// between two hand-maintained specs (the older `docs/api/openapi.yaml` is no
// longer what `/api/openapi.json` returns).
const OPENAPI_YAML: &str = include_str!("../../../openapi.yaml");

/// API version surfaced via the `X-API-Version` response header (CO-452).
const API_VERSION: &str = "v1";

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Serve the OpenAPI 3.1 spec as JSON.
///
/// Adds `X-API-Version: v1` so discovery clients can pin the major version
/// without parsing the body.
pub async fn openapi_json() -> Response {
    // Convert YAML → JSON once per request. The result is small (<50 KB)
    // and parse is cheap; no caching needed.
    let value: serde_json::Value = match serde_yaml::from_str(OPENAPI_YAML) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("Failed to parse embedded openapi.yaml: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(
                    serde_json::json!({"error": "spec_parse_error", "message": e.to_string()}),
                ),
            )
                .into_response();
        }
    };
    (
        [(
            header::HeaderName::from_static("x-api-version"),
            header::HeaderValue::from_static(API_VERSION),
        )],
        axum::Json(value),
    )
        .into_response()
}

/// Serve Swagger UI — loads the spec from `/api/openapi.json`.
///
/// CO-452: the UI runtime is **vendored** under `co-web/static/shared/swagger/`
/// and loaded same-origin (no external CDN) so the page is CSP-safe and has no
/// third-party runtime dependency.
pub async fn api_docs() -> Html<&'static str> {
    Html(SWAGGER_UI_HTML)
}

static SWAGGER_UI_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>CO Web API — Docs</title>
  <link rel="stylesheet" href="/shared/swagger/swagger-ui.css" />
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="/shared/swagger/swagger-ui-bundle.js"></script>
  <script src="/shared/swagger/swagger-ui-standalone-preset.js"></script>
  <script>
    SwaggerUIBundle({
      url: "/api/openapi.json",
      dom_id: "#swagger-ui",
      deepLinking: true,
      presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset],
      plugins: [SwaggerUIBundle.plugins.DownloadUrl],
      layout: "StandaloneLayout",
      tryItOutEnabled: true,
      displayRequestDuration: true
    });
  </script>
</body>
</html>
"##;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/openapi.json", get(openapi_json))
        .route("/docs", get(api_docs))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    // -----------------------------------------------------------------------
    // Spec validation tests (no server needed)
    //
    // CO-452: the served spec is the catalog-generated `co-web/openapi.yaml`
    // (source of truth verified by `npm run openapi:check`), so these assert
    // its shape — not the older hand-maintained `docs/api/openapi.yaml`.
    // -----------------------------------------------------------------------

    #[test]
    fn spec_yaml_is_valid() {
        let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(OPENAPI_YAML);
        assert!(
            parsed.is_ok(),
            "openapi.yaml is not valid YAML: {:?}",
            parsed.err()
        );
    }

    #[test]
    fn spec_is_openapi_31() {
        let doc: serde_json::Value =
            serde_yaml::from_str(OPENAPI_YAML).expect("spec must be valid YAML");
        assert_eq!(
            doc["openapi"].as_str(),
            Some("3.1.0"),
            "spec must declare openapi: 3.1.0"
        );
    }

    #[test]
    fn spec_has_info_block() {
        let doc: serde_json::Value =
            serde_yaml::from_str(OPENAPI_YAML).expect("spec must be valid YAML");
        assert!(
            doc["info"]["title"].is_string(),
            "info.title must be a string"
        );
        assert!(
            doc["info"]["version"].is_string(),
            "info.version must be a string"
        );
    }

    #[test]
    fn spec_documents_universe_endpoints() {
        let doc: serde_json::Value =
            serde_yaml::from_str(OPENAPI_YAML).expect("spec must be valid YAML");
        let paths = doc["paths"].as_object().expect("paths must be an object");

        let required = [
            "/api/v1/universes",
            "/api/v1/universes/public",
            "/api/v1/universes/{slug}",
            "/api/v1/universes/{slug}/entries",
            "/api/v1/universes/{slug}/entries/tags",
            "/api/v1/universes/{slug}/entries/tree",
        ];
        for path in required {
            assert!(
                paths.contains_key(path),
                "spec is missing required path: {path}"
            );
        }
    }

    #[test]
    fn spec_documents_vault_endpoints() {
        let doc: serde_json::Value =
            serde_yaml::from_str(OPENAPI_YAML).expect("spec must be valid YAML");
        let paths = doc["paths"].as_object().expect("paths must be an object");

        assert!(
            paths.contains_key("/api/v1/universes/{slug}/vault/"),
            "spec is missing the vault listing path"
        );
    }

    #[test]
    fn spec_documents_auth_endpoints() {
        let doc: serde_json::Value =
            serde_yaml::from_str(OPENAPI_YAML).expect("spec must be valid YAML");
        let paths = doc["paths"].as_object().expect("paths must be an object");

        let required = [
            "/api/v1/auth/login",
            "/api/v1/auth/verify",
            "/api/v1/auth/me",
            "/api/v1/auth/logout",
            "/api/v1/auth/token",
        ];
        for path in required {
            assert!(
                paths.contains_key(path),
                "spec is missing required auth path: {path}"
            );
        }
    }

    #[test]
    fn spec_has_shared_schemas() {
        let doc: serde_json::Value =
            serde_yaml::from_str(OPENAPI_YAML).expect("spec must be valid YAML");
        let schemas = doc["components"]["schemas"]
            .as_object()
            .expect("components.schemas must be an object");

        // The catalog spec ships shared schemas via openapi-components.yaml.
        for name in ["Error", "Task"] {
            assert!(
                schemas.contains_key(name),
                "components.schemas is missing: {name}"
            );
        }
    }

    #[test]
    fn spec_has_security_schemes() {
        let doc: serde_json::Value =
            serde_yaml::from_str(OPENAPI_YAML).expect("spec must be valid YAML");
        let schemes = doc["components"]["securitySchemes"]
            .as_object()
            .expect("components.securitySchemes must be an object");

        assert!(
            schemes.contains_key("sessionCookie"),
            "missing sessionCookie scheme"
        );
        assert!(schemes.contains_key("apiToken"), "missing apiToken scheme");
    }

    // -----------------------------------------------------------------------
    // HTTP endpoint tests (in-process server)
    // -----------------------------------------------------------------------

    fn build_test_app() -> axum::Router {
        Router::new().nest("/api", super::router::<()>())
    }

    async fn body_bytes(body: Body) -> Vec<u8> {
        body.collect().await.unwrap().to_bytes().to_vec()
    }

    #[tokio::test]
    async fn get_openapi_json_returns_200_with_json() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("application/json"), "content-type: {ct}");

        let body = body_bytes(resp.into_body()).await;
        let doc: serde_json::Value =
            serde_json::from_slice(&body).expect("response must be valid JSON");
        // `openapi: 3...` (3.1.0 today).
        assert!(
            doc["openapi"].as_str().unwrap_or("").starts_with("3."),
            "spec must declare openapi 3.x"
        );
    }

    #[tokio::test]
    async fn get_openapi_json_sets_x_api_version_header() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("x-api-version")
                .and_then(|v| v.to_str().ok()),
            Some("v1"),
            "openapi.json must set X-API-Version: v1"
        );
    }

    #[tokio::test]
    async fn get_openapi_json_contains_universe_paths() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_bytes(resp.into_body()).await;
        let doc: serde_json::Value =
            serde_json::from_slice(&body).expect("response must be valid JSON");
        let paths = doc["paths"].as_object().expect("paths must be present");

        assert!(paths.contains_key("/api/v1/universes"));
        assert!(paths.contains_key("/api/v1/universes/{slug}"));
        assert!(paths.contains_key("/api/v1/universes/{slug}/entries"));
        assert!(paths.contains_key("/api/v1/universes/{slug}/vault/"));
    }

    #[tokio::test]
    async fn get_api_docs_returns_html_with_vendored_assets() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/docs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("text/html"), "content-type: {ct}");

        let body = body_bytes(resp.into_body()).await;
        let html = String::from_utf8(body).expect("HTML must be valid UTF-8");
        assert!(html.contains("swagger-ui"), "Swagger UI not found in HTML");
        assert!(
            html.contains("/api/openapi.json"),
            "spec URL not found in HTML"
        );
        // CO-452: the UI runtime is vendored same-origin, never a CDN.
        assert!(
            html.contains("/shared/swagger/swagger-ui-bundle.js"),
            "Swagger UI bundle must be loaded from the vendored same-origin path"
        );
        assert!(
            !html.contains("unpkg.com") && !html.contains("//cdn"),
            "Swagger UI must not reference an external CDN"
        );
    }
}
