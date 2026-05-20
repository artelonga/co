//! WS auth gate + event tests (original WS tests 1-7).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt as _;
use tempfile::tempdir;
use tokio::sync::broadcast;
use tokio::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tower::ServiceExt;

use crate::server::build_router;
use crate::storage::Storage;

use super::support::{add_member, insert_universe, insert_user, isolate_env, make_jwt};
use super::ws_support::{make_state, spawn_server, ws_connect, ws_try_connect};
use crate::chat::ChatEvent;

// --- 1. Unauthenticated → HTTP 401 before upgrade ---------------------------

#[tokio::test]
async fn test_ws_unauthenticated_4401() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws1@example.com");
    insert_universe(dir.path(), "ws1", &owner_id);
    let state = make_state(dir.path());
    let port = spawn_server(state).await;

    let status = ws_try_connect(port, "/api/v1/universes/ws1/chat/rooms/general/ws", None).await;
    assert_eq!(status, Some(401), "expected 401, got {status:?}");
}

// --- 2. Non-member → HTTP 403 before upgrade ---------------------------------

#[tokio::test]
async fn test_ws_non_member_4403() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws2@example.com");
    let outsider_id = insert_user(dir.path(), "outsider_ws2@example.com");
    insert_universe(dir.path(), "ws2", &owner_id);
    let token = make_jwt(&outsider_id);
    let state = make_state(dir.path());
    let port = spawn_server(state).await;

    let status = ws_try_connect(
        port,
        "/api/v1/universes/ws2/chat/rooms/general/ws",
        Some(&token),
    )
    .await;
    assert_eq!(status, Some(403), "expected 403, got {status:?}");
}

// --- 3. Archived room → HTTP 410 before upgrade ------------------------------

#[tokio::test]
async fn test_ws_archived_room_4410() {
    use crate::storage::Storage as St;
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws3@example.com");
    insert_universe(dir.path(), "ws3", &owner_id);

    // Archive the general room
    {
        let storage = St::new(dir.path().to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "UPDATE chat_rooms SET archived_at = ?1 \
                 WHERE universe_key = ?2 AND slug = ?3",
                rusqlite::params![now, "ws3", "general"],
            )
            .expect("archive room");
    }

    let token = make_jwt(&owner_id);
    let state = make_state(dir.path());
    let port = spawn_server(state).await;

    let status = ws_try_connect(
        port,
        "/api/v1/universes/ws3/chat/rooms/general/ws",
        Some(&token),
    )
    .await;
    assert_eq!(status, Some(410), "expected 410, got {status:?}");
}

// --- 4. Connect rate-limit: 6th connect/min → HTTP 429 ----------------------

#[tokio::test]
async fn test_ws_connect_rate_limit_6th_in_minute_fails() {
    use futures_util::SinkExt as _;
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws4@example.com");
    insert_universe(dir.path(), "ws4", &owner_id);
    let token = make_jwt(&owner_id);
    let state = make_state(dir.path());
    let port = spawn_server(state).await;

    let path = "/api/v1/universes/ws4/chat/rooms/general/ws";

    // First 5 connections should succeed (bucket capacity = 5).
    for _ in 0..5 {
        let url = format!("ws://127.0.0.1:{port}{path}");
        let mut ws = ws_connect(&url, &token).await;
        ws.close(None).await.ok();
    }

    // 6th connection: bucket exhausted → HTTP 429.
    let status = ws_try_connect(port, path, Some(&token)).await;
    assert_eq!(
        status,
        Some(429),
        "6th connect should be rate-limited (429), got {status:?}"
    );
}

// --- 5. Ready event includes presence ----------------------------------------

#[tokio::test]
async fn test_ws_ready_event_includes_presence() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws5@example.com");
    insert_universe(dir.path(), "ws5", &owner_id);
    let state = make_state(dir.path());
    let port = spawn_server(state).await;

    let token = make_jwt(&owner_id);
    let url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws5/chat/rooms/general/ws");
    let mut ws = ws_connect(&url, &token).await;

    // First frame must be the `ready` event
    let frame = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("timeout waiting for ready event")
        .unwrap()
        .unwrap();

    let text = match frame {
        WsMsg::Text(t) => t.to_string(),
        other => panic!("expected Text frame, got {other:?}"),
    };
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        json["type"], "ready",
        "first frame must be ready, got: {text}"
    );
    assert!(json["room_id"].is_string());
    assert!(json["presence"].is_array());

    use futures_util::SinkExt as _;
    ws.close(None).await.ok();
}

// --- 6. Broadcast to all subscribers -----------------------------------------

#[tokio::test]
async fn test_ws_broadcast_to_all_subscribers() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws6@example.com");
    let member_id = insert_user(dir.path(), "member_ws6@example.com");
    insert_universe(dir.path(), "ws6", &owner_id);
    add_member(dir.path(), "ws6", &member_id, "member");

    let state = make_state(dir.path());

    // Pre-register a broadcast channel for the general room so we can
    // subscribe before the REST POST.
    let room_id = {
        let storage = state.storage.lock();
        storage
            .get_chat_room_by_slug("ws6", "general")
            .expect("general room")
            .id
    };

    let (tx, mut rx1) = broadcast::channel::<ChatEvent>(64);
    let mut rx2 = tx.subscribe();
    {
        let mut map = state.chat_rooms_broadcast.lock().unwrap();
        map.insert(room_id.clone(), tx);
    }

    let app = build_router(Arc::clone(&state), None);
    let token = make_jwt(&owner_id);

    // POST a message via REST
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/ws6/chat/rooms/general/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"broadcast test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Both subscribers must receive message.created
    let evt1 = tokio::time::timeout(Duration::from_millis(200), async {
        rx1.recv().await.unwrap()
    })
    .await
    .expect("rx1 timeout");

    let evt2 = tokio::time::timeout(Duration::from_millis(200), async {
        rx2.recv().await.unwrap()
    })
    .await
    .expect("rx2 timeout");

    assert!(
        matches!(&evt1, ChatEvent::MessageCreated { .. }),
        "rx1: expected MessageCreated, got {evt1:?}"
    );
    assert!(
        matches!(&evt2, ChatEvent::MessageCreated { .. }),
        "rx2: expected MessageCreated, got {evt2:?}"
    );
}

// --- 7. No broadcast to other rooms ------------------------------------------

#[tokio::test]
async fn test_ws_no_broadcast_to_other_rooms() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws7@example.com");
    insert_universe(dir.path(), "ws7", &owner_id);

    let state = make_state(dir.path());

    // Get room A (general) and create room B
    let (room_a_id, room_b_id) = {
        let storage = state.storage.lock();
        let room_a = storage
            .get_chat_room_by_slug("ws7", "general")
            .expect("general room");
        let room_b = storage
            .create_chat_room("ws7", "Other", None, &owner_id)
            .expect("create other room");
        (room_a.id, room_b.id)
    };

    // Subscribe only to room B
    let (tx_b, mut rx_b) = broadcast::channel::<ChatEvent>(64);
    {
        let mut map = state.chat_rooms_broadcast.lock().unwrap();
        map.insert(room_b_id.clone(), tx_b);
    }

    let app = build_router(Arc::clone(&state), None);
    let token = make_jwt(&owner_id);

    // POST to room A
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/ws7/chat/rooms/general/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"body":"room a message"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Room B subscriber must NOT receive anything
    let result = tokio::time::timeout(Duration::from_millis(100), rx_b.recv()).await;
    assert!(
        result.is_err(),
        "room B should not receive events from room A"
    );

    let _ = room_a_id;
}
