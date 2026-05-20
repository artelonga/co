//! SQLite-backed job queue for doc-gen and other async work (CO-72 / CO-78).
//!
//! # Schema
//! ```sql
//! CREATE TABLE jobs (
//!     id          TEXT PRIMARY KEY,
//!     universe_key TEXT NOT NULL,
//!     kind        TEXT NOT NULL,
//!     payload     TEXT NOT NULL,       -- JSON
//!     status      TEXT NOT NULL,       -- pending|running|done|failed|dead_letter
//!     attempts    INTEGER NOT NULL DEFAULT 0,
//!     dedupe_key  TEXT UNIQUE,         -- prevents duplicate submissions
//!     created_at  TEXT NOT NULL,
//!     run_at      TEXT NOT NULL,       -- earliest eligible run time
//!     started_at  TEXT,
//!     completed_at TEXT,
//!     error       TEXT
//! );
//! ```
//!
//! ## Queue fairness
//! Jobs are claimed in `(run_at, created_at)` FIFO order. No universe can
//! starve others: the oldest eligible job always wins regardless of
//! which universe submitted it.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::doc_gen::{DocFormat, ResourceLimits, run_adapter};
use crate::entry_index::make_entry;
use crate::error::AppError;
use crate::server::AppState;
use crate::storage::Storage;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Failed,
    DeadLetter,
}

impl JobStatus {
    fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::DeadLetter => "dead_letter",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    DocGen,
}

impl JobKind {
    fn as_str(&self) -> &'static str {
        match self {
            JobKind::DocGen => "doc_gen",
        }
    }
}

/// Payload stored in the `jobs.payload` JSON column for a `doc_gen` job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocGenPayload {
    pub format: String,
    pub source_dir: String,
    pub output_type: String,
    pub limits: ResourceLimits,
}

/// A row from the `jobs` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub universe_key: String,
    pub kind: String,
    pub payload: String,
    pub status: String,
    pub attempts: i64,
    pub dedupe_key: Option<String>,
    pub created_at: String,
    pub run_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Submit
// ---------------------------------------------------------------------------

/// Compute a stable dedupe key for a doc-gen job: prevents re-enqueueing the
/// same (universe, format, source_dir, adapter_version) while a job is
/// pending, running, or done.
fn dedupe_key(universe_key: &str, payload: &DocGenPayload) -> String {
    let fmt = payload.format.parse::<DocFormat>().ok();
    let adapter_ver = fmt.map(|f| f.adapter_version()).unwrap_or("unknown-v1");
    let raw = format!(
        "{}:{}:{}:{}",
        universe_key, payload.format, payload.source_dir, adapter_ver
    );
    // xxh3 gives a 64-bit fingerprint; format as hex for a compact string key.
    let hash = xxhash_rust::xxh3::xxh3_64(raw.as_bytes());
    format!("{hash:016x}")
}

/// Enqueue a doc-gen job. Returns the job ID.
///
/// Idempotency: if a job with the same dedupe key already exists with status
/// `pending`, `running`, or `done`, the existing job ID is returned and no
/// new row is inserted.
pub fn enqueue_doc_gen(
    conn: &Connection,
    universe_key: &str,
    payload: &DocGenPayload,
) -> Result<String, AppError> {
    let dk = dedupe_key(universe_key, payload);

    // Check for existing active/done job with this dedupe key.
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM jobs \
             WHERE dedupe_key = ?1 AND status IN ('pending','running','done') \
             LIMIT 1",
            params![dk],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let payload_json =
        serde_json::to_string(payload).map_err(|e| AppError::Internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO jobs (id, universe_key, kind, payload, status, attempts, dedupe_key, \
                           created_at, run_at) \
         VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5, ?6, ?6)",
        params![
            id,
            universe_key,
            JobKind::DocGen.as_str(),
            payload_json,
            dk,
            now,
        ],
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(id)
}

/// Fetch a job by ID (for status polling).
pub fn get_job(conn: &Connection, job_id: &str) -> Option<Job> {
    conn.query_row(
        "SELECT id, universe_key, kind, payload, status, attempts, dedupe_key, \
                created_at, run_at, started_at, completed_at, error \
         FROM jobs WHERE id = ?1",
        params![job_id],
        job_from_row,
    )
    .optional()
    .ok()
    .flatten()
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        universe_key: row.get(1)?,
        kind: row.get(2)?,
        payload: row.get(3)?,
        status: row.get(4)?,
        attempts: row.get(5)?,
        dedupe_key: row.get(6)?,
        created_at: row.get(7)?,
        run_at: row.get(8)?,
        started_at: row.get(9)?,
        completed_at: row.get(10)?,
        error: row.get(11)?,
    })
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

const MAX_ATTEMPTS: i64 = 5;
// Poll interval between worker ticks (in seconds).
pub const POLL_INTERVAL_SECS: u64 = 3;

/// Claim the oldest eligible pending job. Returns `None` if the queue is empty.
///
/// Uses `RETURNING` to atomically claim the job (prevents double-claiming).
fn claim_next_job(conn: &Connection) -> Option<Job> {
    let now = Utc::now().to_rfc3339();
    let started = now.clone();
    conn.query_row(
        "UPDATE jobs SET status = 'running', started_at = ?1 \
         WHERE id = ( \
           SELECT id FROM jobs \
           WHERE status = 'pending' AND run_at <= ?2 \
           ORDER BY run_at ASC, created_at ASC \
           LIMIT 1 \
         ) \
         RETURNING id, universe_key, kind, payload, status, attempts, dedupe_key, \
                   created_at, run_at, started_at, completed_at, error",
        params![started, now],
        job_from_row,
    )
    .optional()
    .unwrap_or(None)
}

fn mark_done(conn: &Connection, job_id: &str) {
    let now = Utc::now().to_rfc3339();
    let _ = conn.execute(
        "UPDATE jobs SET status = 'done', completed_at = ?1, error = NULL WHERE id = ?2",
        params![now, job_id],
    );
}

fn mark_failed(conn: &Connection, job_id: &str, attempts: i64, error_msg: &str) {
    let now = Utc::now().to_rfc3339();
    let new_status = if attempts >= MAX_ATTEMPTS {
        JobStatus::DeadLetter.as_str()
    } else {
        JobStatus::Failed.as_str()
    };
    // Exponential backoff: 2^attempts minutes, capped at 64 min.
    let backoff_secs = (2_u64.pow((attempts as u32).min(6)) * 60).min(3840);
    let run_at = (Utc::now() + chrono::Duration::seconds(backoff_secs as i64)).to_rfc3339();

    let _ = conn.execute(
        "UPDATE jobs SET status = ?1, attempts = ?2, completed_at = ?3, error = ?4, run_at = ?5 \
         WHERE id = ?6",
        params![new_status, attempts, now, error_msg, run_at, job_id],
    );
}

fn clear_universe_doc_gen_error(conn: &Connection, universe_key: &str) {
    let _ = conn.execute(
        "UPDATE universes SET doc_gen_error = NULL, doc_gen_error_at = NULL WHERE key = ?1",
        params![universe_key],
    );
}

fn set_universe_doc_gen_error(conn: &Connection, universe_key: &str, error_msg: &str) {
    let now = Utc::now().to_rfc3339();
    let _ = conn.execute(
        "UPDATE universes SET doc_gen_error = ?1, doc_gen_error_at = ?2 WHERE key = ?3",
        params![error_msg, now, universe_key],
    );
}

/// Persist doc entries produced by an adapter into the `entries` table.
fn store_doc_entries(
    conn: &Connection,
    universe_key: &str,
    entries: Vec<crate::doc_gen::DocEntry>,
) -> anyhow::Result<()> {
    for doc in entries {
        let fm = doc.frontmatter;
        let entry = make_entry(&doc.path, fm, &doc.body);
        let fm_json = serde_json::to_string(&entry.frontmatter)?;
        let title: Option<&str> = entry.frontmatter.get("title").and_then(|v| v.as_str());
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO entries \
               (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
             ON CONFLICT(universe_key, path) DO UPDATE SET \
               entry_type   = excluded.entry_type, \
               title        = excluded.title, \
               frontmatter_json = excluded.frontmatter_json, \
               body         = excluded.body, \
               body_hash    = excluded.body_hash, \
               updated_at   = excluded.updated_at",
            params![
                entry.path,
                universe_key,
                entry.entry_type,
                title,
                fm_json,
                entry.body,
                entry.body_hash,
                now,
            ],
        )?;
    }
    Ok(())
}

/// Process a single claimed job. Returns `Ok(())` on success or a descriptive
/// error string on failure.
fn process_job(job: &Job, storage: &Storage) -> Result<(), String> {
    if job.kind != JobKind::DocGen.as_str() {
        return Err(format!("unknown job kind: {}", job.kind));
    }

    let payload: DocGenPayload =
        serde_json::from_str(&job.payload).map_err(|e| format!("malformed payload: {e}"))?;

    let format = payload
        .format
        .parse::<DocFormat>()
        .map_err(|_| format!("unknown doc format: {}", payload.format))?;

    let source_dir = std::path::Path::new(&payload.source_dir);
    let entries = run_adapter(format, source_dir, &payload.output_type, &payload.limits)?;

    store_doc_entries(storage.conn(), &job.universe_key, entries)
        .map_err(|e| format!("failed to store entries: {e}"))?;

    Ok(())
}

/// Claim and process one pending job. Returns `Ok(())` whether or not a
/// job was found; job errors are recorded in the DB, not propagated.
pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    let job = {
        let s = state.storage.lock();
        claim_next_job(s.conn())
    };

    let Some(job) = job else {
        return Ok(());
    };

    info!(
        job_id = %job.id,
        universe = %job.universe_key,
        kind = %job.kind,
        attempt = job.attempts + 1,
        "job_queue: processing"
    );

    // Run the job on a blocking thread so the async runtime is not
    // starved. The 5-minute wall-time limit is enforced by the
    // timeout wrapper below.
    let state_clone = Arc::clone(state);
    let job_clone = job.clone();

    let result = tokio::time::timeout(
        Duration::from_secs(ResourceLimits::default().wall_time_secs),
        tokio::task::spawn_blocking(move || {
            let s = state_clone.storage.lock();
            process_job(&job_clone, &s)
        }),
    )
    .await;

    let outcome: Result<(), String> = match result {
        Err(_timeout) => Err(format!(
            "job timed out after {}s",
            ResourceLimits::default().wall_time_secs
        )),
        Ok(Err(join_err)) => Err(format!("task panicked: {join_err}")),
        Ok(Ok(inner)) => inner,
    };

    match outcome {
        Ok(()) => {
            info!(job_id = %job.id, universe = %job.universe_key, "job_queue: done");
            let s = state.storage.lock();
            mark_done(s.conn(), &job.id);
            clear_universe_doc_gen_error(s.conn(), &job.universe_key);
        }
        Err(err) => {
            let attempts = job.attempts + 1;
            warn!(
                job_id = %job.id,
                universe = %job.universe_key,
                attempt = attempts,
                error = %err,
                "job_queue: failed"
            );
            let s = state.storage.lock();
            mark_failed(s.conn(), &job.id, attempts, &err);
            set_universe_doc_gen_error(s.conn(), &job.universe_key, &err);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_storage() -> (Storage, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let s = Storage::new(tmp.path().to_str().unwrap());
        (s, tmp)
    }

    fn test_payload(source_dir: &str) -> DocGenPayload {
        DocGenPayload {
            format: "rustdoc".into(),
            source_dir: source_dir.to_string(),
            output_type: "doc.rust".into(),
            limits: ResourceLimits::default(),
        }
    }

    #[test]
    fn enqueue_returns_job_id() {
        let (storage, _tmp) = make_storage();
        let payload = test_payload("/tmp/nonexistent");
        let id = enqueue_doc_gen(storage.conn(), "test-universe", &payload).unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn enqueue_idempotent_same_dedupe_key() {
        let (storage, _tmp) = make_storage();
        let payload = test_payload("/tmp/src");
        let id1 = enqueue_doc_gen(storage.conn(), "u1", &payload).unwrap();
        let id2 = enqueue_doc_gen(storage.conn(), "u1", &payload).unwrap();
        assert_eq!(id1, id2, "second submission should return existing job ID");
    }

    #[test]
    fn enqueue_different_universe_creates_new_job() {
        let (storage, _tmp) = make_storage();
        let payload = test_payload("/tmp/src");
        let id1 = enqueue_doc_gen(storage.conn(), "universe-a", &payload).unwrap();
        let id2 = enqueue_doc_gen(storage.conn(), "universe-b", &payload).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn claim_next_job_fifo_order() {
        let (storage, _tmp) = make_storage();

        let p1 = test_payload("/tmp/first");
        let p2 = DocGenPayload {
            source_dir: "/tmp/second".into(),
            ..test_payload("/tmp/first")
        };

        let id1 = enqueue_doc_gen(storage.conn(), "u1", &p1).unwrap();
        let id2 = enqueue_doc_gen(storage.conn(), "u1", &p2).unwrap();

        let claimed = claim_next_job(storage.conn()).unwrap();
        assert_eq!(
            claimed.id, id1,
            "first submitted job should be claimed first"
        );

        let claimed2 = claim_next_job(storage.conn()).unwrap();
        assert_eq!(claimed2.id, id2);
    }

    #[test]
    fn no_double_claim() {
        let (storage, _tmp) = make_storage();
        let payload = test_payload("/tmp/src");
        enqueue_doc_gen(storage.conn(), "u1", &payload).unwrap();

        let c1 = claim_next_job(storage.conn());
        let c2 = claim_next_job(storage.conn());
        assert!(c1.is_some());
        assert!(c2.is_none(), "running job should not be claimed again");
    }

    #[test]
    fn mark_failed_increments_attempts_and_sets_error() {
        let (storage, _tmp) = make_storage();
        let payload = test_payload("/tmp/src");
        let id = enqueue_doc_gen(storage.conn(), "u1", &payload).unwrap();
        let job = claim_next_job(storage.conn()).unwrap();
        mark_failed(storage.conn(), &job.id, 1, "test error");

        let updated = get_job(storage.conn(), &id).unwrap();
        assert_eq!(updated.status, "failed");
        assert_eq!(updated.error.as_deref(), Some("test error"));
    }

    #[test]
    fn job_becomes_dead_letter_after_max_attempts() {
        let (storage, _tmp) = make_storage();
        let payload = test_payload("/tmp/src");
        enqueue_doc_gen(storage.conn(), "u1", &payload).unwrap();
        let job = claim_next_job(storage.conn()).unwrap();
        mark_failed(storage.conn(), &job.id, MAX_ATTEMPTS, "too many failures");

        let updated = get_job(storage.conn(), &job.id).unwrap();
        assert_eq!(updated.status, "dead_letter");
    }

    #[test]
    fn get_job_returns_none_for_unknown_id() {
        let (storage, _tmp) = make_storage();
        assert!(get_job(storage.conn(), "no-such-id").is_none());
    }
}
