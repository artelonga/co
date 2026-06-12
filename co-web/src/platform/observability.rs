//! CO-329: In-process analytics buffer + real-time telemetry.
//!
//! A ring buffer (last 1000 events) feeds the `/analytics` WebSocket stream.
//! Events arrive from three sources:
//!   - HTTP request middleware (path, status, latency)
//!   - Event bus subscriber (domain events, agent sessions)
//!   - Periodic worker-status snapshots from InProcessExecutor
//!
//! The buffer is a process-level singleton initialised once at startup via
//! `init_buffer()`.  Handlers access it via `buffer()`.

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnalyticsEvent {
    Request {
        method: String,
        path: String,
        status: u16,
        duration_ms: u64,
        ts: DateTime<Utc>,
    },
    Domain {
        kind: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        universe_key: Option<String>,
        detail: String,
        ts: DateTime<Utc>,
    },
    AgentSession {
        task_id: String,
        universe_key: String,
        duration_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        tokens_in: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tokens_out: Option<u64>,
        exit_code: i32,
        ts: DateTime<Utc>,
    },
    WorkerSnapshot {
        workers: Vec<WorkerInfo>,
        ts: DateTime<Utc>,
    },
    /// CO-426: a usage report landed (co-auto POSTed token/cost for a session).
    /// The Gestão usage panel increments live totals from these.
    Usage {
        task_key: String,
        universe_key: String,
        machine: String,
        model: String,
        total_tokens: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        outcome: String,
        ts: DateTime<Utc>,
    },
    /// CO-426: full snapshot of currently-active launchers, broadcast on every
    /// heartbeat (TTL-filtered server-side). The panel replaces its list on each
    /// one — so a crashed machine drops off as soon as any other launcher beats.
    LauncherSnapshot {
        launchers: Vec<LauncherInfo>,
        ts: DateTime<Utc>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkerInfo {
    pub name: String,
    pub running: bool,
    pub tick_count: u64,
    pub error_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tick_at: Option<DateTime<Utc>>,
}

impl From<crate::worker_supervisor::WorkerStatus> for WorkerInfo {
    fn from(s: crate::worker_supervisor::WorkerStatus) -> Self {
        Self {
            name: s.name,
            running: s.running,
            tick_count: s.tick_count,
            error_count: s.error_count,
            last_tick_at: s.last_tick_at,
        }
    }
}

// ---------------------------------------------------------------------------
// CO-426: active-launcher registry (in-memory, TTL)
// ---------------------------------------------------------------------------

/// One launcher (a co-auto process on some machine) that is currently working a
/// task. Kept in memory only — a crashed machine simply stops heartbeating and
/// ages out of [`ActiveLaunchers`] after [`LAUNCHER_TTL`], no DELETE required.
#[derive(Clone, Debug, Serialize)]
pub struct LauncherInfo {
    pub task_key: String,
    pub universe_key: String,
    pub machine: String,
    pub model: String,
    /// Epoch seconds the launcher started this task.
    pub started_at: i64,
    /// When the most recent heartbeat arrived (server clock).
    pub last_seen: DateTime<Utc>,
    /// Optional free-form progress note from the launcher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
}

/// A heartbeat with no follow-up within this window is considered dead.
const LAUNCHER_TTL: chrono::Duration = chrono::Duration::seconds(90);

/// In-memory registry of active launchers, keyed by `machine:task_key`.
///
/// Lives in `CoreState` (one per server) — *not* a process-global singleton —
/// so parallel integration tests each get an isolated registry.
#[derive(Default)]
pub struct ActiveLaunchers {
    inner: Mutex<std::collections::HashMap<String, LauncherInfo>>,
}

impl ActiveLaunchers {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(machine: &str, task_key: &str) -> String {
        format!("{machine}:{task_key}")
    }

    /// Record/refresh a heartbeat. `last_seen` is stamped to `Utc::now()`.
    pub fn heartbeat(&self, mut info: LauncherInfo) {
        info.last_seen = Utc::now();
        let key = Self::key(&info.machine, &info.task_key);
        self.inner.lock().insert(key, info);
    }

    /// The currently-active launchers (heartbeat within TTL of `now`), pruning
    /// any that have aged out. Sorted by `started_at` descending (newest first).
    pub fn active_as_of(&self, now: DateTime<Utc>) -> Vec<LauncherInfo> {
        let mut map = self.inner.lock();
        map.retain(|_, v| now.signed_duration_since(v.last_seen) <= LAUNCHER_TTL);
        let mut list: Vec<LauncherInfo> = map.values().cloned().collect();
        list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        list
    }

    /// Active launchers as of the wall clock now.
    pub fn active(&self) -> Vec<LauncherInfo> {
        self.active_as_of(Utc::now())
    }
}

// ---------------------------------------------------------------------------
// Ring buffer
// ---------------------------------------------------------------------------

const RING_CAPACITY: usize = 1000;
const BROADCAST_CAPACITY: usize = 512;

pub struct AnalyticsBuffer {
    ring: Mutex<VecDeque<AnalyticsEvent>>,
    tx: broadcast::Sender<AnalyticsEvent>,
}

impl AnalyticsBuffer {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            ring: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            tx,
        }
    }

    pub fn push(&self, event: AnalyticsEvent) {
        let mut ring = self.ring.lock();
        if ring.len() >= RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(event.clone());
        drop(ring);
        let _ = self.tx.send(event);
    }

    /// Snapshot of buffered events in chronological order.
    pub fn snapshot(&self) -> Vec<AnalyticsEvent> {
        self.ring.lock().iter().cloned().collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AnalyticsEvent> {
        self.tx.subscribe()
    }
}

impl Default for AnalyticsBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Process-level singleton
// ---------------------------------------------------------------------------

static BUFFER: OnceLock<Arc<AnalyticsBuffer>> = OnceLock::new();

/// Initialise the global buffer (call once at server startup).
/// Subsequent calls are no-ops; the first buffer is always returned.
pub fn init_buffer() -> Arc<AnalyticsBuffer> {
    let buf = Arc::new(AnalyticsBuffer::new());
    let _ = BUFFER.set(Arc::clone(&buf));
    BUFFER.get().cloned().unwrap_or(buf)
}

/// Access the global buffer.  Returns `None` before `init_buffer()` is called.
pub fn buffer() -> Option<Arc<AnalyticsBuffer>> {
    BUFFER.get().cloned()
}

// ---------------------------------------------------------------------------
// Domain event → AnalyticsEvent conversion
// ---------------------------------------------------------------------------

pub fn from_domain_event(event: &crate::events::DomainEvent) -> Option<AnalyticsEvent> {
    let ts = Utc::now();
    match event {
        crate::events::DomainEvent::EntryWritten {
            universe_key, path, ..
        } => Some(AnalyticsEvent::Domain {
            kind: "entry.written".into(),
            universe_key: Some(universe_key.clone()),
            detail: path.clone(),
            ts,
        }),
        crate::events::DomainEvent::EntryDeleted { universe_key, path } => {
            Some(AnalyticsEvent::Domain {
                kind: "entry.deleted".into(),
                universe_key: Some(universe_key.clone()),
                detail: path.clone(),
                ts,
            })
        }
        crate::events::DomainEvent::AssetUploaded {
            universe_key,
            filename,
            ..
        } => Some(AnalyticsEvent::Domain {
            kind: "asset.uploaded".into(),
            universe_key: Some(universe_key.clone()),
            detail: filename.clone().unwrap_or_default(),
            ts,
        }),
        crate::events::DomainEvent::InvitationAccepted {
            universe_key,
            user_id,
        } => Some(AnalyticsEvent::Domain {
            kind: "invitation.accepted".into(),
            universe_key: Some(universe_key.clone()),
            detail: user_id.clone(),
            ts,
        }),
        crate::events::DomainEvent::ProposalDecided {
            universe_key,
            action,
            proposal_path,
            ..
        } => Some(AnalyticsEvent::Domain {
            kind: format!("proposal.{action}"),
            universe_key: Some(universe_key.clone()),
            detail: proposal_path.clone(),
            ts,
        }),
        crate::events::DomainEvent::AgentSessionComplete {
            task_id,
            universe_key,
            duration_ms,
            tokens_in,
            tokens_out,
            exit_code,
            ..
        } => Some(AnalyticsEvent::AgentSession {
            task_id: task_id.clone(),
            universe_key: universe_key.clone(),
            duration_ms: *duration_ms,
            tokens_in: *tokens_in,
            tokens_out: *tokens_out,
            exit_code: *exit_code,
            ts,
        }),
        crate::events::DomainEvent::UsageReported {
            task_key,
            universe_key,
            machine,
            model,
            total_tokens,
            cost_usd,
            outcome,
        } => Some(AnalyticsEvent::Usage {
            task_key: task_key.clone(),
            universe_key: universe_key.clone(),
            machine: machine.clone(),
            model: model.clone(),
            total_tokens: *total_tokens,
            cost_usd: *cost_usd,
            outcome: outcome.clone(),
            ts,
        }),
        crate::events::DomainEvent::NotificationRequested { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// HTTP request middleware
// ---------------------------------------------------------------------------

use axum::body::Body;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

pub async fn request_middleware(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().to_string();
    let start = std::time::Instant::now();
    let resp = next.run(req).await;

    // Track meaningful API/page paths; skip static assets to reduce noise.
    let is_tracked = path == "/analytics"
        || (!path.starts_with("/_app/") && !path.starts_with("/static/") && !path.contains('.'));
    if is_tracked && let Some(buf) = buffer() {
        buf.push(AnalyticsEvent::Request {
            method,
            path,
            status: resp.status().as_u16(),
            duration_ms: start.elapsed().as_millis() as u64,
            ts: Utc::now(),
        });
    }

    resp
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buf() -> AnalyticsBuffer {
        AnalyticsBuffer::new()
    }

    #[test]
    fn push_and_snapshot() {
        let buf = make_buf();
        buf.push(AnalyticsEvent::Domain {
            kind: "entry.written".into(),
            universe_key: Some("u1".into()),
            detail: "note.md".into(),
            ts: Utc::now(),
        });
        let snap = buf.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(matches!(&snap[0], AnalyticsEvent::Domain { kind, .. } if kind == "entry.written"));
    }

    #[test]
    fn ring_evicts_oldest_at_capacity() {
        let buf = make_buf();
        for i in 0..RING_CAPACITY + 5 {
            buf.push(AnalyticsEvent::Request {
                method: "GET".into(),
                path: format!("/path/{i}"),
                status: 200,
                duration_ms: 1,
                ts: Utc::now(),
            });
        }
        assert_eq!(buf.snapshot().len(), RING_CAPACITY);
    }

    #[tokio::test]
    async fn subscribe_receives_pushed_event() {
        let buf = make_buf();
        let mut rx = buf.subscribe();
        buf.push(AnalyticsEvent::Domain {
            kind: "test".into(),
            universe_key: None,
            detail: "ok".into(),
            ts: Utc::now(),
        });
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, AnalyticsEvent::Domain { kind, .. } if kind == "test"));
    }
}
