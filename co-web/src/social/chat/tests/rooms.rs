//! Room tests (original tests 1-6, 14-16).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::tempdir;
use tower::ServiceExt;

use crate::storage::Storage;

use super::support::*;

// --- 1. GET /rooms — 401 for unauthenticated ---

#[tokio::test]
async fn test_list_rooms_unauthenticated() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner@example.com");
    insert_universe(dir.path(), "uni", &owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/uni/chat/rooms")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- 2. GET /rooms — 403 for non-member ---

#[tokio::test]
async fn test_list_rooms_non_member_forbidden() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner2@example.com");
    let outsider_id = insert_user(dir.path(), "outsider@example.com");
    insert_universe(dir.path(), "uni2", &owner_id);
    let token = make_jwt(&outsider_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/uni2/chat/rooms")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// --- 3. GET /rooms — 200 with general room for member ---

#[tokio::test]
async fn test_list_rooms_member_sees_general() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner3@example.com");
    insert_universe(dir.path(), "uni3", &owner_id);
    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/uni3/chat/rooms")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;
    let rooms = json["rooms"].as_array().expect("rooms array");
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0]["slug"], "general");
    assert_eq!(rooms[0]["is_default"], true);
}

// --- 4. POST /rooms — owner can create a room ---

#[tokio::test]
async fn test_create_room_owner_ok() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner4@example.com");
    insert_universe(dir.path(), "uni4", &owner_id);
    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni4/chat/rooms")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"name":"Random","description":"Off-topic"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp.into_body()).await;
    assert_eq!(json["slug"], "random");
}

// --- 5. POST /rooms — regular member cannot create ---

#[tokio::test]
async fn test_create_room_member_forbidden() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner5@example.com");
    let member_id = insert_user(dir.path(), "member5@example.com");
    insert_universe(dir.path(), "uni5", &owner_id);
    add_member(dir.path(), "uni5", &member_id, "member");
    let token = make_jwt(&member_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni5/chat/rooms")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"name":"nope"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// --- 6. POST /rooms — 409 on slug collision ---

#[tokio::test]
async fn test_create_room_slug_collision_409() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner6@example.com");
    insert_universe(dir.path(), "uni6", &owner_id);
    let token = make_jwt(&owner_id);
    let app = build_test_router(dir.path());

    // First creation
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni6/chat/rooms")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"name":"Clash"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Same name → slug collision
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni6/chat/rooms")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"name":"Clash"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

// --- 14. POST /rooms — admin can create (not just owner) ---

#[tokio::test]
async fn test_create_room_admin_ok() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner14@example.com");
    let admin_id = insert_user(dir.path(), "admin14@example.com");
    insert_universe(dir.path(), "uni14", &owner_id);
    add_member(dir.path(), "uni14", &admin_id, "admin");
    let token = make_jwt(&admin_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/uni14/chat/rooms")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"name":"Admin Room"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
}

// --- 15. backfill_default_rooms is idempotent ---

#[tokio::test]
async fn test_backfill_default_rooms_idempotent() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path().to_str().unwrap());
    let owner_id = "usr_test_owner";
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at) \
             VALUES (?1, 'o@x.com', 'o', 'player', ?2)",
            rusqlite::params![owner_id, now],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT INTO universes (key, name, description, owner_id, created_at, visibility) \
             VALUES ('bf-uni', 'BF', '', ?1, ?2, 'private')",
            rusqlite::params![owner_id, now],
        )
        .unwrap();

    // First run: seeds the new universe (and any others that lack a room).
    let n1 = storage.backfill_default_rooms();
    assert!(n1 >= 1, "first run must insert at least 1 room (got {n1})");

    // Second run: everything already seeded → no-op.
    let n2 = storage.backfill_default_rooms();
    assert_eq!(n2, 0, "second run must be a no-op");
}

// --- 16. GET /rooms — subscriber can read rooms ---

#[tokio::test]
async fn test_subscriber_can_read_rooms() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner16@example.com");
    let sub_id = insert_user(dir.path(), "sub16@example.com");
    insert_universe(dir.path(), "uni16", &owner_id);
    add_subscriber(dir.path(), "uni16", &sub_id);
    let token = make_jwt(&sub_id);
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/uni16/chat/rooms")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}
