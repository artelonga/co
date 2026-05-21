//! Room CRUD handlers and related types.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::auth::extractors::AuthedUser;
use crate::error::AppError;
use crate::server::AppState;

use super::permissions::{can_manage_rooms, can_read, lock_storage, resolve_role};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateRoomRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListRoomsResponse {
    pub rooms: Vec<crate::storage::chat::ChatRoom>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/:slug/chat/rooms
pub async fn list_rooms_handler(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user: AuthedUser,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let include_archived = params
        .get("include_archived")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let storage = lock_storage(&state);

    storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let role = resolve_role(&storage, &slug, &user.user_id)
        .ok_or_else(|| AppError::Forbidden("Chat is only available to universe members".into()))?;

    if !can_read(&role) {
        return Err(AppError::Forbidden("Insufficient role to read chat".into()));
    }

    let rooms = storage.list_chat_rooms(&slug, include_archived);
    Ok(axum::Json(ListRoomsResponse { rooms }))
}

/// POST /api/v1/universes/:slug/chat/rooms
pub async fn create_room_handler(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user: AuthedUser,
    axum::Json(body): axum::Json<CreateRoomRequest>,
) -> Result<impl IntoResponse, AppError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("Room name cannot be empty".into()));
    }
    if name.len() > 100 {
        return Err(AppError::BadRequest(
            "Room name must be 100 characters or fewer".into(),
        ));
    }

    let storage = lock_storage(&state);

    storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let role = resolve_role(&storage, &slug, &user.user_id)
        .ok_or_else(|| AppError::Forbidden("Chat is only available to universe members".into()))?;

    if !can_manage_rooms(&role) {
        return Err(AppError::Forbidden(
            "Only universe owners and admins can create rooms".into(),
        ));
    }

    let room = storage
        .create_chat_room(&slug, &name, body.description.as_deref(), &user.user_id)
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                AppError::Conflict("A room with this name already exists in this universe".into())
            } else {
                AppError::Internal(e.to_string())
            }
        })?;

    Ok((StatusCode::CREATED, axum::Json(room)))
}
