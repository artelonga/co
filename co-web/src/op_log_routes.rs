//! Op log routes — CO-95 phases 2-4.
//!
//! Provides time-travel, diff, and cross-universe branching operations
//! over the per-universe `entry_events` append-only log.
//!
//! # Phases
//!
//! **Phase 2 — Ops listing**
//! ```text
//! GET /{slug}/ops          list entry_events (?after_seq=N&limit=N)
//! ```
//!
//! **Phase 3 — Replay and diff**
//! ```text
//! GET /{slug}/replay       replay to op N (?to_op=N)
//! GET /{slug}/op-diff      diff between two op IDs (?from_op=M&to_op=N)
//! ```
//!
//! **Phase 4 — Branch operations**
//! ```text
//! GET  /{slug}/diff         diff vs another universe (?against=<key>)
//! POST /{slug}/promote      promote entries into target universe
//! POST /{slug}/revert       revert universe to a historical op (?to=<op_id>)
//! POST /{slug}/cherry-pick  cherry-pick specific paths into target
//! ```
//!
//! All routes require authentication and are expected to be nested under
//! `/api/v1/universes` inside the `universe_content_api` router (already
//! behind `universe_visibility_gate` / `universe_writer_gate` middleware).

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::auth::resolve_user_id;
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Public router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        // Phase 2
        .route("/{slug}/ops", get(list_ops))
        // Phase 3
        .route("/{slug}/replay", get(replay_to_op))
        .route("/{slug}/op-diff", get(op_diff))
        // Phase 4
        .route("/{slug}/diff", get(universe_diff))
        .route("/{slug}/promote", post(promote))
        .route("/{slug}/revert", post(revert))
        .route("/{slug}/cherry-pick", post(cherry_pick))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convenience: lock `state.storage` and return the Arc<Mutex<Connection>>
/// for `universe_key`.  Returns `NotFound` when the universe doesn't exist.
fn get_universe_conn(
    state: &AppState,
    universe_key: &str,
) -> Result<std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>, AppError> {
    let storage = state.storage.lock();
    if storage.get_universe(universe_key).is_none() {
        return Err(AppError::NotFound(format!(
            "Universe '{universe_key}' not found"
        )));
    }
    Ok(storage.universe_conn(universe_key))
}

/// Assert that `user_id` has write access (is owner or member) of `universe_key`.
fn require_write_access(
    state: &AppState,
    universe_key: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let storage = state.storage.lock();
    let universe = storage
        .get_universe(universe_key)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))?;
    if universe.owner_id == user_id || storage.is_universe_member(universe_key, user_id) {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "You do not have write access to universe '{universe_key}'"
        )))
    }
}

// ---------------------------------------------------------------------------
// Shared data types
// ---------------------------------------------------------------------------

/// A row from `entry_events` as returned by the API.
#[derive(Debug, Serialize)]
pub struct OpEvent {
    pub seq: i64,
    pub ts_micros: i64,
    pub op: String,
    pub path: String,
    pub body_hash: Option<String>,
    pub prev_body_hash: Option<String>,
    pub frontmatter_json: Option<String>,
    pub author_id: Option<String>,
    pub request_id: Option<String>,
}

/// The last-known state for a path, derived from the event log up to some seq.
#[derive(Debug, Clone)]
struct PathState {
    op: String, // "put" or "delete"
    body_hash: Option<String>,
    body: Option<String>,
    frontmatter_json: Option<String>,
}

/// Build a `path → PathState` map for all events where `seq <= max_seq`.
/// Iterates from newest to oldest; only the first (newest) event per path is kept.
fn replay_state(
    conn: &rusqlite::Connection,
    max_seq: i64,
) -> Result<HashMap<String, PathState>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT path, op, body_hash, body, frontmatter_json \
             FROM entry_events \
             WHERE seq <= ?1 \
             ORDER BY seq DESC",
        )
        .map_err(|e| AppError::Internal(format!("replay_state prepare: {e}")))?;

    let mut map: HashMap<String, PathState> = HashMap::new();
    let rows = stmt
        .query_map(params![max_seq], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| AppError::Internal(format!("replay_state query: {e}")))?;

    for row in rows.flatten() {
        let (path, op, body_hash, body_bytes, frontmatter_json) = row;
        // Keep only the newest event per path (ORDER BY seq DESC → first wins).
        map.entry(path).or_insert_with(|| PathState {
            op,
            body_hash,
            body: body_bytes
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).into_owned()),
            frontmatter_json,
        });
    }

    Ok(map)
}

// ---------------------------------------------------------------------------
// Phase 2: GET /{slug}/ops
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OpsQuery {
    /// Return only events with `seq > after_seq`.  Omit for all events.
    pub after_seq: Option<i64>,
    /// Maximum number of events to return.  Defaults to 100; capped at 1 000.
    pub limit: Option<usize>,
}

/// `GET /api/v1/universes/:slug/ops`
///
/// List `entry_events` for the universe, optionally paginated via `?after_seq=N`.
pub async fn list_ops(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<OpsQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    let limit = q.limit.unwrap_or(100).min(1_000) as i64;
    let after_seq = q.after_seq.unwrap_or(0);

    let uc = get_universe_conn(&state, &slug)?;
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    let mut stmt = uc_guard
        .prepare(
            "SELECT seq, ts_micros, op, path, body_hash, prev_body_hash, \
                    frontmatter_json, author_id, request_id \
             FROM entry_events \
             WHERE seq > ?1 \
             ORDER BY seq ASC \
             LIMIT ?2",
        )
        .map_err(|e| AppError::Internal(format!("list_ops prepare: {e}")))?;

    let events: Vec<OpEvent> = stmt
        .query_map(params![after_seq, limit], |row| {
            Ok(OpEvent {
                seq: row.get(0)?,
                ts_micros: row.get(1)?,
                op: row.get(2)?,
                path: row.get(3)?,
                body_hash: row.get(4)?,
                prev_body_hash: row.get(5)?,
                frontmatter_json: row.get(6)?,
                author_id: row.get(7)?,
                request_id: row.get(8)?,
            })
        })
        .map_err(|e| AppError::Internal(format!("list_ops query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    let total = events.len();
    Ok(Json(json!({
        "events": events,
        "total": total,
        "after_seq": after_seq,
    })))
}

// ---------------------------------------------------------------------------
// Phase 3: GET /{slug}/replay
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ReplayQuery {
    /// Replay all events up to and including this sequence number.
    pub to_op: i64,
}

/// `GET /api/v1/universes/:slug/replay?to_op=N`
///
/// Compute the logical state of the universe as of operation `to_op`.
/// Returns the set of "active" paths (whose last event ≤ to_op was `put`)
/// and "deleted" paths (whose last event ≤ to_op was `delete`).
pub async fn replay_to_op(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<ReplayQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    let uc = get_universe_conn(&state, &slug)?;
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    let state_map = replay_state(&uc_guard, q.to_op)?;

    let mut active: Vec<JsonValue> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();

    for (path, ps) in state_map {
        if ps.op == "put" {
            active.push(json!({
                "path": path,
                "body_hash": ps.body_hash,
                "frontmatter_json": ps.frontmatter_json,
            }));
        } else {
            deleted.push(path);
        }
    }

    active.sort_by(|a, b| {
        a.get("path")
            .and_then(|v| v.as_str())
            .cmp(&b.get("path").and_then(|v| v.as_str()))
    });
    deleted.sort();

    Ok(Json(json!({
        "to_op": q.to_op,
        "active": active,
        "deleted": deleted,
    })))
}

// ---------------------------------------------------------------------------
// Phase 3: GET /{slug}/op-diff
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OpDiffQuery {
    /// Baseline sequence number (exclusive lower bound).
    pub from_op: i64,
    /// Target sequence number (inclusive upper bound).
    pub to_op: i64,
}

/// One changed path in an op-diff result.
#[derive(Debug, Serialize)]
pub struct OpDiffEntry {
    pub path: String,
    /// `"added"`, `"modified"`, or `"deleted"`.
    pub op: String,
    pub body_hash_before: Option<String>,
    pub body_hash_after: Option<String>,
}

/// `GET /api/v1/universes/:slug/op-diff?from_op=M&to_op=N`
///
/// Return the set of paths that changed between op M (exclusive) and op N
/// (inclusive), together with their before/after body hashes.
pub async fn op_diff(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<OpDiffQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    if q.from_op > q.to_op {
        return Err(AppError::BadRequest("from_op must be <= to_op".into()));
    }

    let uc = get_universe_conn(&state, &slug)?;
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    // Find all paths that have at least one event in the window (from_op, to_op].
    let mut changed_paths: Vec<String> = {
        let mut stmt = uc_guard
            .prepare(
                "SELECT DISTINCT path FROM entry_events \
                 WHERE seq > ?1 AND seq <= ?2",
            )
            .map_err(|e| AppError::Internal(format!("op_diff paths prepare: {e}")))?;
        stmt.query_map(params![q.from_op, q.to_op], |row| row.get::<_, String>(0))
            .map_err(|e| AppError::Internal(format!("op_diff paths query: {e}")))?
            .filter_map(|r| r.ok())
            .collect()
    };
    changed_paths.sort();

    // Build state maps for the baseline and the target.
    let before_map = replay_state(&uc_guard, q.from_op)?;
    let after_map = replay_state(&uc_guard, q.to_op)?;

    let mut diffs: Vec<OpDiffEntry> = Vec::new();
    for path in changed_paths {
        let before = before_map.get(&path);
        let after = after_map.get(&path);

        let op = match (before, after) {
            (None, Some(a)) if a.op == "put" => "added",
            (Some(b), None) if b.op == "put" => "deleted",
            (Some(b), Some(a)) => {
                match (b.op.as_str(), a.op.as_str()) {
                    ("put", "delete") => "deleted",
                    ("delete", "put") => "added",
                    ("put", "put") => {
                        if b.body_hash != a.body_hash {
                            "modified"
                        } else {
                            continue; // no meaningful change
                        }
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };

        let body_hash_before = before
            .filter(|b| b.op == "put")
            .and_then(|b| b.body_hash.clone());
        let body_hash_after = after
            .filter(|a| a.op == "put")
            .and_then(|a| a.body_hash.clone());

        diffs.push(OpDiffEntry {
            path,
            op: op.to_string(),
            body_hash_before,
            body_hash_after,
        });
    }

    Ok(Json(json!({
        "from_op": q.from_op,
        "to_op": q.to_op,
        "changes": diffs,
    })))
}

// ---------------------------------------------------------------------------
// Phase 4: GET /{slug}/diff — universe vs universe
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct UniverseDiffQuery {
    /// Universe key to compare against (the "target" side).
    pub against: String,
}

/// `GET /api/v1/universes/:slug/diff?against=<key>`
///
/// Compare the current entry set of `:slug` (source) against another universe
/// (`against`, the target).  Returns `added` (in source, not target),
/// `deleted` (in target, not source), and `modified` (same path, different
/// `body_hash`).
pub async fn universe_diff(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<UniverseDiffQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    // Load source entries (slug).
    let source_entries: HashMap<String, String> = {
        let uc = get_universe_conn(&state, &slug)?;
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let mut stmt = uc_guard
            .prepare("SELECT path, body_hash FROM entries WHERE universe_key = ?1")
            .map_err(|e| AppError::Internal(format!("diff source prepare: {e}")))?;
        stmt.query_map(params![slug], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Internal(format!("diff source query: {e}")))?
        .filter_map(|r| r.ok())
        .collect()
    };

    // Load target entries (against).
    let target_entries: HashMap<String, String> = {
        let uc = get_universe_conn(&state, &q.against)?;
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let mut stmt = uc_guard
            .prepare("SELECT path, body_hash FROM entries WHERE universe_key = ?1")
            .map_err(|e| AppError::Internal(format!("diff target prepare: {e}")))?;
        stmt.query_map(params![q.against], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Internal(format!("diff target query: {e}")))?
        .filter_map(|r| r.ok())
        .collect()
    };

    let mut added: Vec<String> = Vec::new();
    let mut modified: Vec<String> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();

    for (path, src_hash) in &source_entries {
        match target_entries.get(path) {
            None => added.push(path.clone()),
            Some(tgt_hash) if tgt_hash != src_hash => modified.push(path.clone()),
            _ => {}
        }
    }
    for path in target_entries.keys() {
        if !source_entries.contains_key(path) {
            deleted.push(path.clone());
        }
    }

    added.sort();
    modified.sort();
    deleted.sort();

    Ok(Json(json!({
        "source": slug,
        "target": q.against,
        "added": added,
        "modified": modified,
        "deleted": deleted,
    })))
}

// ---------------------------------------------------------------------------
// Phase 4: POST /{slug}/promote
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PromoteRequest {
    pub target: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PromoteConflict {
    pub path: String,
    pub source_hash: String,
    pub target_hash: String,
}

/// `POST /api/v1/universes/:slug/promote`
///
/// Apply all entries from the source universe (`:slug`) into the target
/// universe, last-write-wins.  Conflicts (same path, different hash) are
/// reported but still applied.
///
/// The caller must have write access to both the source and the target.
pub async fn promote(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<PromoteRequest>,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    // Verify write access to source and target.
    require_write_access(&state, &slug, &caller)?;
    require_write_access(&state, &req.target, &caller)?;

    // Snapshot the source entries before we begin writing to the target.
    // We hold no locks across the write loop to avoid deadlock.
    #[allow(clippy::type_complexity)]
    let source_rows: Vec<(String, String, Option<String>, String, String)> = {
        let uc = get_universe_conn(&state, &slug)?;
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let mut stmt = uc_guard
            .prepare(
                "SELECT path, body_hash, frontmatter_json, body, \
                        COALESCE(entry_type, 'note') \
                 FROM entries WHERE universe_key = ?1",
            )
            .map_err(|e| AppError::Internal(format!("promote src prepare: {e}")))?;
        stmt.query_map(params![slug], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| AppError::Internal(format!("promote src query: {e}")))?
        .filter_map(|r| r.ok())
        .collect()
    };

    // Snapshot the target body_hashes for conflict detection.
    let target_hashes: HashMap<String, String> = {
        let uc = get_universe_conn(&state, &req.target)?;
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let mut stmt = uc_guard
            .prepare("SELECT path, body_hash FROM entries WHERE universe_key = ?1")
            .map_err(|e| AppError::Internal(format!("promote tgt prepare: {e}")))?;
        stmt.query_map(params![req.target], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Internal(format!("promote tgt query: {e}")))?
        .filter_map(|r| r.ok())
        .collect()
    };

    let mut added = 0usize;
    let mut overwritten = 0usize;
    let mut conflicts: Vec<PromoteConflict> = Vec::new();

    for (path, src_hash, fm_json, body, _entry_type) in &source_rows {
        let tgt_hash = target_hashes.get(path);
        let is_new = tgt_hash.is_none();
        let is_conflict = tgt_hash.is_some_and(|h| h != src_hash);

        if is_conflict {
            conflicts.push(PromoteConflict {
                path: path.clone(),
                source_hash: src_hash.clone(),
                target_hash: tgt_hash.unwrap().clone(),
            });
        }

        // Parse frontmatter from JSON (fall back to empty object).
        let frontmatter: JsonValue = fm_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(json!({}));

        crate::vault_routes::write_vault_entry(&state, &req.target, path, frontmatter, body)?;

        if is_new {
            added += 1;
        } else {
            overwritten += 1;
        }
    }

    // Write audit entry in the target universe.
    let now = chrono::Utc::now().to_rfc3339();
    let audit_path = format!(
        "_audit/promote-{}.md",
        chrono::Utc::now().timestamp_micros()
    );
    let audit_fm = json!({
        "type": "audit",
        "title": format!("Promote from {slug}"),
        "action": "promote",
        "source": slug,
        "target": req.target,
        "author": caller,
        "timestamp": now,
        "message": req.message,
        "added": added,
        "overwritten": overwritten,
        "conflicts": conflicts.len(),
    });
    let audit_body = format!(
        "# Promote: `{slug}` → `{target}`\n\
        \n\
        - **Source:** {slug}\n\
        - **Target:** {target}\n\
        - **Author:** {caller}\n\
        - **Timestamp:** {now}\n\
        - **Added:** {added}\n\
        - **Overwritten:** {overwritten}\n\
        - **Conflicts:** {n_conflicts}\n",
        slug = slug,
        target = req.target,
        caller = caller,
        now = now,
        added = added,
        overwritten = overwritten,
        n_conflicts = conflicts.len(),
    );
    // Non-fatal: audit entry failure should not block promote response.
    if let Err(e) = crate::vault_routes::write_vault_entry(
        &state,
        &req.target,
        &audit_path,
        audit_fm,
        &audit_body,
    ) {
        tracing::warn!("promote audit entry failed for {}: {e}", req.target);
    }

    // Refresh content_count on the target.
    refresh_content_count(&state, &req.target);

    let audit_seq: Option<i64> = {
        let uc = get_universe_conn(&state, &req.target).ok();
        uc.and_then(|uc| {
            uc.lock().ok().and_then(|g| {
                g.query_row("SELECT MAX(seq) FROM entry_events", [], |r| {
                    r.get::<_, Option<i64>>(0)
                })
                .ok()
                .flatten()
            })
        })
    };

    Ok(Json(json!({
        "added": added,
        "overwritten": overwritten,
        "conflicts": conflicts,
        "audit_seq": audit_seq,
    })))
}

// ---------------------------------------------------------------------------
// Phase 4: POST /{slug}/revert
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RevertQuery {
    /// Revert universe to the state as of this op sequence number.
    pub to: i64,
}

/// `POST /api/v1/universes/:slug/revert?to=<op_id>`
///
/// Revert the universe to its logical state at op `to`.
///
/// - Paths whose last event ≤ `to` was `put`: restore that historical body.
/// - Paths currently in the `entries` table that have no event ≤ `to` (i.e.
///   they were created after `to`), or whose last event ≤ `to` was `delete`:
///   remove from the live index and disk.
pub async fn revert(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(q): Query<RevertQuery>,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    require_write_access(&state, &slug, &caller)?;

    // Build the target state map.
    let target_state: HashMap<String, PathState> = {
        let uc = get_universe_conn(&state, &slug)?;
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        replay_state(&uc_guard, q.to)?
    };

    // Current live paths.
    let live_paths: Vec<String> = {
        let uc = get_universe_conn(&state, &slug)?;
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let mut stmt = uc_guard
            .prepare("SELECT path FROM entries WHERE universe_key = ?1")
            .map_err(|e| AppError::Internal(format!("revert live prepare: {e}")))?;
        stmt.query_map(params![slug], |row| row.get::<_, String>(0))
            .map_err(|e| AppError::Internal(format!("revert live query: {e}")))?
            .filter_map(|r| r.ok())
            .collect()
    };

    let mut restored = 0usize;
    let mut deleted = 0usize;

    // Restore or delete each currently-live path.
    for path in &live_paths {
        match target_state.get(path.as_str()) {
            Some(ps) if ps.op == "put" => {
                // Restore historical body.
                let body = ps.body.as_deref().unwrap_or("");
                let frontmatter: JsonValue = ps
                    .frontmatter_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));
                crate::vault_routes::write_vault_entry(&state, &slug, path, frontmatter, body)?;
                restored += 1;
            }
            _ => {
                // Path did not exist (or was deleted) at op `to` — remove it.
                delete_entry_from_universe(&state, &slug, path)?;
                deleted += 1;
            }
        }
    }

    // Apply puts for paths that exist in the historical state but are NOT
    // currently live (i.e. they were deleted after op `to`).
    for (path, ps) in &target_state {
        if ps.op == "put" && !live_paths.iter().any(|p| p == path) {
            let body = ps.body.as_deref().unwrap_or("");
            let frontmatter: JsonValue = ps
                .frontmatter_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(json!({}));
            crate::vault_routes::write_vault_entry(&state, &slug, path, frontmatter, body)?;
            restored += 1;
        }
    }

    refresh_content_count(&state, &slug);

    Ok(Json(json!({
        "restored": restored,
        "deleted": deleted,
        "reverted_to_op": q.to,
    })))
}

// ---------------------------------------------------------------------------
// Phase 4: POST /{slug}/cherry-pick
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CherryPickRequest {
    pub target: String,
    pub paths: Vec<String>,
}

/// `POST /api/v1/universes/:slug/cherry-pick`
///
/// Copy specific entries from the source universe (`:slug`) into the target.
/// The caller must have write access to both universes.
pub async fn cherry_pick(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CherryPickRequest>,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    require_write_access(&state, &slug, &caller)?;
    require_write_access(&state, &req.target, &caller)?;

    if req.paths.is_empty() {
        return Err(AppError::BadRequest("paths must not be empty".into()));
    }

    // Fetch the requested entries from the source universe in one lock acquisition.
    // We build a map path → (frontmatter_json, body) for all found paths.
    let mut found: HashMap<String, (Option<String>, String)> = HashMap::new();
    {
        let uc = get_universe_conn(&state, &slug)?;
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        for path in &req.paths {
            let row: Result<(Option<String>, String), _> = uc_guard.query_row(
                "SELECT frontmatter_json, body FROM entries \
                 WHERE universe_key = ?1 AND path = ?2",
                params![slug, path],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
            );
            if let Ok((fm_json, body)) = row {
                found.insert(path.clone(), (fm_json, body));
            }
        }
    }

    let mut applied: Vec<String> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    for path in &req.paths {
        if let Some((fm_json, body)) = found.get(path) {
            let frontmatter: JsonValue = fm_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(json!({}));
            crate::vault_routes::write_vault_entry(&state, &req.target, path, frontmatter, body)?;
            applied.push(path.clone());
        } else {
            not_found.push(path.clone());
        }
    }

    refresh_content_count(&state, &req.target);

    Ok(Json(json!({
        "applied": applied,
        "not_found": not_found,
    })))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Delete an entry from the live `entries` index and its on-disk `.md` file.
/// Non-fatal disk errors are logged but do not abort the operation.
fn delete_entry_from_universe(
    state: &AppState,
    universe_key: &str,
    path: &str,
) -> Result<(), AppError> {
    let (uc, universe_root) = {
        let storage = state.storage.lock();
        let uc = storage.universe_conn(universe_key);
        let root = storage.universe_root(universe_key);
        (uc, root)
    };

    // Remove the on-disk file.
    let full_path = universe_root.join(path);
    if full_path.exists()
        && let Err(e) = std::fs::remove_file(&full_path)
    {
        tracing::warn!("revert: failed to remove {}: {e}", full_path.display());
    }

    // Remove from the SQLite index.
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = crate::entry_index::EntryIndex::new(&uc_guard);
    index
        .remove(universe_key, path)
        .map_err(|e| AppError::Internal(format!("delete_entry_from_universe remove: {e}")))?;

    // Log the delete event.
    if let Err(e) = index.log_event(path, "delete", None, None, None, None, None, None) {
        tracing::warn!("revert delete event log failed for {universe_key}/{path}: {e}");
    }

    Ok(())
}

/// Refresh `content_count` in the `universes` table from the live `entries`
/// count.  Non-fatal: errors are logged but do not propagate to callers.
fn refresh_content_count(state: &AppState, universe_key: &str) {
    let uc = {
        let storage = state.storage.lock();
        storage.universe_conn(universe_key)
    };
    if let Ok(uc_guard) = uc.lock() {
        let actual_count: Option<i64> = uc_guard
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .ok();
        if let Some(n) = actual_count {
            let storage = state.storage.lock();
            if let Err(e) = storage.conn().execute(
                "UPDATE universes SET content_count = ?1 WHERE key = ?2",
                params![n, universe_key],
            ) {
                tracing::warn!("refresh_content_count failed for {universe_key}: {e}");
            }
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
    fn test_router_builds() {
        // Smoke test: `router()` should compile and produce a Router without panicking.
        let _r: Router<AppState> = router();
    }

    #[test]
    fn test_ops_query_limit_clamp() {
        let q = OpsQuery {
            after_seq: None,
            limit: Some(99_999),
        };
        let effective = q.limit.unwrap_or(100).min(1_000);
        assert_eq!(effective, 1_000);
    }

    #[test]
    fn test_ops_query_default_limit() {
        let q = OpsQuery {
            after_seq: None,
            limit: None,
        };
        let effective = q.limit.unwrap_or(100).min(1_000);
        assert_eq!(effective, 100);
    }
}
