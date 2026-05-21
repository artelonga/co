//! Chat router: wires HTTP routes to handlers.

use axum::Router;
use axum::routing::{get, patch};

use crate::server::AppState;

use super::members::list_room_members_handler;
use super::messages::{
    delete_message_handler, edit_message_handler, list_messages_handler, post_message_handler,
};
use super::rooms::{create_room_handler, list_rooms_handler};

pub fn chat_router() -> Router<AppState> {
    Router::new()
        .route(
            "/{slug}/chat/rooms",
            get(list_rooms_handler).post(create_room_handler),
        )
        .route(
            "/{slug}/chat/rooms/{room_slug}/members",
            get(list_room_members_handler),
        )
        .route(
            "/{slug}/chat/rooms/{room_slug}/messages",
            get(list_messages_handler).post(post_message_handler),
        )
        .route(
            "/{slug}/chat/rooms/{room_slug}/messages/{msg_id}",
            patch(edit_message_handler).delete(delete_message_handler),
        )
}
