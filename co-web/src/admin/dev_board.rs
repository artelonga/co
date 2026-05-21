//! dev_board — CO-43: Hidden dev board (admin only)
//!
//! Exposes CO development tasks (`data/co/CO-*.md`) as a private universe
//! accessible only to admin users (`tier=admin` or `CO_DEV_OWNER` env var).
//!
//! All unauthorized access returns 404 — the universe's existence is not revealed.

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::auth;
use crate::entry_index::{EntryRow, TagCount};
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

/// Core authorization logic — accepts the owner_id explicitly so it can be tested
/// without env var mutation (which is racy in parallel test runs).
fn is_co_dev_authorized_with(claims: &auth::Claims, owner_id: &str) -> bool {
    if claims.tier == "admin" {
        return true;
    }
    !owner_id.is_empty() && claims.sub == owner_id
}

/// Returns true if the caller is authorized to access the CO dev board.
///
/// Authorized when: `tier = "admin"` OR `sub == CO_DEV_OWNER` env var.
fn is_co_dev_authorized(claims: &auth::Claims) -> bool {
    let owner_id = std::env::var("CO_DEV_OWNER").unwrap_or_default();
    is_co_dev_authorized_with(claims, &owner_id)
}

/// Middleware that enforces admin-only access for the CO dev board.
///
/// Returns 404 on failure — not 403 — so the board's existence is not revealed.
pub async fn require_co_dev_admin(req: Request<Body>, next: Next) -> Result<Response, Response> {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| auth::extract_session_cookie(req.headers()));

    let authorized = token
        .as_deref()
        .and_then(|t| {
            let secret = auth::jwt_secret();
            auth::decode_claims(t, &secret).ok()
        })
        .map(|claims| is_co_dev_authorized(&claims))
        .unwrap_or(false);

    if !authorized {
        return Err((StatusCode::NOT_FOUND, "Not found").into_response());
    }

    Ok(next.run(req).await)
}

// ---------------------------------------------------------------------------
// Filesystem scanning
// ---------------------------------------------------------------------------

/// Parse a single CO-*.md or ROADMAP.md file into an EntryRow.
/// Returns None if the file is unreadable or has no valid frontmatter.
fn parse_co_entry(file_path: &std::path::Path) -> Option<EntryRow> {
    let raw = std::fs::read_to_string(file_path).ok()?;
    let trimmed = raw.trim_start();

    if !trimmed.starts_with("---") {
        return None;
    }
    let after_open = &trimmed[3..];
    // Find the closing ---
    let close = after_open.find("\n---")?;
    let yaml_str = &after_open[..close];
    let body_raw = &after_open[close + 4..]; // skip "\n---"
    let body = body_raw.trim_start().to_string();

    let fm_yaml: serde_yaml::Value = serde_yaml::from_str(yaml_str).ok()?;
    let frontmatter: serde_json::Value = serde_json::to_value(&fm_yaml).ok()?;

    let title = frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .map(String::from);
    let created_at = frontmatter
        .get("created_at")
        .and_then(|v| v.as_str())
        .map(String::from);
    let updated_at = frontmatter
        .get("updated_at")
        .and_then(|v| v.as_str())
        .map(String::from);

    let file_name = file_path.file_name()?.to_string_lossy().into_owned();
    let entry_type = if file_name == "ROADMAP.md" || file_name == "project.yaml" {
        "page"
    } else {
        "task"
    }
    .to_string();

    // Stable hash of the body for cache-busting.
    let body_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        body.hash(&mut h);
        format!("{:x}", h.finish())
    };

    Some(EntryRow {
        path: file_name,
        universe_key: "co-dev".to_string(),
        entry_type,
        title,
        frontmatter,
        body,
        body_hash,
        created_at,
        updated_at,
        _score: None,
    })
}

/// Sort key: extract the numeric part from "CO-N.md".
fn co_sort_key(path: &str) -> u32 {
    path.strip_prefix("CO-")
        .and_then(|s| s.strip_suffix(".md"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(u32::MAX)
}

/// Scan `{data_dir}/co/` for `CO-*.md` and `ROADMAP.md` files.
/// Returns entries sorted by numeric ID.
fn scan_co_entries(data_dir: &str) -> Vec<EntryRow> {
    let co_dir = std::path::Path::new(data_dir).join("co");
    if !co_dir.exists() {
        return vec![];
    }

    let mut entries: Vec<EntryRow> = std::fs::read_dir(&co_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let n = name.to_string_lossy();
            (n.starts_with("CO-") && n.ends_with(".md")) || n == "ROADMAP.md"
        })
        .filter_map(|e| parse_co_entry(&e.path()))
        .collect();

    entries.sort_by_key(|e| co_sort_key(&e.path));
    entries
}

// ---------------------------------------------------------------------------
// Frontmatter update helpers
// ---------------------------------------------------------------------------

/// Rewrite specific YAML frontmatter fields, optionally replacing the body.
///
/// `fm_updates`: slice of `(field_name, new_value)` pairs to replace.
/// Only replaces the first occurrence of each field name.
fn update_entry_content(
    content: &str,
    fm_updates: &[(&str, &str)],
    new_body: Option<&str>,
) -> String {
    let mut result = String::new();
    let mut in_fm = false;
    let mut fm_started = false;
    let mut replaced: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut body_start_offset: Option<usize> = None;
    let mut char_pos: usize = 0;

    for line in content.lines() {
        if !fm_started && line.trim() == "---" {
            in_fm = true;
            fm_started = true;
            result.push_str(line);
            result.push('\n');
        } else if in_fm && line.trim() == "---" {
            in_fm = false;
            result.push_str(line);
            result.push('\n');
            // Mark where the body starts (after closing ---)
            body_start_offset = Some(char_pos + line.len() + 1);
        } else if in_fm {
            let mut matched = false;
            for (key, val) in fm_updates {
                let prefix = format!("{key}:");
                if line.starts_with(&prefix) && !replaced.contains(key) {
                    result.push_str(&format!("{key}: {val}\n"));
                    replaced.insert(key);
                    matched = true;
                    break;
                }
            }
            if !matched {
                result.push_str(line);
                result.push('\n');
            }
        } else {
            // Body section — only written here if new_body is None.
            if new_body.is_none() {
                result.push_str(line);
                result.push('\n');
            }
        }

        char_pos += line.len() + 1; // +1 for '\n'
        let _ = body_start_offset; // used conceptually above
    }

    if let Some(body) = new_body {
        result.push('\n');
        result.push_str(body.trim_start());
        if !body.ends_with('\n') {
            result.push('\n');
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Query parameters for `GET /co-dev/entries`.
#[derive(Debug, Deserialize)]
pub struct DevEntryQuery {
    /// Filter by status (todo, in_progress, in_review, done)
    pub status: Option<String>,
    /// Filter by priority (low, medium, high, critical)
    pub priority: Option<String>,
    /// Filter by module (e.g. co-web, core)
    pub module: Option<String>,
    /// Filter by label
    pub label: Option<String>,
    /// Full-text search — matches title or CO-N id
    pub q: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DevEntryListResponse {
    pub entries: Vec<EntryRow>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct DevUniverseInfo {
    pub key: String,
    pub name: String,
    pub description: String,
    pub content_count: usize,
    pub is_anonymous: bool,
    pub is_template: bool,
}

/// Body for `PUT /co-dev/entries/*path`.
#[derive(Debug, Deserialize)]
pub struct UpdateCoDevEntry {
    /// Update the `status` field in frontmatter.
    pub status: Option<String>,
    /// Replace the body section of the markdown file.
    pub body: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/universes/co-dev — universe info
pub async fn get_co_dev_info(State(state): State<AppState>) -> Json<DevUniverseInfo> {
    let count = scan_co_entries(&state.core.config.data_dir).len();
    Json(DevUniverseInfo {
        key: "co-dev".to_string(),
        name: "CO Dev Board".to_string(),
        description: "CO platform development tasks — admin only".to_string(),
        content_count: count,
        is_anonymous: false,
        is_template: false,
    })
}

/// GET /api/v1/universes/co-dev/entries — list entries with optional filtering
pub async fn list_co_dev_entries(
    State(state): State<AppState>,
    Query(q): Query<DevEntryQuery>,
) -> Json<DevEntryListResponse> {
    let mut entries = scan_co_entries(&state.core.config.data_dir);

    if let Some(ref status) = q.status {
        entries.retain(|e| {
            e.frontmatter
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s == status.as_str())
                .unwrap_or(false)
        });
    }

    if let Some(ref priority) = q.priority {
        entries.retain(|e| {
            e.frontmatter
                .get("priority")
                .and_then(|v| v.as_str())
                .map(|s| s == priority.as_str())
                .unwrap_or(false)
        });
    }

    if let Some(ref module) = q.module {
        entries.retain(|e| {
            e.frontmatter
                .get("module")
                .and_then(|v| v.as_str())
                .map(|s| s == module.as_str())
                .unwrap_or(false)
        });
    }

    if let Some(ref label) = q.label {
        entries.retain(|e| {
            e.frontmatter
                .get("labels")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().any(|l| l.as_str() == Some(label.as_str())))
                .unwrap_or(false)
        });
    }

    if let Some(ref search) = q.q {
        let needle = search.to_lowercase();
        entries.retain(|e| {
            let title_hit = e
                .title
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains(&needle);
            let path_hit = e.path.to_lowercase().contains(&needle);
            let id_hit = e
                .frontmatter
                .get("id")
                .and_then(|v| v.as_u64())
                .map(|id| id.to_string().contains(&needle))
                .unwrap_or(false);
            title_hit || path_hit || id_hit
        });
    }

    let total = entries.len();
    Json(DevEntryListResponse { entries, total })
}

/// GET /api/v1/universes/co-dev/entries/tags — aggregate labels
pub async fn list_co_dev_tags(State(state): State<AppState>) -> Json<Vec<TagCount>> {
    let entries = scan_co_entries(&state.core.config.data_dir);
    let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for e in &entries {
        if let Some(labels) = e.frontmatter.get("labels").and_then(|v| v.as_array()) {
            for label in labels {
                if let Some(s) = label.as_str() {
                    *counts.entry(s.to_string()).or_insert(0) += 1;
                }
            }
        }
    }

    let mut tags: Vec<TagCount> = counts
        .into_iter()
        .map(|(tag, count)| TagCount { tag, count })
        .collect();
    tags.sort_by(|a, b| b.count.cmp(&a.count));
    Json(tags)
}

/// GET /api/v1/universes/co-dev/entries/*path — read a single entry
pub async fn get_co_dev_entry(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> Result<Json<EntryRow>, AppError> {
    let entry = resolve_co_entry(&state.core.config.data_dir, &path)?;
    Ok(Json(entry))
}

/// PUT /api/v1/universes/co-dev/entries/*path — update status and/or body
///
/// Updates are written back to the markdown file on disk.
/// Hint: run `git add data/co && git commit` after saving.
pub async fn update_co_dev_entry(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Json(body): Json<UpdateCoDevEntry>,
) -> Result<Json<EntryRow>, AppError> {
    if body.status.is_none() && body.body.is_none() {
        return Err(AppError::BadRequest(
            "Provide at least one of: status, body".into(),
        ));
    }

    if let Some(ref s) = body.status {
        let valid = ["todo", "in_progress", "in_review", "done"];
        if !valid.contains(&s.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Invalid status '{s}'. Must be one of: {}",
                valid.join(", ")
            )));
        }
    }

    let file_path = safe_co_path(&state.core.config.data_dir, &path)?;
    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| AppError::Internal(format!("Failed to read {path}: {e}")))?;

    let now = chrono::Utc::now().to_rfc3339();
    let mut fm_updates: Vec<(&str, &str)> = vec![("updated_at", &now)];
    let status_ref: String;
    if let Some(ref s) = body.status {
        status_ref = s.clone();
        fm_updates.push(("status", &status_ref));
    }

    let updated = update_entry_content(&content, &fm_updates, body.body.as_deref());
    std::fs::write(&file_path, updated)
        .map_err(|e| AppError::Internal(format!("Failed to write {path}: {e}")))?;

    let entry = parse_co_entry(&file_path)
        .ok_or_else(|| AppError::Internal("Failed to parse updated entry".into()))?;
    Ok(Json(entry))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve a request path to a file path inside `{data_dir}/co/`, rejecting traversal.
fn safe_co_path(data_dir: &str, path: &str) -> Result<std::path::PathBuf, AppError> {
    // Reject any path that looks like directory traversal.
    if path.contains("..") || path.contains('/') || path.contains('\\') {
        return Err(AppError::NotFound(format!("Entry '{path}' not found")));
    }

    let co_dir = std::path::Path::new(data_dir).join("co");
    let file_path = co_dir.join(path);

    if !file_path.exists() {
        return Err(AppError::NotFound(format!("Entry '{path}' not found")));
    }

    Ok(file_path)
}

/// Parse an entry after validating the path.
fn resolve_co_entry(data_dir: &str, path: &str) -> Result<EntryRow, AppError> {
    let file_path = safe_co_path(data_dir, path)?;
    parse_co_entry(&file_path)
        .ok_or_else(|| AppError::NotFound(format!("Entry '{path}' not found")))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    // CO-142: mounted at /api/v1/admin (was /api/v1/universes) so it no
    // longer shadows the public-subscribable co-dev universe row.
    // Routes: /api/v1/admin/co-dev, /api/v1/admin/co-dev/entries, etc.
    Router::new()
        .route("/co-dev", get(get_co_dev_info))
        .route("/co-dev/entries/tags", get(list_co_dev_tags))
        .route("/co-dev/entries", get(list_co_dev_entries))
        .route(
            "/co-dev/entries/{*path}",
            get(get_co_dev_entry).put(update_co_dev_entry),
        )
        .layer(axum::middleware::from_fn(require_co_dev_admin))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- Fixtures ---

    fn make_co_dir(dir: &TempDir) -> String {
        let data_dir = dir.path().to_str().unwrap().to_string();
        let co_dir = dir.path().join("co");
        std::fs::create_dir_all(&co_dir).unwrap();

        std::fs::write(
            co_dir.join("CO-1.md"),
            "---\nid: 1\ntitle: \"Board UI Overhaul\"\nstatus: done\npriority: critical\nlabels:\n  - epic\n  - board\nmodule: co-web\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\n---\n\nFix critical UX issues.\n",
        )
        .unwrap();

        std::fs::write(
            co_dir.join("CO-43.md"),
            "---\nid: 43\ntitle: \"Hidden dev board\"\nstatus: in_progress\npriority: high\nlabels:\n  - dev\n  - admin\nmodule: co-web\ncreated_at: 2026-04-10T00:00:00Z\nupdated_at: 2026-04-10T00:00:00Z\n---\n\n- [ ] task one\n- [x] task two\n",
        )
        .unwrap();

        data_dir
    }

    // --- Unit tests ---

    #[test]
    fn test_parse_co_entry_task() {
        let dir = TempDir::new().unwrap();
        let _data_dir = make_co_dir(&dir);
        let file_path = dir.path().join("co").join("CO-1.md");

        let entry = parse_co_entry(&file_path).unwrap();
        assert_eq!(entry.path, "CO-1.md");
        assert_eq!(entry.entry_type, "task");
        assert_eq!(entry.universe_key, "co-dev");
        assert_eq!(entry.title, Some("Board UI Overhaul".to_string()));
        assert_eq!(
            entry.frontmatter.get("status").and_then(|v| v.as_str()),
            Some("done")
        );
        assert_eq!(
            entry.frontmatter.get("priority").and_then(|v| v.as_str()),
            Some("critical")
        );
        assert!(entry.body.contains("Fix critical UX issues."));
    }

    #[test]
    fn test_parse_roadmap_entry_type() {
        let dir = TempDir::new().unwrap();
        let co_dir = dir.path().join("co");
        std::fs::create_dir_all(&co_dir).unwrap();
        std::fs::write(
            co_dir.join("ROADMAP.md"),
            "---\ntitle: Roadmap\n---\n\nContent here.\n",
        )
        .unwrap();

        let entry = parse_co_entry(&co_dir.join("ROADMAP.md")).unwrap();
        assert_eq!(entry.entry_type, "page");
    }

    #[test]
    fn test_scan_co_entries_sorted() {
        let dir = TempDir::new().unwrap();
        let data_dir = make_co_dir(&dir);

        let entries = scan_co_entries(&data_dir);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "CO-1.md");
        assert_eq!(entries[1].path, "CO-43.md");
    }

    #[test]
    fn test_scan_co_entries_empty_dir() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().to_str().unwrap().to_string();
        let entries = scan_co_entries(&data_dir);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_co_sort_key() {
        assert_eq!(co_sort_key("CO-1.md"), 1);
        assert_eq!(co_sort_key("CO-43.md"), 43);
        assert_eq!(co_sort_key("ROADMAP.md"), u32::MAX);
    }

    #[test]
    fn test_is_co_dev_authorized_admin_tier() {
        let claims = auth::Claims {
            sub: "user-123".to_string(),
            email: "test@example.com".to_string(),
            tier: "admin".to_string(),
            usuario: String::new(),
            papel: String::new(),
            exp: 9_999_999_999,
            iat: 0,
        };
        assert!(is_co_dev_authorized(&claims));
    }

    #[test]
    fn test_is_co_dev_authorized_by_owner_id() {
        let claims = auth::Claims {
            sub: "yuri-abc".to_string(),
            email: String::new(),
            tier: "player".to_string(),
            usuario: String::new(),
            papel: String::new(),
            exp: 9_999_999_999,
            iat: 0,
        };
        // Correct owner_id → authorized.
        assert!(is_co_dev_authorized_with(&claims, "yuri-abc"));
        // Wrong owner_id → not authorized.
        assert!(!is_co_dev_authorized_with(&claims, "wrong-id"));
        // Empty owner_id → not authorized.
        assert!(!is_co_dev_authorized_with(&claims, ""));
    }

    #[test]
    fn test_is_co_dev_not_authorized_non_admin() {
        let claims = auth::Claims {
            sub: "regular-user".to_string(),
            email: "user@example.com".to_string(),
            tier: "player".to_string(),
            usuario: String::new(),
            papel: String::new(),
            exp: 9_999_999_999,
            iat: 0,
        };
        assert!(!is_co_dev_authorized_with(&claims, ""));
        assert!(!is_co_dev_authorized_with(&claims, "someone-else"));
    }

    #[test]
    fn test_update_entry_content_status() {
        let content =
            "---\nid: 43\nstatus: todo\nupdated_at: 2026-01-01T00:00:00Z\n---\n\nBody text.\n";
        let updated = update_entry_content(content, &[("status", "done")], None);
        assert!(updated.contains("status: done"));
        assert!(!updated.contains("status: todo"));
        assert!(updated.contains("Body text."));
    }

    #[test]
    fn test_update_entry_content_body() {
        let content = "---\nid: 43\nstatus: todo\n---\n\nOld body.\n";
        let updated = update_entry_content(content, &[], Some("New body.\n"));
        assert!(updated.contains("New body."));
        assert!(!updated.contains("Old body."));
        assert!(updated.contains("status: todo"));
    }

    #[test]
    fn test_update_entry_content_both() {
        let content = "---\nstatus: todo\n---\n\nOld.\n";
        let updated = update_entry_content(content, &[("status", "done")], Some("New.\n"));
        assert!(updated.contains("status: done"));
        assert!(!updated.contains("status: todo"));
        assert!(updated.contains("New."));
        assert!(!updated.contains("Old."));
    }

    #[test]
    fn test_safe_co_path_rejects_traversal() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().to_str().unwrap().to_string();
        std::fs::create_dir_all(dir.path().join("co")).unwrap();

        assert!(safe_co_path(&data_dir, "../secret.txt").is_err());
        assert!(safe_co_path(&data_dir, "sub/dir/file.md").is_err());
    }

    #[test]
    fn test_safe_co_path_missing_file() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().to_str().unwrap().to_string();
        std::fs::create_dir_all(dir.path().join("co")).unwrap();

        assert!(safe_co_path(&data_dir, "CO-9999.md").is_err());
    }
}
