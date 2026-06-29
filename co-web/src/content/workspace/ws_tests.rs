//! CO-353 — Sala WebSocket integration tests.
//!
//! These spin up a real axum server bound to `127.0.0.1:0` (never `0.0.0.0`)
//! and drive it with a `tokio-tungstenite` client — axum's WS upgrade can't be
//! exercised through `tower::ServiceExt::oneshot`. Each test uses an isolated
//! `tempfile::tempdir()` database and `JWT_SECRET=test-jwt-secret`.

use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use rusqlite::params;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::tungstenite::http::Request as TRequest;

use crate::config::WebConfig;
use crate::experiment::ExperimentStore;
use crate::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use crate::storage::Storage;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn isolate_env() {
    unsafe {
        std::env::set_var("JWT_SECRET", "test-jwt-secret");
        std::env::set_var("RUST_LOG", "off");
    }
}

fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 0,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: false,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: dir.join("universes").to_string_lossy().to_string(),
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        bypass_rate_limit: false,
    }
}

fn make_state(dir: &std::path::Path) -> AppState {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = crate::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage =
        Arc::new(game_core::storage::Storage::open(&game_db_path).expect("open test game storage"));
    let (embedding_tx, _rx) = crate::embedding_worker::channel();
    AppState::new(AppStateInner {
        core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
        realtime: Arc::new(RealtimeState {
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
            chat_presence: Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: crate::cache::CacheLayer::new(),
            embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: Arc::new(crate::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
            rate_limiter: Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
        }),
    })
}

fn insert_user(dir: &std::path::Path, email: &str) -> String {
    let storage = Storage::new(dir.to_str().unwrap());
    let id = format!("usr_{}", &nanoid::nanoid!(8));
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES (?1, ?2, ?3, 'player', ?4, ?5)",
            params![id, email, email, now, email.split('@').next().unwrap()],
        )
        .expect("insert user");
    id
}

fn make_jwt(user_id: &str) -> String {
    crate::auth::sign_jwt(user_id, "t@example.com", "player", "test-jwt-secret")
        .unwrap()
        .0
}

async fn spawn_server(state: AppState) -> u16 {
    let app = build_router(state, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}

/// Connect to the sala WS. `token` = `Some` for a signed-in user, `None` for an
/// anonymous (read-only) visitor.
async fn connect(port: u16, path: &str, token: Option<&str>) -> WsStream {
    let url = format!("ws://127.0.0.1:{port}{path}");
    let host = format!("127.0.0.1:{port}");
    let mut b = TRequest::builder()
        .uri(&url)
        .header("Host", &host)
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    let (ws, _) = connect_async(b.body(()).unwrap())
        .await
        .expect("ws connect");
    ws
}

async fn send(ws: &mut WsStream, v: serde_json::Value) {
    ws.send(WsMsg::Text(v.to_string().into())).await.unwrap();
}

/// Read frames until one whose `type` equals `want` (skipping pings/others).
/// The generous timeout absorbs CPU contention when the whole suite runs in
/// parallel (the 50-connection load test in particular).
async fn recv_until(ws: &mut WsStream, want: &str) -> serde_json::Value {
    loop {
        let frame = timeout(Duration::from_secs(20), ws.next())
            .await
            .unwrap_or_else(|_| panic!("timeout waiting for {want}"))
            .expect("stream closed")
            .expect("ws error");
        if let WsMsg::Text(t) = frame {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["type"] == want {
                return v;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Upgrade + snapshot for an anonymous (no-token) client → read-only socket.
#[tokio::test]
async fn anon_upgrades_and_gets_snapshot() {
    isolate_env();
    let dir = tempdir().unwrap();
    let port = spawn_server(make_state(dir.path())).await;

    let mut ws = connect(port, "/ws/sala/mbya/default", None).await;
    let snap = recv_until(&mut ws, "snapshot").await;
    assert!(snap["state"]["nodes"].is_array());
    assert!(snap["users"].is_array());
    ws.close(None).await.ok();
}

/// Two clients in the same room each see the other's cursor live.
#[tokio::test]
async fn two_clients_see_each_others_cursor() {
    isolate_env();
    let dir = tempdir().unwrap();
    let a_id = insert_user(dir.path(), "a@example.com");
    let b_id = insert_user(dir.path(), "b@example.com");
    let port = spawn_server(make_state(dir.path())).await;

    let mut a = connect(port, "/ws/sala/mbya/default", Some(&make_jwt(&a_id))).await;
    recv_until(&mut a, "snapshot").await;
    let mut b = connect(port, "/ws/sala/mbya/default", Some(&make_jwt(&b_id))).await;
    recv_until(&mut b, "snapshot").await;

    // B moves its cursor; A must receive it.
    send(
        &mut b,
        serde_json::json!({"type": "cursor", "x": 42.0, "y": 7.0}),
    )
    .await;
    let evt = recv_until(&mut a, "cursor").await;
    assert_eq!(evt["x"], 42.0);
    assert_eq!(evt["y"], 7.0);
    assert_eq!(evt["user_id"], b_id);

    a.close(None).await.ok();
    b.close(None).await.ok();
}

/// `node_move` from client A appears in client B.
#[tokio::test]
async fn node_move_propagates_between_clients() {
    isolate_env();
    let dir = tempdir().unwrap();
    let a_id = insert_user(dir.path(), "a2@example.com");
    let b_id = insert_user(dir.path(), "b2@example.com");
    let port = spawn_server(make_state(dir.path())).await;

    let mut a = connect(port, "/ws/sala/mbya/default", Some(&make_jwt(&a_id))).await;
    recv_until(&mut a, "snapshot").await;
    let mut b = connect(port, "/ws/sala/mbya/default", Some(&make_jwt(&b_id))).await;
    recv_until(&mut b, "snapshot").await;

    send(
        &mut a,
        serde_json::json!({"type": "node_move", "entry_path": "notes/x.md", "x": 100.0, "y": 200.0, "op_id": "op1"}),
    )
    .await;
    let evt = recv_until(&mut b, "node_move").await;
    assert_eq!(evt["entry_path"], "notes/x.md");
    assert_eq!(evt["x"], 100.0);
    assert_eq!(evt["y"], 200.0);

    a.close(None).await.ok();
    b.close(None).await.ok();
}

/// On last-user-disconnect the layout flushes to `workspace_states`, and a
/// later connection's snapshot restores it.
#[tokio::test]
async fn flush_on_last_disconnect_and_restore_on_reconnect() {
    isolate_env();
    let dir = tempdir().unwrap();
    let a_id = insert_user(dir.path(), "a3@example.com");
    let state = make_state(dir.path());
    let port = spawn_server(state.clone()).await;

    {
        let mut a = connect(port, "/ws/sala/mbya/default", Some(&make_jwt(&a_id))).await;
        recv_until(&mut a, "snapshot").await;
        send(
            &mut a,
            serde_json::json!({"type": "node_add", "entry_path": "persisted.md", "x": 5.0, "y": 6.0, "op_id": "p1"}),
        )
        .await;
        // Give the server a beat to apply the op, then disconnect (last user).
        tokio::time::sleep(Duration::from_millis(50)).await;
        a.close(None).await.ok();
    }

    // Poll storage until the shared layout has been flushed.
    let mut flushed = None;
    for _ in 0..50 {
        let ws = state
            .core
            .storage
            .lock()
            .workspace_state_for_user("mbya", "default", super::lobby::SHARED_USER)
            .unwrap();
        if let Some(ws) = ws {
            if ws.layout_json.contains("persisted.md") {
                flushed = Some(ws);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(flushed.is_some(), "layout should flush on last disconnect");

    // Reconnect: the new client's snapshot must contain the persisted node.
    let mut b = connect(port, "/ws/sala/mbya/default", Some(&make_jwt(&a_id))).await;
    let snap = recv_until(&mut b, "snapshot").await;
    let nodes = snap["state"]["nodes"].as_array().unwrap();
    assert!(
        nodes.iter().any(|n| n["entry_path"] == "persisted.md"),
        "snapshot should restore persisted layout, got: {}",
        snap["state"]
    );
    b.close(None).await.ok();
}

/// Anonymous clients are read-only: a write op is rejected (revert) and the
/// room layout is unchanged.
#[tokio::test]
async fn anon_writes_are_rejected_with_revert() {
    isolate_env();
    let dir = tempdir().unwrap();
    let state = make_state(dir.path());
    let port = spawn_server(state.clone()).await;

    let mut anon = connect(port, "/ws/sala/mbya/default", None).await;
    recv_until(&mut anon, "snapshot").await;

    send(
        &mut anon,
        serde_json::json!({"type": "node_add", "entry_path": "nope.md", "x": 1.0, "y": 1.0, "op_id": "bad1"}),
    )
    .await;

    let revert = recv_until(&mut anon, "revert").await;
    assert_eq!(revert["op_id"], "bad1");

    // The room's authoritative layout must NOT contain the rejected node.
    let id = super::lobby::workspace_id("mbya", "default");
    let room = state.core.sala_lobby.get(&id).expect("room exists");
    let layout = room.inner.lock().layout.clone();
    let nodes = layout["nodes"].as_array().cloned().unwrap_or_default();
    assert!(
        !nodes.iter().any(|n| n["entry_path"] == "nope.md"),
        "anon write must not mutate room layout"
    );
    anon.close(None).await.ok();
}

/// Load smoke: 50 concurrent connections to one room all connect + snapshot,
/// the server registers all of them, and a broadcast op still propagates.
#[tokio::test]
async fn fifty_connections_stay_responsive() {
    isolate_env();
    let dir = tempdir().unwrap();
    let driver = insert_user(dir.path(), "driver@example.com");
    let state = make_state(dir.path());
    let port = spawn_server(state.clone()).await;

    // 50 anonymous viewers all connect and receive their snapshot.
    let mut conns = Vec::new();
    for _ in 0..50 {
        let mut ws = connect(port, "/ws/sala/load/room", None).await;
        recv_until(&mut ws, "snapshot").await;
        conns.push(ws);
    }
    assert_eq!(conns.len(), 50);

    // The server holds all 50 connections in the one room (responsiveness =
    // it accepted and is tracking every socket).
    let room = state
        .core
        .sala_lobby
        .get(&super::lobby::workspace_id("load", "room"))
        .expect("room exists");
    assert_eq!(room.inner.lock().connections.len(), 50);

    // Propagation under load: actively read a sampled subscriber in a task (so
    // its socket buffer keeps draining), then a signed-in driver moves a node.
    let mut sample = conns.pop().unwrap();
    let reader = tokio::spawn(async move {
        let evt = recv_until(&mut sample, "node_move").await;
        (sample, evt)
    });

    let mut d = connect(port, "/ws/sala/load/room", Some(&make_jwt(&driver))).await;
    recv_until(&mut d, "snapshot").await;
    send(
        &mut d,
        serde_json::json!({"type": "node_move", "entry_path": "n.md", "x": 1.0, "y": 1.0, "op_id": "L"}),
    )
    .await;

    let (mut sample, evt) = reader.await.unwrap();
    assert_eq!(evt["entry_path"], "n.md");

    sample.close(None).await.ok();
    for mut ws in conns {
        ws.close(None).await.ok();
    }
    d.close(None).await.ok();
}
