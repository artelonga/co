//! Message + edit tests (original tests 7-13, 17-22).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::tempdir;
use tower::ServiceExt;

use crate::storage::Storage;

use super::support::*;

// --- 7. POST /messages — member can post ---

#[tokio::test]
async fn test_post_message_member_ok() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner7@example.com");
    let member_id = insert_user(dir.path(), "member7@example.com");
    insert_universe(dir.path(), "uni7", &owner_id);
    add_member(dir.path(), "uni7", &member_id, "member");
    let token = make_jwt(&member_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni7/chat/rooms/general/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"Olá pessoal!"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "body: {:?}",
        body_json(resp.into_body()).await
    );
}

// --- 8. POST /messages — viewer cannot post ---

#[tokio::test]
async fn test_post_message_viewer_forbidden() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner8@example.com");
    let viewer_id = insert_user(dir.path(), "viewer8@example.com");
    insert_universe(dir.path(), "uni8", &owner_id);
    add_member(dir.path(), "uni8", &viewer_id, "viewer");
    let token = make_jwt(&viewer_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni8/chat/rooms/general/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"can I post?"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// --- 9. POST /messages — subscriber cannot post ---

#[tokio::test]
async fn test_post_message_subscriber_forbidden() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner9@example.com");
    let sub_id = insert_user(dir.path(), "sub9@example.com");
    insert_universe(dir.path(), "uni9", &owner_id);
    add_subscriber(dir.path(), "uni9", &sub_id);
    let token = make_jwt(&sub_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni9/chat/rooms/general/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"subscriber post?"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// --- 10. POST /messages — 400 on empty body ---

#[tokio::test]
async fn test_post_message_empty_body_400() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner10@example.com");
    insert_universe(dir.path(), "uni10", &owner_id);
    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni10/chat/rooms/general/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"   "}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// --- 11. POST /messages — 400 on body too long ---

#[tokio::test]
async fn test_post_message_too_long_400() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner11@example.com");
    insert_universe(dir.path(), "uni11", &owner_id);
    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let long_body = "a".repeat(4001);
    let payload = serde_json::json!({"body": long_body}).to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni11/chat/rooms/general/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// --- 12. GET /messages — pagination with has_more ---

#[tokio::test]
async fn test_list_messages_pagination() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner12@example.com");
    insert_universe(dir.path(), "uni12", &owner_id);

    // Insert 5 messages directly into storage
    {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni12", "general")
            .expect("general room");
        for i in 0..5 {
            let body = format!("message {i}");
            storage
                .post_chat_message(&room.id, &owner_id, &body, None, None)
                .unwrap();
        }
    }

    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    // Request only 3 → has_more = true
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/uni12/chat/rooms/general/messages?limit=3")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    assert_eq!(json["messages"].as_array().unwrap().len(), 3);
    assert_eq!(json["has_more"], true);

    // Request all 10 → has_more = false
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/uni12/chat/rooms/general/messages?limit=10")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    assert_eq!(json["messages"].as_array().unwrap().len(), 5);
    assert_eq!(json["has_more"], false);
}

// --- 13. Soft-deleted message returns tombstone ---

#[tokio::test]
async fn test_soft_deleted_message_tombstone() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner13@example.com");
    insert_universe(dir.path(), "uni13", &owner_id);

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni13", "general")
            .expect("general room");
        let mid = storage
            .post_chat_message(&room.id, &owner_id, "original text", None, None)
            .unwrap();
        // Soft-delete it
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
                .method("GET")
                .uri("/api/v1/universes/uni13/chat/rooms/general/messages")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    let msgs = json["messages"].as_array().unwrap();
    let msg = msgs.iter().find(|m| m["id"] == msg_id).expect("msg found");
    assert_eq!(msg["body"], "[mensagem removida]");
    assert!(msg["deleted_at"].is_string());
}
