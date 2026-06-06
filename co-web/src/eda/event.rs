//! CO-380: Universal EDA event types.
//!
//! `Event` is the single envelope that every state-changing route publishes
//! to the `EdaBus`. Subscribers filter by `event_type` and `visibility`.
//! IDs are ULID-format (chronologically sortable, 26-char Crockford base32).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Crockford base32 ULID generator (no external crate needed)
// ---------------------------------------------------------------------------

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Generate a ULID: 48-bit ms timestamp || 80 random bits, Crockford base32.
pub fn new_ulid() -> String {
    let ts_ms = Utc::now().timestamp_millis() as u128;
    let rand_a = rand::random::<u64>() as u128;
    let rand_b = rand::random::<u64>() as u128;
    // 80 random bits: upper 16 bits from rand_a, lower 64 from rand_b
    let rand_bits: u128 = ((rand_a & 0xFFFF) << 64) | rand_b;
    let n: u128 = (ts_ms << 80) | rand_bits;

    let mut chars = [0u8; 26];
    let mut v = n;
    for c in chars.iter_mut().rev() {
        *c = CROCKFORD[(v & 0x1f) as usize];
        v >>= 5;
    }
    // SAFETY: CROCKFORD contains only ASCII bytes.
    unsafe { String::from_utf8_unchecked(chars.to_vec()) }
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// Determines who may receive this event from a subscriber.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Aggregated-only data for unauthenticated observers.
    Public,
    /// Full data for members of the event's universe.
    UniverseMembers,
    /// Full data for the universe owner only.
    UniverseOwner,
    /// Only the user the event is about.
    UserOnly,
    /// Admin/system consumers only — never forwarded to WebSocket clients.
    System,
}

// ---------------------------------------------------------------------------
// Event
// ---------------------------------------------------------------------------

/// Universal event envelope published to the `EdaBus`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    /// ULID — chronologically sortable unique identifier.
    pub id: String,
    /// Dot-namespaced type: `"entry.created"`, `"analytics.visit"`, etc.
    pub event_type: String,
    /// `None` for system-wide events.
    pub universe_key: Option<String>,
    /// `None` for anonymous or system-generated events.
    pub user_id: Option<String>,
    /// Event-specific payload — schema is producer-defined.
    pub payload: serde_json::Value,
    /// Who may observe this event (enforced server-side before WebSocket push).
    pub visibility: Visibility,
    pub created_at: DateTime<Utc>,
}

impl Event {
    pub fn new(
        event_type: impl Into<String>,
        universe_key: Option<String>,
        user_id: Option<String>,
        payload: serde_json::Value,
        visibility: Visibility,
    ) -> Self {
        Self {
            id: new_ulid(),
            event_type: event_type.into(),
            universe_key,
            user_id,
            payload,
            visibility,
            created_at: Utc::now(),
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
    fn ulid_is_26_chars_alphanumeric() {
        let id = new_ulid();
        assert_eq!(id.len(), 26, "ULID must be 26 chars, got: {id}");
        assert!(
            id.chars().all(|c| c.is_ascii_alphanumeric()),
            "ULID must be ASCII alphanumeric, got: {id}"
        );
    }

    #[test]
    fn ulid_is_chronologically_sortable() {
        let a = new_ulid();
        // Give clock a chance to tick (chrono is monotonic at ms granularity)
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = new_ulid();
        assert!(a < b, "older ULID should sort before newer: {a} vs {b}");
    }

    #[test]
    fn ulid_ids_are_unique() {
        let ids: std::collections::HashSet<_> = (0..100).map(|_| new_ulid()).collect();
        assert_eq!(ids.len(), 100, "all 100 ULIDs should be unique");
    }

    #[test]
    fn visibility_ordering() {
        assert!(Visibility::Public < Visibility::System);
        assert!(Visibility::UniverseMembers > Visibility::Public);
    }

    #[test]
    fn event_new_sets_id_and_type() {
        let ev = Event::new(
            "entry.created",
            Some("u1".into()),
            None,
            serde_json::json!({"path": "note.md"}),
            Visibility::UniverseMembers,
        );
        assert_eq!(ev.event_type, "entry.created");
        assert_eq!(ev.universe_key.as_deref(), Some("u1"));
        assert_eq!(ev.id.len(), 26);
    }
}
