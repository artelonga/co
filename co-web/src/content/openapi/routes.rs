//! OpenAPI spec routes — `/api/openapi.json` and `/api/docs`
//!
//! Serves the formal v1 Universe Content API specification as machine-readable
//! JSON and as a human-readable Swagger UI page.

use axum::{
    Router,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};

// Embed the spec at compile time so the binary is self-contained.
const OPENAPI_YAML: &str = include_str!("../../../../docs/api/openapi.yaml");

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Serve the OpenAPI 3.1 spec as JSON.
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
    axum::Json(value).into_response()
}

/// Serve Swagger UI — loads the spec from `/api/openapi.json`.
pub async fn api_docs() -> Html<&'static str> {
    Html(SWAGGER_UI_HTML)
}

static SWAGGER_UI_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>CO Universe Content API</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css" />
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>
    SwaggerUIBundle({
      url: "/api/openapi.json",
      dom_id: "#swagger-ui",
      deepLinking: true,
      presets: [SwaggerUIBundle.presets.apis, SwaggerUIBundle.SwaggerUIStandalonePreset],
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
            "/api/v1/universes/{slug}/entries/{path}",
            "/api/v1/universes/{slug}/manifest",
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

        let required = [
            "/api/v1/universes/{slug}/vault/",
            "/api/v1/universes/{slug}/vault/tags",
            "/api/v1/universes/{slug}/vault/{path}",
        ];
        for path in required {
            assert!(
                paths.contains_key(path),
                "spec is missing required vault path: {path}"
            );
        }

        // vault/{path} must have GET, PUT, DELETE
        let vault_path = &doc["paths"]["/api/v1/universes/{slug}/vault/{path}"];
        assert!(
            vault_path["get"].is_object(),
            "vault/{{path}} must have GET"
        );
        assert!(
            vault_path["put"].is_object(),
            "vault/{{path}} must have PUT"
        );
        assert!(
            vault_path["delete"].is_object(),
            "vault/{{path}} must have DELETE"
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

        let required = [
            "Universe",
            "Entry",
            "EntryList",
            "TagCount",
            "TreeNode",
            "VaultFile",
            "VaultFileInfo",
            "VaultStat",
            "User",
            "Session",
            "ApiToken",
            "Error",
        ];
        for name in required {
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
            schemes.contains_key("BearerAuth"),
            "missing BearerAuth scheme"
        );
        assert!(
            schemes.contains_key("SessionAuth"),
            "missing SessionAuth scheme"
        );
    }

    #[test]
    fn spec_documents_common_error_responses() {
        let doc: serde_json::Value =
            serde_yaml::from_str(OPENAPI_YAML).expect("spec must be valid YAML");
        let responses = doc["components"]["responses"]
            .as_object()
            .expect("components.responses must be an object");

        let required = [
            "BadRequest",
            "Unauthorized",
            "Forbidden",
            "NotFound",
            "Conflict",
            "TooManyRequests",
            "InternalServerError",
        ];
        for name in required {
            assert!(
                responses.contains_key(name),
                "components.responses is missing: {name}"
            );
        }
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
        assert_eq!(doc["openapi"].as_str(), Some("3.1.0"));
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
    async fn get_api_docs_returns_html() {
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
    }
}
