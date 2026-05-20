//! Permission helpers shared by REST routes and WebSocket handler.

/// Lock storage from AppState.
pub fn lock_storage(
    state: &crate::server::AppState,
) -> parking_lot::MutexGuard<'_, crate::storage::Storage> {
    state.storage.lock()
}

/// Resolve the effective role of a caller for a universe.
/// Returns `Some(role)` if the caller is a member, `None` if anonymous / non-member.
/// Subscribers (in the `subscriptions` table but not `universe_members`) get role "subscriber".
pub fn resolve_role(
    storage: &crate::storage::Storage,
    universe_key: &str,
    user_id: &str,
) -> Option<String> {
    if let Some(role) = storage.universe_member_role(universe_key, user_id) {
        return Some(role);
    }
    if storage.is_subscribed(user_id, universe_key) {
        return Some("subscriber".to_string());
    }
    None
}

/// True when the role allows reading (list rooms, read messages).
pub fn can_read(role: &str) -> bool {
    matches!(role, "owner" | "admin" | "member" | "viewer" | "subscriber")
}

/// True when the role allows posting messages (member or above, not viewer/subscriber).
pub fn can_post(role: &str) -> bool {
    matches!(role, "owner" | "admin" | "member")
}

/// True when the role allows creating or archiving rooms.
pub fn can_manage_rooms(role: &str) -> bool {
    matches!(role, "owner" | "admin")
}
