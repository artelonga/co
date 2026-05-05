//! Snapshots — atomic point-in-time captures of a universe (Phase 1 of the
//! CO-native versioning that replaces git for development workflows).
//!
//! A snapshot is just an entry with `type=snapshot`, stored at
//! `snapshots/<ISO-timestamp>-<nanoid>.md`. Its body is a stable
//! line-per-entry serialization of `<sha256>  <path>` for every non-snapshot
//! entry in the universe at the moment of capture, sorted by path. The hash
//! of that body is the universe's `state_hash` — two snapshots with the
//! same `state_hash` represent identical content.
//!
//! `parent` (in frontmatter) chains snapshots into a linear history. The
//! POST handler auto-discovers the most recent snapshot to wire as parent.
//! Branches (named pointers to snapshots) come in Phase 2.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use sha2::{Digest, Sha256};

use crate::auth::resolve_user_id;
use crate::error::AppError;
use crate::server::AppState;

#[derive(Debug, Deserialize, Default)]
pub struct CreateSnapshotRequest {
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    pub path: String,
    pub author: Option<String>,
    pub message: String,
    pub parent: Option<String>,
    pub entry_count: usize,
    pub state_hash: String,
    pub created_at: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/{slug}/snapshots", post(create_snapshot))
}

/// `POST /api/v1/universes/:slug/snapshots` — capture the current state.
///
/// 1. Walk every entry in the universe (excluding `type=snapshot` so the
///    history doesn't recursively include itself).
/// 2. Sort by path, hash each `(frontmatter_json + body)` pair, and build a
///    line-per-entry body of `<sha256>  <path>`.
/// 3. Hash that body → `state_hash`.
/// 4. Find the most recent existing snapshot and wire it as `parent`.
/// 5. Write the snapshot itself as an entry via the vault writer (so it
///    flows through the same indexing, FTS, and WS-broadcast paths every
///    other write uses).
pub async fn create_snapshot(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<impl IntoResponse, AppError> {
    let author = resolve_user_id(&state, &headers);

    let now = chrono::Utc::now();
    let now_iso = now.to_rfc3339();
    let timestamp = now.format("%Y-%m-%dT%H%M%SZ").to_string();
    let suffix = nanoid::nanoid!(8);
    let snapshot_path = format!("snapshots/{}-{}.md", timestamp, suffix);

    // --- 1. Walk all non-snapshot entries, build the state body ---

    let (state_lines, parent_path) = {
        let uc = {
            let storage = state
                .storage
                .lock()
                .map_err(|_| AppError::Internal("storage lock failed".into()))?;
            // Verify the universe exists; the visibility middleware already
            // gated this route, but a defensive 404 here keeps the error
            // shape consistent with other handlers when called from tests.
            if storage.get_universe(&slug).is_none() {
                return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
            }
            storage.universe_conn(&slug)
        };
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        let index = crate::entry_index::EntryIndex::new(&uc_guard);

        let all = index
            .query(&slug, "", &json!({}))
            .map_err(|e| AppError::Internal(format!("query entries: {e}")))?;

        // Exclude snapshots from their own state (otherwise the hash drifts
        // every time you take a snapshot — recursion).
        let mut state_entries: Vec<(String, String)> = all
            .iter()
            .filter(|e| e.entry_type != "snapshot")
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

        // Parent: the snapshot with the largest path (paths sort
        // lexicographically by ISO timestamp, so largest = most recent).
        let parent = all
            .iter()
            .filter(|e| e.entry_type == "snapshot")
            .map(|e| e.path.clone())
            .max();

        (lines, parent)
    };

    let entry_count = state_lines.lines().count();
    let state_hash = format!("{:x}", Sha256::digest(state_lines.as_bytes()));

    // --- 2. Build the snapshot entry's frontmatter + body ---

    let mut frontmatter = json!({
        "type": "snapshot",
        "title": format!("Snapshot {timestamp}"),
        "message": req.message,
        "entry_count": entry_count,
        "state_hash": state_hash.clone(),
        "created_at": now_iso.clone(),
    });
    if let Some(ref p) = parent_path {
        frontmatter["parent"] = JsonValue::String(p.clone());
    }
    if let Some(ref a) = author {
        frontmatter["author"] = JsonValue::String(a.clone());
    }

    let body = format!(
        "# Snapshot {timestamp}\n\
        \n\
        - **state_hash:** `{state_hash}`\n\
        - **entries:** {entry_count}\n\
        - **parent:** {}\n\
        \n\
        ## Manifest\n\
        \n\
        ```\n{}\n```\n",
        parent_path
            .as_deref()
            .map(|p| format!("`{p}`"))
            .unwrap_or_else(|| "_(none — first snapshot)_".to_string()),
        state_lines.trim_end()
    );

    // --- 3. Persist via the existing vault writer ---

    crate::vault_routes::write_vault_entry(&state, &slug, &snapshot_path, frontmatter, &body)?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(SnapshotResponse {
            path: snapshot_path,
            author,
            message: req.message,
            parent: parent_path,
            entry_count,
            state_hash,
            created_at: now_iso,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: hashing two snapshots of the same content yields the same
    /// `state_hash` (the operation is deterministic), and adding a new
    /// entry between them changes the hash.
    #[test]
    fn test_state_hash_deterministic() {
        let pairs1 = vec![
            ("a.md".to_string(), "hash-a".to_string()),
            ("b.md".to_string(), "hash-b".to_string()),
        ];
        let lines1: String = pairs1.iter().map(|(p, h)| format!("{h}  {p}\n")).collect();
        let h1 = format!("{:x}", Sha256::digest(lines1.as_bytes()));

        let pairs2 = pairs1.clone();
        let lines2: String = pairs2.iter().map(|(p, h)| format!("{h}  {p}\n")).collect();
        let h2 = format!("{:x}", Sha256::digest(lines2.as_bytes()));

        assert_eq!(h1, h2, "same content must produce same state_hash");

        let mut pairs3 = pairs1;
        pairs3.push(("c.md".to_string(), "hash-c".to_string()));
        let lines3: String = pairs3.iter().map(|(p, h)| format!("{h}  {p}\n")).collect();
        let h3 = format!("{:x}", Sha256::digest(lines3.as_bytes()));

        assert_ne!(h1, h3, "new entry must change state_hash");
    }
}
