//! Delete + broadcast tests (original tests 23-31).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::tempdir;
use tokio::sync::broadcast;
use tokio::time::Duration;
use tower::ServiceExt;

use crate::storage::Storage;

use super::support::*;
use crate::chat::ChatEvent;

// --- 23. DELETE by author → 200 ---

#[tokio::test]
async fn test_delete_by_author_200() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner23@example.com");
    insert_universe(dir.path(), "uni23", &owner_id);

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni23", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &owner_id, "my msg", None)
            .unwrap()
    };

    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/universes/uni23/chat/rooms/general/messages/{msg_id}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    assert!(json["deleted_at"].is_string());
}

// --- 24. DELETE by owner of other member's msg → 200 ---

#[tokio::test]
async fn test_delete_by_owner_of_other_msg_200() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner24@example.com");
    let member_id = insert_user(dir.path(), "member24@example.com");
    insert_universe(dir.path(), "uni24", &owner_id);
    add_member(dir.path(), "uni24", &member_id, "member");

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni24", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &member_id, "member msg", None)
            .unwrap()
    };

    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/universes/uni24/chat/rooms/general/messages/{msg_id}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// --- 25. DELETE by admin of other member's msg → 200 ---

#[tokio::test]
async fn test_delete_by_admin_of_other_msg_200() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner25@example.com");
    let admin_id = insert_user(dir.path(), "admin25@example.com");
    let member_id = insert_user(dir.path(), "member25@example.com");
    insert_universe(dir.path(), "uni25", &owner_id);
    add_member(dir.path(), "uni25", &admin_id, "admin");
    add_member(dir.path(), "uni25", &member_id, "member");

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni25", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &member_id, "member msg", None)
            .unwrap()
    };

    let token = make_jwt(&admin_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/universes/uni25/chat/rooms/general/messages/{msg_id}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// --- 26. DELETE by member of someone else's msg → 403 ---

#[tokio::test]
async fn test_delete_by_member_of_other_msg_403() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner26@example.com");
    let member_id = insert_user(dir.path(), "member26@example.com");
    insert_universe(dir.path(), "uni26", &owner_id);
    add_member(dir.path(), "uni26", &member_id, "member");

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni26", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &owner_id, "owner msg", None)
            .unwrap()
    };

    let token = make_jwt(&member_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/universes/uni26/chat/rooms/general/messages/{msg_id}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// --- 27. DELETE by viewer of someone else's msg → 403 ---

#[tokio::test]
async fn test_delete_by_viewer_of_other_msg_403() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner27@example.com");
    let viewer_id = insert_user(dir.path(), "viewer27@example.com");
    insert_universe(dir.path(), "uni27", &owner_id);
    add_member(dir.path(), "uni27", &viewer_id, "viewer");

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni27", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &owner_id, "owner msg", None)
            .unwrap()
    };

    let token = make_jwt(&viewer_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/universes/uni27/chat/rooms/general/messages/{msg_id}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// --- 28. DELETE on already-deleted message → 410 ---

#[tokio::test]
async fn test_delete_already_deleted_410() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner28@example.com");
    insert_universe(dir.path(), "uni28", &owner_id);

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni28", "general")
            .expect("room");
        let mid = storage
            .post_chat_message(&room.id, &owner_id, "msg", None)
            .unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "UPDATE chat_messages SET deleted_at = ?1 WHERE id = ?2",
                rusqlite::params![now, mid],
            )
            .unwrap();
        mid
    };

    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/universes/uni28/chat/rooms/general/messages/{msg_id}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::GONE);
}

// --- 29. PATCH broadcasts message.edited ---

#[tokio::test]
async fn test_edit_broadcasts_message_edited_event() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner29@example.com");
    insert_universe(dir.path(), "uni29", &owner_id);

    let state = make_state_inner(dir.path());

    let (msg_id, room_id) = {
        let storage = state.storage.lock();
        let room = storage
            .get_chat_room_by_slug("uni29", "general")
            .expect("room");
        let mid = storage
            .post_chat_message(&room.id, &owner_id, "original", None)
            .unwrap();
        (mid, room.id.clone())
    };

    let (tx, mut rx) = broadcast::channel::<ChatEvent>(64);
    {
        let mut map = state.chat_rooms_broadcast.lock().unwrap();
        map.insert(room_id.clone(), tx);
    }

    let app = crate::server::build_router(Arc::clone(&state), None);
    let token = make_jwt(&owner_id);

    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!(
                    "/api/v1/universes/uni29/chat/rooms/general/messages/{msg_id}"
                ))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"edited!"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let evt = tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("timeout waiting for broadcast")
        .unwrap();

    assert!(
        matches!(evt, ChatEvent::MessageEdited { .. }),
        "expected MessageEdited, got {evt:?}"
    );
}

// --- 30. DELETE broadcasts message.deleted ---

#[tokio::test]
async fn test_delete_broadcasts_message_deleted_event() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner30@example.com");
    insert_universe(dir.path(), "uni30", &owner_id);

    let state = make_state_inner(dir.path());

    let (msg_id, room_id) = {
        let storage = state.storage.lock();
        let room = storage
            .get_chat_room_by_slug("uni30", "general")
            .expect("room");
        let mid = storage
            .post_chat_message(&room.id, &owner_id, "to delete", None)
            .unwrap();
        (mid, room.id.clone())
    };

    let (tx, mut rx) = broadcast::channel::<ChatEvent>(64);
    {
        let mut map = state.chat_rooms_broadcast.lock().unwrap();
        map.insert(room_id.clone(), tx);
    }

    let app = crate::server::build_router(Arc::clone(&state), None);
    let token = make_jwt(&owner_id);

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/universes/uni30/chat/rooms/general/messages/{msg_id}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let evt = tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("timeout waiting for broadcast")
        .unwrap();

    assert!(
        matches!(evt, ChatEvent::MessageDeleted { .. }),
        "expected MessageDeleted, got {evt:?}"
    );
}

// --- 31. GET messages returns tombstone after DELETE ---

#[tokio::test]
async fn test_get_messages_returns_tombstone_for_deleted() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner31@example.com");
    insert_universe(dir.path(), "uni31", &owner_id);

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni31", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &owner_id, "secret msg", None)
            .unwrap()
    };

    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let del_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/universes/uni31/chat/rooms/general/messages/{msg_id}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del_resp.status(), StatusCode::OK);

    let get_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/uni31/chat/rooms/general/messages")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);
    let json = body_json(get_resp.into_body()).await;
    let msgs = json["messages"].as_array().unwrap();
    let msg = msgs
        .iter()
        .find(|m| m["id"] == msg_id)
        .expect("msg in list");
    assert_eq!(msg["body"], "[mensagem removida]");
    assert!(msg["deleted_at"].is_string());
}
