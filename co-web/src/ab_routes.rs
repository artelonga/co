//! Admin routes for A/B flag management — CO-121.
//!
//! POST /api/v1/admin/flags — create a new flag
//! PUT  /api/v1/admin/flags/{key} — toggle enabled/disabled
//! GET  /api/v1/admin/flags — list all flags

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};

use crate::ab::{CreateFlagRequest, ToggleFlagRequest};
use crate::github_auth::GitHubAdmin;
use crate::server::AppState;

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/flags", post(create_flag_handler))
        .route("/flags", get(list_flags_handler))
        .route("/flags/{key}", put(toggle_flag_handler))
        .layer(axum::middleware::from_fn(
            crate::github_auth::require_github_admin,
        ))
}

async fn create_flag_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Json(body): Json<CreateFlagRequest>,
) -> impl IntoResponse {
    if body.flag_key.is_empty() || body.flag_key.len() > 128 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "flag_key must be 1–128 chars"})),
        )
            .into_response();
    }

    let storage = state.storage.lock();
    match crate::ab::create_flag(storage.conn(), &body) {
        Ok(true) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"flag_key": body.flag_key, "created": true})),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "flag already exists"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_flags_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
) -> impl IntoResponse {
    let storage = state.storage.lock();
    match crate::ab::list_flags(storage.conn()) {
        Ok(flags) => Json(flags).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn toggle_flag_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Path(key): Path<String>,
    Json(body): Json<ToggleFlagRequest>,
) -> impl IntoResponse {
    let storage = state.storage.lock();
    match crate::ab::toggle_flag(storage.conn(), &key, body.enabled) {
        Ok(true) => {
            Json(serde_json::json!({"flag_key": key, "enabled": body.enabled})).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "flag not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
