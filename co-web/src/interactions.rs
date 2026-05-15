//! Content interactions — registry-driven RPC contract.
//!
//! `co-web/e2e/interactions/registry.yaml` is a canonical OpenAPI 3.1
//! document. The server embeds it via `include_str!`, parses to a
//! `serde_json::Value` once at startup, and serves it under
//! `/api/v1/interactions/`:
//!
//!   GET  /api/v1/interactions/                   list operations
//!   GET  /api/v1/interactions/openapi.json       full OpenAPI doc
//!   GET  /api/v1/interactions/{operationId}      single operation block
//!   POST /api/v1/interactions/{operationId}      execute (reserved, 501)
//!
//! Operations are discovered by walking `paths × methods` in the
//! OpenAPI doc — exactly the structure REST clients already
//! understand. No flat operation list, no separate registry shape.

use std::sync::OnceLock;

use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use serde_json::{Value, json};

use crate::server::AppState;

const REGISTRY_YAML: &str = include_str!("../e2e/interactions/registry.yaml");

const HTTP_METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head", "options"];

static OPENAPI: OnceLock<Value> = OnceLock::new();

fn openapi() -> &'static Value {
    OPENAPI.get_or_init(|| {
        let yaml: serde_yaml::Value = serde_yaml::from_str(REGISTRY_YAML)
            .expect("registry.yaml must be valid YAML");
        serde_json::to_value(yaml).expect("YAML → JSON conversion")
    })
}

/// Each (path, method) pair in the OpenAPI doc that has an operationId.
#[derive(Debug, Clone)]
struct OperationRef<'a> {
    path: &'a str,
    method: &'static str,
    operation_id: &'a str,
    block: &'a Value,
}

fn iter_operations(doc: &Value) -> Vec<OperationRef<'_>> {
    let mut out = Vec::new();
    let Some(paths) = doc.get("paths").and_then(|p| p.as_object()) else {
        return out;
    };
    for (path, item) in paths {
        let Some(item_obj) = item.as_object() else {
            continue;
        };
        for &method in HTTP_METHODS {
            let Some(op) = item_obj.get(method) else {
                continue;
            };
            let Some(op_id) = op.get("operationId").and_then(|v| v.as_str()) else {
                continue;
            };
            out.push(OperationRef {
                path,
                method,
                operation_id: op_id,
                block: op,
            });
        }
    }
    out
}

// --- handlers --------------------------------------------------------------

async fn openapi_json() -> impl IntoResponse {
    Json(openapi().clone())
}

async fn list_operations() -> impl IntoResponse {
    let doc = openapi();
    let info = doc.get("info").cloned().unwrap_or(Value::Null);
    let ops: Vec<Value> = iter_operations(doc)
        .into_iter()
        .map(|o| {
            json!({
                "operationId": o.operation_id,
                "method": o.method.to_uppercase(),
                "path": o.path,
                "summary": o.block.get("summary"),
                "tags": o.block.get("tags"),
                "xSafety": o.block.get("x-safety"),
            })
        })
        .collect();
    Json(json!({
        "info": info,
        "openapi": doc.get("openapi"),
        "operations": ops,
    }))
}

async fn get_operation(Path(op_id): Path<String>) -> impl IntoResponse {
    let doc = openapi();
    for op in iter_operations(doc) {
        if op.operation_id == op_id {
            return (
                StatusCode::OK,
                Json(json!({
                    "operationId": op.operation_id,
                    "method": op.method.to_uppercase(),
                    "path": op.path,
                    "operation": op.block,
                })),
            )
                .into_response();
        }
    }
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": "not_found", "operationId": op_id})),
    )
        .into_response()
}

async fn run_operation(Path(op_id): Path<String>) -> impl IntoResponse {
    let doc = openapi();
    if iter_operations(doc).iter().any(|o| o.operation_id == op_id) {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "error": "runtime_not_implemented",
                "message": "Interaction runtime is reserved. Call the actual entry endpoint directly, or run the Playwright spec.",
                "operationId": op_id,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "operationId": op_id})),
        )
            .into_response()
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_operations))
        .route("/openapi.json", get(openapi_json))
        .route("/{op}", get(get_operation).post(run_operation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_parses() {
        let doc = openapi();
        assert_eq!(doc.get("openapi").and_then(|v| v.as_str()), Some("3.1.0"));
        assert!(doc.get("paths").is_some());
    }

    #[test]
    fn four_operations_under_entry_resource() {
        let ops: Vec<_> = iter_operations(openapi())
            .iter()
            .map(|o| o.operation_id.to_string())
            .collect();
        for required in ["getEntry", "putEntry", "deleteEntry", "listEntries"] {
            assert!(
                ops.contains(&required.to_string()),
                "{required} should be present",
            );
        }
        assert_eq!(ops.len(), 4, "exactly four operations on entry resource");
    }

    #[test]
    fn every_operation_has_pre_and_post_conditions() {
        for op in iter_operations(openapi()) {
            assert!(
                op.block.get("x-preconditions").is_some(),
                "{} missing x-preconditions",
                op.operation_id
            );
            assert!(
                op.block.get("x-postconditions").is_some(),
                "{} missing x-postconditions",
                op.operation_id
            );
            assert!(
                op.block.get("x-safety").is_some(),
                "{} missing x-safety",
                op.operation_id
            );
        }
    }

    #[test]
    fn universe_is_a_path_parameter() {
        // The interaction should work for all universes — universe is
        // a path parameter, not a top-level field. Verify each path
        // template includes {universe}.
        let doc = openapi();
        let paths = doc
            .get("paths")
            .and_then(|p| p.as_object())
            .expect("paths");
        for path_template in paths.keys() {
            assert!(
                path_template.contains("{universe}"),
                "path `{path_template}` should template {{universe}}",
            );
        }
    }
}
