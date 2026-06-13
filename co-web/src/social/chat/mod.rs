//! CO-219 — Chat module: REST routes + WebSocket handler.
//!
//! ## Sub-modules
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | `permissions` | `resolve_role`, `can_read`, `can_post`, `can_manage_rooms`, `lock_storage` |
//! | `routes` | `chat_router()` — wires HTTP routes to handlers |
//! | `rooms` | Room CRUD handlers + types |
//! | `members` | `list_room_members_handler` |
//! | `messages` | Message handlers + types |
//! | `ws` | WebSocket upgrade handler + `ChatEvent` |

pub mod admin;
pub mod members;
pub mod messages;
pub mod permissions;
pub mod rooms;
pub mod routes;
pub mod ws;

// Re-exports consumed by server/state.rs and server/router.rs
pub use admin::chat_admin_router;
pub use routes::chat_router;
pub use ws::ChatEvent;
pub use ws::chat_ws_handler;

#[cfg(test)]
mod tests;
