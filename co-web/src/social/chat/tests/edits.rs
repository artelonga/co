//! Message rate-limit + edit tests (original tests 17-22).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::tempdir;
use tower::ServiceExt;

use crate::storage::Storage;

use super::support::*;

// --- 17. POST /messages — rate limit 429 ---

#[tokio::test]
async fn test_post_message_rate_limit() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner17@example.com");
    insert_universe(dir.path(), "uni17", &owner_id);
    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    // Send 21 messages — the 21st should hit rate limit
    let mut last_status = StatusCode::CREATED;
    for _ in 0..21 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni17/chat/rooms/general/messages")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"body":"ping"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        last_status = resp.status();
    }

    assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);
}

// =========================================================================
// CO-196 — message edit
// =========================================================================

// --- 18. PATCH within edit window by author → 200 ---

#[tokio::test]
async fn test_edit_within_window_by_author_200() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner18@example.com");
    insert_universe(dir.path(), "uni18", &owner_id);

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni18", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &owner_id, "original", None, None)
            .unwrap()
    };

    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!(
                    "/api/v1/universes/uni18/chat/rooms/general/messages/{msg_id}"
                ))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"edited text"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    assert_eq!(json["body"], "edited text");
    assert!(json["edited_at"].is_string());
}

// --- 19. PATCH outside edit window → 403 ---

#[tokio::test]
async fn test_edit_outside_window_403() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner19@example.com");
    insert_universe(dir.path(), "uni19", &owner_id);

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni19", "general")
            .expect("room");
        let mid = storage
            .post_chat_message(&room.id, &owner_id, "old msg", None, None)
            .unwrap();
        let old_ts = (chrono::Utc::now() - chrono::Duration::minutes(16)).to_rfc3339();
        storage
            .conn()
            .execute(
                "UPDATE chat_messages SET created_at = ?1 WHERE id = ?2",
                rusqlite::params![old_ts, mid],
            )
            .unwrap();
        mid
    };

    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!(
                    "/api/v1/universes/uni19/chat/rooms/general/messages/{msg_id}"
                ))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"try edit"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// --- 20. PATCH by non-author → 403 ---

#[tokio::test]
async fn test_edit_by_non_author_403() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner20@example.com");
    let member_id = insert_user(dir.path(), "member20@example.com");
    insert_universe(dir.path(), "uni20", &owner_id);
    add_member(dir.path(), "uni20", &member_id, "member");

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni20", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &owner_id, "owner msg", None, None)
            .unwrap()
    };

    let token = make_jwt(&member_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!(
                    "/api/v1/universes/uni20/chat/rooms/general/messages/{msg_id}"
                ))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"hacked edit"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// --- 21. PATCH on deleted message → 410 ---

#[tokio::test]
async fn test_edit_deleted_message_410() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner21@example.com");
    insert_universe(dir.path(), "uni21", &owner_id);

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni21", "general")
            .expect("room");
        let mid = storage
            .post_chat_message(&room.id, &owner_id, "msg", None, None)
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
                .method("PATCH")
                .uri(format!(
                    "/api/v1/universes/uni21/chat/rooms/general/messages/{msg_id}"
                ))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"edit deleted"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::GONE);
}

// --- 22. PATCH with empty body → 400 ---

#[tokio::test]
async fn test_edit_empty_body_400() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner22@example.com");
    insert_universe(dir.path(), "uni22", &owner_id);

    let msg_id = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage
            .get_chat_room_by_slug("uni22", "general")
            .expect("room");
        storage
            .post_chat_message(&room.id, &owner_id, "msg", None, None)
            .unwrap()
    };

    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!(
                    "/api/v1/universes/uni22/chat/rooms/general/messages/{msg_id}"
                ))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"   "}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
