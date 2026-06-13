//! CO-384: `/api/v1/events/bridge` — federated bridge WebSocket endpoint.
//!
//! Accepts persistent bidirectional WS connections from trusted peer deployments.
//! Trust is enforced via `CO_BRIDGE_TRUSTED_SOURCES` (CSV of allowed source hosts).
//!
//! Protocol:
//!  1. Remote connects with `?source=<host>&token=<jwt>` — 403 if not trusted.
//!  2. Remote sends `ReplayRequest{last_received_id}` — server replays missed events.
//!  3. Bidirectional `Event` messages flow until disconnect.
//!
//! Privacy: only `Public` and `UniverseMembers` events are federated; `UserOnly`,
//! `UniverseOwner`, and `System` events are dropped at the bridge level.
//!
//! Loop guard: events with `hop_count > MAX_HOP_COUNT` are silently dropped.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::federated_event::{BridgeMessage, FederatedEvent, MAX_HOP_COUNT};
use crate::eda::EdaBus;
use crate::eda::bus::Filter;
use crate::eda::event::{Event, Visibility};
use crate::server::AppState;
use crate::storage::Storage;

// ---------------------------------------------------------------------------
// Config helpers
// ---------------------------------------------------------------------------

/// Parse `CO_BRIDGE_TRUSTED_SOURCES` (comma-separated deployment hosts).
/// CO-434: routed through the process-global SecretsProvider seam.
pub fn trusted_sources_from_env() -> Vec<String> {
    crate::infra::secrets::global()
        .get_or("CO_BRIDGE_TRUSTED_SOURCES", "")
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `true` if `source` is in `trusted` list (testable without env).
pub fn is_trusted_in(trusted: &[String], source: &str) -> bool {
    trusted.iter().any(|s| s == source)
}

/// `true` if `source` is listed in `CO_BRIDGE_TRUSTED_SOURCES`.
pub fn is_trusted(source: &str) -> bool {
    is_trusted_in(&trusted_sources_from_env(), source)
}

fn env_heartbeat_secs() -> u64 {
    use crate::infra::secrets::SecretsProviderExt;
    crate::infra::secrets::global().get_parsed("CO_BRIDGE_HEARTBEAT_S", 30)
}

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BridgeQuery {
    /// Source deployment host (e.g. "yggdrasil.artelonga.com.br").
    pub source: String,
    /// JWT token identifying the source deployment.
    pub token: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /api/v1/events/bridge — federated EDA bridge WebSocket endpoint.
///
/// Only accessible to deployments listed in `CO_BRIDGE_TRUSTED_SOURCES`.
/// Returns 403 for unknown sources; upgrades to `co.eda.bridge.v1` for trusted ones.
pub async fn bridge_ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<BridgeQuery>,
    State(state): State<AppState>,
) -> Response {
    // Trust-list enforcement — reject unknown sources immediately.
    if !is_trusted(&params.source) {
        warn!(
            source = %params.source,
            "EDA bridge: rejected connection from untrusted source"
        );
        return (StatusCode::FORBIDDEN, "Untrusted bridge source").into_response();
    }

    if params.token.is_empty() {
        return (StatusCode::UNAUTHORIZED, "Missing bridge token").into_response();
    }

    let source = params.source.clone();
    let our_id = super::our_deployment_id();
    let bus = Arc::clone(&state.core.eda_bus);
    let storage = Arc::clone(&state.core.storage);

    ws.protocols(["co.eda.bridge.v1"])
        .on_upgrade(move |socket| handle_bridge_socket(socket, bus, storage, source, our_id))
}

// ---------------------------------------------------------------------------
// Socket loop
// ---------------------------------------------------------------------------

async fn handle_bridge_socket(
    mut socket: WebSocket,
    bus: Arc<dyn EdaBus>,
    storage: Arc<Mutex<Storage>>,
    source: String,
    our_deployment: String,
) {
    info!(source = %source, "EDA bridge: peer connected");

    // Telemetry — visible in /agora.
    bus.publish(Event::new(
        "bridge.connected",
        None,
        None,
        serde_json::json!({ "source": &source, "target": &our_deployment }),
        Visibility::Public,
    ));
    update_bridge_state(&storage, &source, &our_deployment, "connected", None);

    // Subscribe to local bus; only federatable events are forwarded.
    let mut local_sub = bus.subscribe(Filter::default());

    let mut heartbeat =
        tokio::time::interval(tokio::time::Duration::from_secs(env_heartbeat_secs()));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip the immediate first tick so we don't send a ping before the handshake.
    heartbeat.tick().await;

    let mut last_received_id: Option<String> = None;

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(d))) => {
                        let _ = socket.send(Message::Pong(d)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {} // heartbeat ack, ignore
                    Some(Ok(Message::Text(text))) => {
                        handle_inbound_text(
                            text.as_str(),
                            &bus,
                            &storage,
                            &mut socket,
                            &source,
                            &our_deployment,
                            &mut last_received_id,
                        )
                        .await;
                    }
                    Some(Ok(Message::Binary(_))) => {} // not used in protocol
                }
            }

            ev = local_sub.recv() => {
                match ev {
                    None => break, // bus dropped
                    Some(ev) => {
                        // Privacy filter: only federate Public + UniverseMembers.
                        if !FederatedEvent::is_federatable(&ev.visibility) {
                            continue;
                        }
                        let fed = FederatedEvent::new(
                            (*ev).clone(),
                            our_deployment.clone(),
                            our_deployment.clone(),
                        );
                        let msg = BridgeMessage::Event { federated: fed };
                        match serde_json::to_string(&msg) {
                            Ok(json) => {
                                if socket.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                                bus.publish(Event::new(
                                    "bridge.event_sent",
                                    None,
                                    None,
                                    serde_json::json!({ "target": &source }),
                                    Visibility::Public,
                                ));
                            }
                            Err(e) => warn!("EDA bridge: serialize error: {e}"),
                        }
                    }
                }
            }

            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
        }
    }

    info!(source = %source, "EDA bridge: peer disconnected");
    bus.publish(Event::new(
        "bridge.disconnected",
        None,
        None,
        serde_json::json!({ "source": &source, "target": &our_deployment, "reason": "ws_closed" }),
        Visibility::Public,
    ));
    update_bridge_state(
        &storage,
        &source,
        &our_deployment,
        "disconnected",
        last_received_id.as_deref(),
    );
}

/// Handle a text frame received from the remote peer.
async fn handle_inbound_text(
    text: &str,
    bus: &Arc<dyn EdaBus>,
    storage: &Arc<Mutex<Storage>>,
    socket: &mut WebSocket,
    source: &str,
    our_deployment: &str,
    last_received_id: &mut Option<String>,
) {
    let msg: BridgeMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            warn!(source = %source, "EDA bridge: parse error: {e}");
            return;
        }
    };

    match msg {
        BridgeMessage::ReplayRequest {
            last_received_id: since,
        } => {
            // Remote is requesting replay of events it missed while disconnected.
            let events = load_events_since(storage, since.as_deref());
            let count = events.len();
            for ev in events {
                if !FederatedEvent::is_federatable(&ev.visibility) {
                    continue;
                }
                let fed =
                    FederatedEvent::new(ev, our_deployment.to_string(), our_deployment.to_string());
                let reply = BridgeMessage::Event { federated: fed };
                if let Ok(json) = serde_json::to_string(&reply)
                    && socket.send(Message::Text(json.into())).await.is_err()
                {
                    return;
                }
            }
            if count > 0 {
                bus.publish(Event::new(
                    "bridge.replay_completed",
                    None,
                    None,
                    serde_json::json!({ "source": source, "events_count": count }),
                    Visibility::Public,
                ));
                info!(source = %source, count, "EDA bridge: replay sent");
            }
        }

        BridgeMessage::Event { mut federated } => {
            // Loop guard.
            if !federated.within_hop_limit() {
                debug!(
                    hop_count = federated.hop_count,
                    "EDA bridge: dropping event (hop_count > {})", MAX_HOP_COUNT
                );
                return;
            }
            // Privacy filter.
            if !FederatedEvent::is_federatable(&federated.event.visibility) {
                debug!(
                    visibility = ?federated.event.visibility,
                    "EDA bridge: dropping non-federatable event"
                );
                return;
            }

            *last_received_id = Some(federated.event.id.clone());
            federated.hop_count += 1;
            federated.bridge_received_at = Utc::now();

            // CO-385: Hash-skip optimization — entry events with matching body_hash
            // never enter the conflict pipeline. Differing hashes create a conflict
            // record and publish `sync.conflict_detected` for the live timeline.
            if matches!(
                federated.event.event_type.as_str(),
                "entry.created" | "entry.updated"
            ) {
                check_entry_conflict(&federated.event, source, storage, bus);
            }

            // Republish on local bus — local subscribers see it like any other event.
            bus.publish(federated.event.clone());

            bus.publish(Event::new(
                "bridge.event_received",
                None,
                None,
                serde_json::json!({ "source": &federated.origin_deployment }),
                Visibility::Public,
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// CO-385: bridge-layer conflict detection
// ---------------------------------------------------------------------------

/// Hash-skip + conflict creation for federated `entry.*` events.
///
/// If the remote `body_hash` matches what we have locally → hash-skip (no conflict).
/// If it differs → persist a conflict record and publish `sync.conflict_detected`.
fn check_entry_conflict(
    ev: &Event,
    source: &str,
    storage: &Arc<Mutex<Storage>>,
    bus: &Arc<dyn EdaBus>,
) {
    let payload = &ev.payload;

    let (universe_key, path, remote_hash) = match (
        ev.universe_key.as_deref(),
        payload.get("path").and_then(|v| v.as_str()),
        payload.get("body_hash").and_then(|v| v.as_str()),
    ) {
        (Some(u), Some(p), Some(h)) => (u, p, h),
        _ => return, // missing fields → skip silently
    };

    // Read local body_hash for this (universe_key, path).
    let local_hash: Option<String> = {
        let st = storage.lock();
        let uc = st.universe_conn(universe_key);
        let conn = uc.lock().unwrap();
        conn.query_row(
            "SELECT body_hash FROM entries WHERE universe_key = ?1 AND path = ?2",
            rusqlite::params![universe_key, path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    };

    let local_hash_str = match local_hash {
        Some(ref h) => h.clone(),
        None => {
            // Entry doesn't exist locally → RemoteOnlyNew, not a conflict by default.
            return;
        }
    };

    // Hash-skip: same hash → no conflict needed.
    if local_hash_str == remote_hash {
        debug!(
            universe = %universe_key, path = %path,
            "EDA bridge: hash-skip — no conflict (hashes match)"
        );
        return;
    }

    // Hashes differ → create conflict record.
    use crate::sync::conflict_detector::{Conflict, ConflictKind, EntryRevision};
    use crate::sync::routes::persist_conflict;

    let conflict = Conflict::new(
        universe_key,
        path,
        EntryRevision {
            path: path.into(),
            body_hash: local_hash_str.clone(),
            body: None,
            updated_at: None,
            source: "local".into(),
        },
        EntryRevision {
            path: path.into(),
            body_hash: remote_hash.into(),
            body: None,
            updated_at: None,
            source: source.into(),
        },
        None,
        ConflictKind::BothModified,
    );

    {
        let st = storage.lock();
        if let Err(e) = persist_conflict(st.conn(), &conflict) {
            warn!(
                universe = %universe_key, path = %path,
                "EDA bridge: persist_conflict failed: {e}"
            );
            return;
        }
    }

    bus.publish(Event::new(
        "sync.conflict_detected",
        Some(universe_key.into()),
        None,
        serde_json::json!({
            "conflict_id": &conflict.id,
            "path": path,
            "kind": "both_modified",
            "local_body_hash": local_hash_str,
            "remote_body_hash": remote_hash,
            "source": source,
            "resolve_url": format!("/sync/conflicts?universe={universe_key}"),
        }),
        Visibility::UniverseOwner,
    ));
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Persist bridge connection state in the `bridge_state` table.
fn update_bridge_state(
    storage: &Arc<Mutex<Storage>>,
    source: &str,
    target: &str,
    state: &str,
    last_event_id: Option<&str>,
) {
    let st = storage.lock();
    if let Err(e) = st.upsert_bridge_state(source, target, state, last_event_id) {
        warn!("EDA bridge: bridge_state upsert failed: {e}");
    }
}

/// Load events from `event_log` published after `since_id` (exclusive).
///
/// Returns at most 1000 events in ULID order (chronological).
fn load_events_since(storage: &Arc<Mutex<Storage>>, since_id: Option<&str>) -> Vec<Event> {
    let st = storage.lock();
    let since = since_id.unwrap_or("");
    let rows = match st.load_event_log_since(since) {
        Ok(rows) => rows,
        Err(e) => {
            warn!("EDA bridge: event_log query failed: {e}");
            return vec![];
        }
    };

    rows.into_iter()
        .map(|r| {
            let payload = serde_json::from_str(&r.payload_json).unwrap_or(serde_json::Value::Null);
            let visibility = match r.visibility.as_str() {
                "UniverseMembers" => Visibility::UniverseMembers,
                "UniverseOwner" => Visibility::UniverseOwner,
                "UserOnly" => Visibility::UserOnly,
                "System" => Visibility::System,
                _ => Visibility::Public,
            };
            let created_at = r
                .created_at
                .parse::<chrono::DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now());

            Event {
                id: r.id,
                event_type: r.event_type,
                universe_key: r.universe_key,
                user_id: r.user_id,
                payload,
                visibility,
                created_at,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::http::Request as WsRequest;
    use tower::ServiceExt;

    use crate::auth::AuthStore;
    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::server::{
        AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
        build_router,
    };
    use crate::storage::Storage;

    #[test]
    fn trust_list_empty_rejects_all() {
        assert!(!is_trusted_in(&[], "any.com"));
    }

    #[test]
    fn trust_list_accepts_known_source() {
        let trusted = vec![
            "yggdrasil.artelonga.com.br".to_string(),
            "co.artelonga.com.br".to_string(),
        ];
        assert!(is_trusted_in(&trusted, "yggdrasil.artelonga.com.br"));
        assert!(is_trusted_in(&trusted, "co.artelonga.com.br"));
        assert!(!is_trusted_in(&trusted, "evil.com"));
    }

    #[test]
    fn trust_list_is_exact_match() {
        let trusted = vec!["co.artelonga.com.br".to_string()];
        assert!(!is_trusted_in(&trusted, "co.artelonga.com.br.evil.example"));
        assert!(!is_trusted_in(&trusted, "evil.co.artelonga.com.br"));
    }

    // -----------------------------------------------------------------------
    // CO-391: real-axum WebSocket handshake integration tests.
    //
    // The `is_trusted_in` unit tests above cover the trust-list predicate only —
    // no `WebSocketUpgrade`, no real socket. These tests boot the real axum router
    // with the bridge route mounted and dial it with a real WS client speaking
    // `co.eda.bridge.v1`, locking down the handshake contract from the CO side so a
    // YG-122-class regression (axum rejecting a connect a lenient mock accepted)
    // fails loudly at PR time instead of at first prod dial.
    // -----------------------------------------------------------------------

    /// Host written into `CO_BRIDGE_TRUSTED_SOURCES` by every test below. All
    /// tests write the SAME value so concurrent env writes are harmless (mirrors
    /// the shared-`JWT_SECRET` pattern in `social/sync_ws.rs`).
    const TRUSTED_PEER: &str = "co-bridge-test-peer.local";

    fn set_bridge_env() {
        // Safety: tests share one process; we always write the same values.
        unsafe {
            std::env::set_var("JWT_SECRET", "test-jwt-secret");
            std::env::set_var("CO_BRIDGE_TRUSTED_SOURCES", TRUSTED_PEER);
        }
    }

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
            port: 0,
            data_dir: dir.to_string_lossy().into(),
            static_dir: "co-web/static".into(),
            default_variant: "a".into(),
            experiments: false,
            plugins_dir: "plugins".into(),
            game_db_path: None,
            universo_dir: dir.join("universes").to_string_lossy().into(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "test".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        }
    }

    fn make_test_state(dir: &std::path::Path) -> AppState {
        set_bridge_env();
        let storage = Storage::new(dir);
        let experiment = ExperimentStore::new(dir);
        let auth_store = AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&dir.join("game_test.db")).unwrap());
        let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
        AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(
                storage,
                test_config(dir),
                auth_store,
            )),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
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
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: std::sync::Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        })
    }

    /// Boot the real router on an ephemeral port; return `(port, bus)`. The bus is
    /// the *same* instance the server uses, so the test can observe events the
    /// handler publishes locally (e.g. `bridge.connected`).
    async fn spawn_bridge_server() -> (u16, Arc<dyn EdaBus>) {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = make_test_state(tmp.path());
        let bus = Arc::clone(&state.core.eda_bus);
        let app = build_router(state, None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        std::mem::forget(tmp); // keep the data dir alive for the server's lifetime
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (port, bus)
    }

    /// Build the WS upgrade request a Yggdrasil peer sends. `connect_async` always
    /// emits the upgrade headers — the missing-`Upgrade` case is tested separately
    /// via a plain HTTP GET, which a WS client cannot produce.
    fn ws_request(
        port: u16,
        source: &str,
        token: &str,
        subprotocol: Option<&str>,
    ) -> WsRequest<()> {
        let url =
            format!("ws://127.0.0.1:{port}/api/v1/events/bridge?source={source}&token={token}");
        let mut b = WsRequest::builder()
            .uri(&url)
            .header("Host", format!("127.0.0.1:{port}"))
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket");
        if let Some(sp) = subprotocol {
            b = b.header("Sec-WebSocket-Protocol", sp);
        }
        b.body(()).unwrap()
    }

    /// Happy path YG-119's lenient mock falsely passed: trusted source + non-empty
    /// token + subprotocol → 101, subprotocol echo, and `bridge.connected` on the bus.
    #[tokio::test]
    async fn bridge_handshake_accepts_trusted_source() {
        let (port, bus) = spawn_bridge_server().await;
        // Subscribe BEFORE connecting so we don't miss the connect telemetry.
        let mut sub = bus.subscribe(Filter::default());

        let (mut ws, response) = connect_async(ws_request(
            port,
            TRUSTED_PEER,
            "any-token",
            Some("co.eda.bridge.v1"),
        ))
        .await
        .expect("handshake must succeed");

        assert_eq!(response.status().as_u16(), 101);
        assert_eq!(
            response
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|h| h.to_str().ok()),
            Some("co.eda.bridge.v1"),
            "server must echo the negotiated subprotocol",
        );

        // `bridge.connected` is published to the LOCAL bus, not echoed down the
        // socket — assert it lands there.
        let connected = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match sub.recv().await {
                    Some(ev) if ev.event_type == "bridge.connected" => break ev,
                    Some(_) => continue,
                    None => panic!("bus closed before bridge.connected"),
                }
            }
        })
        .await
        .expect("bridge.connected must land on the local bus within 2s");

        assert_eq!(connected.payload["source"], TRUSTED_PEER);
        ws.close(None).await.ok();
    }

    /// Regression guard for the trust list.
    #[tokio::test]
    async fn bridge_handshake_rejects_untrusted_source() {
        let (port, _bus) = spawn_bridge_server().await;
        match connect_async(ws_request(
            port,
            "untrusted.example",
            "any-token",
            Some("co.eda.bridge.v1"),
        ))
        .await
        {
            Ok(_) => panic!("untrusted source must be rejected"),
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
                assert_eq!(resp.status().as_u16(), 403);
            }
            Err(e) => panic!("expected HTTP 403, got error: {e:?}"),
        }
    }

    /// Regression guard for the token-presence check.
    #[tokio::test]
    async fn bridge_handshake_rejects_empty_token() {
        let (port, _bus) = spawn_bridge_server().await;
        match connect_async(ws_request(port, TRUSTED_PEER, "", Some("co.eda.bridge.v1"))).await {
            Ok(_) => panic!("empty token must be rejected"),
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
                assert_eq!(resp.status().as_u16(), 401);
            }
            Err(e) => panic!("expected HTTP 401, got error: {e:?}"),
        }
    }

    /// The exact failure that bit us on 2026-06-09: a GET without `Connection: Upgrade`
    /// is rejected by axum's `WebSocketUpgrade` extractor with 400 before the handler
    /// body runs. A real WS client always sends the header, so this case requires a
    /// plain HTTP GET via `oneshot`.
    #[tokio::test]
    async fn bridge_handshake_rejects_missing_upgrade_header() {
        let tmp = tempfile::TempDir::new().unwrap();
        let app = build_router(make_test_state(tmp.path()), None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "/api/v1/events/bridge?source={TRUSTED_PEER}&token=any-token"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 400);
    }
}
