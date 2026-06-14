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

use std::time::Duration;

use axum::{Json, extract::State};
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

/// The kinds of work the queue can carry. Each kind maps to a stable
/// snake_case string persisted in `jobs.kind`, a per-kind wall-time budget, and
/// a per-universe hourly rate limit (CO-80).
///
/// CO-78 generalizes the CO-72 single-kind (`doc_gen`) queue into the full set
/// of CPU-heavy, non-real-time jobs the platform offloads from request threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Generate API/code docs from a source tree (CO-72).
    DocGen,
    /// Rebuild a universe's search/embedding index from scratch (heavy).
    IndexRebuild,
    /// Regenerate a universe's changelog (CO-75, heavy).
    ChangelogRegen,
    /// Pull + re-sync a full universe from its upstream source.
    SyncPull,
    /// Deliver an outbound webhook (light, high-volume).
    WebhookDispatch,
    /// Background cleanup (orphaned blobs, expired rows, …).
    Cleanup,
}

/// Default per-job wall-time budget when no kind is specified or recognised.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300; // 5 minutes (spec default)

impl JobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobKind::DocGen => "doc_gen",
            JobKind::IndexRebuild => "index_rebuild",
            JobKind::ChangelogRegen => "changelog_regen",
            JobKind::SyncPull => "sync_pull",
            JobKind::WebhookDispatch => "webhook_dispatch",
            JobKind::Cleanup => "cleanup",
        }
    }

    /// Parse the persisted `jobs.kind` string back into a `JobKind`.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "doc_gen" => Some(JobKind::DocGen),
            "index_rebuild" => Some(JobKind::IndexRebuild),
            "changelog_regen" => Some(JobKind::ChangelogRegen),
            "sync_pull" => Some(JobKind::SyncPull),
            "webhook_dispatch" => Some(JobKind::WebhookDispatch),
            "cleanup" => Some(JobKind::Cleanup),
            _ => None,
        }
    }

    /// Per-job wall-time budget. Heavy kinds (full re-index, changelog regen,
    /// full re-sync) get a longer budget than light, high-volume kinds. Stamped
    /// into `jobs.timeout_secs` at enqueue so the worker enforces it per job.
    pub fn default_timeout_secs(&self) -> u64 {
        match self {
            JobKind::DocGen => 300,
            JobKind::IndexRebuild => 900,
            JobKind::ChangelogRegen => 600,
            JobKind::SyncPull => 600,
            JobKind::WebhookDispatch => 30,
            JobKind::Cleanup => 120,
        }
    }

    /// CO-80: max jobs of this kind a single universe may enqueue per hour.
    /// Heavy kinds get stricter limits so one universe cannot monopolise the
    /// worker pool. `None` ⇒ no per-kind limit.
    pub fn hourly_rate_limit(&self) -> Option<i64> {
        match self {
            JobKind::DocGen => Some(60),
            JobKind::IndexRebuild => Some(4),
            JobKind::ChangelogRegen => Some(4),
            JobKind::SyncPull => Some(30),
            JobKind::WebhookDispatch => Some(600),
            JobKind::Cleanup => Some(12),
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
    /// CO-78: per-job wall-time budget in seconds. `None` ⇒ use the per-kind
    /// default ([`JobKind::default_timeout_secs`]).
    pub timeout_secs: Option<i64>,
}

impl Job {
    /// Effective wall-time budget: the per-job override, else the per-kind
    /// default, else the global [`DEFAULT_TIMEOUT_SECS`].
    pub fn effective_timeout_secs(&self) -> u64 {
        if let Some(t) = self.timeout_secs.filter(|t| *t > 0) {
            return t as u64;
        }
        JobKind::from_db_str(&self.kind)
            .map(|k| k.default_timeout_secs())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
    }
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

/// CO-78 internal submit API — enqueue any [`JobKind`].
///
/// `payload` is the already-serialized JSON body the worker will deserialize for
/// this kind. `dedupe_key` is an optional content-derived key: when supplied and
/// a job with the same key is already `pending`, `running`, or `done`, the
/// existing job ID is returned and no new row is inserted (idempotency). Pass
/// `None` to always create a fresh job.
///
/// CO-80 rate limiting: a genuinely-new job is rejected with
/// [`AppError::TooManyRequests`] when the universe has already enqueued
/// [`JobKind::hourly_rate_limit`] jobs of this kind in the trailing hour. A
/// deduped (no-op) submission never consumes rate budget.
///
/// The job is stamped with the per-kind wall-time budget
/// ([`JobKind::default_timeout_secs`]) into `jobs.timeout_secs`.
pub fn enqueue_job(
    conn: &Connection,
    universe_key: &str,
    kind: JobKind,
    payload: &str,
    dedupe_key: Option<&str>,
) -> Result<String, AppError> {
    // Idempotency: an active/done job with the same dedupe key short-circuits.
    if let Some(dk) = dedupe_key {
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
    }

    // CO-80: per-universe, per-kind hourly rate limit. Counted only for
    // genuinely-new jobs (a deduped submission already returned above).
    if let Some(limit) = kind.hourly_rate_limit() {
        let since = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let recent: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM jobs \
                 WHERE universe_key = ?1 AND kind = ?2 AND created_at >= ?3",
                params![universe_key, kind.as_str(), since],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if recent >= limit {
            return Err(AppError::TooManyRequests(format!(
                "rate limit: universe '{universe_key}' exceeded {limit} {kind}/hour jobs",
                kind = kind.as_str()
            )));
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let timeout = kind.default_timeout_secs() as i64;

    conn.execute(
        "INSERT INTO jobs (id, universe_key, kind, payload, status, attempts, dedupe_key, \
                           created_at, run_at, timeout_secs) \
         VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5, ?6, ?6, ?7)",
        params![
            id,
            universe_key,
            kind.as_str(),
            payload,
            dedupe_key,
            now,
            timeout
        ],
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(id)
}

/// Enqueue a doc-gen job. Returns the job ID.
///
/// Thin wrapper over [`enqueue_job`] that computes the doc-gen dedupe key and
/// serializes the payload. Idempotency + rate limiting apply as documented on
/// [`enqueue_job`].
pub fn enqueue_doc_gen(
    conn: &Connection,
    universe_key: &str,
    payload: &DocGenPayload,
) -> Result<String, AppError> {
    let dk = dedupe_key(universe_key, payload);
    let payload_json =
        serde_json::to_string(payload).map_err(|e| AppError::Internal(e.to_string()))?;
    enqueue_job(
        conn,
        universe_key,
        JobKind::DocGen,
        &payload_json,
        Some(&dk),
    )
}

/// Fetch a job by ID (for status polling).
pub fn get_job(conn: &Connection, job_id: &str) -> Option<Job> {
    conn.query_row(
        &format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = ?1"),
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
        timeout_secs: row.get(12)?,
    })
}

/// The column list selected into [`job_from_row`], in order. Kept as one
/// constant so every `SELECT … RETURNING` stays in lock-step with the mapper.
const JOB_COLUMNS: &str = "id, universe_key, kind, payload, status, attempts, dedupe_key, \
     created_at, run_at, started_at, completed_at, error, timeout_secs";

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
        &format!(
            "UPDATE jobs SET status = 'running', started_at = ?1 \
             WHERE id = ( \
               SELECT id FROM jobs \
               WHERE status = 'pending' AND run_at <= ?2 \
               ORDER BY run_at ASC, created_at ASC \
               LIMIT 1 \
             ) \
             RETURNING {JOB_COLUMNS}"
        ),
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
///
/// Dispatches on [`JobKind`]. `doc_gen` runs the adapter and persists entries.
/// The remaining kinds are wired by their owning features (CO-75 changelog,
/// CO-164 index, sync, webhooks); until then they are acknowledged as a no-op so
/// a mistakenly-enqueued job drains rather than spinning the retry loop. An
/// unrecognised kind string is a hard error so it surfaces in the dead-letter
/// queue for inspection.
fn process_job(job: &Job, storage: &Storage) -> Result<(), String> {
    let kind =
        JobKind::from_db_str(&job.kind).ok_or_else(|| format!("unknown job kind: {}", job.kind))?;

    match kind {
        JobKind::DocGen => process_doc_gen(job, storage),
        // Handlers for these kinds are owned by their feature tasks. CO-78
        // provides the queue/worker substrate; acknowledge so the row drains.
        JobKind::IndexRebuild
        | JobKind::ChangelogRegen
        | JobKind::SyncPull
        | JobKind::WebhookDispatch
        | JobKind::Cleanup => {
            info!(
                job_id = %job.id,
                kind = %job.kind,
                "job_queue: kind has no handler yet — acknowledged as no-op"
            );
            Ok(())
        }
    }
}

fn process_doc_gen(job: &Job, storage: &Storage) -> Result<(), String> {
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
    // Housekeeping first: reclaim jobs whose worker died mid-run (so they retry)
    // and cap the dead-letter queue (acceptance: ≤ 100 entries in normal ops).
    {
        let s = state.core.storage.lock();
        let reaped = reap_timed_out_jobs(s.conn());
        if reaped > 0 {
            warn!(reaped, "job_queue: reclaimed timed-out running jobs");
        }
        prune_dead_letter(s.conn(), DEAD_LETTER_CAP);
    }

    let job = {
        let s = state.core.storage.lock();
        claim_next_job(s.conn())
    };

    let Some(job) = job else {
        return Ok(());
    };

    let is_doc_gen = job.kind == JobKind::DocGen.as_str();
    let timeout_secs = job.effective_timeout_secs();

    info!(
        job_id = %job.id,
        universe = %job.universe_key,
        kind = %job.kind,
        attempt = job.attempts + 1,
        timeout_secs,
        "job_queue: processing"
    );

    // Run the job on a blocking thread so the async runtime is not starved. The
    // per-job wall-time limit (per-kind default, or `jobs.timeout_secs` override)
    // is enforced by the timeout wrapper below.
    let state_clone = state.clone();
    let job_clone = job.clone();

    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        tokio::task::spawn_blocking(move || {
            let s = state_clone.core.storage.lock();
            process_job(&job_clone, &s)
        }),
    )
    .await;

    let outcome: Result<(), String> = match result {
        Err(_timeout) => Err(format!("job timed out after {timeout_secs}s")),
        Ok(Err(join_err)) => Err(format!("task panicked: {join_err}")),
        Ok(Ok(inner)) => inner,
    };

    match outcome {
        Ok(()) => {
            info!(job_id = %job.id, universe = %job.universe_key, "job_queue: done");
            let s = state.core.storage.lock();
            mark_done(s.conn(), &job.id);
            if is_doc_gen {
                clear_universe_doc_gen_error(s.conn(), &job.universe_key);
            }
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
            let s = state.core.storage.lock();
            mark_failed(s.conn(), &job.id, attempts, &err);
            if is_doc_gen {
                set_universe_doc_gen_error(s.conn(), &job.universe_key, &err);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Maintenance: timeout reaping + dead-letter cap
// ---------------------------------------------------------------------------

/// Acceptance: the dead-letter queue holds at most this many rows in normal
/// operation. Older rows beyond the cap are pruned (the most recent failures are
/// the ones worth a human's attention).
pub const DEAD_LETTER_CAP: usize = 100;

/// Reclaim jobs stuck in `running` past their wall-time budget — the signature
/// of a worker that crashed or was killed mid-job mid-run. Each is failed via
/// the normal retry path (`mark_failed`), so it retries with backoff or moves to
/// the dead-letter queue once it exhausts its attempts. Returns the count reaped.
pub fn reap_timed_out_jobs(conn: &Connection) -> usize {
    // A generous grace beyond the budget avoids racing a still-running job: only
    // reclaim once `started_at + timeout + grace` is in the past.
    const GRACE_SECS: i64 = 30;
    let now = Utc::now();

    let stuck: Vec<(String, i64, i64)> = {
        let mut stmt = match conn.prepare(
            "SELECT id, attempts, COALESCE(timeout_secs, 0), started_at \
             FROM jobs WHERE status = 'running' AND started_at IS NOT NULL",
        ) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let attempts: i64 = row.get(1)?;
            let timeout: i64 = row.get(2)?;
            let started_at: String = row.get(3)?;
            Ok((id, attempts, timeout, started_at))
        });
        let Ok(rows) = rows else { return 0 };
        rows.flatten()
            .filter_map(|(id, attempts, timeout, started_at)| {
                let budget = if timeout > 0 {
                    timeout
                } else {
                    DEFAULT_TIMEOUT_SECS as i64
                };
                let started = chrono::DateTime::parse_from_rfc3339(&started_at).ok()?;
                let deadline =
                    started.with_timezone(&Utc) + chrono::Duration::seconds(budget + GRACE_SECS);
                (now > deadline).then_some((id, attempts, budget))
            })
            .collect()
    };

    let count = stuck.len();
    for (id, attempts, budget) in stuck {
        mark_failed(
            conn,
            &id,
            attempts + 1,
            &format!("worker died: running job exceeded {budget}s wall-time"),
        );
    }
    count
}

/// Keep the dead-letter queue bounded: delete all but the `keep` most recently
/// failed `dead_letter` rows. Returns the number of rows deleted.
pub fn prune_dead_letter(conn: &Connection, keep: usize) -> usize {
    conn.execute(
        "DELETE FROM jobs WHERE status = 'dead_letter' AND id NOT IN ( \
           SELECT id FROM jobs WHERE status = 'dead_letter' \
           ORDER BY COALESCE(completed_at, created_at) DESC LIMIT ?1 \
         )",
        params![keep as i64],
    )
    .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Per-kind queue metrics: status breakdown, success rate and p99 latency.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct KindMetrics {
    pub kind: String,
    pub pending: i64,
    pub running: i64,
    pub done: i64,
    pub failed: i64,
    pub dead_letter: i64,
    /// done / (done + dead_letter), in `[0,1]`. `1.0` when nothing terminal yet.
    pub success_rate: f64,
    /// 99th-percentile `completed_at - started_at` for recent `done` jobs, in ms.
    pub p99_latency_ms: i64,
}

/// Top-level queue metrics exposed at `/metrics`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct JobMetrics {
    /// Total pending (claimable) jobs across all kinds — the autoscale signal.
    pub queue_depth: i64,
    /// Total jobs currently being processed.
    pub running: i64,
    /// Total jobs in the dead-letter queue (acceptance: ≤ 100 in normal ops).
    pub dead_letter: i64,
    /// Recommended worker count for the current queue depth (4–16).
    pub desired_workers: u32,
    /// Per-kind breakdown.
    pub by_kind: Vec<KindMetrics>,
}

/// Number of pending (claimable) jobs — the autoscale signal.
pub fn queue_depth(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE status = 'pending'",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// Number of jobs in the dead-letter queue.
pub fn dead_letter_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE status = 'dead_letter'",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// p99 of `completed_at - started_at` (ms) over the most recent `done` jobs of
/// a kind. Computed in Rust (SQLite has no percentile aggregate). Bounded to the
/// last `LIMIT` rows so the scan stays cheap as the table grows.
fn p99_latency_ms(conn: &Connection, kind: &str) -> i64 {
    let mut stmt = match conn.prepare(
        "SELECT started_at, completed_at FROM jobs \
         WHERE kind = ?1 AND status = 'done' AND started_at IS NOT NULL \
           AND completed_at IS NOT NULL \
         ORDER BY completed_at DESC LIMIT 500",
    ) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let rows = stmt.query_map(params![kind], |row| {
        let started: String = row.get(0)?;
        let completed: String = row.get(1)?;
        Ok((started, completed))
    });
    let Ok(rows) = rows else { return 0 };

    let mut durations: Vec<i64> = rows
        .flatten()
        .filter_map(|(started, completed)| {
            let s = chrono::DateTime::parse_from_rfc3339(&started).ok()?;
            let c = chrono::DateTime::parse_from_rfc3339(&completed).ok()?;
            Some((c - s).num_milliseconds().max(0))
        })
        .collect();
    if durations.is_empty() {
        return 0;
    }
    durations.sort_unstable();
    // Nearest-rank p99: index ceil(0.99 * n) - 1.
    let rank = ((durations.len() as f64) * 0.99).ceil() as usize;
    let idx = rank.saturating_sub(1).min(durations.len() - 1);
    durations[idx]
}

/// Collect full queue metrics for every job kind.
pub fn collect_metrics(conn: &Connection) -> JobMetrics {
    let kinds = [
        JobKind::DocGen,
        JobKind::IndexRebuild,
        JobKind::ChangelogRegen,
        JobKind::SyncPull,
        JobKind::WebhookDispatch,
        JobKind::Cleanup,
    ];

    let mut by_kind = Vec::with_capacity(kinds.len());
    let (mut total_pending, mut total_running, mut total_dead) = (0i64, 0i64, 0i64);

    for kind in kinds {
        let count = |status: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM jobs WHERE kind = ?1 AND status = ?2",
                params![kind.as_str(), status],
                |r| r.get(0),
            )
            .unwrap_or(0)
        };
        let pending = count("pending");
        let running = count("running");
        let done = count("done");
        let failed = count("failed");
        let dead_letter = count("dead_letter");

        let terminal = done + dead_letter;
        let success_rate = if terminal > 0 {
            done as f64 / terminal as f64
        } else {
            1.0
        };

        total_pending += pending;
        total_running += running;
        total_dead += dead_letter;

        by_kind.push(KindMetrics {
            kind: kind.as_str().to_string(),
            pending,
            running,
            done,
            failed,
            dead_letter,
            success_rate,
            p99_latency_ms: p99_latency_ms(conn, kind.as_str()),
        });
    }

    JobMetrics {
        queue_depth: total_pending,
        running: total_running,
        dead_letter: total_dead,
        desired_workers: desired_worker_count(total_pending, total_running),
        by_kind,
    }
}

// ---------------------------------------------------------------------------
// Worker-pool autoscale
// ---------------------------------------------------------------------------

/// Minimum / maximum worker machines in the pool (spec: 4–16).
pub const MIN_WORKERS: u32 = 4;
pub const MAX_WORKERS: u32 = 16;
/// Scale up one machine per this many pending jobs above the floor.
const JOBS_PER_WORKER: i64 = 100;

/// Recommended worker-machine count for the current queue depth.
///
/// Spec: 4–16 machines, scale up on queue depth > 100. Idle pools (no pending,
/// nothing running) hold at the floor of [`MIN_WORKERS`]; deeper queues add one
/// machine per [`JOBS_PER_WORKER`] backlog, clamped to [`MAX_WORKERS`].
pub fn desired_worker_count(queue_depth: i64, running: i64) -> u32 {
    if queue_depth <= 0 && running <= 0 {
        return MIN_WORKERS;
    }
    let extra = (queue_depth / JOBS_PER_WORKER) as u32;
    (MIN_WORKERS + extra).clamp(MIN_WORKERS, MAX_WORKERS)
}

// ---------------------------------------------------------------------------
// HTTP: GET /metrics
// ---------------------------------------------------------------------------

/// `GET /metrics` — queue depth, dead-letter count, autoscale recommendation,
/// and per-kind success rate + p99 latency. Public + aggregate only (no
/// universe keys), mirroring `/health`.
pub async fn metrics_handler(State(state): State<AppState>) -> Json<JobMetrics> {
    let s = state.core.storage.lock();
    Json(collect_metrics(s.conn()))
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

    // -----------------------------------------------------------------------
    // CO-78: generic submit API, kinds, timeout, rate limit, metrics, autoscale
    // -----------------------------------------------------------------------

    /// Insert a raw job row directly (bypassing enqueue) so tests can stage
    /// arbitrary status / timestamps for metrics, reaper, and prune coverage.
    #[allow(clippy::too_many_arguments)]
    fn insert_raw(
        conn: &Connection,
        id: &str,
        kind: &str,
        status: &str,
        attempts: i64,
        timeout_secs: Option<i64>,
        started_at: Option<&str>,
        completed_at: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO jobs (id, universe_key, kind, payload, status, attempts, \
                               created_at, run_at, started_at, completed_at, timeout_secs) \
             VALUES (?1, 'u1', ?2, '{}', ?3, ?4, '2026-01-01T00:00:00Z', \
                     '2026-01-01T00:00:00Z', ?5, ?6, ?7)",
            params![
                id,
                kind,
                status,
                attempts,
                started_at,
                completed_at,
                timeout_secs
            ],
        )
        .unwrap();
    }

    #[test]
    fn job_kind_string_round_trips() {
        for kind in [
            JobKind::DocGen,
            JobKind::IndexRebuild,
            JobKind::ChangelogRegen,
            JobKind::SyncPull,
            JobKind::WebhookDispatch,
            JobKind::Cleanup,
        ] {
            assert_eq!(JobKind::from_db_str(kind.as_str()), Some(kind));
        }
        assert_eq!(JobKind::from_db_str("nope"), None);
    }

    #[test]
    fn enqueue_job_generic_surfaces_in_queue_immediately() {
        let (storage, _tmp) = make_storage();
        let id = enqueue_job(
            storage.conn(),
            "u1",
            JobKind::ChangelogRegen,
            r#"{"v":1}"#,
            Some("dedupe-cr-1"),
        )
        .unwrap();

        // Visible right away as pending, contributing to queue depth.
        let job = get_job(storage.conn(), &id).unwrap();
        assert_eq!(job.status, "pending");
        assert_eq!(job.kind, "changelog_regen");
        assert_eq!(queue_depth(storage.conn()), 1);
    }

    #[test]
    fn enqueue_job_without_dedupe_key_always_creates_new() {
        let (storage, _tmp) = make_storage();
        let a = enqueue_job(storage.conn(), "u1", JobKind::Cleanup, "{}", None).unwrap();
        let b = enqueue_job(storage.conn(), "u1", JobKind::Cleanup, "{}", None).unwrap();
        assert_ne!(a, b, "no dedupe key ⇒ distinct jobs");
        assert_eq!(queue_depth(storage.conn()), 2);
    }

    #[test]
    fn enqueue_job_dedupes_on_key() {
        let (storage, _tmp) = make_storage();
        let a = enqueue_job(storage.conn(), "u1", JobKind::SyncPull, "{}", Some("k")).unwrap();
        let b = enqueue_job(storage.conn(), "u1", JobKind::SyncPull, "{}", Some("k")).unwrap();
        assert_eq!(a, b, "same dedupe key ⇒ existing job id, no new row");
        assert_eq!(queue_depth(storage.conn()), 1);
    }

    #[test]
    fn enqueue_stamps_per_kind_timeout() {
        let (storage, _tmp) = make_storage();
        let id = enqueue_job(storage.conn(), "u1", JobKind::IndexRebuild, "{}", None).unwrap();
        let job = get_job(storage.conn(), &id).unwrap();
        assert_eq!(
            job.timeout_secs,
            Some(JobKind::IndexRebuild.default_timeout_secs() as i64)
        );
        assert_eq!(
            job.effective_timeout_secs(),
            JobKind::IndexRebuild.default_timeout_secs()
        );
    }

    #[test]
    fn effective_timeout_falls_back_to_kind_default_when_null() {
        let (storage, _tmp) = make_storage();
        insert_raw(
            storage.conn(),
            "j",
            "changelog_regen",
            "pending",
            0,
            None,
            None,
            None,
        );
        let job = get_job(storage.conn(), "j").unwrap();
        assert_eq!(
            job.effective_timeout_secs(),
            JobKind::ChangelogRegen.default_timeout_secs()
        );
    }

    #[test]
    fn rate_limit_rejects_after_hourly_cap_for_kind() {
        let (storage, _tmp) = make_storage();
        let limit = JobKind::IndexRebuild.hourly_rate_limit().unwrap();
        // Fill the budget (no dedupe key ⇒ each counts).
        for _ in 0..limit {
            enqueue_job(storage.conn(), "u1", JobKind::IndexRebuild, "{}", None).unwrap();
        }
        let err = enqueue_job(storage.conn(), "u1", JobKind::IndexRebuild, "{}", None).unwrap_err();
        assert!(
            matches!(err, AppError::TooManyRequests(_)),
            "over-limit submit must be TooManyRequests, got {err:?}"
        );
        // A different universe is unaffected (per-universe limit).
        assert!(enqueue_job(storage.conn(), "u2", JobKind::IndexRebuild, "{}", None).is_ok());
    }

    #[test]
    fn deduped_submit_does_not_consume_rate_budget() {
        let (storage, _tmp) = make_storage();
        let limit = JobKind::ChangelogRegen.hourly_rate_limit().unwrap();
        // Submitting the SAME dedupe key many times stays at one job.
        for _ in 0..(limit + 5) {
            enqueue_job(
                storage.conn(),
                "u1",
                JobKind::ChangelogRegen,
                "{}",
                Some("same"),
            )
            .unwrap();
        }
        assert_eq!(queue_depth(storage.conn()), 1);
    }

    #[test]
    fn desired_worker_count_scales_within_bounds() {
        // Idle pool holds at the floor.
        assert_eq!(desired_worker_count(0, 0), MIN_WORKERS);
        // Busy but shallow queue stays at the floor.
        assert_eq!(desired_worker_count(50, 2), MIN_WORKERS);
        // Each 100 backlog adds one worker.
        assert_eq!(desired_worker_count(250, 0), MIN_WORKERS + 2);
        // Clamped to the ceiling for huge backlogs.
        assert_eq!(desired_worker_count(100_000, 0), MAX_WORKERS);
    }

    #[test]
    fn metrics_report_depth_success_rate_and_p99() {
        let (storage, _tmp) = make_storage();
        let conn = storage.conn();

        // doc_gen: 2 pending, 1 done (200ms), 1 dead_letter.
        insert_raw(conn, "p1", "doc_gen", "pending", 0, None, None, None);
        insert_raw(conn, "p2", "doc_gen", "pending", 0, None, None, None);
        insert_raw(
            conn,
            "d1",
            "doc_gen",
            "done",
            1,
            None,
            Some("2026-01-01T00:00:00Z"),
            Some("2026-01-01T00:00:00.200Z"),
        );
        insert_raw(conn, "x1", "doc_gen", "dead_letter", 5, None, None, None);

        let m = collect_metrics(conn);
        assert_eq!(m.queue_depth, 2);
        assert_eq!(m.dead_letter, 1);
        assert!(m.desired_workers >= MIN_WORKERS);

        let dg = m.by_kind.iter().find(|k| k.kind == "doc_gen").unwrap();
        assert_eq!(dg.pending, 2);
        assert_eq!(dg.done, 1);
        assert_eq!(dg.dead_letter, 1);
        // success_rate = done / (done + dead_letter) = 1/2.
        assert!((dg.success_rate - 0.5).abs() < 1e-9);
        assert_eq!(dg.p99_latency_ms, 200);
    }

    #[test]
    fn success_rate_defaults_to_one_with_no_terminal_jobs() {
        let (storage, _tmp) = make_storage();
        insert_raw(
            storage.conn(),
            "p",
            "sync_pull",
            "pending",
            0,
            None,
            None,
            None,
        );
        let m = collect_metrics(storage.conn());
        let sp = m.by_kind.iter().find(|k| k.kind == "sync_pull").unwrap();
        assert_eq!(sp.success_rate, 1.0);
    }

    #[test]
    fn prune_dead_letter_caps_queue() {
        let (storage, _tmp) = make_storage();
        let conn = storage.conn();
        for i in 0..150 {
            let ts = format!("2026-01-01T00:{:02}:{:02}Z", i / 60, i % 60);
            insert_raw(
                conn,
                &format!("dl{i}"),
                "doc_gen",
                "dead_letter",
                5,
                None,
                None,
                Some(&ts),
            );
        }
        assert_eq!(dead_letter_count(conn), 150);
        let deleted = prune_dead_letter(conn, DEAD_LETTER_CAP);
        assert_eq!(deleted, 50);
        assert_eq!(dead_letter_count(conn), DEAD_LETTER_CAP as i64);
    }

    #[test]
    fn reap_reclaims_jobs_stuck_running_past_budget() {
        let (storage, _tmp) = make_storage();
        let conn = storage.conn();

        // Running far past its budget (started in the distant past) → reaped.
        insert_raw(
            conn,
            "stuck",
            "doc_gen",
            "running",
            0,
            Some(60),
            Some("2020-01-01T00:00:00Z"),
            None,
        );
        // Running but only just started (recent) → not reaped.
        let recent = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        insert_raw(
            conn,
            "fresh",
            "doc_gen",
            "running",
            0,
            Some(600),
            Some(&recent),
            None,
        );

        let reaped = reap_timed_out_jobs(conn);
        assert_eq!(reaped, 1);

        // The stuck job is failed (retryable) with attempts incremented.
        let stuck = get_job(conn, "stuck").unwrap();
        assert_eq!(stuck.status, "failed");
        assert_eq!(stuck.attempts, 1);
        // The fresh job is untouched.
        assert_eq!(get_job(conn, "fresh").unwrap().status, "running");
    }

    #[test]
    fn process_job_acks_unwired_kinds_and_errors_on_unknown() {
        let (storage, _tmp) = make_storage();
        let mut cleanup = get_job_template("cleanup");
        cleanup.kind = "cleanup".into();
        assert!(process_job(&cleanup, &storage).is_ok(), "known kind no-ops");

        let mut bogus = get_job_template("???");
        bogus.kind = "not_a_kind".into();
        assert!(
            process_job(&bogus, &storage).is_err(),
            "unknown kind is a hard error → dead-letter for inspection"
        );
    }

    fn get_job_template(kind: &str) -> Job {
        Job {
            id: "t".into(),
            universe_key: "u1".into(),
            kind: kind.into(),
            payload: "{}".into(),
            status: "running".into(),
            attempts: 0,
            dedupe_key: None,
            created_at: "now".into(),
            run_at: "now".into(),
            started_at: None,
            completed_at: None,
            error: None,
            timeout_secs: None,
        }
    }
}
