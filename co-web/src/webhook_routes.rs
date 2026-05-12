//! Admin API for outbound webhook management. CO-168.
//!
//! All routes require GitHub admin auth (via `require_github_admin` middleware,
//! applied at registration in server.rs).
//!
//! ```text
//! POST   /api/v1/gestao/webhooks                     register (returns secret once)
//! GET    /api/v1/gestao/webhooks                     list (secret redacted)
//! PUT    /api/v1/gestao/webhooks/:id                 update url / events / enabled
//! DELETE /api/v1/gestao/webhooks/:id                 delete + cascade notifications
//! GET    /api/v1/gestao/webhooks/:id/deliveries      last 100 delivery rows
//! ```

use axum::Router;
use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::github_auth::{self, GitHubAdmin};
use crate::server::AppState;
use crate::webhook;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub url: String,
    #[serde(default = "default_events")]
    pub events: Vec<String>,
}

fn default_events() -> Vec<String> {
    vec!["*".to_string()]
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub id: String,
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/webhooks", post(register_handler).get(list_handler))
        .route("/webhooks/{id}", put(update_handler).delete(delete_handler))
        .route("/webhooks/{id}/deliveries", get(deliveries_handler))
        .layer(middleware::from_fn(github_auth::require_github_admin))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn register_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Json(body): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.url.is_empty() {
        return Err(AppError::BadRequest("url is required".into()));
    }
    if !body.url.starts_with("http://") && !body.url.starts_with("https://") {
        return Err(AppError::BadRequest(
            "url must start with http:// or https://".into(),
        ));
    }

    let storage = state.storage.lock();

    let wh = webhook::create_webhook(storage.conn(), &body.url, &body.events)?;

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            id: wh.id,
            url: wh.url,
            secret: wh.secret.unwrap_or_default(),
            events: wh.events,
            enabled: wh.enabled,
            created_at: wh.created_at,
        }),
    ))
}

async fn list_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
) -> Result<Json<Vec<webhook::Webhook>>, AppError> {
    let storage = state.storage.lock();

    Ok(Json(webhook::list_webhooks(storage.conn())?))
}

async fn update_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Path(id): Path<String>,
    Json(body): Json<UpdateRequest>,
) -> Result<StatusCode, AppError> {
    if let Some(ref url) = body.url
        && !url.starts_with("http://")
        && !url.starts_with("https://")
    {
        return Err(AppError::BadRequest(
            "url must start with http:// or https://".into(),
        ));
    }

    let storage = state.storage.lock();

    webhook::update_webhook(
        storage.conn(),
        &id,
        body.url.as_deref(),
        body.events.as_deref(),
        body.enabled,
    )?;

    Ok(StatusCode::NO_CONTENT)
}

async fn delete_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let storage = state.storage.lock();

    webhook::delete_webhook(storage.conn(), &id)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn deliveries_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Path(id): Path<String>,
) -> Result<Json<Vec<webhook::Notification>>, AppError> {
    let storage = state.storage.lock();

    Ok(Json(webhook::list_deliveries(storage.conn(), &id)?))
}
