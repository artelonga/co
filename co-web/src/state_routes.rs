//! States — atomic point-in-time captures of a universe (Phase 1 of the
//! CO-native versioning that replaces git for development workflows).
//!
//! A state is just an entry with `type=state`, stored at
//! `states/<ISO-timestamp>-<nanoid>.md`. Its body is a stable line-per-entry
//! serialization of `<sha256>  <path>` for every non-state entry in the
//! universe at the moment of capture, sorted by path. The hash of that
//! body is `state_hash` — two states with the same hash represent
//! identical content (the equivalent of a git commit ID).
//!
//! `parent` (in frontmatter) chains states into a linear history. The
//! POST handler auto-discovers the most recent prior state to wire as
//! parent. Branches (named pointers to a state) come in Phase 2.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use sha2::{Digest, Sha256};

use crate::auth::resolve_user_id;
use crate::error::AppError;
use crate::server::AppState;

#[derive(Debug, Deserialize, Default)]
pub struct CreateStateRequest {
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct StateResponse {
    pub path: String,
    pub author: Option<String>,
    pub message: String,
    pub parent: Option<String>,
    pub entry_count: usize,
    pub state_hash: String,
    pub created_at: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{slug}/states", post(create_state))
        .route("/{slug}/states/diff", get(diff_states))
}

/// `POST /api/v1/universes/:slug/states` — capture the current state.
///
/// 1. Walk every entry in the universe (excluding `type=state` so the
///    history doesn't recursively include itself).
/// 2. Sort by path, hash each `(frontmatter_json + body)` pair, and build a
///    line-per-entry body of `<sha256>  <path>`.
/// 3. Hash that body → `state_hash`.
/// 4. Find the most recent existing state and wire it as `parent`.
/// 5. Write the state itself as an entry via the vault writer (so it flows
///    through the same indexing, FTS, and WS-broadcast paths every other
///    write uses).
pub async fn create_state(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateStateRequest>,
) -> Result<impl IntoResponse, AppError> {
    let author = resolve_user_id(&state, &headers);

    let now = chrono::Utc::now();
    let now_iso = now.to_rfc3339();
    let timestamp = now.format("%Y-%m-%dT%H%M%SZ").to_string();
    let suffix = nanoid::nanoid!(8);
    let state_path = format!("states/{}-{}.md", timestamp, suffix);

    // --- 1. Walk all non-state entries, build the manifest body ---

    let (state_lines, parent_path) = {
        let uc = {
            let storage = state
                .storage
                .lock()
                .map_err(|_| AppError::Internal("storage lock failed".into()))?;
            // Defensive 404: visibility middleware already gated this route,
            // but a direct test-call without it should still 404 cleanly.
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

        // Exclude states from their own state (otherwise the hash drifts
        // every time you take a state — recursion).
        // 1.74.0: manifest line gains a third column — `body_hash` — so the
        // pin rewind path (Phase 8 step 4) can look up the entry body
        // verbatim from the CAS blob store. Format becomes
        // `{combined_hash}  {body_hash}  {path}\n`. The combined hash stays
        // first for backward-compat with the diff API; legacy 2-column lines
        // continue to parse as no-body-hash.
        let mut state_entries: Vec<(String, String, String)> = all
            .iter()
            .filter(|e| e.entry_type != "state")
            .map(|e| {
                let mut h = Sha256::new();
                h.update(e.frontmatter.to_string().as_bytes());
                h.update(b"\n");
                h.update(e.body.as_bytes());
                let combined = format!("{:x}", h.finalize());
                let body_hash = format!("{:x}", Sha256::digest(e.body.as_bytes()));
                (e.path.clone(), combined, body_hash)
            })
            .collect();
        state_entries.sort_by(|a, b| a.0.cmp(&b.0));

        let lines: String = state_entries
            .iter()
            .map(|(path, combined, body_hash)| format!("{combined}  {body_hash}  {path}\n"))
            .collect();

        // Parent: the state with the largest path (paths sort
        // lexicographically by ISO timestamp, so largest = most recent).
        let parent = all
            .iter()
            .filter(|e| e.entry_type == "state")
            .map(|e| e.path.clone())
            .max();

        (lines, parent)
    };

    let entry_count = state_lines.lines().count();
    let state_hash = format!("{:x}", Sha256::digest(state_lines.as_bytes()));

    // --- 2. Build the state entry's frontmatter + body ---

    let mut frontmatter = json!({
        "type": "state",
        "title": format!("State {timestamp}"),
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
        "# State {timestamp}\n\
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
            .unwrap_or_else(|| "_(none — first state)_".to_string()),
        state_lines.trim_end()
    );

    // --- 3. Persist via the existing vault writer ---

    crate::vault_routes::write_vault_entry(&state, &slug, &state_path, frontmatter, &body)?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(StateResponse {
            path: state_path,
            author,
            message: req.message,
            parent: parent_path,
            entry_count,
            state_hash,
            created_at: now_iso,
        }),
    ))
}

// ---------------------------------------------------------------------------
// GET /:slug/states/diff?from=&to= — compare two state manifests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize)]
pub struct DiffEntry {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_hash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiffResponse {
    pub from: String,
    pub to: String,
    pub added: Vec<DiffEntry>,
    pub removed: Vec<DiffEntry>,
    pub modified: Vec<DiffEntry>,
    pub unchanged: usize,
}

/// `GET /api/v1/universes/:slug/states/diff?from=<state>&to=<state>` —
/// list the path-level differences between two state manifests. Both
/// state paths must live in the same universe and start with `states/`.
///
/// Returns three sorted lists:
/// - `added`:    paths in `to` but not in `from`
/// - `removed`:  paths in `from` but not in `to`
/// - `modified`: paths in both with different content hash
///
/// `unchanged` is just the count of paths whose hash matched on both
/// sides — kept lightweight (no body) so a "what changed" UI can show
/// "47 unchanged" without flooding the response.
pub async fn diff_states(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<DiffQuery>,
) -> Result<impl IntoResponse, AppError> {
    let from_manifest = read_state_manifest(&state, &slug, &q.from)?;
    let to_manifest = read_state_manifest(&state, &slug, &q.to)?;

    use std::collections::HashMap;
    let from_map: HashMap<&str, &str> = from_manifest
        .iter()
        .map(|(p, h)| (p.as_str(), h.as_str()))
        .collect();
    let to_map: HashMap<&str, &str> = to_manifest
        .iter()
        .map(|(p, h)| (p.as_str(), h.as_str()))
        .collect();

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged = 0usize;
    for (path, to_hash) in &to_map {
        match from_map.get(path) {
            None => added.push(DiffEntry {
                path: path.to_string(),
                from_hash: None,
                to_hash: Some(to_hash.to_string()),
            }),
            Some(from_hash) if from_hash != to_hash => modified.push(DiffEntry {
                path: path.to_string(),
                from_hash: Some(from_hash.to_string()),
                to_hash: Some(to_hash.to_string()),
            }),
            _ => unchanged += 1,
        }
    }
    let mut removed = Vec::new();
    for (path, from_hash) in &from_map {
        if !to_map.contains_key(path) {
            removed.push(DiffEntry {
                path: path.to_string(),
                from_hash: Some(from_hash.to_string()),
                to_hash: None,
            });
        }
    }
    added.sort_by(|a, b| a.path.cmp(&b.path));
    removed.sort_by(|a, b| a.path.cmp(&b.path));
    modified.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(DiffResponse {
        from: q.from,
        to: q.to,
        added,
        removed,
        modified,
        unchanged,
    }))
}

/// Read the line-per-entry manifest stored in a state's body. Returns
/// `Vec<(path, hash)>`. The body shape is fixed by `create_state` —
/// fenced code-block whose lines are `<hash>  <path>`.
fn read_state_manifest(
    state: &AppState,
    slug: &str,
    state_path: &str,
) -> Result<Vec<(String, String)>, AppError> {
    if !state_path.starts_with("states/") {
        return Err(AppError::BadRequest(format!(
            "'{state_path}' is not a states/ path"
        )));
    }
    let uc = {
        let storage = state
            .storage
            .lock()
            .map_err(|_| AppError::Internal("storage lock failed".into()))?;
        if storage.get_universe(slug).is_none() {
            return Err(AppError::NotFound(format!("Universe '{slug}' not found")));
        }
        storage.universe_conn(slug)
    };
    let uc_guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = crate::entry_index::EntryIndex::new(&uc_guard);
    let row = index
        .get(slug, state_path)
        .map_err(|e| AppError::Internal(format!("get state: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("State '{state_path}' not found")))?;
    if row.entry_type != "state" {
        return Err(AppError::BadRequest(format!(
            "'{state_path}' is type '{}', not 'state'",
            row.entry_type
        )));
    }
    Ok(parse_state_manifest_body(&row.body))
}

/// Parse the fenced manifest block: lines of `<combined_hash>  <path>` or
/// `<combined_hash>  <body_hash>  <path>` between the first and second
/// triple-backticks. Permissive — bad lines are skipped. Returns
/// `(path, combined_hash)` pairs (the combined-hash form), dropping any
/// body_hash column for backward compat. Use `parse_state_manifest_full`
/// when the body_hash is needed (Phase 8 rewind).
pub(crate) fn parse_state_manifest_body(body: &str) -> Vec<(String, String)> {
    parse_state_manifest_full(body)
        .into_iter()
        .map(|(path, combined, _body_hash)| (path, combined))
        .collect()
}

/// 1.74.0: full manifest parse — returns `(path, combined_hash, Option<body_hash>)`
/// per entry. Lines with 2 whitespace-separated tokens are legacy
/// (combined+path); lines with 3 tokens carry body_hash too. Used by the
/// rewind path to fetch verbatim historical bytes from the CAS blob store.
pub(crate) fn parse_state_manifest_full(body: &str) -> Vec<(String, String, Option<String>)> {
    let mut in_fence = false;
    let mut entries = Vec::new();
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        match tokens.as_slice() {
            [combined, path] if !combined.is_empty() && !path.is_empty() => {
                entries.push((path.to_string(), combined.to_string(), None));
            }
            [combined, body_hash, path]
                if !combined.is_empty() && !body_hash.is_empty() && !path.is_empty() =>
            {
                entries.push((
                    path.to_string(),
                    combined.to_string(),
                    Some(body_hash.to_string()),
                ));
            }
            _ => {}
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: hashing two states of the same content yields the same
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

    #[test]
    fn test_parse_state_manifest_body_extracts_fenced_lines() {
        let body = "# State 2026-01-01T000000Z\n\n\
            - **state_hash:** `abc`\n\n\
            ## Manifest\n\n\
            ```\n\
            hash-a  path/to/a.md\n\
            hash-b  path/to/b.md\n\
            \n\
            hash-c  c.md\n\
            ```\n";
        let parsed = parse_state_manifest_body(body);
        assert_eq!(parsed.len(), 3);
        assert_eq!(
            parsed[0],
            ("path/to/a.md".to_string(), "hash-a".to_string())
        );
        assert_eq!(
            parsed[1],
            ("path/to/b.md".to_string(), "hash-b".to_string())
        );
        assert_eq!(parsed[2], ("c.md".to_string(), "hash-c".to_string()));
    }

    #[test]
    fn test_parse_state_manifest_body_handles_empty_fence() {
        let body = "# State\n\n## Manifest\n\n```\n```\n";
        let parsed = parse_state_manifest_body(body);
        assert_eq!(parsed.len(), 0);
    }

    #[test]
    fn test_parse_state_manifest_body_ignores_text_outside_fence() {
        let body = "garbage  fake-path\n```\nreal-hash  real-path.md\n```\nmore  garbage\n";
        let parsed = parse_state_manifest_body(body);
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0],
            ("real-path.md".to_string(), "real-hash".to_string())
        );
    }
}
