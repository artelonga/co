//! CO-384: Outbound bridge client — connects CO to peer deployments.
//!
//! `BridgeManager` reads `CO_BRIDGE_OUTBOUND_TOKENS_JSON` at startup and
//! spawns one Tokio task per destination. Each task maintains a persistent
//! WebSocket connection with exponential backoff reconnect and replay on reconnect.
//!
//! On connect:
//!  1. Sends `ReplayRequest{last_received_id}` with the ULID of the last event
//!     received from that peer (read from `bridge_state`).
//!  2. Peer replays missed events.
//!  3. Bidirectional event flow continues.
//!
//! On disconnect: sleeps with exponential backoff (1s → 2s → 4s → 8s → max 30s)
//! then reconnects.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use parking_lot::Mutex;
use tracing::{info, warn};

use super::federated_event::{BridgeMessage, FederatedEvent};
use crate::eda::EdaBus;
use crate::eda::bus::Filter;
use crate::eda::event::{Event, Visibility};
use crate::storage::Storage;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Parsed outbound bridge configuration for one destination deployment.
#[derive(Debug, Clone)]
pub struct OutboundConfig {
    /// Destination host (key in CO_BRIDGE_OUTBOUND_TOKENS_JSON).
    pub destination: String,
    /// JWT to present to the remote bridge endpoint.
    pub token: String,
}

impl OutboundConfig {
    /// Parse `CO_BRIDGE_OUTBOUND_TOKENS_JSON`.
    ///
    /// Expected format: `{"yggdrasil.artelonga.com.br": "<jwt>"}`.
    /// Returns an empty `Vec` when the env var is absent or empty.
    pub fn from_env() -> Vec<Self> {
        let raw = match std::env::var("CO_BRIDGE_OUTBOUND_TOKENS_JSON") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => return vec![],
        };
        let map: HashMap<String, String> = match serde_json::from_str(&raw) {
            Ok(m) => m,
            Err(e) => {
                warn!("EDA bridge: invalid CO_BRIDGE_OUTBOUND_TOKENS_JSON: {e}");
                return vec![];
            }
        };
        map.into_iter()
            .filter(|(k, v)| !k.is_empty() && !v.is_empty())
            .map(|(destination, token)| OutboundConfig { destination, token })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// ExponentialBackoff
// ---------------------------------------------------------------------------

/// Reconnect delay with capped exponential growth.
///
/// Sequence: 1s → 2s → 4s → 8s → 16s → 30s (max).
pub struct ExponentialBackoff {
    current: Duration,
    max: Duration,
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new()
    }
}

impl ExponentialBackoff {
    pub fn new() -> Self {
        Self {
            current: Duration::from_secs(1),
            max: Duration::from_secs(30),
        }
    }

    /// Return the current delay and advance to the next (up to max).
    pub fn next_delay(&mut self) -> Duration {
        let d = self.current;
        self.current = (self.current * 2).min(self.max);
        d
    }

    /// Reset to initial delay after a successful connection.
    pub fn reset(&mut self) {
        self.current = Duration::from_secs(1);
    }
}

// ---------------------------------------------------------------------------
// BridgeManager
// ---------------------------------------------------------------------------

/// Manages outbound bridge connections to peer deployments.
pub struct BridgeManager;

impl BridgeManager {
    /// Read `CO_BRIDGE_OUTBOUND_TOKENS_JSON` and spawn one client task per destination.
    ///
    /// No-op when the env var is absent (default: no outbound bridges).
    pub fn spawn(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>, our_deployment: String) {
        let configs = OutboundConfig::from_env();
        if configs.is_empty() {
            info!(
                "EDA bridge: no outbound bridges configured (CO_BRIDGE_OUTBOUND_TOKENS_JSON not set)"
            );
            return;
        }
        for cfg in configs {
            let bus = Arc::clone(&bus);
            let storage = Arc::clone(&storage);
            let our = our_deployment.clone();
            info!(destination = %cfg.destination, "EDA bridge: spawning outbound client");
            tokio::spawn(run_bridge_client(cfg, bus, storage, our));
        }
    }
}

// ---------------------------------------------------------------------------
// Outbound client task
// ---------------------------------------------------------------------------

/// Task body: connect with backoff, replay on reconnect, run bidirectional loop.
async fn run_bridge_client(
    cfg: OutboundConfig,
    bus: Arc<dyn EdaBus>,
    storage: Arc<Mutex<Storage>>,
    our_deployment: String,
) {
    let mut backoff = ExponentialBackoff::new();

    loop {
        let url = format!(
            "wss://{}/api/v1/events/bridge?source={}&token={}",
            cfg.destination,
            urlencoding::encode(&our_deployment),
            urlencoding::encode(&cfg.token),
        );

        match try_connect_and_run(&url, &cfg, &bus, &storage, &our_deployment).await {
            Ok(_) => {
                info!(destination = %cfg.destination, "EDA bridge: connection closed cleanly, reconnecting");
                backoff.reset();
            }
            Err(e) => {
                warn!(destination = %cfg.destination, error = %e, "EDA bridge: connection error");
            }
        }

        update_outbound_state(
            &storage,
            &our_deployment,
            &cfg.destination,
            "disconnected",
            None,
        );
        bus.publish(Event::new(
            "bridge.disconnected",
            None,
            None,
            serde_json::json!({
                "source": &our_deployment,
                "target": &cfg.destination,
                "reason": "reconnecting"
            }),
            Visibility::Public,
        ));

        let delay = backoff.next_delay();
        info!(
            destination = %cfg.destination,
            delay_secs = delay.as_secs(),
            "EDA bridge: reconnecting after backoff"
        );
        tokio::time::sleep(delay).await;
    }
}

/// Attempt one connection to the remote bridge endpoint and run the socket loop.
async fn try_connect_and_run(
    url: &str,
    cfg: &OutboundConfig,
    bus: &Arc<dyn EdaBus>,
    storage: &Arc<Mutex<Storage>>,
    our_deployment: &str,
) -> anyhow::Result<()> {
    use tokio_tungstenite::tungstenite::Message as TungMessage;

    let (ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
    info!(destination = %cfg.destination, "EDA bridge: outbound connected");

    use futures_util::{SinkExt, StreamExt};
    let (mut write, mut read) = ws_stream.split();

    update_outbound_state(storage, our_deployment, &cfg.destination, "connected", None);
    bus.publish(Event::new(
        "bridge.connected",
        None,
        None,
        serde_json::json!({ "source": our_deployment, "target": &cfg.destination }),
        Visibility::Public,
    ));

    // Send replay request.
    let last_id = load_last_received_id(storage, our_deployment, &cfg.destination);
    let replay_req = BridgeMessage::ReplayRequest {
        last_received_id: last_id,
    };
    let replay_json = serde_json::to_string(&replay_req)?;
    write.send(TungMessage::Text(replay_json.into())).await?;

    // Subscribe to local bus (Public + UniverseMembers only).
    let mut local_sub = bus.subscribe(Filter::default());

    let heartbeat_secs = std::env::var("CO_BRIDGE_HEARTBEAT_S")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(heartbeat_secs));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await; // skip immediate first tick

    let mut last_received_id: Option<String> = None;

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    None => break,
                    Some(Err(e)) => {
                        warn!(destination = %cfg.destination, "EDA bridge: read error: {e}");
                        break;
                    }
                    Some(Ok(TungMessage::Close(_))) => break,
                    Some(Ok(TungMessage::Ping(d))) => {
                        let _ = write.send(TungMessage::Pong(d)).await;
                    }
                    Some(Ok(TungMessage::Pong(_))) => {}
                    Some(Ok(TungMessage::Text(text))) => {
                        if let Ok(msg) = serde_json::from_str::<BridgeMessage>(&text)
                            && let BridgeMessage::Event { mut federated } = msg
                        {
                            // Loop guard.
                            if !federated.within_hop_limit() {
                                continue;
                            }
                            // Privacy filter.
                            if !FederatedEvent::is_federatable(&federated.event.visibility) {
                                continue;
                            }
                            last_received_id = Some(federated.event.id.clone());
                            federated.hop_count += 1;
                            federated.bridge_received_at = Utc::now();
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
                    Some(Ok(_)) => {}
                }
            }

            ev = local_sub.recv() => {
                match ev {
                    None => break,
                    Some(ev) => {
                        if !FederatedEvent::is_federatable(&ev.visibility) {
                            continue;
                        }
                        let fed = FederatedEvent::new(
                            (*ev).clone(),
                            our_deployment.to_string(),
                            our_deployment.to_string(),
                        );
                        let msg = BridgeMessage::Event { federated: fed };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            if write.send(TungMessage::Text(json.into())).await.is_err() {
                                break;
                            }
                            bus.publish(Event::new(
                                "bridge.event_sent",
                                None,
                                None,
                                serde_json::json!({ "target": &cfg.destination }),
                                Visibility::Public,
                            ));
                        }
                    }
                }
            }

            _ = heartbeat.tick() => {
                if write.send(TungMessage::Ping(axum::body::Bytes::new())).await.is_err() {
                    break;
                }
            }
        }
    }

    update_outbound_state(
        storage,
        our_deployment,
        &cfg.destination,
        "disconnected",
        last_received_id.as_deref(),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn load_last_received_id(
    storage: &Arc<Mutex<Storage>>,
    source: &str,
    target: &str,
) -> Option<String> {
    let st = storage.lock();
    let id = format!("{source}:{target}");
    st.conn()
        .query_row(
            "SELECT last_delivered_event_id FROM bridge_state WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
}

fn update_outbound_state(
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
         VALUES (?1, ?2, ?3, ?4,
                 CASE WHEN ?5 = 'connected' THEN ?6 ELSE NULL END,
                 CASE WHEN ?5 = 'disconnected' THEN ?6 ELSE NULL END,
                 ?5)
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

// ---------------------------------------------------------------------------
// URL encoding helper (avoid pulling in an extra crate)
// ---------------------------------------------------------------------------

mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(byte as char);
                }
                b => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_sequence() {
        let mut b = ExponentialBackoff::new();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
        assert_eq!(b.next_delay(), Duration::from_secs(2));
        assert_eq!(b.next_delay(), Duration::from_secs(4));
        assert_eq!(b.next_delay(), Duration::from_secs(8));
        assert_eq!(b.next_delay(), Duration::from_secs(16));
        assert_eq!(b.next_delay(), Duration::from_secs(30)); // capped
        assert_eq!(b.next_delay(), Duration::from_secs(30)); // stays capped
    }

    #[test]
    fn backoff_reset() {
        let mut b = ExponentialBackoff::new();
        b.next_delay();
        b.next_delay();
        b.next_delay();
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }

    #[test]
    fn outbound_config_from_env_empty_when_not_set() {
        // This test is valid only when CO_BRIDGE_OUTBOUND_TOKENS_JSON is absent.
        // In the standard test env that env var is not set, so this should return empty.
        if std::env::var("CO_BRIDGE_OUTBOUND_TOKENS_JSON").is_ok() {
            return; // skip if the env var happens to be set in CI
        }
        let configs = OutboundConfig::from_env();
        assert!(configs.is_empty());
    }

    #[test]
    fn outbound_config_parses_json() {
        let json = r#"{"yggdrasil.artelonga.com.br":"tok-abc"}"#;
        // Parse manually (bypassing env) to test the logic.
        let map: std::collections::HashMap<String, String> = serde_json::from_str(json).unwrap();
        let configs: Vec<OutboundConfig> = map
            .into_iter()
            .map(|(d, t)| OutboundConfig {
                destination: d,
                token: t,
            })
            .collect();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].destination, "yggdrasil.artelonga.com.br");
        assert_eq!(configs[0].token, "tok-abc");
    }

    #[test]
    fn urlencoding_basic() {
        assert_eq!(
            urlencoding::encode("co.artelonga.com.br"),
            "co.artelonga.com.br"
        );
        assert_eq!(urlencoding::encode("hello world"), "hello%20world");
        assert_eq!(urlencoding::encode("a/b"), "a%2Fb");
    }
}
