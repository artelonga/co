//! WS presence/typing tests (original WS tests 8-11).

use futures_util::{SinkExt as _, StreamExt as _};
use tempfile::tempdir;
use tokio::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use super::support::{add_member, insert_universe, insert_user, isolate_env, make_jwt};
use super::ws_support::{make_state, spawn_server, ws_connect};

// --- 8. Presence join/leave events -------------------------------------------

#[tokio::test]
async fn test_ws_presence_join_leave_events() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws8@example.com");
    let member_id = insert_user(dir.path(), "member_ws8@example.com");
    insert_universe(dir.path(), "ws8", &owner_id);
    add_member(dir.path(), "ws8", &member_id, "member");

    let state = make_state(dir.path());
    let port = spawn_server(state.clone()).await;

    let tok_owner = make_jwt(&owner_id);
    let tok_member = make_jwt(&member_id);

    let ws_url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws8/chat/rooms/general/ws");

    // Client 1 (owner) connects
    let mut ws_owner = ws_connect(&ws_url, &tok_owner).await;

    // Receive ready frame for client 1
    let _ready1 = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Client 2 (member) connects; client 1 should receive presence.join
    let mut ws_member = ws_connect(&ws_url, &tok_member).await;

    // Drain frames from client 1 until we get presence.join
    let mut got_join = false;
    for _ in 0..10 {
        let frame = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
            .await
            .expect("timeout waiting for presence.join")
            .unwrap()
            .unwrap();
        if let WsMsg::Text(text) = frame {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            if json["type"] == "presence.join" {
                got_join = true;
                break;
            }
        }
    }
    assert!(
        got_join,
        "client 1 must receive presence.join when client 2 connects"
    );

    // Consume client 2's ready frame
    let _ready2 = tokio::time::timeout(Duration::from_secs(2), ws_member.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Client 2 disconnects; client 1 should receive presence.leave
    ws_member.close(None).await.ok();

    let mut got_leave = false;
    for _ in 0..10 {
        let frame = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
            .await
            .expect("timeout waiting for presence.leave")
            .unwrap()
            .unwrap();
        if let WsMsg::Text(text) = frame {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            if json["type"] == "presence.leave" {
                got_leave = true;
                break;
            }
        }
    }
    assert!(
        got_leave,
        "client 1 must receive presence.leave when client 2 disconnects"
    );

    ws_owner.close(None).await.ok();
}

// --- 9. Same user, multiple connections → one presence.join -----------------

#[tokio::test]
async fn test_ws_same_user_multiple_connections_dedup() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws9@example.com");
    let member_id = insert_user(dir.path(), "member_ws9@example.com");
    insert_universe(dir.path(), "ws9", &owner_id);
    add_member(dir.path(), "ws9", &member_id, "member");

    let state = make_state(dir.path());
    let port = spawn_server(state.clone()).await;

    let tok_owner = make_jwt(&owner_id);
    let tok_member = make_jwt(&member_id);

    let ws_url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws9/chat/rooms/general/ws");

    // Owner (observer) connects first
    let mut ws_owner = ws_connect(&ws_url, &tok_owner).await;
    let _ready_owner = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Member connects TWICE (simulating two tabs)
    let mut ws_m1 = ws_connect(&ws_url, &tok_member).await;
    let _ready_m1 = tokio::time::timeout(Duration::from_secs(2), ws_m1.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let mut ws_m2 = ws_connect(&ws_url, &tok_member).await;
    let _ready_m2 = tokio::time::timeout(Duration::from_secs(2), ws_m2.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Collect frames from owner for a short window
    let mut join_count = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, ws_owner.next()).await {
            Ok(Some(Ok(WsMsg::Text(text)))) => {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                if json["type"] == "presence.join" {
                    join_count += 1;
                }
            }
            _ => {}
        }
    }

    // Only ONE presence.join should have been emitted for the member
    assert_eq!(
        join_count, 1,
        "expected exactly 1 presence.join for member (refcount dedup), got {join_count}"
    );

    ws_m1.close(None).await.ok();
    ws_m2.close(None).await.ok();
    ws_owner.close(None).await.ok();
}

// --- 10. Typing rate-limit: second typing.start within 2s is dropped ---------

#[tokio::test]
async fn test_ws_typing_rate_limit_2s() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws10@example.com");
    let member_id = insert_user(dir.path(), "member_ws10@example.com");
    insert_universe(dir.path(), "ws10", &owner_id);
    add_member(dir.path(), "ws10", &member_id, "member");

    let state = make_state(dir.path());
    let port = spawn_server(state.clone()).await;

    let tok_owner = make_jwt(&owner_id);
    let tok_member = make_jwt(&member_id);
    let ws_url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws10/chat/rooms/general/ws");

    // Owner connects as the observer.
    let mut ws_owner = ws_connect(&ws_url, &tok_owner).await;
    let _ready_owner = tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Member connects as the typist.
    let mut ws_member = ws_connect(&ws_url, &tok_member).await;
    let _ready_member = tokio::time::timeout(Duration::from_secs(2), ws_member.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Owner receives the presence.join for member (drain it).
    tokio::time::timeout(Duration::from_secs(2), ws_owner.next())
        .await
        .ok();

    // Member sends two rapid typing.start messages.
    ws_member
        .send(WsMsg::Text(r#"{"type":"typing.start"}"#.into()))
        .await
        .unwrap();
    ws_member
        .send(WsMsg::Text(r#"{"type":"typing.start"}"#.into()))
        .await
        .unwrap();

    // Drain the owner's WS stream for a short window; count typing.start events.
    let mut typing_count = 0usize;
    for _ in 0..10 {
        match tokio::time::timeout(Duration::from_millis(200), ws_owner.next()).await {
            Ok(Some(Ok(WsMsg::Text(text)))) => {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                if json["type"] == "typing.start" {
                    typing_count += 1;
                }
            }
            _ => break,
        }
    }

    // Only the first typing.start should have been broadcast; the second
    // is silently dropped by the rate limiter (capacity = 1 per key).
    assert_eq!(
        typing_count, 1,
        "second rapid typing.start should be rate-limited (got {typing_count} events)"
    );

    ws_member.close(None).await.ok();
    ws_owner.close(None).await.ok();
}

// --- 11. Keep-alive: silent client is dropped --------------------------------

#[tokio::test]
async fn test_ws_keepalive_drops_silent_client() {
    isolate_env();
    let dir = tempdir().unwrap();
    let owner_id = insert_user(dir.path(), "owner_ws11@example.com");
    insert_universe(dir.path(), "ws11", &owner_id);

    let state = make_state(dir.path());
    let port = spawn_server(state).await;

    let token = make_jwt(&owner_id);
    let ws_url = format!("ws://127.0.0.1:{port}/api/v1/universes/ws11/chat/rooms/general/ws");

    let mut ws = ws_connect(&ws_url, &token).await;

    // Consume ready frame
    let _ready = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("timeout waiting for ready frame")
        .unwrap()
        .unwrap();

    // Advance tokio simulated time past the silence threshold (30s ping interval + 40s timeout = 70s)
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(35)).await; // writer sends ping at t=30
    tokio::time::advance(Duration::from_secs(35)).await; // t=70 > 40s silence threshold

    // Client should eventually see the connection close
    let mut saw_close = false;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_millis(50), ws.next()).await {
            Ok(Some(Ok(WsMsg::Close(_)))) => {
                saw_close = true;
                break;
            }
            Ok(None) => {
                saw_close = true;
                break;
            }
            Ok(Some(Err(_))) => {
                saw_close = true;
                break;
            }
            Ok(Some(Ok(WsMsg::Ping(_) | WsMsg::Pong(_) | WsMsg::Text(_)))) => {
                continue;
            }
            Err(_timeout) => {
                // no data yet — time might not have advanced enough
                break;
            }
            _ => {}
        }
    }

    // Note: this test is inherently racy with real TCP + simulated time.
    // We assert connection is closed, but allow some slack.
    assert!(
        saw_close,
        "server should close the connection after silence timeout"
    );
}
