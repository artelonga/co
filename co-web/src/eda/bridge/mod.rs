//! CO-384: Federated event bus bridge — cross-deployment WS pub/sub.
//!
//! Extends CO-380's in-process `EdaBus` with cross-deployment fan-out via
//! persistent WebSocket connections. Star-with-bridges topology: each deployment
//! runs its own bus; bridges connect pairs of deployments.
//!
//! # Components
//!
//! - `FederatedEvent` — CO-380 Event wrapped with origin + hop metadata.
//! - `BridgeMessage` — wire protocol: ReplayRequest | Event.
//! - `bridge_ws_handler` — inbound WS endpoint (`/api/v1/events/bridge`).
//! - `BridgeManager` — spawns outbound connections from `CO_BRIDGE_OUTBOUND_TOKENS_JSON`.
//!
//! # Privacy rules (enforced at bridge level)
//!
//! Only `Public` and `UniverseMembers` events cross the bridge.
//! `UserOnly`, `UniverseOwner`, and `System` events remain local.
//!
//! # Loop guard
//!
//! `hop_count > MAX_HOP_COUNT (3)` → event dropped silently.

pub mod client;
pub mod federated_event;
pub mod handler;

pub use client::{BridgeManager, OutboundConfig};
pub use federated_event::{BridgeMessage, FederatedEvent, MAX_HOP_COUNT};
pub use handler::bridge_ws_handler;

/// Our own deployment identifier — used as `origin_deployment` in federated events.
///
/// Reads (in priority order):
///  1. `CO_DEPLOYMENT_ID` env var
///  2. `FLY_APP_NAME` (Fly.io machines)
///  3. Falls back to `"co-local"`
pub fn our_deployment_id() -> String {
    // CO-434: routed through the process-global SecretsProvider seam.
    let secrets = crate::infra::secrets::global();
    secrets
        .get("CO_DEPLOYMENT_ID")
        .or_else(|| secrets.get("FLY_APP_NAME"))
        .unwrap_or_else(|| "co-local".into())
}
