//! CO-204 — chat message origin telemetry tests.
//!
//! Covers: explicit-origin write, privacy drop for a non-member origin,
//! default-to-room behavior for universe and DM rooms, GET surfacing the
//! field, and the admin origin-breakdown endpoint (aggregation + admin gate).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::tempdir;
use tower::ServiceExt;

use crate::storage::Storage;

use super::support::*;

/// Helper: post a message and return the parsed response JSON.
async fn post_message(
    app: &axum::Router,
    slug: &str,
    room_slug: &str,
    token: &str,
    json_body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/universes/{slug}/chat/rooms/{room_slug}/messages"
                ))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(json_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp.into_body()).await)
}

/// Helper: GET the room messages and return the JSON.
async fn list_messages(
    app: &axum::Router,
    slug: &str,
    room_slug: &str,
    token: &str,
) -> serde_json::Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/universes/{slug}/chat/rooms/{room_slug}/messages"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp.into_body()).await
}

// --- 1. Explicit origin from a universe the caller belongs to is written ---

#[tokio::test]
async fn test_post_with_valid_origin_writes_it() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner = insert_user(dir.path(), "owner-o1@example.com");
    insert_universe(dir.path(), "unio1", &owner);
    let token = make_jwt(&owner);
    let app = build_test_router(dir.path());

    let (status, _) = post_message(
        &app,
        "unio1",
        "general",
        &token,
        serde_json::json!({ "body": "olá", "origin_universe_key": "unio1" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let json = list_messages(&app, "unio1", "general", &token).await;
    let msg = &json["messages"][0];
    assert_eq!(msg["origin_universe_key"], "unio1");
}

// --- 2. Origin the caller doesn't belong to is silently dropped (NULL) ---

#[tokio::test]
async fn test_post_with_foreign_origin_dropped() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner = insert_user(dir.path(), "owner-o2@example.com");
    insert_universe(dir.path(), "unio2", &owner);
    let token = make_jwt(&owner);
    let app = build_test_router(dir.path());

    // "secretuni" is a universe the caller is NOT a member/subscriber of.
    let (status, _) = post_message(
        &app,
        "unio2",
        "general",
        &token,
        serde_json::json!({ "body": "olá", "origin_universe_key": "secretuni" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let json = list_messages(&app, "unio2", "general", &token).await;
    let msg = &json["messages"][0];
    // Privacy-respecting: claimed origin dropped, persisted with NULL.
    assert!(
        msg["origin_universe_key"].is_null(),
        "expected null origin, got {:?}",
        msg["origin_universe_key"]
    );
}

// --- 3. No origin on a universe room defaults to the room's universe ---

#[tokio::test]
async fn test_post_without_origin_defaults_to_room_universe() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner = insert_user(dir.path(), "owner-o3@example.com");
    insert_universe(dir.path(), "unio3", &owner);
    let token = make_jwt(&owner);
    let app = build_test_router(dir.path());

    let (status, _) = post_message(
        &app,
        "unio3",
        "general",
        &token,
        serde_json::json!({ "body": "sem origin" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let json = list_messages(&app, "unio3", "general", &token).await;
    let msg = &json["messages"][0];
    assert_eq!(msg["origin_universe_key"], "unio3");
}

// --- 4. Subscriber may claim a universe they're subscribed to as origin ---

#[tokio::test]
async fn test_subscriber_origin_is_accepted() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner = insert_user(dir.path(), "owner-o4@example.com");
    let member = insert_user(dir.path(), "member-o4@example.com");
    // The room lives in unio4 (member can post there).
    insert_universe(dir.path(), "unio4", &owner);
    add_member(dir.path(), "unio4", &member, "member");
    // The author is a *subscriber* of a different universe, "bloga".
    insert_universe(dir.path(), "blogo4", &owner);
    add_subscriber(dir.path(), "blogo4", &member);
    let token = make_jwt(&member);
    let app = build_test_router(dir.path());

    let (status, _) = post_message(
        &app,
        "unio4",
        "general",
        &token,
        serde_json::json!({ "body": "via blog", "origin_universe_key": "blogo4" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let json = list_messages(&app, "unio4", "general", &token).await;
    let msg = &json["messages"][0];
    assert_eq!(msg["origin_universe_key"], "blogo4");
}

// --- 5. DM without origin persists NULL; DM with a valid origin keeps it ---

#[tokio::test]
async fn test_dm_origin_default_null_and_explicit_kept() {
    isolate_env();
    let dir = tempdir().unwrap();
    let a = insert_user(dir.path(), "a-o5@example.com");
    let b = insert_user(dir.path(), "b-o5@example.com");
    insert_universe(dir.path(), "shared-o5", &a);
    add_member(dir.path(), "shared-o5", &b, "member");

    let dm_slug = {
        let storage = Storage::new(dir.path().to_str().unwrap());
        storage.open_dm(&a, &b).unwrap().slug
    };
    let token_a = make_jwt(&a);
    let app = build_test_router(dir.path());

    // 5a. No origin → NULL (DM room has no universe context).
    let (status, _) = post_message(
        &app,
        "dm",
        &dm_slug,
        &token_a,
        serde_json::json!({ "body": "oi, sem origin" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // 5b. Explicit origin from a universe A belongs to → kept (the breadcrumb).
    let (status, _) = post_message(
        &app,
        "dm",
        &dm_slug,
        &token_a,
        serde_json::json!({ "body": "te conheci no quilombo", "origin_universe_key": "shared-o5" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let json = list_messages(&app, "dm", &dm_slug, &token_a).await;
    let msgs = json["messages"].as_array().unwrap();
    // Messages come back newest-first.
    let with_origin = msgs
        .iter()
        .find(|m| m["body"] == "te conheci no quilombo")
        .unwrap();
    let without_origin = msgs.iter().find(|m| m["body"] == "oi, sem origin").unwrap();
    assert_eq!(with_origin["origin_universe_key"], "shared-o5");
    assert!(without_origin["origin_universe_key"].is_null());
}

// --- 6. Admin origin-breakdown aggregates by origin universe ---

#[tokio::test]
async fn test_admin_origin_breakdown_aggregates() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner = insert_user(dir.path(), "owner-o6@example.com");
    insert_universe(dir.path(), "unio6", &owner);
    let token = make_jwt(&owner);
    let app = build_test_router(dir.path());

    // Two messages with origin unio6, one with no origin (NULL via DM-less
    // path we simulate by writing directly with NULL).
    for _ in 0..2 {
        let (status, _) = post_message(
            &app,
            "unio6",
            "general",
            &token,
            serde_json::json!({ "body": "x", "origin_universe_key": "unio6" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    {
        let storage = Storage::new(dir.path().to_str().unwrap());
        let room = storage.get_chat_room_by_slug("unio6", "general").unwrap();
        storage
            .post_chat_message(&room.id, &owner, "no origin", None, None)
            .unwrap();
    }

    let admin = insert_user(dir.path(), "admin-o6@example.com");
    let admin_token = make_admin_jwt(&admin);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/chat/origin-breakdown")
                .header("authorization", format!("Bearer {admin_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp.into_body()).await;

    assert_eq!(json["total_messages"], 3);
    let origins = json["origins"].as_array().unwrap();
    let unio6 = origins
        .iter()
        .find(|o| o["universe_key"] == "unio6")
        .expect("unio6 bucket present");
    assert_eq!(unio6["message_count"], 2);
    let null_bucket = origins
        .iter()
        .find(|o| o["universe_key"].is_null())
        .expect("null bucket present");
    assert_eq!(null_bucket["message_count"], 1);
}

// --- 7. Admin breakdown rejects non-admin callers ---

#[tokio::test]
async fn test_admin_origin_breakdown_requires_admin() {
    isolate_env();
    let dir = tempdir().unwrap();
    let user = insert_user(dir.path(), "plain-o7@example.com");
    let token = make_jwt(&user); // tier == "player"
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/chat/origin-breakdown")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
