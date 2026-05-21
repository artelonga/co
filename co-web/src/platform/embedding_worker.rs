//! Background embedding worker for CO-164 semantic search.
//!
//! Receives `EmbeddingJob`s from a sync channel, batches up to 32 entries per
//! call to amortize model overhead, and writes results to `entry_embeddings`.
//!
//! The worker runs on a dedicated OS thread so CPU-bound inference never
//! blocks the tokio runtime.

use std::sync::mpsc;
use std::time::Duration;

use tracing::{debug, warn};

use crate::embedding_index::EmbeddingIndex;
use crate::server::AppState;

const BATCH_SIZE: usize = 32;
const BATCH_WINDOW_MS: u64 = 100; // max wait for a full batch

/// A unit of work for the embedding worker.
#[derive(Debug)]
pub enum EmbeddingJob {
    Upsert {
        universe_key: String,
        path: String,
        body: String,
        body_hash: String,
    },
    Delete {
        universe_key: String,
        path: String,
    },
    /// CO-223: health-check probe sent by `WorkerSupervisor`. No-op in the
    /// OS thread — confirms the receiver is still alive.
    Probe,
}

pub type EmbeddingSender = mpsc::SyncSender<EmbeddingJob>;
pub type EmbeddingReceiver = mpsc::Receiver<EmbeddingJob>;

/// Create a bounded channel sized for `BATCH_SIZE * 1000` pending jobs.
pub fn channel() -> (EmbeddingSender, EmbeddingReceiver) {
    mpsc::sync_channel(BATCH_SIZE * 1_000)
}

/// Enqueue an upsert job (fire-and-forget; drops silently if channel full).
pub fn enqueue_upsert(
    state: &AppState,
    universe_key: &str,
    path: &str,
    body: &str,
    body_hash: &str,
) {
    let job = EmbeddingJob::Upsert {
        universe_key: universe_key.to_string(),
        path: path.to_string(),
        body: body.to_string(),
        body_hash: body_hash.to_string(),
    };
    let _ = state.index.embedding_tx.try_send(job);
}

/// Enqueue a delete job (fire-and-forget; drops silently if channel full).
pub fn enqueue_delete(state: &AppState, universe_key: &str, path: &str) {
    let job = EmbeddingJob::Delete {
        universe_key: universe_key.to_string(),
        path: path.to_string(),
    };
    let _ = state.index.embedding_tx.try_send(job);
}

/// Spawn the embedding worker on a dedicated OS thread.
pub fn spawn(rx: EmbeddingReceiver, state: AppState) {
    std::thread::Builder::new()
        .name("co-embedding-worker".into())
        .spawn(move || run_worker(rx, state))
        .expect("failed to spawn embedding worker thread");
}

/// At boot, scan all universes for entries whose embedding is missing or stale,
/// then enqueue them. Runs as an async background task (non-blocking).
pub fn boot_scan(state: AppState) {
    tokio::spawn(async move {
        let universe_keys = { state.core.storage.lock().all_universe_keys() };

        let mut queued = 0usize;
        for key in universe_keys {
            let uc = { state.core.storage.lock().universe_conn(&key) };
            let conn = match uc.lock() {
                Ok(c) => c,
                Err(_) => continue,
            };
            let stale = match EmbeddingIndex::new(&conn).stale_paths(&key) {
                Ok(v) => v,
                Err(e) => {
                    warn!("CO-164 boot scan error for '{key}': {e}");
                    continue;
                }
            };
            for (path, body_hash, body) in stale {
                let _ = state.index.embedding_tx.try_send(EmbeddingJob::Upsert {
                    universe_key: key.clone(),
                    path,
                    body,
                    body_hash,
                });
                queued += 1;
            }
        }
        if queued > 0 {
            tracing::info!("CO-164 boot scan: queued {queued} stale embeddings");
        }
    });
}

// ---------------------------------------------------------------------------
// Worker internals
// ---------------------------------------------------------------------------

fn run_worker(rx: EmbeddingReceiver, state: AppState) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];

        // Drain up to BATCH_SIZE - 1 more without blocking (short window).
        let deadline = std::time::Instant::now() + Duration::from_millis(BATCH_WINDOW_MS);
        while batch.len() < BATCH_SIZE {
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(j) => batch.push(j),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Process the last batch before exiting.
                    process_batch(&batch, &state);
                    return;
                }
            }
        }

        process_batch(&batch, &state);
    }
}

fn process_batch(batch: &[EmbeddingJob], state: &AppState) {
    // --- Handle deletes first ---
    for job in batch {
        if let EmbeddingJob::Delete { universe_key, path } = job {
            let uc = state.core.storage.lock().universe_conn(universe_key);
            if let Ok(conn) = uc.lock() {
                let _ = EmbeddingIndex::new(&conn).remove(universe_key, path);
            }
        }
    }

    // --- Handle upserts ---
    let upserts: Vec<(&str, &str, &str, &str)> = batch
        .iter()
        .filter_map(|j| {
            if let EmbeddingJob::Upsert {
                universe_key,
                path,
                body,
                body_hash,
            } = j
            {
                Some((
                    universe_key.as_str(),
                    path.as_str(),
                    body.as_str(),
                    body_hash.as_str(),
                ))
            } else {
                None
            }
        })
        .collect();

    if upserts.is_empty() {
        return;
    }

    // Compute embeddings for all bodies in one batch call.
    let texts: Vec<&str> = upserts.iter().map(|(_, _, body, _)| *body).collect();
    let embeddings = match state.index.embeddings.embed(&texts) {
        Some(e) => e,
        None => {
            debug!(
                "CO-164: model unavailable, skipping batch of {}",
                upserts.len()
            );
            return;
        }
    };

    if embeddings.len() != upserts.len() {
        warn!(
            "CO-164: embedding count mismatch: got {} for {} entries",
            embeddings.len(),
            upserts.len()
        );
        return;
    }

    // Write each embedding to the per-universe DB.
    for ((universe_key, path, _body, body_hash), embedding) in upserts.iter().zip(embeddings.iter())
    {
        let uc = state.core.storage.lock().universe_conn(universe_key);
        let conn = match uc.lock() {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Err(e) = EmbeddingIndex::new(&conn).upsert(
            universe_key,
            path,
            body_hash,
            embedding,
            crate::embedding::MODEL_NAME,
        ) {
            warn!("CO-164: upsert error for {universe_key}/{path}: {e}");
        }
    }
    debug!("CO-164: indexed batch of {} entries", upserts.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_is_bounded() {
        let (tx, _rx) = channel();
        // Should not block — channel has large capacity
        for i in 0..100 {
            let _ = tx.try_send(EmbeddingJob::Delete {
                universe_key: "u".into(),
                path: format!("{i}.md"),
            });
        }
    }

    #[test]
    fn test_enqueue_upsert_does_not_panic_when_full() {
        // Create a channel with capacity 1, fill it, then try_send should not panic.
        let (tx, _rx) = mpsc::sync_channel::<EmbeddingJob>(1);
        let _ = tx.try_send(EmbeddingJob::Delete {
            universe_key: "u".into(),
            path: "a.md".into(),
        });
        // This try_send should fail silently (channel full)
        let result = tx.try_send(EmbeddingJob::Delete {
            universe_key: "u".into(),
            path: "b.md".into(),
        });
        assert!(result.is_err());
    }
}
