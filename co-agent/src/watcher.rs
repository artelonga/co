//! CO-151 — filesystem watcher: watches local directories for changes and
//! streams SyncDelta batches over the sync WebSocket endpoint.
//!
//! Replaces `scripts/co-watch.py` (v1).  The v1 script keeps working until
//! this watcher is deployed — they talk to separate endpoints and can run
//! simultaneously.
//!
//! # Platform
//!
//! Uses the `notify` crate which selects the native backend:
//! - macOS: FSEvents (kqueue fallback)
//! - Linux: inotify
//!
//! # Architecture
//!
//! ```text
//! notify::Watcher
//!   └─ raw events ──► debouncer ──► Vec<DebouncedEvent>
//!                                        │
//!                               encode_delta()
//!                                        │
//!                               SyncBatch (protobuf + zstd)
//!                                        │
//!                               tokio-tungstenite WS
//!                                        │
//!                               co-web /api/v1/sync/ws
//! ```
//!
//! Downlink batches (server-push) are received on the same WS connection and
//! applied to the local filesystem by `apply_batch()`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use co::proto::sync::{SyncBatch, SyncDelta};
use co::sync::delta as codec;
use futures_util::{SinkExt, StreamExt};
use notify::RecursiveMode;
use notify::Watcher as NotifyWatcher;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// How long to collect events before sending a batch.
const DEBOUNCE_MS: u64 = 200;

/// Configuration for the sync watcher.
pub struct WatcherConfig {
    /// Local directories to watch (absolute paths).
    pub watch_dirs: Vec<PathBuf>,
    /// Universe key this watcher belongs to.
    pub universe_key: String,
    /// WS endpoint, e.g. `"wss://co-artelonga.fly.dev/api/v1/sync/ws"`.
    pub server_url: String,
    /// JWT or session token for auth.
    pub auth_token: String,
    /// Resume token from last session (0 = start fresh).
    pub resume_token: u64,
}

// ---------------------------------------------------------------------------
// SyncWatcher
// ---------------------------------------------------------------------------

pub struct SyncWatcher {
    config: Arc<WatcherConfig>,
}

impl SyncWatcher {
    pub fn new(config: WatcherConfig) -> Self {
        SyncWatcher {
            config: Arc::new(config),
        }
    }

    /// Run the watcher loop until cancelled.
    ///
    /// This function is async and runs until the WS connection is closed or
    /// an unrecoverable error occurs.
    pub async fn run(&self) -> Result<()> {
        let url = format!(
            "{}?token={}",
            self.config.server_url, self.config.auth_token
        );
        let resume = self.config.resume_token;

        let mut request =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                &url as &str,
            )?;
        if resume > 0 {
            request
                .headers_mut()
                .insert("X-Sync-Resume", resume.to_string().parse()?);
        }
        request
            .headers_mut()
            .insert("Accept", "application/vnd.co+protobuf+zstd".parse()?);

        let (mut ws, _response) = connect_async(request).await.context("connect to sync WS")?;

        info!(
            url = %self.config.server_url,
            universe = %self.config.universe_key,
            "sync-watcher connected"
        );

        // Channel: notify → uplink sender
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Vec<WatchEvent>>();

        // Spawn the filesystem watcher in a blocking thread.
        let dirs = self.config.watch_dirs.clone();
        let universe_key = self.config.universe_key.clone();
        let tx = event_tx.clone();
        let _watcher_handle =
            tokio::task::spawn_blocking(move || watch_dirs_blocking(dirs, universe_key, tx));

        let universe_key = self.config.universe_key.clone();
        loop {
            tokio::select! {
                biased;

                // Filesystem events → uplink.
                Some(events) = event_rx.recv() => {
                    let batch = build_batch(&events, &universe_key);
                    match codec::encode_batch(&batch) {
                        Ok(encoded) => {
                            if ws.send(WsMsg::Binary(encoded.into())).await.is_err() {
                                warn!("sync-watcher WS send failed — reconnecting");
                                break;
                            }
                        }
                        Err(e) => warn!("sync-watcher encode error: {e}"),
                    }
                }

                // Downlink from server.
                msg = ws.next() => {
                    match msg {
                        Some(Ok(WsMsg::Binary(data))) => {
                            match codec::decode_batch(&data) {
                                Ok(batch) => apply_batch(&batch),
                                Err(e) => warn!("sync-watcher decode error: {e}"),
                            }
                        }
                        Some(Ok(WsMsg::Ping(p))) => {
                            ws.send(WsMsg::Pong(p)).await.ok();
                        }
                        Some(Ok(WsMsg::Close(_))) | None => {
                            info!("sync-watcher WS closed by server");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("sync-watcher WS error: {e}");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Filesystem event
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: WatchEventKind,
}

#[derive(Debug, Clone, Copy)]
pub enum WatchEventKind {
    Upserted,
    Deleted,
}

// ---------------------------------------------------------------------------
// Blocking watcher (runs in its own thread)
// ---------------------------------------------------------------------------

fn watch_dirs_blocking(
    dirs: Vec<PathBuf>,
    universe_key: String,
    tx: mpsc::UnboundedSender<Vec<WatchEvent>>,
) -> Result<()> {
    let (raw_tx, raw_rx) = std::sync::mpsc::channel();

    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(ev) = res {
                let _ = raw_tx.send(ev);
            }
        })?;

    for dir in &dirs {
        watcher.watch(dir, RecursiveMode::Recursive)?;
        info!(dir = %dir.display(), universe = %universe_key, "watching");
    }

    // Debounce loop: collect events over DEBOUNCE_MS windows.
    let debounce = Duration::from_millis(DEBOUNCE_MS);
    let mut pending: HashMap<PathBuf, WatchEventKind> = HashMap::new();

    loop {
        match raw_rx.recv_timeout(debounce) {
            Ok(ev) => {
                for path in ev.paths {
                    use notify::EventKind;
                    let kind = match ev.kind {
                        EventKind::Remove(_) => WatchEventKind::Deleted,
                        _ => WatchEventKind::Upserted,
                    };
                    pending.insert(path, kind);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !pending.is_empty() {
                    let batch: Vec<WatchEvent> = pending
                        .drain()
                        .map(|(path, kind)| WatchEvent { path, kind })
                        .collect();
                    if tx.send(batch).is_err() {
                        break; // receiver dropped — exit watcher
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Batch building
// ---------------------------------------------------------------------------

fn build_batch(events: &[WatchEvent], universe_key: &str) -> SyncBatch {
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64;

    let deltas: Vec<SyncDelta> = events
        .iter()
        .filter_map(|ev| encode_event(ev, universe_key, now_ns))
        .collect();

    SyncBatch {
        deltas,
        client_id: format!("co-watcher-{}", std::process::id()),
        batch_ts_ns: now_ns,
        resume_token: 0,
    }
}

fn encode_event(ev: &WatchEvent, universe_key: &str, ts_ns: i64) -> Option<SyncDelta> {
    let path_str = ev.path.to_string_lossy();

    match ev.kind {
        WatchEventKind::Deleted => Some(codec::deleted_delta(universe_key, &*path_str, ts_ns)),
        WatchEventKind::Upserted => {
            let content = std::fs::read(&ev.path).ok()?;
            let sha256 = hex::encode(Sha256::digest(&content));
            Some(codec::upserted_delta(
                universe_key,
                &*path_str,
                content,
                sha256,
                ts_ns,
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Downlink apply
// ---------------------------------------------------------------------------

/// Apply a server-pushed SyncBatch to local files (last-write-wins).
fn apply_batch(batch: &SyncBatch) {
    use co::proto::sync::sync_delta::{Body, Kind};

    for delta in &batch.deltas {
        let kind = Kind::try_from(delta.kind).unwrap_or(Kind::Unspecified);
        let path = Path::new(&delta.entry_path);

        match kind {
            Kind::Upserted => {
                if let Some(Body::Cofile(ref cofile)) = delta.body {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    if let Err(e) = std::fs::write(path, &cofile.content) {
                        warn!("sync-watcher apply write failed: {path:?}: {e}");
                    } else {
                        debug!("sync-watcher applied upsert: {path:?}");
                    }
                }
            }
            Kind::Deleted => {
                if let Err(e) = std::fs::remove_file(path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        warn!("sync-watcher apply delete failed: {path:?}: {e}");
                    }
                } else {
                    debug!("sync-watcher applied delete: {path:?}");
                }
            }
            Kind::Renamed => {
                // Handled by upsert of the new path + delete of the old.
                debug!("sync-watcher skipping rename delta (handled as upsert+delete)");
            }
            Kind::Unspecified => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use co::proto::sync::sync_delta;
    use tempfile::TempDir;

    // ── T1: encode_event for upserted file ───────────────────────────────────

    #[test]
    fn test_encode_upserted_event() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("hello.md");
        std::fs::write(&path, b"# Hello world").unwrap();

        let ev = WatchEvent {
            path: path.clone(),
            kind: WatchEventKind::Upserted,
        };
        let delta = encode_event(&ev, "my-universe", 1_000_000).unwrap();

        assert_eq!(delta.universe_key, "my-universe");
        assert_eq!(delta.entry_path, path.to_string_lossy().as_ref());
        assert_eq!(delta.kind, sync_delta::Kind::Upserted as i32);

        match delta.body.unwrap() {
            sync_delta::Body::Cofile(f) => {
                assert_eq!(f.content, b"# Hello world");
                assert!(!f.sha256.is_empty());
            }
            _ => panic!("expected CoFile"),
        }
    }

    // ── T2: encode_event for deleted file ────────────────────────────────────

    #[test]
    fn test_encode_deleted_event() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("gone.md");

        let ev = WatchEvent {
            path: path.clone(),
            kind: WatchEventKind::Deleted,
        };
        let delta = encode_event(&ev, "u", 999).unwrap();

        assert_eq!(delta.kind, sync_delta::Kind::Deleted as i32);
        assert!(delta.body.is_none());
    }

    // ── T3: build_batch produces correct structure ────────────────────────────

    #[test]
    fn test_build_batch() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("a.md");
        let p2 = tmp.path().join("b.md");
        std::fs::write(&p1, b"file a").unwrap();
        std::fs::write(&p2, b"file b").unwrap();

        let events = vec![
            WatchEvent {
                path: p1,
                kind: WatchEventKind::Upserted,
            },
            WatchEvent {
                path: p2,
                kind: WatchEventKind::Upserted,
            },
        ];
        let batch = build_batch(&events, "test-universe");

        assert_eq!(batch.deltas.len(), 2);
        assert!(batch.client_id.starts_with("co-watcher-"));
        assert!(batch.batch_ts_ns > 0);
    }

    // ── T4: apply_batch writes files locally ─────────────────────────────────

    #[test]
    fn test_apply_batch_writes_file() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("sub").join("file.md");

        use co::sync::delta as dc;
        let delta = dc::upserted_delta(
            "u",
            target.to_str().unwrap(),
            b"# Applied".to_vec(),
            "sha",
            0,
        );
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "server".into(),
            batch_ts_ns: 0,
            resume_token: 1,
        };

        apply_batch(&batch);

        assert!(target.exists(), "file should have been written");
        let content = std::fs::read_to_string(&target).unwrap();
        assert_eq!(content, "# Applied");
    }

    // ── T5: apply_batch deletes file ─────────────────────────────────────────

    #[test]
    fn test_apply_batch_deletes_file() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("to-delete.md");
        std::fs::write(&target, b"bye").unwrap();
        assert!(target.exists());

        use co::sync::delta as dc;
        let delta = dc::deleted_delta("u", target.to_str().unwrap(), 0);
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "server".into(),
            batch_ts_ns: 0,
            resume_token: 1,
        };

        apply_batch(&batch);

        assert!(!target.exists(), "file should have been deleted");
    }

    // ── T6: apply_batch ignores NotFound on delete ────────────────────────────

    #[test]
    fn test_apply_batch_delete_not_found_is_ok() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("nonexistent.md");

        use co::sync::delta as dc;
        let delta = dc::deleted_delta("u", target.to_str().unwrap(), 0);
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "server".into(),
            batch_ts_ns: 0,
            resume_token: 0,
        };

        // Should not panic.
        apply_batch(&batch);
    }

    // ── T7: round-trip encode_batch → decode_batch ───────────────────────────

    #[test]
    fn test_watcher_batch_codec_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("roundtrip.md");
        std::fs::write(&path, b"content").unwrap();

        let ev = WatchEvent {
            path,
            kind: WatchEventKind::Upserted,
        };
        let batch = build_batch(&[ev], "u");
        let encoded = codec::encode_batch(&batch).unwrap();
        let decoded = codec::decode_batch(&encoded).unwrap();

        assert_eq!(decoded.deltas.len(), 1);
    }
}
