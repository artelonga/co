//! Room member listing handler.

use axum::extract::{Path, State};

use crate::auth::extractors::AuthedUser;
use crate::error::AppError;
use crate::server::AppState;

use super::permissions::{can_read, lock_storage, resolve_role};

/// GET /api/v1/universes/:slug/chat/rooms/:room_slug/members (CO-209)
pub async fn list_room_members_handler(
    State(state): State<AppState>,
    Path((slug, room_slug)): Path<(String, String)>,
    user: AuthedUser,
) -> Result<axum::Json<Vec<crate::storage::chat::ChatRoomMemberInfo>>, AppError> {
    let storage = lock_storage(&state);

    let room = if slug == "dm" {
        storage
            .get_dm_room_by_slug(&room_slug)
            .ok_or_else(|| AppError::NotFound(format!("DM room '{room_slug}' not found")))?
    } else {
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        let role = resolve_role(&storage, &slug, &user.user_id)
            .ok_or_else(|| AppError::Forbidden("Not a member of this universe".into()))?;
        if !can_read(&role) {
            return Err(AppError::Forbidden("Insufficient role to read chat".into()));
        }
        storage
            .get_chat_room_by_slug(&slug, &room_slug)
            .ok_or_else(|| AppError::NotFound(format!("Room '{room_slug}' not found")))?
    };

    // For DMs, verify caller is a participant.
    if slug == "dm" && !storage.is_dm_member(&room.id, &user.user_id) {
        return Err(AppError::Forbidden("Not a member of this DM".into()));
    }

    let members = storage.list_chat_room_members(&room.id);
    Ok(axum::Json(members))
}
