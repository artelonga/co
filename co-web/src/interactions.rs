//! E2E Interactions — registry-driven RPC endpoints.
//!
//! The single source of truth is `co-web/e2e/interactions/registry.yaml`.
//! At build time the file is embedded via `include_str!`, parsed once
//! per startup, and exposed under `/api/v1/interactions/`:
//!
//!   GET  /api/v1/interactions/                  list interactions
//!   GET  /api/v1/interactions/openapi.json      derived OpenAPI 3.1
//!   GET  /api/v1/interactions/{operationId}     single interaction spec
//!
//! The executable runtime (POST handler that actually runs an
//! interaction server-side) is a follow-up — this module ships the
//! contract first so agents (co-auto, claude-code) can discover and
//! reason about interactions today.

use std::sync::OnceLock;

use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::server::AppState;

const REGISTRY_YAML: &str = include_str!("../e2e/interactions/registry.yaml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionDocument {
    pub openapi: String,
    pub info: DocInfo,
    pub interactions: Vec<Interaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocInfo {
    pub title: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    pub id: String,
    #[serde(rename = "operationId")]
    pub operation_id: String,
    pub title: String,
    pub spec: String,
    pub universe: String,
    pub parameters: Value,
    #[serde(default)]
    pub preconditions: Vec<Condition>,
    #[serde(default)]
    pub postconditions: Vec<Condition>,
    #[serde(default)]
    pub produces: Vec<Value>,
    #[serde(default)]
    pub auth: Value,
    #[serde(default)]
    pub safety: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub id: String,
    pub rule: String,
}

static REGISTRY: OnceLock<InteractionDocument> = OnceLock::new();

/// Parse `registry.yaml` once and cache it. Returns None only if the
/// embedded YAML is malformed — which would be a build-time bug, not
/// a runtime condition, so callers can `unwrap` after the first call.
fn registry() -> &'static InteractionDocument {
    REGISTRY.get_or_init(|| {
        serde_yaml::from_str(REGISTRY_YAML).expect("registry.yaml must be valid")
    })
}

/// Build an OpenAPI 3.1 paths object from the interaction list.
///
/// Each interaction maps to one path `/{operationId}` under the
/// `/api/v1/interactions` prefix. Today only the GET (fetch spec)
/// is documented; the POST that actually executes the interaction
/// is reserved.
fn build_openapi() -> Value {
    let reg = registry();
    let mut paths = serde_json::Map::new();
    for interaction in &reg.interactions {
        let path_key = format!("/{}", interaction.operation_id);
        let post_request = serde_json::json!({
            "operationId": interaction.operation_id,
            "summary": interaction.title,
            "description": format!(
                "Executes interaction `{}`. Reads `{}` and writes the produced entries listed in the registry. Pre/postconditions are checked server-side; the response includes the criterion-level pass/fail.",
                interaction.id, interaction.spec,
            ),
            "tags": interaction.tags,
            "requestBody": {
                "required": false,
                "content": {
                    "application/json": {
                        "schema": interaction.parameters,
                    }
                }
            },
            "responses": {
                "200": {
                    "description": "Interaction executed (regardless of pass/fail — see criteria[]).",
                    "content": {
                        "application/json": {
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "operationId": {"type": "string"},
                                    "criteria": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "id": {"type": "string"},
                                                "rule": {"type": "string"},
                                                "passed": {"type": "boolean"},
                                                "evidence": {"type": "string"}
                                            }
                                        }
                                    },
                                    "produced": {
                                        "type": "array",
                                        "items": {"type": "string"}
                                    }
                                }
                            }
                        }
                    }
                },
                "501": {
                    "description": "Runtime not implemented — only the contract is served today."
                }
            }
        });
        let get_spec = serde_json::json!({
            "operationId": format!("get_{}", interaction.operation_id),
            "summary": format!("Fetch spec for {}", interaction.operation_id),
            "tags": interaction.tags,
            "responses": {
                "200": {
                    "description": "Interaction spec (matches one entry in registry.yaml)."
                }
            }
        });
        paths.insert(
            path_key,
            serde_json::json!({
                "get": get_spec,
                "post": post_request,
            }),
        );
    }
    json!({
        "openapi": reg.openapi,
        "info": reg.info,
        "servers": [{"url": "/api/v1/interactions"}],
        "paths": paths,
    })
}

// --- handlers ---

async fn list_interactions() -> impl IntoResponse {
    let reg = registry();
    Json(json!({
        "openapi": reg.openapi,
        "info": reg.info,
        "interactions": reg.interactions.iter().map(|i| json!({
            "id": i.id,
            "operationId": i.operation_id,
            "title": i.title,
            "universe": i.universe,
            "tags": i.tags,
            "safety": i.safety,
        })).collect::<Vec<_>>(),
    }))
}

async fn openapi_json() -> impl IntoResponse {
    Json(build_openapi())
}

async fn get_interaction(Path(op): Path<String>) -> impl IntoResponse {
    let reg = registry();
    if let Some(i) = reg.interactions.iter().find(|i| i.operation_id == op) {
        (StatusCode::OK, Json(serde_json::to_value(i).unwrap())).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "operationId": op})),
        )
            .into_response()
    }
}

async fn run_interaction(Path(op): Path<String>) -> impl IntoResponse {
    let reg = registry();
    if reg.interactions.iter().any(|i| i.operation_id == op) {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "error": "runtime_not_implemented",
                "message": "Interaction runtime is reserved. Execute via Playwright today: `npx playwright test e2e/interactions/<spec>` with CO_TEST_USER_EMAIL/PASSWORD set.",
                "operationId": op,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "operationId": op})),
        )
            .into_response()
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_interactions))
        .route("/openapi.json", get(openapi_json))
        .route("/{op}", get(get_interaction).post(run_interaction))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_parses() {
        let r = registry();
        assert_eq!(r.openapi, "3.1.0");
        assert!(
            !r.interactions.is_empty(),
            "registry should have at least one interaction"
        );
    }

    #[test]
    fn first_interaction_has_expected_shape() {
        let r = registry();
        let first = &r.interactions[0];
        assert_eq!(first.id, "01");
        assert_eq!(first.operation_id, "artelongaSwitchSocialToProfiles");
        assert_eq!(first.universe, "artelonga");
        assert!(!first.preconditions.is_empty());
        assert!(!first.postconditions.is_empty());
        assert_eq!(first.safety, "snapshot-restore");
    }

    #[test]
    fn operation_ids_are_unique() {
        let r = registry();
        let mut seen = std::collections::HashSet::new();
        for i in &r.interactions {
            assert!(
                seen.insert(i.operation_id.clone()),
                "duplicate operationId: {}",
                i.operation_id
            );
        }
    }

    #[test]
    fn build_openapi_produces_path_per_interaction() {
        let openapi = build_openapi();
        let paths = openapi
            .get("paths")
            .and_then(|p| p.as_object())
            .expect("paths object");
        let count = registry().interactions.len();
        assert_eq!(paths.len(), count);
    }
}
