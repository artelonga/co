//! CO-384: Federated event envelope + bridge protocol framing.
//!
//! `FederatedEvent` extends CO-380's `Event` with cross-deployment metadata.
//! `BridgeMessage` is the wire protocol: either a replay request or a federated event.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::eda::event::{Event, Visibility};

/// Maximum hop count before dropping an event (loop guard).
pub const MAX_HOP_COUNT: u8 = 3;

// ---------------------------------------------------------------------------
// FederatedEvent
// ---------------------------------------------------------------------------

/// CO-380 Event extended with cross-deployment provenance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FederatedEvent {
    /// Original local event.
    pub event: Event,
    /// Deployment that first published this event (e.g. "yggdrasil.artelonga.com.br").
    pub origin_deployment: String,
    /// Identity of the bridge hop that signed/forwarded this event.
    pub signed_by: String,
    /// Timestamp when this bridge hop received the event.
    pub bridge_received_at: DateTime<Utc>,
    /// Prevents infinite federation loops. Incremented at each hop. Dropped when > MAX_HOP_COUNT.
    pub hop_count: u8,
}

impl FederatedEvent {
    pub fn new(event: Event, origin_deployment: String, signed_by: String) -> Self {
        Self {
            event,
            origin_deployment,
            signed_by,
            bridge_received_at: Utc::now(),
            hop_count: 0,
        }
    }

    /// `true` if this event's visibility allows cross-deployment federation.
    ///
    /// `UserOnly`, `UniverseOwner`, and `System` events are NEVER federated.
    pub fn is_federatable(visibility: &Visibility) -> bool {
        matches!(visibility, Visibility::Public | Visibility::UniverseMembers)
    }

    /// `true` if hop_count is within the loop-guard limit.
    pub fn within_hop_limit(&self) -> bool {
        self.hop_count <= MAX_HOP_COUNT
    }
}

// ---------------------------------------------------------------------------
// BridgeMessage — wire protocol
// ---------------------------------------------------------------------------

/// Messages exchanged over the bridge WebSocket connection.
///
/// On connect: the initiating side sends `ReplayRequest` with the last event
/// it received. The accepting side replays from `event_log` and then continues
/// with normal `Event` messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "bridge_msg_type")]
pub enum BridgeMessage {
    /// Sent immediately after WS upgrade to request missed-event replay.
    ReplayRequest {
        /// ULID of the last event received from the remote side. `None` = replay all.
        last_received_id: Option<String>,
    },
    /// A federated event forwarded across the bridge.
    Event {
        #[serde(flatten)]
        federated: FederatedEvent,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eda::event::{Event, Visibility};

    fn make_event(vis: Visibility) -> Event {
        Event {
            id: "01J0000000000000000000000A".into(),
            event_type: "entry.created".into(),
            universe_key: Some("u1".into()),
            user_id: None,
            payload: serde_json::json!({}),
            visibility: vis,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn public_is_federatable() {
        assert!(FederatedEvent::is_federatable(&Visibility::Public));
    }

    #[test]
    fn universe_members_is_federatable() {
        assert!(FederatedEvent::is_federatable(&Visibility::UniverseMembers));
    }

    #[test]
    fn private_visibilities_are_not_federatable() {
        assert!(!FederatedEvent::is_federatable(&Visibility::UserOnly));
        assert!(!FederatedEvent::is_federatable(&Visibility::UniverseOwner));
        assert!(!FederatedEvent::is_federatable(&Visibility::System));
    }

    #[test]
    fn hop_limit_guard() {
        let ev = make_event(Visibility::Public);
        let mut fed = FederatedEvent::new(ev, "src".into(), "src".into());
        assert!(fed.within_hop_limit());

        fed.hop_count = MAX_HOP_COUNT;
        assert!(fed.within_hop_limit());

        fed.hop_count = MAX_HOP_COUNT + 1;
        assert!(!fed.within_hop_limit());
    }

    #[test]
    fn replay_request_roundtrips() {
        let msg = BridgeMessage::ReplayRequest {
            last_received_id: Some("01J0000000000000000000000A".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("ReplayRequest"));
        let back: BridgeMessage = serde_json::from_str(&json).unwrap();
        if let BridgeMessage::ReplayRequest { last_received_id } = back {
            assert_eq!(
                last_received_id.as_deref(),
                Some("01J0000000000000000000000A")
            );
        } else {
            panic!("expected ReplayRequest");
        }
    }

    #[test]
    fn event_message_roundtrips() {
        let ev = make_event(Visibility::Public);
        let fed = FederatedEvent::new(ev.clone(), "origin".into(), "origin".into());
        let msg = BridgeMessage::Event { federated: fed };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("Event"));
        let back: BridgeMessage = serde_json::from_str(&json).unwrap();
        if let BridgeMessage::Event { federated } = back {
            assert_eq!(federated.origin_deployment, "origin");
            assert_eq!(federated.event.id, ev.id);
        } else {
            panic!("expected Event");
        }
    }
}
