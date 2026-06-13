//! CO-204 — admin-only chat origin telemetry.
//!
//! `GET /api/v1/admin/chat/origin-breakdown` — aggregated message counts
//! grouped by the sender's `origin_universe_key`. Admin-gated via the
//! [`AdminUser`](crate::auth::extractors::AdminUser) extractor (JWT `tier == "admin"`).

use axum::{Router, response::IntoResponse, routing::get};
use serde::Serialize;

use crate::auth::extractors::AdminUser;
use crate::server::AppState;

use super::permissions::lock_storage;

#[derive(Debug, Serialize)]
pub struct OriginCount {
    /// `None` ⇒ unknown / pre-CO-204 (serialized as JSON `null`).
    pub universe_key: Option<String>,
    pub message_count: i64,
}

#[derive(Debug, Serialize)]
pub struct OriginBreakdownResponse {
    pub origins: Vec<OriginCount>,
    pub total_messages: i64,
}

/// GET /api/v1/admin/chat/origin-breakdown
async fn origin_breakdown_handler(
    _admin: AdminUser,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    let storage = lock_storage(&state);
    let rows = storage.chat_origin_breakdown();

    let total_messages: i64 = rows.iter().map(|(_, n)| n).sum();
    let origins = rows
        .into_iter()
        .map(|(universe_key, message_count)| OriginCount {
            universe_key,
            message_count,
        })
        .collect();

    axum::Json(OriginBreakdownResponse {
        origins,
        total_messages,
    })
}

/// Router for the admin chat telemetry surface, nested at `/api/v1/admin`.
pub fn chat_admin_router() -> Router<AppState> {
    Router::new().route("/chat/origin-breakdown", get(origin_breakdown_handler))
}
