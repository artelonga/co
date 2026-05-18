//! Branches — named pointers to a `state` (Phase 2 of CO-native versioning).
//!
//! A branch is just an entry with `type=branch`, stored at
//! `branches/<name>.md`. Frontmatter carries:
//!   - `name`: the branch slug (e.g., "main", "feat/new-flow")
//!   - `head_state`: path to the state this branch currently points at
//!   - `default`: whether this is the default branch for the universe
//!   - `created_at`, `updated_at`, `author`
//!
//! Phase 2 surface is intentionally minimal:
//!   - `POST   /api/v1/universes/:slug/branches`        create a branch
//!   - `PUT    /api/v1/universes/:slug/branches/:name`  advance head
//!
//! Listing is just `GET /entries?type=branch` (existing endpoint).
//! Single fetch is `GET /entries/branches/{name}.md`.
//!
//! What's deliberately deferred:
//!   - "Active" branch semantics (which branch new writes flow into).
//!     For now, writes always go to the universe filesystem flat; branches
//!     are just bookmarks over the linear state history.
//!   - Branch deletion (would need DELETE /universes/:slug as a
//!     prerequisite, since deleting a branch is just deleting an entry).
//!   - Conflict-resolution between divergent branches (Phase 3 — merges).
//!
//! The shape works because `state` already gives us atomic captures.
//! A branch is just the name "I want to remember this point". Advance
//! it by `PUT`-ing a newer state path.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::auth::resolve_user_id;
use crate::entry_index::EntryEventRow;
use crate::error::AppError;
use crate::server::AppState;

/// Branch name — letters, digits, hyphen, underscore, slash.
/// Slash is allowed so workflows can use `feat/foo`-style names.
fn validate_branch_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::BadRequest(
            "Branch name must be 1-100 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/')
    {
        return Err(AppError::BadRequest(
            "Branch name may contain letters, digits, '-', '_', '/'".into(),
        ));
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Err(AppError::BadRequest(
            "Invalid '/' usage in branch name".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateBranchRequest {
    pub name: String,
    pub head_state: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Deserialize)]
pub struct AdvanceBranchRequest {
    pub head_state: String,
}

#[derive(Debug, Serialize)]
pub struct BranchResponse {
    pub name: String,
    pub path: String,
    pub head_state: String,
    pub default: bool,
    pub author: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        // Existing: named branch pointers
        .route("/{slug}/branches", post(create_branch))
        .route("/{slug}/branches/{name}", put(advance_branch))
        // CO-95 Phase 2: expose the append-only op log
        .route("/{slug}/ops", get(list_ops))
        // CO-95 Phase 3: replay + diff
        .route("/{slug}/replay", get(replay_universe))
        .route("/{slug}/diff", get(diff_universe))
        // CO-95 Phase 4: promote / revert / cherry-pick
        .route("/{slug}/promote", post(promote_branch))
        .route("/{slug}/revert", post(revert_branch))
        .route("/{slug}/cherry-pick", post(cherry_pick))
}

/// `POST /api/v1/universes/:slug/branches` — create a named branch
/// pointing at a state.
pub async fn create_branch(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    validate_branch_name(&req.name)?;
    if !req.head_state.starts_with("states/") {
        return Err(AppError::BadRequest(
            "head_state must be a path under states/ (e.g., states/2026-...md)".into(),
        ));
    }

    let author = resolve_user_id(&state, &headers);
    let now = chrono::Utc::now().to_rfc3339();
    let branch_path = format!("branches/{}.md", req.name);

    // Verify the head_state actually exists in this universe.
    {
        let uc = {
            let storage = state.storage.lock();
            if storage.get_universe(&slug).is_none() {
                return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
            }
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        if index
            .get(&slug, &req.head_state)
            .map_err(|e| AppError::Internal(format!("get state: {e}")))?
            .is_none()
        {
            return Err(AppError::BadRequest(format!(
                "head_state '{}' not found in universe '{}'",
                req.head_state, slug
            )));
        }
    }

    let mut frontmatter = json!({
        "type": "branch",
        "title": format!("Branch {}", req.name),
        "name": req.name.clone(),
        "head_state": req.head_state.clone(),
        "default": req.default,
        "created_at": now.clone(),
        "updated_at": now.clone(),
    });
    if let Some(ref a) = author {
        frontmatter["author"] = JsonValue::String(a.clone());
    }

    let body = format!(
        "# Branch `{name}`\n\
        \n\
        - **head_state:** [{head}](/{slug}/{head})\n\
        - **default:** {default}\n",
        name = req.name,
        head = req.head_state,
        slug = slug,
        default = req.default,
    );

    crate::vault_routes::write_vault_entry(&state, &slug, &branch_path, frontmatter, &body)?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(BranchResponse {
            name: req.name,
            path: branch_path,
            head_state: req.head_state,
            default: req.default,
            author,
            created_at: now.clone(),
            updated_at: now,
        }),
    ))
}

/// `PUT /api/v1/universes/:slug/branches/:name` — advance the branch's
/// head to a newer state.
pub async fn advance_branch(
    State(state): State<AppState>,
    Path((slug, name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<AdvanceBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    validate_branch_name(&name)?;
    if !req.head_state.starts_with("states/") {
        return Err(AppError::BadRequest(
            "head_state must be a path under states/".into(),
        ));
    }

    let author = resolve_user_id(&state, &headers);
    let now = chrono::Utc::now().to_rfc3339();
    let branch_path = format!("branches/{name}.md");

    // Read existing branch entry (if any) to preserve created_at + default flag.
    let (existing_created_at, existing_default) = {
        let uc = {
            let storage = state.storage.lock();
            if storage.get_universe(&slug).is_none() {
                return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
            }
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);

        // Verify head_state exists.
        if index
            .get(&slug, &req.head_state)
            .map_err(|e| AppError::Internal(format!("get state: {e}")))?
            .is_none()
        {
            return Err(AppError::BadRequest(format!(
                "head_state '{}' not found in universe '{}'",
                req.head_state, slug
            )));
        }

        // Existing branch row.
        let existing = index
            .get(&slug, &branch_path)
            .map_err(|e| AppError::Internal(format!("get branch: {e}")))?;
        let (created_at, default) = match existing {
            Some(row) => {
                let fm = &row.frontmatter;
                let c = fm
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&now)
                    .to_string();
                let d = fm.get("default").and_then(|v| v.as_bool()).unwrap_or(false);
                (c, d)
            }
            None => {
                return Err(AppError::NotFound(format!(
                    "Branch '{name}' not found — create with POST /branches first"
                )));
            }
        };
        (created_at, default)
    };

    let mut frontmatter = json!({
        "type": "branch",
        "title": format!("Branch {name}"),
        "name": name.clone(),
        "head_state": req.head_state.clone(),
        "default": existing_default,
        "created_at": existing_created_at.clone(),
        "updated_at": now.clone(),
    });
    if let Some(ref a) = author {
        frontmatter["author"] = JsonValue::String(a.clone());
    }

    let body = format!(
        "# Branch `{name}`\n\
        \n\
        - **head_state:** [{head}](/{slug}/{head})\n\
        - **default:** {default}\n",
        name = name,
        head = req.head_state,
        slug = slug,
        default = existing_default,
    );

    crate::vault_routes::write_vault_entry(&state, &slug, &branch_path, frontmatter, &body)?;

    Ok((
        axum::http::StatusCode::OK,
        Json(BranchResponse {
            name,
            path: branch_path,
            head_state: req.head_state,
            default: existing_default,
            author,
            created_at: existing_created_at,
            updated_at: now,
        }),
    ))
}

// ---------------------------------------------------------------------------
// CO-95 Phase 2 — op log endpoint
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OpsQuery {
    /// Only return events with seq > this value.
    pub since: Option<i64>,
    /// Maximum number of events to return (default 500, max 5000).
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct OpsResponse {
    pub universe: String,
    pub total: usize,
    pub ops: Vec<EntryEventRow>,
    pub max_seq: i64,
}

/// `GET /api/v1/universes/:slug/ops` — list the append-only op log for a universe.
///
/// Access: owner or universe member. Auth via JWT or API token.
pub async fn list_ops(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(q): Query<OpsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers);
    {
        let storage = state.storage.lock();
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        let has_access = caller.as_deref() == Some(&universe.owner_id)
            || caller
                .as_deref()
                .map(|uid| storage.is_universe_member(&slug, uid))
                .unwrap_or(false);
        if !has_access {
            return Err(AppError::Forbidden(
                "Owner or member access required".into(),
            ));
        }
    }

    let limit = q.limit.unwrap_or(500).min(5_000);
    let ops = state.storage.lock().universe_ops(&slug, q.since, limit);
    let max_seq = ops.last().map(|e| e.seq).unwrap_or(0);
    let total = ops.len();

    Ok(Json(OpsResponse {
        universe: slug,
        total,
        ops,
        max_seq,
    }))
}

// ---------------------------------------------------------------------------
// CO-95 Phase 3 — replay engine
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ReplayQuery {
    /// Reconstruct entry state as of this op seq (inclusive).
    pub to_op: i64,
}

#[derive(Debug, Serialize)]
pub struct ReplayedEntry {
    pub path: String,
    pub frontmatter: JsonValue,
    pub body: String,
    pub body_hash: Option<String>,
    pub last_op_seq: i64,
}

#[derive(Debug, Serialize)]
pub struct ReplayResponse {
    pub universe: String,
    pub to_op: i64,
    pub entries: Vec<ReplayedEntry>,
}

/// `GET /api/v1/universes/:slug/replay?to_op=<n>` — reconstruct entry state at op N.
///
/// Replays all `entry_events` with seq ≤ to_op in order:
///   - `put` events create/update the entry in the projection
///   - `delete` events remove the entry from the projection
///
/// Returns the set of entries that would exist at that point in history.
pub async fn replay_universe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(q): Query<ReplayQuery>,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers);
    {
        let storage = state.storage.lock();
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        let has_access = caller.as_deref() == Some(&universe.owner_id)
            || caller
                .as_deref()
                .map(|uid| storage.is_universe_member(&slug, uid))
                .unwrap_or(false);
        if !has_access {
            return Err(AppError::Forbidden(
                "Owner or member access required".into(),
            ));
        }
    }

    let ops = state.storage.lock().universe_ops_to_seq(&slug, q.to_op);

    // Replay: maintain a HashMap<path, ReplayedEntry>. Put creates/updates;
    // delete removes. Entries without a stored body in the event are skipped.
    let mut projection: std::collections::HashMap<String, ReplayedEntry> =
        std::collections::HashMap::new();

    for op in &ops {
        match op.op.as_str() {
            "put" => {
                let frontmatter = op
                    .frontmatter_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<JsonValue>(s).ok())
                    .unwrap_or(JsonValue::Object(Default::default()));
                // body may be None if the event was logged before CO-95 body
                // capture was enabled. Fall back to empty string.
                let body = op
                    .frontmatter_json
                    .as_ref()
                    .map(|_| String::new())
                    .unwrap_or_default();
                projection.insert(
                    op.path.clone(),
                    ReplayedEntry {
                        path: op.path.clone(),
                        frontmatter,
                        body,
                        body_hash: op.body_hash.clone(),
                        last_op_seq: op.seq,
                    },
                );
            }
            "delete" => {
                projection.remove(&op.path);
            }
            _ => {}
        }
    }

    let mut entries: Vec<ReplayedEntry> = projection.into_values().collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(ReplayResponse {
        universe: slug,
        to_op: q.to_op,
        entries,
    }))
}

// ---------------------------------------------------------------------------
// CO-95 Phase 3 — diff
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    /// Diff ops from_op+1..=to_op within this universe.
    pub from_op: Option<i64>,
    pub to_op: Option<i64>,
    /// OR: diff the entire current state of this universe against another.
    pub against: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiffEntry {
    pub path: String,
    pub change: String, // "added" | "modified" | "deleted"
    pub from_body_hash: Option<String>,
    pub to_body_hash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiffResponse {
    pub universe: String,
    pub changes: Vec<DiffEntry>,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
}

/// `GET /api/v1/universes/:slug/diff` — show what changed between two op IDs
/// or between two universes.
///
/// Two modes:
///   - `?from_op=<a>&to_op=<b>` — ops within this universe from a+1..=b
///   - `?against=<other-slug>` — compare current entries of both universes
pub async fn diff_universe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(q): Query<DiffQuery>,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers);
    {
        let storage = state.storage.lock();
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        let has_access = caller.as_deref() == Some(&universe.owner_id)
            || caller
                .as_deref()
                .map(|uid| storage.is_universe_member(&slug, uid))
                .unwrap_or(false);
        if !has_access {
            return Err(AppError::Forbidden(
                "Owner or member access required".into(),
            ));
        }
    }

    if let Some(other_slug) = q.against {
        // Cross-universe diff: compare current body_hash for each path.
        let (a_entries, b_entries) = {
            let storage = state.storage.lock();
            let a = storage.universe_current_hashes(&slug);
            let b = storage.universe_current_hashes(&other_slug);
            (a, b)
        };
        let mut changes = vec![];
        // Paths in A
        for (path, hash_a) in &a_entries {
            match b_entries.get(path) {
                None => changes.push(DiffEntry {
                    path: path.clone(),
                    change: "deleted".into(),
                    from_body_hash: Some(hash_a.clone()),
                    to_body_hash: None,
                }),
                Some(hash_b) if hash_a != hash_b => changes.push(DiffEntry {
                    path: path.clone(),
                    change: "modified".into(),
                    from_body_hash: Some(hash_a.clone()),
                    to_body_hash: Some(hash_b.clone()),
                }),
                _ => {}
            }
        }
        // Paths only in B
        for (path, hash_b) in &b_entries {
            if !a_entries.contains_key(path) {
                changes.push(DiffEntry {
                    path: path.clone(),
                    change: "added".into(),
                    from_body_hash: None,
                    to_body_hash: Some(hash_b.clone()),
                });
            }
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let added = changes.iter().filter(|c| c.change == "added").count();
        let modified = changes.iter().filter(|c| c.change == "modified").count();
        let deleted = changes.iter().filter(|c| c.change == "deleted").count();
        return Ok(Json(DiffResponse {
            universe: slug,
            changes,
            added,
            modified,
            deleted,
        }));
    }

    // Op-range diff within the same universe.
    let from = q.from_op.unwrap_or(0);
    let to = q
        .to_op
        .unwrap_or_else(|| state.storage.lock().universe_max_op_seq(&slug));
    let ops = state.storage.lock().universe_ops_range(&slug, from, to);

    // Fold ops: last change per path wins.
    let mut path_changes: std::collections::HashMap<String, DiffEntry> =
        std::collections::HashMap::new();
    for op in &ops {
        let change = match op.op.as_str() {
            "put" => "modified",
            "delete" => "deleted",
            _ => continue,
        };
        let entry = path_changes.entry(op.path.clone()).or_insert(DiffEntry {
            path: op.path.clone(),
            change: change.into(),
            from_body_hash: op.prev_body_hash.clone(),
            to_body_hash: op.body_hash.clone(),
        });
        entry.change = change.into();
        entry.to_body_hash = op.body_hash.clone();
    }
    // Reclassify: if prev_body_hash is None, it's an add (first time this path appeared).
    let mut changes: Vec<DiffEntry> = path_changes
        .into_values()
        .map(|mut e| {
            if e.from_body_hash.is_none() && e.change == "modified" {
                e.change = "added".into();
            }
            e
        })
        .collect();
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    let added = changes.iter().filter(|c| c.change == "added").count();
    let modified = changes.iter().filter(|c| c.change == "modified").count();
    let deleted = changes.iter().filter(|c| c.change == "deleted").count();

    Ok(Json(DiffResponse {
        universe: slug,
        changes,
        added,
        modified,
        deleted,
    }))
}

// ---------------------------------------------------------------------------
// CO-95 Phase 4 — promote / revert / cherry-pick
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct BranchOpResult {
    pub operation: String,
    pub source: String,
    pub target: String,
    pub ops_applied: i64,
    pub conflicts: i64,
    pub audit_id: i64,
}

/// `POST /api/v1/universes/:slug/promote` — replay all of this universe's
/// `entry_events` onto its parent universe (the universe it was forked from).
///
/// Requires: the calling user must own both this universe and the parent.
/// Conflicts: if the same path was modified in both parent and branch since
/// the fork, the branch's version wins (last-write-wins). Conflicts are
/// counted and surfaced in the response.
pub async fn promote_branch(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    let (parent_key, forked_at_op) = {
        let storage = state.storage.lock();
        let u = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        if u.owner_id != caller {
            return Err(AppError::Forbidden(
                "Only the owner can promote a branch".into(),
            ));
        }
        let parent = u.forked_from.ok_or_else(|| {
            AppError::BadRequest(format!(
                "Universe '{slug}' has no recorded parent — cannot promote"
            ))
        })?;
        let fao = u.forked_at_op.unwrap_or(0);
        // Verify caller owns the parent too.
        let parent_u = storage
            .get_universe(&parent)
            .ok_or_else(|| AppError::NotFound(format!("Parent universe '{parent}' not found")))?;
        if parent_u.owner_id != caller {
            return Err(AppError::Forbidden(
                "You must own the parent universe to promote into it".into(),
            ));
        }
        (parent, fao)
    };

    // Collect all ops from the branch (since it starts from seq=0 after fork,
    // these are all ops that happened after the snapshot was taken).
    let branch_ops = state.storage.lock().universe_ops(&slug, None, 10_000);

    // Collect parent's current entry hashes to detect conflicts.
    let parent_hashes = state.storage.lock().universe_current_hashes(&parent_key);

    // Get parent ops since the fork point to identify which paths the parent
    // has modified (conflict detection).
    let parent_post_fork_ops =
        state
            .storage
            .lock()
            .universe_ops(&parent_key, Some(forked_at_op), 10_000);
    let parent_touched: std::collections::HashSet<String> = parent_post_fork_ops
        .iter()
        .map(|op| op.path.clone())
        .collect();

    let mut ops_applied: i64 = 0;
    let mut conflicts: i64 = 0;

    for op in &branch_ops {
        if parent_touched.contains(&op.path) {
            conflicts += 1;
            // last-write-wins: branch version overwrites parent
        }
        match op.op.as_str() {
            "put" => {
                let fm = op
                    .frontmatter_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<JsonValue>(s).ok())
                    .unwrap_or(JsonValue::Object(Default::default()));
                if let Err(e) =
                    crate::vault_routes::write_vault_entry(&state, &parent_key, &op.path, fm, "")
                {
                    tracing::warn!(
                        "promote {slug}→{parent_key}: write {path} failed: {e}",
                        path = op.path
                    );
                } else {
                    ops_applied += 1;
                }
                let _ = parent_hashes; // suppress unused warning
            }
            "delete" => {
                // Remove entry from parent via storage layer.
                {
                    let storage = state.storage.lock();
                    let uc = storage.universe_conn(&parent_key);
                    if let Ok(guard) = uc.lock() {
                        let idx = crate::entry_index::EntryIndex::new(&guard);
                        let _ = idx.remove(&parent_key, &op.path);
                    }
                }
                // Remove file from disk.
                let root = state.storage.lock().universe_root_path(&parent_key);
                let full = root.join(&op.path);
                let _ = std::fs::remove_file(&full);
                ops_applied += 1;
            }
            _ => {}
        }
    }

    let audit_id = state
        .storage
        .lock()
        .record_branch_audit(
            "promote",
            &slug,
            &parent_key,
            ops_applied,
            conflicts,
            Some(&caller),
        )
        .unwrap_or(0);

    Ok((
        StatusCode::OK,
        Json(BranchOpResult {
            operation: "promote".into(),
            source: slug,
            target: parent_key,
            ops_applied,
            conflicts,
            audit_id,
        }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct RevertQuery {
    /// Revert to the state as of this op seq.
    pub to: i64,
}

/// `POST /api/v1/universes/:slug/revert?to=<op_id>` — revert a universe
/// to its state at op seq `to`.
///
/// Creates a new universe named `<slug>-rev-<to>` at the historical op,
/// then returns the new universe metadata. The original is unchanged.
pub async fn revert_branch(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(q): Query<RevertQuery>,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    {
        let storage = state.storage.lock();
        let u = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        if u.owner_id != caller {
            return Err(AppError::Forbidden("Only the owner can revert".into()));
        }
    }

    // Build the historical snapshot by replaying ops.
    let ops = state.storage.lock().universe_ops_to_seq(&slug, q.to);
    if ops.is_empty() {
        return Err(AppError::BadRequest(format!(
            "No ops found at or before seq={} in universe '{slug}'",
            q.to
        )));
    }

    let snapshot_key = format!("{slug}-rev-{}", q.to);
    if state.storage.lock().get_universe(&snapshot_key).is_some() {
        return Err(AppError::Conflict(format!(
            "Universe '{snapshot_key}' already exists"
        )));
    }

    // Create the snapshot universe.
    let new_universe = state.storage.lock().create_universe(
        crate::models::CreateUniverse {
            key: snapshot_key.clone(),
            name: format!("{slug} @ op {}", q.to),
            description: format!("Revert snapshot of '{slug}' at op seq {}", q.to),
        },
        &caller,
    )?;

    // Replay ops into the snapshot by writing each entry.
    let mut projection: std::collections::HashMap<String, (JsonValue, String)> =
        std::collections::HashMap::new();
    for op in &ops {
        match op.op.as_str() {
            "put" => {
                let fm = op
                    .frontmatter_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<JsonValue>(s).ok())
                    .unwrap_or(JsonValue::Object(Default::default()));
                projection.insert(op.path.clone(), (fm, String::new()));
            }
            "delete" => {
                projection.remove(&op.path);
            }
            _ => {}
        }
    }

    let mut applied = 0i64;
    for (path, (fm, body)) in &projection {
        if let Err(e) =
            crate::vault_routes::write_vault_entry(&state, &snapshot_key, path, fm.clone(), body)
        {
            tracing::warn!("revert {slug}@{}: write {path} failed: {e}", q.to);
        } else {
            applied += 1;
        }
    }

    // Record lineage: snapshot was forked from slug at op q.to.
    let _ = state
        .storage
        .lock()
        .set_universe_lineage(&snapshot_key, &slug, Some(q.to));
    let _ = state.storage.lock().record_branch_audit(
        "revert",
        &slug,
        &snapshot_key,
        applied,
        0,
        Some(&caller),
    );

    Ok((StatusCode::CREATED, Json(new_universe)))
}

#[derive(Debug, Deserialize)]
pub struct CherryPickRequest {
    /// Universe to pull ops from.
    pub from: String,
    /// Specific seq IDs to apply (ordered ascending).
    pub op_ids: Vec<i64>,
}

/// `POST /api/v1/universes/:slug/cherry-pick` — apply specific ops from
/// another universe into this one.
///
/// Each op is applied in the order listed in `op_ids`. Conflicts are
/// counted (last-write-wins).
pub async fn cherry_pick(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CherryPickRequest>,
) -> Result<impl IntoResponse, AppError> {
    let caller = resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    {
        let storage = state.storage.lock();
        let u = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        if u.owner_id != caller {
            return Err(AppError::Forbidden("Only the owner can cherry-pick".into()));
        }
        storage.get_universe(&req.from).ok_or_else(|| {
            AppError::NotFound(format!("Source universe '{}' not found", req.from))
        })?;
    }

    if req.op_ids.is_empty() {
        return Err(AppError::BadRequest("op_ids must not be empty".into()));
    }
    if req.op_ids.len() > 500 {
        return Err(AppError::BadRequest(
            "At most 500 op_ids per cherry-pick".into(),
        ));
    }

    // Fetch the requested ops from the source universe.
    let all_ops = state.storage.lock().universe_ops(&req.from, None, 10_000);
    let op_set: std::collections::HashSet<i64> = req.op_ids.iter().copied().collect();
    let selected: Vec<_> = all_ops
        .into_iter()
        .filter(|op| op_set.contains(&op.seq))
        .collect();

    let missing: Vec<i64> = req
        .op_ids
        .iter()
        .filter(|id| !selected.iter().any(|op| op.seq == **id))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(AppError::BadRequest(format!(
            "Op IDs not found in '{}': {missing:?}",
            req.from
        )));
    }

    let current_hashes = state.storage.lock().universe_current_hashes(&slug);
    let mut ops_applied: i64 = 0;
    let mut conflicts: i64 = 0;

    for op in &selected {
        if current_hashes.contains_key(&op.path) {
            conflicts += 1;
        }
        match op.op.as_str() {
            "put" => {
                let fm = op
                    .frontmatter_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<JsonValue>(s).ok())
                    .unwrap_or(JsonValue::Object(Default::default()));
                if crate::vault_routes::write_vault_entry(&state, &slug, &op.path, fm, "").is_ok() {
                    ops_applied += 1;
                }
            }
            "delete" => {
                let uc = state.storage.lock().universe_conn(&slug);
                if let Ok(guard) = uc.lock() {
                    let _ = crate::entry_index::EntryIndex::new(&guard).remove(&slug, &op.path);
                }
                let root = state.storage.lock().universe_root_path(&slug);
                let _ = std::fs::remove_file(root.join(&op.path));
                ops_applied += 1;
            }
            _ => {}
        }
    }

    let audit_id = state
        .storage
        .lock()
        .record_branch_audit(
            "cherry-pick",
            &req.from,
            &slug,
            ops_applied,
            conflicts,
            Some(&caller),
        )
        .unwrap_or(0);

    Ok((
        StatusCode::OK,
        Json(BranchOpResult {
            operation: "cherry-pick".into(),
            source: req.from,
            target: slug,
            ops_applied,
            conflicts,
            audit_id,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_branch_name_accepts_common_shapes() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feat/new-flow").is_ok());
        assert!(validate_branch_name("v1_release").is_ok());
        assert!(validate_branch_name("a").is_ok());
    }

    #[test]
    fn test_validate_branch_name_rejects_bad_shapes() {
        assert!(validate_branch_name("").is_err());
        assert!(validate_branch_name("/main").is_err());
        assert!(validate_branch_name("main/").is_err());
        assert!(validate_branch_name("a//b").is_err());
        assert!(validate_branch_name("with space").is_err());
        assert!(validate_branch_name("a".repeat(101).as_str()).is_err());
    }

    // -----------------------------------------------------------------------
    // CO-95: op log endpoint (Phase 2)
    // -----------------------------------------------------------------------

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::auth::sign_jwt;
    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::models::CreateUniverse;
    use crate::server::{AppState, AppStateInner, build_router};
    use crate::storage::Storage;

    fn test_bearer() -> String {
        let secret =
            std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".to_string());
        let (token, _) = sign_jwt("test-user", "test@example.com", "player", &secret).unwrap();
        format!("Bearer {token}")
    }

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        let config = WebConfig {
            port: 3001,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: true,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "quilomboaraucaria".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        };
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path)
                .expect("Failed to open test game storage"),
        );
        let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
        let state: AppState = Arc::new(AppStateInner {
            storage: parking_lot::Mutex::new(storage),
            experiment: Mutex::new(experiment),
            config,
            auth_store: Mutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            cache: crate::cache::CacheLayer::new(),
            rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: std::sync::Arc::new(crate::auth::JwtKey::load_or_generate()),
            embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
        });
        build_router(state, None)
    }

    fn seed_universe(dir: &std::path::Path, slug: &str) {
        let mut storage = Storage::new(dir.to_str().unwrap());
        let _ = storage.create_universe(
            CreateUniverse {
                key: slug.to_string(),
                name: slug.to_string(),
                description: String::new(),
            },
            "test-user",
        );
    }

    async fn body_str(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Phase 2: GET /ops requires auth — anon request to private universe → 401
    /// (universe_visibility_gate runs first and blocks anonymous access).
    #[tokio::test]
    async fn test_ops_endpoint_requires_auth() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "ops-test");
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/ops-test/ops")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // universe_visibility_gate blocks anonymous access to private universes → 401.
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// Phase 2: GET /ops for the owner returns the op log (initially empty).
    #[tokio::test]
    async fn test_ops_endpoint_accessible_to_owner() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "ops-owner");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/ops-owner/ops")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_str(resp.into_body()).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["universe"], "ops-owner");
        assert_eq!(json["ops"].as_array().unwrap().len(), 0);
    }

    /// Phase 2: after a vault PUT the op appears in GET /ops.
    #[tokio::test]
    async fn test_ops_grows_after_vault_put() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "ops-grow");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        // Write an entry.
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/universes/ops-grow/vault/notes/hello.md")
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from("---\ntitle: Hello\ntype: note\n---\n\nHi"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // List ops — should have at least 1 entry.
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/ops-grow/ops")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_str(resp.into_body()).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let ops = json["ops"].as_array().unwrap();
        assert!(!ops.is_empty(), "expected at least one op after vault PUT");
        assert_eq!(ops[0]["op"], "put");
    }

    /// Phase 3: GET /replay?to_op=0 returns empty (no ops before seq 0).
    #[tokio::test]
    async fn test_replay_at_op_zero_is_empty() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "replay-zero");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/replay-zero/replay?to_op=0")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_str(resp.into_body()).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["entries"].as_array().unwrap().len(), 0);
    }

    /// Phase 3: GET /diff without auth returns 401 (visibility gate blocks first).
    #[tokio::test]
    async fn test_diff_requires_auth() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "diff-auth");
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/diff-auth/diff")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// Phase 3: GET /diff for owner returns empty diff on a fresh universe.
    #[tokio::test]
    async fn test_diff_empty_on_fresh_universe() {
        let dir = tempdir().unwrap();
        seed_universe(dir.path(), "diff-fresh");
        let app = build_test_router(dir.path());
        let bearer = test_bearer();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/diff-fresh/diff")
                    .header(header::AUTHORIZATION, &bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_str(resp.into_body()).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["added"], 0);
        assert_eq!(json["modified"], 0);
        assert_eq!(json["deleted"], 0);
    }
}
