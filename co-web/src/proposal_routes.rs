//! Proposals + merges — Phase 3 of CO-native versioning.
//!
//! A proposal is a cross-universe change request: "I want the content from
//! `source_universe` (captured in `source_state`) to land in
//! `target_universe`'s default branch." Stored as an entry with
//! `type=proposal` at `proposals/<ISO-timestamp>-<nanoid>.md`. Frontmatter:
//!
//! - `source_universe`, `source_state`
//! - `title`, `description`, `author`
//! - `target_branch` (default: "main")
//! - `status`: "open" | "merged" | "rejected" | "withdrawn"
//! - `created_at`, `updated_at`
//!
//! A merge is the event-record of a proposal's acceptance, also stored as
//! an entry (`type=merge`, `merges/<ISO>-<nanoid>.md`):
//!
//! - `proposal`: path of the merge'd proposal
//! - `source_universe`, `source_state` (denormalized for forensics)
//! - `target_branch`, `target_state` (the new state in the target)
//! - `merger`, `merged_at`
//!
//! Endpoints:
//!
//! - `POST /api/v1/universes/:slug/proposals` — create a proposal
//!   targeting `:slug`.
//! - `POST /api/v1/universes/:slug/merges` — execute a merge. Body
//!   `{proposal: "proposals/...md"}`. Copies the source state's entries
//!   into target, takes a fresh state, advances target_branch, records
//!   the merge event, flips proposal.status="merged".
//!
//! Naive merge semantics for Phase 3: source state wins. Every entry in
//! source state is written into target (overwriting same-path entries).
//! Entries in target that aren't in source state are left untouched
//! (additive merge). Conflict resolution beyond this is Phase 4.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::auth::resolve_user_id;
use crate::error::AppError;
use crate::server::AppState;

/// Entry types that don't get copied during a merge — these are
/// universe-local metadata, not "user content" to propagate.
const NON_MERGEABLE_TYPES: &[&str] = &["state", "branch", "proposal", "merge"];

#[derive(Debug, Deserialize)]
pub struct CreateProposalRequest {
    pub source_universe: String,
    pub source_state: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_branch_name")]
    pub target_branch: String,
}

fn default_branch_name() -> String {
    "main".to_string()
}

#[derive(Debug, Serialize)]
pub struct ProposalResponse {
    pub path: String,
    pub source_universe: String,
    pub source_state: String,
    pub target_universe: String,
    pub target_branch: String,
    pub title: String,
    pub description: String,
    pub author: Option<String>,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct MergeResponse {
    pub proposal: String,
    pub merge_path: String,
    pub source_universe: String,
    pub source_state: String,
    pub target_universe: String,
    pub target_branch: String,
    pub target_state: String,
    pub entries_copied: usize,
    pub merger: Option<String>,
    pub merged_at: String,
}

#[derive(Debug, Deserialize)]
pub struct MergeRequest {
    /// Path of the open proposal to merge — e.g., `proposals/2026-...md`.
    pub proposal: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{slug}/proposals", post(create_proposal))
        .route("/{slug}/merges", post(merge_proposal))
}

// ---------------------------------------------------------------------------
// POST /:slug/proposals — create a proposal
// ---------------------------------------------------------------------------

pub async fn create_proposal(
    State(state): State<AppState>,
    Path(target_slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateProposalRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.title.trim().is_empty() || req.title.len() > 200 {
        return Err(AppError::BadRequest(
            "Proposal title must be 1-200 characters".into(),
        ));
    }
    if !req.source_state.starts_with("states/") {
        return Err(AppError::BadRequest(
            "source_state must be a path under states/".into(),
        ));
    }
    if req.source_universe == target_slug {
        return Err(AppError::BadRequest(
            "source_universe must differ from target_universe (use a fork)".into(),
        ));
    }

    let author = resolve_user_id(&state, &headers);
    let now = chrono::Utc::now();
    let now_iso = now.to_rfc3339();
    let timestamp = now.format("%Y-%m-%dT%H%M%SZ").to_string();
    let suffix = nanoid::nanoid!(8);
    let proposal_path = format!("proposals/{}-{}.md", timestamp, suffix);

    // Verify target universe + source universe + source state all exist.
    {
        let storage = state
            .storage
            .lock()
            .map_err(|_| AppError::Internal("storage lock failed".into()))?;
        if storage.get_universe(&target_slug).is_none() {
            return Err(AppError::NotFound(format!(
                "Target universe '{target_slug}' not found"
            )));
        }
        if storage.get_universe(&req.source_universe).is_none() {
            return Err(AppError::BadRequest(format!(
                "Source universe '{}' not found",
                req.source_universe
            )));
        }
    }
    {
        let uc = {
            let storage = state
                .storage
                .lock()
                .map_err(|_| AppError::Internal("storage lock failed".into()))?;
            storage.universe_conn(&req.source_universe)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("source universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        if index
            .get(&req.source_universe, &req.source_state)
            .map_err(|e| AppError::Internal(format!("get source state: {e}")))?
            .is_none()
        {
            return Err(AppError::BadRequest(format!(
                "source_state '{}' not found in '{}'",
                req.source_state, req.source_universe
            )));
        }
    }

    let mut frontmatter = json!({
        "type": "proposal",
        "title": req.title.clone(),
        "description": req.description.clone(),
        "source_universe": req.source_universe.clone(),
        "source_state": req.source_state.clone(),
        "target_universe": target_slug.clone(),
        "target_branch": req.target_branch.clone(),
        "status": "open",
        "created_at": now_iso.clone(),
        "updated_at": now_iso.clone(),
    });
    if let Some(ref a) = author {
        frontmatter["author"] = JsonValue::String(a.clone());
    }

    let body = format!(
        "# {title}\n\n{desc}\n\n\
        - **source:** `{src_u}` @ `{src_s}`\n\
        - **target:** `{tgt_u}` (branch `{tgt_b}`)\n\
        - **status:** open\n",
        title = req.title,
        desc = if req.description.is_empty() {
            "_(no description)_".to_string()
        } else {
            req.description.clone()
        },
        src_u = req.source_universe,
        src_s = req.source_state,
        tgt_u = target_slug,
        tgt_b = req.target_branch,
    );

    crate::vault_routes::write_vault_entry(
        &state,
        &target_slug,
        &proposal_path,
        frontmatter,
        &body,
    )?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(ProposalResponse {
            path: proposal_path,
            source_universe: req.source_universe,
            source_state: req.source_state,
            target_universe: target_slug,
            target_branch: req.target_branch,
            title: req.title,
            description: req.description,
            author,
            status: "open".to_string(),
            created_at: now_iso,
        }),
    ))
}

// ---------------------------------------------------------------------------
// POST /:slug/proposals/{*path}/merge — execute the merge
// ---------------------------------------------------------------------------

pub async fn merge_proposal(
    State(state): State<AppState>,
    Path(target_slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<MergeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let proposal_path = req.proposal;
    if !proposal_path.starts_with("proposals/") {
        return Err(AppError::BadRequest(
            "proposal must be a path under proposals/".into(),
        ));
    }
    let merger = resolve_user_id(&state, &headers);
    let now = chrono::Utc::now();
    let now_iso = now.to_rfc3339();
    let timestamp = now.format("%Y-%m-%dT%H%M%SZ").to_string();

    // --- 1. Read the proposal ---

    let (source_universe, source_state, target_branch, current_status, prop_title, prop_desc) = {
        let uc = {
            let storage = state
                .storage
                .lock()
                .map_err(|_| AppError::Internal("storage lock failed".into()))?;
            if storage.get_universe(&target_slug).is_none() {
                return Err(AppError::NotFound(format!(
                    "Universe '{target_slug}' not found"
                )));
            }
            storage.universe_conn(&target_slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        let row = index
            .get(&target_slug, &proposal_path)
            .map_err(|e| AppError::Internal(format!("get proposal: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("Proposal '{proposal_path}' not found")))?;
        if row.entry_type != "proposal" {
            return Err(AppError::BadRequest(format!(
                "'{proposal_path}' is type '{t}', not 'proposal'",
                t = row.entry_type
            )));
        }
        let fm = &row.frontmatter;
        let su = fm
            .get("source_universe")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Internal("proposal missing source_universe".into()))?
            .to_string();
        let ss = fm
            .get("source_state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Internal("proposal missing source_state".into()))?
            .to_string();
        let tb = fm
            .get("target_branch")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string();
        let st = fm
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("open")
            .to_string();
        let ttl = fm
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let dsc = fm
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        (su, ss, tb, st, ttl, dsc)
    };

    if current_status != "open" {
        return Err(AppError::BadRequest(format!(
            "Proposal status is '{current_status}', not 'open' — cannot merge"
        )));
    }

    // --- 2. Pull source-state entries from the source universe ---

    let source_entries = {
        let uc = {
            let storage = state
                .storage
                .lock()
                .map_err(|_| AppError::Internal("storage lock failed".into()))?;
            if storage.get_universe(&source_universe).is_none() {
                return Err(AppError::BadRequest(format!(
                    "Source universe '{source_universe}' no longer exists"
                )));
            }
            storage.universe_conn(&source_universe)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("source conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        let all = index
            .query(&source_universe, "", &json!({}))
            .map_err(|e| AppError::Internal(format!("query source entries: {e}")))?;
        // Filter out universe-local metadata: states, branches, proposals,
        // merges. Only "real content" propagates.
        all.into_iter()
            .filter(|e| !NON_MERGEABLE_TYPES.contains(&e.entry_type.as_str()))
            .collect::<Vec<_>>()
    };

    // --- 3. Copy each source entry into target_slug ---

    let mut copied = 0usize;
    for src in &source_entries {
        crate::vault_routes::write_vault_entry(
            &state,
            &target_slug,
            &src.path,
            src.frontmatter.clone(),
            &src.body,
        )?;
        copied += 1;
    }

    // --- 4. Take a fresh state in target (captures the merged content) ---
    //
    // We invoke the same logic as `POST /:slug/states` would. Building a
    // local helper keeps this self-contained; future refactor can extract
    // a shared `compute_state(&state, slug, message, author)`.
    let target_state_path = take_state_for_merge(
        &state,
        &target_slug,
        &format!("merge: {prop_title}"),
        merger.as_deref(),
    )?;

    // --- 5. Advance target_branch's head (best-effort — branch may not exist) ---

    let branch_path = format!("branches/{target_branch}.md");
    let branch_advanced = {
        let uc = {
            let storage = state
                .storage
                .lock()
                .map_err(|_| AppError::Internal("storage lock failed".into()))?;
            storage.universe_conn(&target_slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("target conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        index
            .get(&target_slug, &branch_path)
            .map_err(|e| AppError::Internal(format!("get branch: {e}")))?
            .is_some()
    };

    if branch_advanced {
        let now2 = chrono::Utc::now().to_rfc3339();
        let mut br_fm = json!({
            "type": "branch",
            "title": format!("Branch {target_branch}"),
            "name": target_branch.clone(),
            "head_state": target_state_path.clone(),
            "default": true,
            "updated_at": now2,
        });
        if let Some(ref m) = merger {
            br_fm["author"] = JsonValue::String(m.clone());
        }
        let br_body = format!(
            "# Branch `{target_branch}`\n\n- **head_state:** [{target_state_path}](/{target_slug}/{target_state_path})\n"
        );
        crate::vault_routes::write_vault_entry(
            &state,
            &target_slug,
            &branch_path,
            br_fm,
            &br_body,
        )?;
    }

    // --- 6. Write the merge event entry ---

    let merge_suffix = nanoid::nanoid!(8);
    let merge_path = format!("merges/{}-{}.md", timestamp, merge_suffix);
    let mut merge_fm = json!({
        "type": "merge",
        "title": format!("Merge: {prop_title}"),
        "proposal": proposal_path.clone(),
        "source_universe": source_universe.clone(),
        "source_state": source_state.clone(),
        "target_universe": target_slug.clone(),
        "target_branch": target_branch.clone(),
        "target_state": target_state_path.clone(),
        "entries_copied": copied,
        "merged_at": now_iso.clone(),
    });
    if let Some(ref m) = merger {
        merge_fm["merger"] = JsonValue::String(m.clone());
    }
    let merge_body = format!(
        "# Merge {timestamp}\n\n\
        - **proposal:** [`{proposal_path}`](/{target_slug}/{proposal_path})\n\
        - **source:** `{source_universe}` @ `{source_state}`\n\
        - **target:** `{target_slug}` (branch `{target_branch}`)\n\
        - **target state:** [`{target_state_path}`](/{target_slug}/{target_state_path})\n\
        - **entries copied:** {copied}\n\
        \n\
        {prop_desc}\n"
    );
    crate::vault_routes::write_vault_entry(
        &state,
        &target_slug,
        &merge_path,
        merge_fm,
        &merge_body,
    )?;

    // --- 7. Flip proposal.status → "merged" ---

    let mut prop_fm = json!({
        "type": "proposal",
        "title": prop_title.clone(),
        "description": prop_desc.clone(),
        "source_universe": source_universe.clone(),
        "source_state": source_state.clone(),
        "target_universe": target_slug.clone(),
        "target_branch": target_branch.clone(),
        "status": "merged",
        "merged_at": now_iso.clone(),
        "merge": merge_path.clone(),
        "updated_at": now_iso.clone(),
    });
    if let Some(ref m) = merger {
        prop_fm["merger"] = JsonValue::String(m.clone());
    }
    let prop_body_after = format!(
        "# {prop_title}\n\n{desc}\n\n\
        - **source:** `{source_universe}` @ `{source_state}`\n\
        - **target:** `{target_slug}` (branch `{target_branch}`)\n\
        - **status:** merged\n\
        - **merge event:** [`{merge_path}`](/{target_slug}/{merge_path})\n",
        desc = if prop_desc.is_empty() {
            "_(no description)_".to_string()
        } else {
            prop_desc.clone()
        },
    );
    crate::vault_routes::write_vault_entry(
        &state,
        &target_slug,
        &proposal_path,
        prop_fm,
        &prop_body_after,
    )?;

    Ok((
        axum::http::StatusCode::OK,
        Json(MergeResponse {
            proposal: proposal_path,
            merge_path,
            source_universe,
            source_state,
            target_universe: target_slug,
            target_branch,
            target_state: target_state_path,
            entries_copied: copied,
            merger,
            merged_at: now_iso,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Internal helper — take a state in `slug` (mirrors state_routes::create_state
// without the HTTP layer). Returns the new state's path.
// ---------------------------------------------------------------------------

fn take_state_for_merge(
    state: &AppState,
    slug: &str,
    message: &str,
    author: Option<&str>,
) -> Result<String, AppError> {
    use sha2::{Digest, Sha256};

    let now = chrono::Utc::now();
    let timestamp = now.format("%Y-%m-%dT%H%M%SZ").to_string();
    let suffix = nanoid::nanoid!(8);
    let state_path = format!("states/{}-{}.md", timestamp, suffix);

    let (state_lines, parent_path) = {
        let uc = {
            let storage = state
                .storage
                .lock()
                .map_err(|_| AppError::Internal("storage lock failed".into()))?;
            storage.universe_conn(slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);
        let all = index
            .query(slug, "", &json!({}))
            .map_err(|e| AppError::Internal(format!("query entries: {e}")))?;

        let mut state_entries: Vec<(String, String)> = all
            .iter()
            .filter(|e| e.entry_type != "state")
            .map(|e| {
                let mut h = Sha256::new();
                h.update(e.frontmatter.to_string().as_bytes());
                h.update(b"\n");
                h.update(e.body.as_bytes());
                let hash = format!("{:x}", h.finalize());
                (e.path.clone(), hash)
            })
            .collect();
        state_entries.sort_by(|a, b| a.0.cmp(&b.0));

        let lines: String = state_entries
            .iter()
            .map(|(path, hash)| format!("{hash}  {path}\n"))
            .collect();

        let parent = all
            .iter()
            .filter(|e| e.entry_type == "state")
            .map(|e| e.path.clone())
            .max();
        (lines, parent)
    };

    let entry_count = state_lines.lines().count();
    let state_hash = format!("{:x}", Sha256::digest(state_lines.as_bytes()));

    let mut fm = json!({
        "type": "state",
        "title": format!("State {timestamp}"),
        "message": message,
        "entry_count": entry_count,
        "state_hash": state_hash.clone(),
        "created_at": now.to_rfc3339(),
    });
    if let Some(p) = parent_path.as_deref() {
        fm["parent"] = JsonValue::String(p.to_string());
    }
    if let Some(a) = author {
        fm["author"] = JsonValue::String(a.to_string());
    }

    let body = format!(
        "# State {timestamp}\n\n- **state_hash:** `{state_hash}`\n- **entries:** {entry_count}\n\n## Manifest\n\n```\n{}\n```\n",
        state_lines.trim_end()
    );

    crate::vault_routes::write_vault_entry(state, slug, &state_path, fm, &body)?;
    Ok(state_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_mergeable_types_includes_metadata() {
        for t in &["state", "branch", "proposal", "merge"] {
            assert!(NON_MERGEABLE_TYPES.contains(t), "{t} must be filtered");
        }
        assert!(!NON_MERGEABLE_TYPES.contains(&"task"));
        assert!(!NON_MERGEABLE_TYPES.contains(&"doc"));
    }
}
