use std::future::Future;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single telemetry event to be shipped to the CO ingest endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub universe_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
}

impl TelemetryEvent {
    pub fn new(
        universe_id: impl Into<String>,
        kind: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            universe_id: universe_id.into(),
            kind: kind.into(),
            payload,
        }
    }
}

/// Configuration for a CoAgent implementation.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Maximum events per batch before an automatic flush.
    pub batch_size: usize,
    /// Interval in seconds between time-based flushes.
    pub flush_interval_secs: u64,
    /// Total number of POST attempts (1 initial + N-1 retries) before dropping.
    pub max_retries: u32,
    /// Ring buffer capacity. When full, oldest events are dropped.
    pub ring_buffer_cap: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            batch_size: 200,
            flush_interval_secs: 10,
            max_retries: 3,
            ring_buffer_cap: 10_000,
        }
    }
}

/// Stable shipping contract for CO telemetry agents.
///
/// Implement this trait to add a new adapter variant:
/// - [`co_agent::FlySidecarAgent`] — tails container logs on a Fly Machine
/// - CF Worker tail (CO-124)
/// - Vercel Log Drain (CO-124)
/// - Browser SDK beacon (Phase 2)
pub trait CoAgent: Send + Sync {
    /// Buffer events and flush automatically when the batch size threshold is reached.
    fn ship(&self, events: &[TelemetryEvent]) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Emit a heartbeat (logs dropped-event counter, optionally pings ingest).
    fn heartbeat(&self) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Flush all buffered events immediately, regardless of batch size.
    fn flush(&self) -> impl Future<Output = anyhow::Result<()>> + Send;
}
