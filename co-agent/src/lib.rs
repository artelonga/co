/// co-sync config file format.
pub mod sync_config;
/// CO-151: filesystem watcher + sync WebSocket client.
pub mod watcher;

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use rand::Rng;
use sha2::{Digest, Sha256};

use co::agent::{AgentConfig, CoAgent, TelemetryEvent};

type HmacSha256 = Hmac<Sha256>;

/// Fly sidecar implementation of the [`CoAgent`] trait.
///
/// Tails log lines from a configurable source (stdin / named pipe / HTTP-localhost),
/// batches events, zstd-compresses the JSON-Lines payload, signs with
/// HMAC-SHA256, and POSTs to the CO ingest endpoint.
///
/// ## Overflow policy
///
/// The ring buffer holds at most `config.ring_buffer_cap` events.  When full,
/// the **oldest** event is dropped and `dropped_count` is incremented.  The
/// counter is emitted on every heartbeat so operators can alert on it.
///
/// ## Retry policy
///
/// On 5xx or network error: exponential backoff with jitter, up to
/// `config.max_retries` total attempts, then drop.
/// On 4xx: log + drop immediately (no retry — the payload is bad, not the server).
pub struct FlySidecarAgent {
    ingest_url: String,
    universe_id: String,
    hmac_key: Vec<u8>,
    batch: Mutex<VecDeque<TelemetryEvent>>,
    config: AgentConfig,
    client: reqwest::Client,
    pub dropped_count: AtomicU64,
}

impl FlySidecarAgent {
    pub fn new(
        ingest_url: impl Into<String>,
        universe_id: impl Into<String>,
        hmac_key: Vec<u8>,
        config: AgentConfig,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("build reqwest client")?;
        let cap = config.ring_buffer_cap;
        Ok(Self {
            ingest_url: ingest_url.into(),
            universe_id: universe_id.into(),
            hmac_key,
            batch: Mutex::new(VecDeque::with_capacity(cap)),
            config,
            client,
            dropped_count: AtomicU64::new(0),
        })
    }

    /// Replace the HTTP client (useful for testing against a local mock server).
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Returns the number of events dropped due to ring-buffer overflow.
    pub fn dropped(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }

    async fn ship_batch(&self, events: Vec<TelemetryEvent>) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        // Serialize to JSON-Lines
        let mut lines = String::with_capacity(events.len() * 128);
        for event in &events {
            lines.push_str(&serde_json::to_string(event)?);
            lines.push('\n');
        }

        // zstd-compress
        let body = zstd::encode_all(lines.as_bytes(), 3).context("zstd compress")?;

        // HMAC-SHA256 over "timestamp|universe_id|hex(SHA256(body))"
        let timestamp = Utc::now().timestamp();
        let signature = sign_payload(&self.hmac_key, timestamp, &self.universe_id, &body)?;

        let mut attempts = 0u32;
        loop {
            let result = self
                .client
                .post(&self.ingest_url)
                .header("Content-Type", "application/x-ndjson")
                .header("Content-Encoding", "zstd")
                .header("X-Co-Timestamp", timestamp.to_string())
                .header("X-Co-Universe", &self.universe_id)
                .header("X-Co-Signature", &signature)
                .body(body.clone())
                .send()
                .await;

            match result {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(r) if r.status().is_client_error() => {
                    tracing::warn!(
                        status = r.status().as_u16(),
                        events = events.len(),
                        "4xx from ingest — dropping batch"
                    );
                    return Ok(());
                }
                other => {
                    let status = other
                        .as_ref()
                        .ok()
                        .map(|r| r.status().as_u16())
                        .unwrap_or(0);
                    attempts += 1;
                    if attempts >= self.config.max_retries {
                        tracing::error!(
                            attempts,
                            status,
                            events = events.len(),
                            "persistent failure — dropping batch"
                        );
                        return Ok(());
                    }
                    let base_ms = 100u64 * (1u64 << attempts.min(10));
                    let jitter: u64 = rand::thread_rng().gen_range(0..100);
                    tracing::warn!(
                        attempt = attempts,
                        status,
                        delay_ms = base_ms + jitter,
                        "transient failure — retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter)).await;
                }
            }
        }
    }
}

impl CoAgent for FlySidecarAgent {
    fn ship(&self, events: &[TelemetryEvent]) -> impl Future<Output = Result<()>> + Send {
        let events = events.to_vec();
        async move {
            let to_flush: Vec<TelemetryEvent> = {
                let mut batch = self.batch.lock().unwrap();
                for event in events {
                    if batch.len() >= self.config.ring_buffer_cap {
                        batch.pop_front();
                        self.dropped_count.fetch_add(1, Ordering::Relaxed);
                    }
                    batch.push_back(event);
                }
                if batch.len() >= self.config.batch_size {
                    batch.drain(..).collect()
                } else {
                    vec![]
                }
            };
            if !to_flush.is_empty() {
                self.ship_batch(to_flush).await?;
            }
            Ok(())
        }
    }

    async fn heartbeat(&self) -> Result<()> {
        let dropped = self.dropped_count.load(Ordering::Relaxed);
        tracing::info!(dropped_events_total = dropped, "co-agent heartbeat");
        Ok(())
    }

    fn flush(&self) -> impl Future<Output = Result<()>> + Send {
        let events: Vec<TelemetryEvent> = {
            let mut batch = self.batch.lock().unwrap();
            batch.drain(..).collect()
        };
        async move { self.ship_batch(events).await }
    }
}

/// HMAC-SHA256 over "timestamp|universe_id|hex(SHA256(body))".
///
/// The ingest endpoint recomputes the same signature and rejects requests
/// where signatures do not match or the timestamp is outside a ±5 min window.
pub fn sign_payload(key: &[u8], timestamp: i64, universe_id: &str, body: &[u8]) -> Result<String> {
    let body_hash = hex::encode(Sha256::digest(body));
    let message = format!("{}|{}|{}", timestamp, universe_id, body_hash);
    let mut mac =
        HmacSha256::new_from_slice(key).map_err(|e| anyhow::anyhow!("invalid HMAC key: {e}"))?;
    mac.update(message.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Verify a received HMAC signature.
pub fn verify_signature(
    key: &[u8],
    timestamp: i64,
    universe_id: &str,
    body: &[u8],
    received: &str,
) -> Result<bool> {
    let expected = sign_payload(key, timestamp, universe_id, body)?;
    Ok(expected == received)
}

use std::future::Future;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU32;

    fn make_event(universe_id: &str) -> TelemetryEvent {
        TelemetryEvent::new(universe_id, "log", serde_json::json!({"message": "test"}))
    }

    // ── Helper: start an in-process axum server and return its URL ────────────
    async fn start_mock_server(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://127.0.0.1:{}/v1/events", addr.port())
    }

    fn agent_with_url(url: &str, config: AgentConfig) -> FlySidecarAgent {
        FlySidecarAgent::new(
            url,
            "test-universe",
            b"test-hmac-key-32bytes-padding!!".to_vec(),
            config,
        )
        .unwrap()
    }

    // ── T1: batch flush at size threshold ─────────────────────────────────────
    #[tokio::test]
    async fn test_flush_at_size_threshold() {
        use axum::{Router, http::StatusCode, routing::post};
        use std::sync::atomic::Ordering;

        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let app = Router::new().route(
            "/v1/events",
            post(move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
        let url = start_mock_server(app).await;

        let config = AgentConfig {
            batch_size: 5,
            max_retries: 1,
            ..Default::default()
        };
        let agent = agent_with_url(&url, config);

        // ship 5 events — 5th crosses threshold → auto-flush
        for _ in 0..5 {
            agent.ship(&[make_event("test-universe")]).await.unwrap();
        }

        // Give tokio a moment to complete async work
        tokio::task::yield_now().await;

        // Exactly one POST (one batch of 5 events)
        assert_eq!(count.load(Ordering::SeqCst), 1, "expected 1 batch POST");
        // Buffer is now empty
        let remaining = agent.batch.lock().unwrap().len();
        assert_eq!(
            remaining, 0,
            "buffer should be empty after size-threshold flush"
        );
    }

    // ── T2: batch flush at time threshold (explicit flush call) ───────────────
    #[tokio::test]
    async fn test_flush_at_time_threshold() {
        use axum::{Router, http::StatusCode, routing::post};
        use std::sync::atomic::Ordering;

        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let app = Router::new().route(
            "/v1/events",
            post(move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
        let url = start_mock_server(app).await;

        // High batch_size so auto-flush never triggers
        let config = AgentConfig {
            batch_size: 1000,
            max_retries: 1,
            ..Default::default()
        };
        let agent = agent_with_url(&url, config);

        // Add 50 events — stays below threshold
        for _ in 0..50 {
            agent.ship(&[make_event("test-universe")]).await.unwrap();
        }
        assert_eq!(count.load(Ordering::SeqCst), 0, "no auto-flush yet");

        // Time-based flush: the timer calls flush()
        agent.flush().await.unwrap();

        assert_eq!(count.load(Ordering::SeqCst), 1, "flush() sent 1 batch");
        assert_eq!(
            agent.batch.lock().unwrap().len(),
            0,
            "buffer empty after flush"
        );
    }

    // ── T3: retry-then-drop on persistent 5xx ─────────────────────────────────
    #[tokio::test(start_paused = true)]
    async fn test_retry_then_drop_on_persistent_5xx() {
        use axum::{Router, http::StatusCode, routing::post};
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let app = Router::new().route(
            "/v1/events",
            post(move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        );
        let url = start_mock_server(app).await;

        let config = AgentConfig {
            batch_size: 1000,
            max_retries: 2, // 2 total attempts, then drop
            flush_interval_secs: 60,
            ring_buffer_cap: 1000,
        };
        let agent = Arc::new(agent_with_url(&url, config));

        // Add 1 event (below auto-flush threshold)
        agent.ship(&[make_event("test-universe")]).await.unwrap();

        // Spawn the flush so we can advance time concurrently
        let flush_task = {
            let a = agent.clone();
            tokio::spawn(async move { a.flush().await })
        };

        // Advance virtual time past all retry back-offs (max ~200ms + 100ms jitter each)
        for _ in 0..5 {
            tokio::time::advance(Duration::from_secs(1)).await;
            tokio::task::yield_now().await;
        }

        let result = flush_task.await.unwrap();
        assert!(result.is_ok(), "flush must return Ok even after drop");
        // max_retries=2 → 2 POST requests before giving up
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "expected exactly 2 attempts"
        );
    }

    // ── T4: HMAC signature validation ─────────────────────────────────────────
    #[tokio::test]
    async fn test_hmac_validation() {
        use axum::{Router, body::Bytes, http::HeaderMap, http::StatusCode, routing::post};
        use std::sync::Mutex as StdMutex;

        let captured: Arc<StdMutex<Option<(HeaderMap, Bytes)>>> = Arc::new(StdMutex::new(None));
        let cap = captured.clone();

        let app = Router::new().route(
            "/v1/events",
            post(move |headers: HeaderMap, body: Bytes| {
                let cap = cap.clone();
                async move {
                    *cap.lock().unwrap() = Some((headers, body));
                    StatusCode::OK
                }
            }),
        );
        let url = start_mock_server(app).await;

        let hmac_key = b"test-hmac-key-32bytes-padding!!".to_vec();
        let config = AgentConfig {
            batch_size: 1000,
            max_retries: 1,
            ..Default::default()
        };
        let agent = FlySidecarAgent::new(&url, "test-universe", hmac_key.clone(), config).unwrap();

        agent.ship(&[make_event("test-universe")]).await.unwrap();
        agent.flush().await.unwrap();

        let (headers, body) = captured
            .lock()
            .unwrap()
            .take()
            .expect("server received request");

        let timestamp: i64 = headers
            .get("X-Co-Timestamp")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let universe_id = headers.get("X-Co-Universe").unwrap().to_str().unwrap();
        let received_sig = headers.get("X-Co-Signature").unwrap().to_str().unwrap();

        assert_eq!(universe_id, "test-universe");
        assert!(
            verify_signature(&hmac_key, timestamp, universe_id, &body, received_sig).unwrap(),
            "HMAC signature should be valid"
        );
    }

    // ── T5: end-to-end — emit 100 events, confirm ≥99 land ───────────────────
    #[tokio::test]
    async fn test_e2e_100_events_land() {
        use axum::{Router, body::Bytes, http::StatusCode, routing::post};
        use std::sync::atomic::Ordering;

        // Count individual events by decompressing each request payload
        let event_count = Arc::new(AtomicU64::new(0));
        let ec = event_count.clone();

        let app = Router::new().route(
            "/v1/events",
            post(move |body: Bytes| {
                let ec = ec.clone();
                async move {
                    if let Ok(decompressed) = zstd::decode_all(body.as_ref()) {
                        let n = decompressed
                            .split(|&b| b == b'\n')
                            .filter(|l| !l.is_empty())
                            .count();
                        ec.fetch_add(n as u64, Ordering::SeqCst);
                    }
                    StatusCode::OK
                }
            }),
        );
        let url = start_mock_server(app).await;

        let config = AgentConfig {
            batch_size: 10,
            max_retries: 3,
            flush_interval_secs: 60,
            ring_buffer_cap: 10_000,
        };
        let agent = FlySidecarAgent::new(
            &url,
            "test-universe",
            b"test-hmac-key-32bytes-padding!!".to_vec(),
            config,
        )
        .unwrap();

        // Emit 100 events (every 10 triggers an auto-flush = 10 batches)
        for i in 0..100u64 {
            let event =
                TelemetryEvent::new("test-universe", "log", serde_json::json!({ "seq": i }));
            agent.ship(&[event]).await.unwrap();
        }
        // Flush any remainder
        agent.flush().await.unwrap();

        let landed = event_count.load(Ordering::SeqCst);
        assert!(landed >= 99, "expected ≥99 events to land, got {landed}");
    }

    // ── Ring buffer overflow: drop-oldest policy ───────────────────────────────
    #[tokio::test]
    async fn test_ring_buffer_drop_oldest() {
        use axum::{Router, http::StatusCode, routing::post};

        let app = Router::new().route("/v1/events", post(|| async { StatusCode::OK }));
        let url = start_mock_server(app).await;

        let config = AgentConfig {
            batch_size: 1000,
            ring_buffer_cap: 5,
            max_retries: 1,
            ..Default::default()
        };
        let agent = agent_with_url(&url, config);

        // Ship 7 events into a cap-5 buffer: 2 should be dropped
        for _ in 0..7 {
            agent.ship(&[make_event("test-universe")]).await.unwrap();
        }

        assert_eq!(agent.dropped(), 2, "2 events should have been dropped");
        assert_eq!(agent.batch.lock().unwrap().len(), 5, "buffer capped at 5");
    }
}
