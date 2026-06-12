//! CO-220: In-process domain event bus for decoupled cross-feature signaling.
//!
//! Replaces direct cross-feature calls:
//!   - `invitation_routes` → storage.create_notification()  ↦ `NotificationRequested`
//!   - `proposal_routes`   → storage.create_notification()  ↦ `NotificationRequested`
//!   - `entry_routes`      → embedding_worker::enqueue_*()  ↦ `EntryWritten` / `EntryDeleted`
//!   - `asset_routes`      → reference-card creation         ↦ `AssetUploaded`

use tokio::sync::broadcast;
use tracing::warn;

const BUS_CAPACITY: usize = 2_048;

/// Typed domain events emitted by route handlers.
#[derive(Debug, Clone)]
pub enum DomainEvent {
    EntryWritten {
        universe_key: String,
        path: String,
        body: String,
        body_hash: String,
    },
    EntryDeleted {
        universe_key: String,
        path: String,
    },
    NotificationRequested {
        recipient_id: String,
        kind: String,
        universe_key: Option<String>,
        room_id: Option<String>,
        actor_id: String,
        object_id: String,
        summary_key: String,
        summary_params: serde_json::Value,
    },
    InvitationAccepted {
        universe_key: String,
        user_id: String,
    },
    ProposalDecided {
        universe_key: String,
        proposal_path: String,
        action: String,
        proposer_id: String,
    },
    AssetUploaded {
        universe_key: String,
        sha256: String,
        mime: String,
        size_bytes: i64,
        user_id: String,
        filename: Option<String>,
    },
    /// CO-275: emitted by co-auto after each claude invocation completes.
    AgentSessionComplete {
        task_id: String,
        universe_key: String,
        duration_ms: u64,
        tokens_in: Option<u64>,
        tokens_out: Option<u64>,
        tool_calls: std::collections::HashMap<String, u32>,
        skills_loaded: Vec<String>,
        context_chars: u64,
        exit_code: i32,
        final_commit_sha: Option<String>,
        pr_number: Option<u32>,
    },
    /// CO-426: emitted when a usage report is ingested at `/api/v1/usage/sessions`.
    /// Bridged to `AnalyticsEvent::Usage` so the Gestão usage panel updates live.
    UsageReported {
        task_key: String,
        universe_key: String,
        machine: String,
        model: String,
        total_tokens: i64,
        cost_usd: Option<f64>,
        outcome: String,
    },
}

/// Coarse-grained filter for `Bus::subscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFilter {
    All,
    Entry,
    Notification,
    Asset,
    InvitationAccepted,
    ProposalDecided,
    AgentSession,
    Usage,
}

impl EventFilter {
    pub fn matches(&self, event: &DomainEvent) -> bool {
        match self {
            EventFilter::All => true,
            EventFilter::Entry => {
                matches!(
                    event,
                    DomainEvent::EntryWritten { .. } | DomainEvent::EntryDeleted { .. }
                )
            }
            EventFilter::Notification => {
                matches!(event, DomainEvent::NotificationRequested { .. })
            }
            EventFilter::Asset => matches!(event, DomainEvent::AssetUploaded { .. }),
            EventFilter::InvitationAccepted => {
                matches!(event, DomainEvent::InvitationAccepted { .. })
            }
            EventFilter::ProposalDecided => {
                matches!(event, DomainEvent::ProposalDecided { .. })
            }
            EventFilter::AgentSession => {
                matches!(event, DomainEvent::AgentSessionComplete { .. })
            }
            EventFilter::Usage => matches!(event, DomainEvent::UsageReported { .. }),
        }
    }
}

/// Thin in-process broadcast bus.  `Clone` is cheap (internally Arc-backed).
#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<DomainEvent>,
}

impl Bus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BUS_CAPACITY);
        Bus { tx }
    }

    /// Publish an event to all active subscribers.  Fire-and-forget; drops
    /// silently when no subscribers are listening.
    pub fn publish(&self, event: DomainEvent) {
        let _ = self.tx.send(event);
    }

    /// Return a filtered receiver for the given event category.
    pub fn subscribe(&self, filter: EventFilter) -> BusReceiver {
        BusReceiver {
            rx: self.tx.subscribe(),
            filter,
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Bus::new()
    }
}

/// A filtered receiver returned by `Bus::subscribe`.
pub struct BusReceiver {
    rx: broadcast::Receiver<DomainEvent>,
    filter: EventFilter,
}

impl BusReceiver {
    /// Await the next matching event.  Returns `None` when the bus is dropped.
    /// Lagged messages are skipped with a warning.
    pub async fn recv(&mut self) -> Option<DomainEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) if self.filter.matches(&event) => return Some(event),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("event bus: receiver lagged, skipped {n} messages");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_and_subscribe_all() {
        let bus = Bus::new();
        let mut rx = bus.subscribe(EventFilter::All);
        bus.publish(DomainEvent::EntryWritten {
            universe_key: "u1".into(),
            path: "note.md".into(),
            body: "hello".into(),
            body_hash: "abc".into(),
        });
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, DomainEvent::EntryWritten { .. }));
    }

    #[tokio::test]
    async fn filter_blocks_non_matching() {
        let bus = Bus::new();
        let mut rx = bus.subscribe(EventFilter::Notification);

        bus.publish(DomainEvent::EntryDeleted {
            universe_key: "u1".into(),
            path: "old.md".into(),
        });
        bus.publish(DomainEvent::NotificationRequested {
            recipient_id: "user1".into(),
            kind: "universe.invitation".into(),
            universe_key: Some("u1".into()),
            room_id: None,
            actor_id: "inviter".into(),
            object_id: "token123".into(),
            summary_key: "notif.invitation".into(),
            summary_params: serde_json::json!({}),
        });
        let ev = rx.recv().await.unwrap();
        assert!(
            matches!(ev, DomainEvent::NotificationRequested { .. }),
            "entry event should have been filtered out"
        );
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let bus = Bus::new();
        let mut rx1 = bus.subscribe(EventFilter::Entry);
        let mut rx2 = bus.subscribe(EventFilter::Entry);

        bus.publish(DomainEvent::EntryWritten {
            universe_key: "u2".into(),
            path: "doc.md".into(),
            body: "content".into(),
            body_hash: "def".into(),
        });

        let ev1 = rx1.recv().await.unwrap();
        let ev2 = rx2.recv().await.unwrap();
        assert!(matches!(ev1, DomainEvent::EntryWritten { .. }));
        assert!(matches!(ev2, DomainEvent::EntryWritten { .. }));
    }

    #[tokio::test]
    async fn asset_filter_matches_uploaded() {
        let bus = Bus::new();
        let mut rx = bus.subscribe(EventFilter::Asset);

        bus.publish(DomainEvent::AssetUploaded {
            universe_key: "u1".into(),
            sha256: "a".repeat(64),
            mime: "application/pdf".into(),
            size_bytes: 1024,
            user_id: "user1".into(),
            filename: Some("doc.pdf".into()),
        });

        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, DomainEvent::AssetUploaded { .. }));
    }

    #[tokio::test]
    async fn entry_filter_matches_both_written_and_deleted() {
        let bus = Bus::new();
        let mut rx = bus.subscribe(EventFilter::Entry);

        bus.publish(DomainEvent::EntryWritten {
            universe_key: "u1".into(),
            path: "a.md".into(),
            body: "x".into(),
            body_hash: "h1".into(),
        });
        bus.publish(DomainEvent::EntryDeleted {
            universe_key: "u1".into(),
            path: "b.md".into(),
        });

        let ev1 = rx.recv().await.unwrap();
        let ev2 = rx.recv().await.unwrap();
        assert!(matches!(ev1, DomainEvent::EntryWritten { .. }));
        assert!(matches!(ev2, DomainEvent::EntryDeleted { .. }));
    }

    #[tokio::test]
    async fn no_subscribers_publish_does_not_panic() {
        let bus = Bus::new();
        bus.publish(DomainEvent::EntryDeleted {
            universe_key: "u".into(),
            path: "x.md".into(),
        });
    }
}
