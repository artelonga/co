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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// Window during which a recently-applied (web→local) sha256 will suppress
/// our own outbound notify event for that path. Long enough to cover the
/// FSEvents → debouncer → encode round trip, short enough that legitimate
/// fast-edit sequences still get through.
const APPLIED_DEDUP_WINDOW: Duration = Duration::from_secs(5);

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
        // Shared dedup map: sha256 → instant the watcher last applied that
        // content from a downlink. encode_event consults this and skips
        // sending a delta when the on-disk content matches what we just
        // wrote — closing the web→local→web echo loop.
        let applied: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        // Server expects ?universe=<key>&token=<jwt>; missing universe → HTTP 400.
        // Universe keys are slugs ([a-z0-9-]) and JWTs are url-safe base64
        // ([A-Za-z0-9._-]) — both safe to inline without percent-encoding.
        let url = format!(
            "{}?universe={}&token={}",
            self.config.server_url, self.config.universe_key, self.config.auth_token,
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

        // Spawn the filesystem watcher on a dedicated OS thread, NOT tokio's
        // blocking pool. notify's macOS backend (FSEvents) needs a thread with
        // a CFRunLoop that lives for the whole stream — tokio::task::spawn_
        // blocking pool threads can be torn down between blocking calls and
        // FSEvents stops delivering events. Empirically, with spawn_blocking
        // the same notify config that works in a `cargo run` standalone
        // delivers zero events when wrapped in tokio.
        let dirs = self.config.watch_dirs.clone();
        let universe_key = self.config.universe_key.clone();
        let tx = event_tx.clone();
        let _watcher_handle = std::thread::spawn(move || {
            if let Err(e) = watch_dirs_blocking(dirs, universe_key, tx) {
                warn!("filesystem watcher thread exited: {e:#}");
            }
        });

        let universe_key = self.config.universe_key.clone();
        loop {
            tokio::select! {
                biased;

                // Filesystem events → uplink.
                Some(events) = event_rx.recv() => {
                    let batch = build_batch(&events, &universe_key, &applied);
                    if batch.deltas.is_empty() {
                        // Nothing left after dedup — likely an echo of a
                        // change we just applied locally. Don't bother the
                        // server.
                        continue;
                    }
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
                                Ok(batch) => apply_batch(
                                    &batch,
                                    &self.config.watch_dirs,
                                    &applied,
                                ),
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
    /// Absolute path on the local filesystem (used for std::fs::read).
    pub abs_path: PathBuf,
    /// Path relative to the universe root that this watch is rooted at
    /// (used as `entry_path` on the wire so the server can resolve it).
    pub rel_path: PathBuf,
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

/// Convert an absolute path emitted by `notify` to a path relative to the
/// first watch root that contains it.
fn relativize(absolute: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let canon = absolute.canonicalize().ok();
    let target = canon.as_deref().unwrap_or(absolute);
    for root in roots {
        let canon_root = root.canonicalize().ok();
        let r = canon_root.as_deref().unwrap_or(root);
        if let Ok(rel) = target.strip_prefix(r) {
            return Some(rel.to_path_buf());
        }
    }
    None
}

/// True if the file's extension marks it as content the WS sync path
/// handles. Cuts down on notify spam from .DS_Store, .swp, .git/index, etc.
///
/// Currently `.md` only — the SyncDelta wire format encodes content as
/// `CoFile.content` and the server's `apply_deltas_to_storage` requires
/// UTF-8 bytes. Binaries (PDF, image, audio, video) need the dedicated
/// `/api/v1/universes/{u}/assets` path with sha256 content addressing;
/// run `scripts/bulk-upload-binary.py` to upload them. CO-151 Phase 2
/// will add a typed `Asset` body to `SyncDelta` so the watcher can stream
/// binaries too.
fn is_syncable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    matches!(ext.as_deref(), Some("md"))
}

fn watch_dirs_blocking(
    dirs: Vec<PathBuf>,
    universe_key: String,
    tx: mpsc::UnboundedSender<Vec<WatchEvent>>,
) -> Result<()> {
    let (raw_tx, raw_rx) = std::sync::mpsc::channel();

    let mut watcher = notify::recommended_watcher(
        move |res: Result<notify::Event, notify::Error>| match res {
            Ok(ev) => {
                debug!(?ev, "raw notify event");
                let _ = raw_tx.send(ev);
            }
            Err(e) => warn!("notify error: {e}"),
        },
    )?;

    for dir in &dirs {
        watcher.watch(dir, RecursiveMode::Recursive)?;
        info!(dir = %dir.display(), universe = %universe_key, "watching");
    }
    info!("filesystem watcher loop entered");

    // Debounce loop: collect events over DEBOUNCE_MS windows.
    let debounce = Duration::from_millis(DEBOUNCE_MS);
    let mut pending: HashMap<PathBuf, WatchEventKind> = HashMap::new();

    loop {
        match raw_rx.recv_timeout(debounce) {
            Ok(ev) => {
                for path in ev.paths {
                    if !is_syncable(&path) {
                        continue;
                    }
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
                    // CO-151 fix: server expects entry_path relative to the
                    // universe root (e.g. "notes/hello.md"), not the absolute
                    // path notify emits. Carry both: rel_path for the wire,
                    // abs_path so encode_event can still read the file.
                    let batch: Vec<WatchEvent> = pending
                        .drain()
                        .filter_map(|(abs, kind)| {
                            let rel = relativize(&abs, &dirs)?;
                            Some(WatchEvent {
                                abs_path: abs,
                                rel_path: rel,
                                kind,
                            })
                        })
                        .collect();
                    if !batch.is_empty() && tx.send(batch).is_err() {
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

fn build_batch(
    events: &[WatchEvent],
    universe_key: &str,
    applied: &Arc<Mutex<HashMap<String, Instant>>>,
) -> SyncBatch {
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64;

    let deltas: Vec<SyncDelta> = events
        .iter()
        .filter_map(|ev| encode_event(ev, universe_key, now_ns, applied))
        .collect();

    SyncBatch {
        deltas,
        client_id: format!("co-watcher-{}", std::process::id()),
        batch_ts_ns: now_ns,
        resume_token: 0,
    }
}

fn encode_event(
    ev: &WatchEvent,
    universe_key: &str,
    ts_ns: i64,
    applied: &Arc<Mutex<HashMap<String, Instant>>>,
) -> Option<SyncDelta> {
    // entry_path on the wire is the universe-rooted relative path; reads on
    // disk use the absolute path the watcher captured.
    let rel = ev.rel_path.to_string_lossy();

    // macOS FSEvents sometimes reports a `rm` as a Modify event instead of
    // Remove, so trust the filesystem state at flush time: if the absolute
    // path no longer exists, emit a Deleted regardless of how notify
    // classified the event.
    if !ev.abs_path.exists() {
        debug!(rel = %rel, kind = "auto-Deleted", "encoding sync delta (path missing)");
        return Some(codec::deleted_delta(universe_key, &*rel, ts_ns));
    }

    match ev.kind {
        WatchEventKind::Deleted => Some(codec::deleted_delta(universe_key, &*rel, ts_ns)),
        WatchEventKind::Upserted => {
            let content = std::fs::read(&ev.abs_path).ok()?;
            let sha256 = hex::encode(Sha256::digest(&content));

            // Dedup: if we just applied this exact content from a downlink,
            // notify is firing because OUR write triggered FSEvents. Skip
            // sending it back — server already has it. Cleans up the map
            // opportunistically while we're holding the lock.
            if let Ok(mut guard) = applied.lock() {
                let now = Instant::now();
                guard.retain(|_, t| now.duration_since(*t) < APPLIED_DEDUP_WINDOW);
                if guard.contains_key(&sha256) {
                    debug!(rel = %rel, sha = %&sha256[..12], "suppressing fs-notify echo (recently applied)");
                    return None;
                }
            }

            debug!(rel = %rel, abs = %ev.abs_path.display(), kind = ?ev.kind, "encoding sync delta");
            Some(codec::upserted_delta(
                universe_key,
                &*rel,
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
///
/// `delta.entry_path` is the universe-rooted relative path (e.g.
/// `notes/hello.md`). Each delta is resolved against the first watch root
/// in `roots` — a v2 watcher process is single-universe so there's exactly
/// one root, and any path outside it is invalid.
///
/// Records each applied content's sha256 in `applied` so the uplink path
/// can suppress notify-driven echo of the change we just made.
fn apply_batch(
    batch: &SyncBatch,
    roots: &[PathBuf],
    applied: &Arc<Mutex<HashMap<String, Instant>>>,
) {
    use co::proto::sync::sync_delta::{Body, Kind};

    let Some(root) = roots.first() else {
        warn!("sync-watcher apply_batch: no watch roots configured");
        return;
    };

    for delta in &batch.deltas {
        let kind = Kind::try_from(delta.kind).unwrap_or(Kind::Unspecified);
        // Reject absolute paths defensively — server should always send
        // universe-rooted relative paths post-1.38.2.
        let rel = Path::new(&delta.entry_path);
        if rel.is_absolute() {
            warn!("sync-watcher apply_batch: rejecting absolute path {:?}", rel);
            continue;
        }
        let abs = root.join(rel);

        match kind {
            Kind::Upserted => {
                if let Some(Body::Cofile(ref cofile)) = delta.body {
                    if let Some(parent) = abs.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    // Skip the write if local content is already byte-identical.
                    // notify might still fire on metadata, but the dedup map
                    // suppresses the echo at encode time.
                    let already_match = std::fs::read(&abs)
                        .ok()
                        .is_some_and(|cur| cur == cofile.content);
                    if !already_match
                        && let Err(e) = std::fs::write(&abs, &cofile.content)
                    {
                        warn!("sync-watcher apply write failed: {abs:?}: {e}");
                        continue;
                    }
                    let sha = hex::encode(Sha256::digest(&cofile.content));
                    if let Ok(mut g) = applied.lock() {
                        g.insert(sha, Instant::now());
                    }
                    debug!("sync-watcher applied upsert: {abs:?}");
                }
            }
            Kind::Deleted => {
                match std::fs::remove_file(&abs) {
                    Ok(()) => {
                        // Mark a sentinel "deleted" entry so encode_event
                        // can skip the corresponding fs-notify Remove echo.
                        if let Ok(mut g) = applied.lock() {
                            // Use a path-derived key so deletes also dedup.
                            g.insert(
                                format!("DEL:{}", delta.entry_path),
                                Instant::now(),
                            );
                        }
                        debug!("sync-watcher applied delete: {abs:?}");
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // already gone
                    }
                    Err(e) => warn!("sync-watcher apply delete failed: {abs:?}: {e}"),
                }
            }
            Kind::Renamed => {
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

    /// Test helper: empty applied-dedup map for tests that don't care about it.
    fn empty_applied() -> Arc<Mutex<HashMap<String, Instant>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }

    // ── T1: encode_event for upserted file ───────────────────────────────────

    #[test]
    fn test_encode_upserted_event() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("hello.md");
        std::fs::write(&path, b"# Hello world").unwrap();

        let ev = WatchEvent {
            abs_path: path.clone(),
            rel_path: path.clone(),
            kind: WatchEventKind::Upserted,
        };
        let delta = encode_event(&ev, "my-universe", 1_000_000, &empty_applied()).unwrap();

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
            abs_path: path.clone(),
            rel_path: path.clone(),
            kind: WatchEventKind::Deleted,
        };
        let delta = encode_event(&ev, "u", 999, &empty_applied()).unwrap();

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
                abs_path: p1.clone(),
                rel_path: p1,
                kind: WatchEventKind::Upserted,
            },
            WatchEvent {
                abs_path: p2.clone(),
                rel_path: p2,
                kind: WatchEventKind::Upserted,
            },
        ];
        let batch = build_batch(&events, "test-universe", &empty_applied());

        assert_eq!(batch.deltas.len(), 2);
        assert!(batch.client_id.starts_with("co-watcher-"));
        assert!(batch.batch_ts_ns > 0);
    }

    // ── T4: apply_batch writes files locally ─────────────────────────────────

    #[test]
    fn test_apply_batch_writes_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();

        use co::sync::delta as dc;
        // Universe-rooted relative path; apply_batch joins against `root`.
        let delta = dc::upserted_delta("u", "sub/file.md", b"# Applied".to_vec(), "sha", 0);
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "server".into(),
            batch_ts_ns: 0,
            resume_token: 1,
        };

        apply_batch(&batch, std::slice::from_ref(&root), &empty_applied());

        let target = root.join("sub").join("file.md");
        assert!(target.exists(), "file should have been written");
        let content = std::fs::read_to_string(&target).unwrap();
        assert_eq!(content, "# Applied");
    }

    // ── T5: apply_batch deletes file ─────────────────────────────────────────

    #[test]
    fn test_apply_batch_deletes_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let target = root.join("to-delete.md");
        std::fs::write(&target, b"bye").unwrap();
        assert!(target.exists());

        use co::sync::delta as dc;
        let delta = dc::deleted_delta("u", "to-delete.md", 0);
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "server".into(),
            batch_ts_ns: 0,
            resume_token: 1,
        };

        apply_batch(&batch, &[root], &empty_applied());

        assert!(!target.exists(), "file should have been deleted");
    }

    // ── T6: apply_batch ignores NotFound on delete ────────────────────────────

    #[test]
    fn test_apply_batch_delete_not_found_is_ok() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();

        use co::sync::delta as dc;
        let delta = dc::deleted_delta("u", "nonexistent.md", 0);
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "server".into(),
            batch_ts_ns: 0,
            resume_token: 0,
        };

        // Should not panic.
        apply_batch(&batch, &[root], &empty_applied());
    }

    // ── T7: applied-dedup suppresses fs-notify echo ──────────────────────────

    #[test]
    fn test_encode_event_skips_recently_applied_content() {
        // Simulates the web→local→web echo path: server pushes a change,
        // watcher applies it, FSEvents fires Modify. encode_event should
        // skip emitting a delta for a sha256 that's in the applied map.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("echo.md");
        std::fs::write(&path, b"server content").unwrap();

        let applied = empty_applied();
        let sha = hex::encode(Sha256::digest(b"server content"));
        applied.lock().unwrap().insert(sha, Instant::now());

        let ev = WatchEvent {
            abs_path: path.clone(),
            rel_path: path.clone(),
            kind: WatchEventKind::Upserted,
        };
        let result = encode_event(&ev, "u", 0, &applied);
        assert!(
            result.is_none(),
            "encode_event should skip when sha256 was just applied",
        );

        // After the dedup window expires, the same content emits normally.
        applied.lock().unwrap().clear();
        let result = encode_event(&ev, "u", 0, &applied);
        assert!(result.is_some(), "fresh applied map → delta emitted");
    }

    // ── T8: round-trip encode_batch → decode_batch ───────────────────────────

    #[test]
    fn test_watcher_batch_codec_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("roundtrip.md");
        std::fs::write(&path, b"content").unwrap();

        let ev = WatchEvent {
            abs_path: path.clone(),
            rel_path: path,
            kind: WatchEventKind::Upserted,
        };
        let batch = build_batch(&[ev], "u", &empty_applied());
        let encoded = codec::encode_batch(&batch).unwrap();
        let decoded = codec::decode_batch(&encoded).unwrap();

        assert_eq!(decoded.deltas.len(), 1);
    }
}
