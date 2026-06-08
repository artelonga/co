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
pub fn trusted_sources_from_env() -> Vec<String> {
    std::env::var("CO_BRIDGE_TRUSTED_SOURCES")
        .unwrap_or_default()
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
    std::env::var("CO_BRIDGE_HEARTBEAT_S")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
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
    let id = format!("{source}:{target}");
    let now = Utc::now().to_rfc3339();
    if let Err(e) = st.conn().execute(
        "INSERT INTO bridge_state (id, source_deployment, target_deployment,
            last_delivered_event_id, last_connected_at, last_disconnected_at, state)
         VALUES (?1, ?2, ?3, ?4, CASE WHEN ?5 = 'connected' THEN ?6 ELSE NULL END,
                 CASE WHEN ?5 = 'disconnected' THEN ?6 ELSE NULL END, ?5)
         ON CONFLICT(id) DO UPDATE SET
             state = excluded.state,
             last_connected_at = CASE WHEN ?5 = 'connected' THEN ?6 ELSE last_connected_at END,
             last_disconnected_at = CASE WHEN ?5 = 'disconnected' THEN ?6 ELSE last_disconnected_at END,
             last_delivered_event_id = COALESCE(?4, last_delivered_event_id)",
        rusqlite::params![id, source, target, last_event_id, state, now],
    ) {
        warn!("EDA bridge: bridge_state upsert failed: {e}");
    }
}

/// Load events from `event_log` published after `since_id` (exclusive).
///
/// Returns at most 1000 events in ULID order (chronological).
fn load_events_since(storage: &Arc<Mutex<Storage>>, since_id: Option<&str>) -> Vec<Event> {
    let st = storage.lock();
    let since = since_id.unwrap_or("");
    let mut stmt = match st.conn().prepare(
        "SELECT id, event_type, universe_key, user_id, payload_json, visibility, created_at
         FROM event_log
         WHERE id > ?1
         ORDER BY id ASC
         LIMIT 1000",
    ) {
        Ok(s) => s,
        Err(e) => {
            warn!("EDA bridge: event_log query prepare failed: {e}");
            return vec![];
        }
    };

    let rows = stmt.query_map(rusqlite::params![since], |row| {
        let id: String = row.get(0)?;
        let event_type: String = row.get(1)?;
        let universe_key: Option<String> = row.get(2)?;
        let user_id: Option<String> = row.get(3)?;
        let payload_json: String = row.get(4)?;
        let visibility_str: String = row.get(5)?;
        let created_at_str: String = row.get(6)?;

        let payload = serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);
        let visibility = match visibility_str.as_str() {
            "UniverseMembers" => Visibility::UniverseMembers,
            "UniverseOwner" => Visibility::UniverseOwner,
            "UserOnly" => Visibility::UserOnly,
            "System" => Visibility::System,
            _ => Visibility::Public,
        };
        let created_at = created_at_str
            .parse::<chrono::DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now());

        Ok(Event {
            id,
            event_type,
            universe_key,
            user_id,
            payload,
            visibility,
            created_at,
        })
    });

    match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            warn!("EDA bridge: event_log query failed: {e}");
            vec![]
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
}
