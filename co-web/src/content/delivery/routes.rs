//! CO-398: Delivery pipeline routes.
//!
//! Two surfaces:
//!  - `POST /api/v1/delivery/github`               — inbound GitHub webhooks
//!  - `GET  /api/v1/universes/:slug/delivery/metrics` — lead-time telemetry
//!
//! # GitHub webhook setup
//!
//! Configure `CO_GITHUB_WEBHOOK_SECRET` in the server environment, then point
//! GitHub to `POST /api/v1/delivery/github?universe=<slug>`.
//!
//! Events handled:
//!  - `create` (ref_type=branch, ref contains `CO-<n>`)  → task n: `started`
//!  - `push`   (ref contains `CO-<n>`, 1+ commits)       → task n: `in_progress`
//!  - `pull_request` (action=opened, title/body `CO-<n>`) → task n: `review`, attach pr_url
//!  - `pull_request` (action=closed, merged=true)        → task n: `done`
//!
//! Manual drag-and-drop transitions in the board still work — automation fills
//! the status but does not lock it.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::Deserialize;
use tracing::{info, warn};

use crate::eda::event::{Event, Visibility};
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract `CO-<n>` task ID from a git ref string like `feat/CO-398-desc`.
fn extract_co_id(ref_str: &str) -> Option<u64> {
    // Match CO- followed by digits, ignoring surrounding characters.
    let upper = ref_str.to_uppercase();
    let idx = upper.find("CO-")?;
    let after = &upper[idx + 3..];
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Find the path of a task entry by CO task id within a universe.
///
/// Searches `entries` by `json_extract(frontmatter_json, '$.id')` matching
/// the given numeric id, restricted to task/user-story entry types.
fn find_task_path_by_id(
    uc_guard: &rusqlite::Connection,
    universe_key: &str,
    task_id: u64,
) -> Option<String> {
    const TASK_TYPES: &[&str] = &["task", "user-story"];
    for entry_type in TASK_TYPES {
        let result: rusqlite::Result<String> = uc_guard.query_row(
            "SELECT path FROM entries \
             WHERE universe_key = ?1 \
               AND entry_type = ?2 \
               AND CAST(json_extract(frontmatter_json, '$.id') AS INTEGER) = ?3 \
             LIMIT 1",
            rusqlite::params![universe_key, entry_type, task_id as i64],
            |row| row.get(0),
        );
        if let Ok(path) = result {
            return Some(path);
        }
    }
    None
}

/// Read the current status of an entry from the index.
fn read_entry_status(
    uc_guard: &rusqlite::Connection,
    universe_key: &str,
    path: &str,
) -> Option<String> {
    uc_guard
        .query_row(
            "SELECT json_extract(frontmatter_json, '$.status') \
             FROM entries WHERE universe_key = ?1 AND path = ?2",
            rusqlite::params![universe_key, path],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
}

/// Apply a status transition to a task: read the existing entry, merge the
/// new status (and optional extra fields), write back, reindex, and publish
/// `task.status_changed` to the EDA bus.
fn apply_status_transition(
    state: &AppState,
    universe_key: &str,
    entry_path: &str,
    new_status: &str,
    trigger: &str,
    extra_fields: Option<&serde_json::Value>,
) -> Result<(), AppError> {
    let universe_root = {
        let storage = state.core.storage.lock();
        let universe = storage
            .get_universe(universe_key)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", universe_key)))?;
        // Reject event-bus-backed universes.
        crate::service::EntryService::check_not_event_bus(
            universe.source_kind.as_deref(),
            universe.source_url.clone(),
        )?;
        storage.universe_root(universe_key)
    };

    let uc = {
        let storage = state.core.storage.lock();
        storage.universe_conn(universe_key)
    };
    let index = crate::repository::SqliteEntryRepository::new(uc);
    let existing = index
        .get(universe_key, entry_path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{}' not found", entry_path)))?;

    let old_status = existing
        .frontmatter
        .get("status")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Skip if already at target status.
    if old_status.as_deref() == Some(new_status) {
        return Ok(());
    }

    let mut new_fm = existing.frontmatter.clone();
    new_fm["status"] = serde_json::Value::String(new_status.to_string());

    // Merge extra fields (e.g., pr_url, preview_url).
    if let Some(extra) = extra_fields
        && let Some(obj) = extra.as_object()
    {
        for (k, v) in obj {
            new_fm[k] = v.clone();
        }
    }

    let entry = crate::entry_index::make_entry(entry_path, new_fm.clone(), &existing.body);
    co::write_entry(&universe_root, &entry).map_err(|e| AppError::Internal(e.to_string()))?;

    index
        .upsert_entry(universe_key, &entry)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // Invalidate manifest cache if needed.
    state.index.cache.invalidate_universe(universe_key);
    state
        .index
        .cache
        .query
        .invalidate_prefix(&format!("{universe_key}:"));

    // Publish task.status_changed to EDA bus.
    let title = new_fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    state.core.eda_bus.publish(Event::new(
        "task.status_changed",
        Some(universe_key.to_string()),
        None,
        serde_json::json!({
            "path": entry_path,
            "title": title,
            "from": old_status,
            "to": new_status,
            "trigger": trigger,
        }),
        Visibility::UniverseMembers,
    ));

    Ok(())
}

// ---------------------------------------------------------------------------
// GitHub webhook
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GithubWebhookQuery {
    /// Universe key that this webhook is associated with.
    pub universe: String,
}

/// `POST /api/v1/delivery/github?universe=<slug>`
///
/// Receives GitHub webhook events and drives task status transitions.
/// Validates `X-Hub-Signature-256` using `CO_GITHUB_WEBHOOK_SECRET`.
pub async fn github_webhook(
    State(state): State<AppState>,
    Query(params): Query<GithubWebhookQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Signature validation.
    let secret = std::env::var("CO_GITHUB_WEBHOOK_SECRET").unwrap_or_default();
    if !secret.is_empty() {
        let sig_header = headers
            .get("x-hub-signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let expected = crate::webhook::hmac_signature(&secret, &body);
        if sig_header != expected {
            warn!(
                "delivery: GitHub webhook signature mismatch for universe '{}'",
                params.universe
            );
            return Err(AppError::Unauthorized("Invalid webhook signature".into()));
        }
    }

    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let payload: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| AppError::BadRequest("Invalid JSON".into()))?;

    let universe = params.universe.as_str();

    match event_type.as_str() {
        "create" => handle_create_event(state, universe, &payload).await,
        "push" => handle_push_event(state, universe, &payload).await,
        "pull_request" => handle_pull_request_event(state, universe, &payload).await,
        other => {
            info!(
                "delivery: ignoring unhandled GitHub event '{}' for universe '{}'",
                other, universe
            );
            Ok((
                StatusCode::OK,
                Json(serde_json::json!({"ok": true, "skipped": true})),
            ))
        }
    }
}

/// `create` event: branch created with `CO-<n>` in name → task to `started`.
async fn handle_create_event(
    state: AppState,
    universe: &str,
    payload: &serde_json::Value,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let ref_type = payload["ref_type"].as_str().unwrap_or("");
    if ref_type != "branch" {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "skipped": true})),
        ));
    }
    let ref_name = payload["ref"].as_str().unwrap_or("");
    let Some(task_id) = extract_co_id(ref_name) else {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "skipped": true})),
        ));
    };

    let path = {
        let storage = state.core.storage.lock();
        let uc = storage.universe_conn(universe);
        drop(storage);
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        find_task_path_by_id(&uc_guard, universe, task_id)
    };

    let Some(entry_path) = path else {
        info!("delivery: create — CO-{task_id} not found in universe '{universe}'");
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "not_found": true})),
        ));
    };

    apply_status_transition(
        &state,
        universe,
        &entry_path,
        "started",
        "github.branch",
        None,
    )?;
    info!("delivery: CO-{task_id} → started (branch '{ref_name}')");
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"ok": true, "task_id": task_id, "new_status": "started"})),
    ))
}

/// `push` event: commits on a `CO-<n>` branch → task to `in_progress`.
async fn handle_push_event(
    state: AppState,
    universe: &str,
    payload: &serde_json::Value,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let ref_name = payload["ref"].as_str().unwrap_or("");
    let Some(task_id) = extract_co_id(ref_name) else {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "skipped": true})),
        ));
    };
    let commit_count = payload["commits"].as_array().map(|a| a.len()).unwrap_or(0);
    if commit_count == 0 {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "skipped": true})),
        ));
    }

    let path = {
        let storage = state.core.storage.lock();
        let uc = storage.universe_conn(universe);
        drop(storage);
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        find_task_path_by_id(&uc_guard, universe, task_id)
    };

    let Some(entry_path) = path else {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "not_found": true})),
        ));
    };

    // Only advance from started → in_progress (don't overwrite review/done).
    let current = {
        let storage = state.core.storage.lock();
        let uc = storage.universe_conn(universe);
        drop(storage);
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        read_entry_status(&uc_guard, universe, &entry_path)
    };
    let should_advance = matches!(current.as_deref(), Some("todo") | Some("started") | None);
    if !should_advance {
        return Ok((
            StatusCode::OK,
            Json(
                serde_json::json!({"ok": true, "skipped": true, "reason": "already past in_progress"}),
            ),
        ));
    }

    apply_status_transition(
        &state,
        universe,
        &entry_path,
        "in_progress",
        "github.push",
        None,
    )?;
    info!("delivery: CO-{task_id} → in_progress ({commit_count} commits on '{ref_name}')");
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"ok": true, "task_id": task_id, "new_status": "in_progress"})),
    ))
}

/// `pull_request` event:
///  - opened → `review` + attach pr_url
///  - closed + merged → `done`
async fn handle_pull_request_event(
    state: AppState,
    universe: &str,
    payload: &serde_json::Value,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let action = payload["action"].as_str().unwrap_or("");
    let pr = &payload["pull_request"];

    // Extract CO-N from PR title or body.
    let pr_title = pr["title"].as_str().unwrap_or("");
    let pr_body = pr["body"].as_str().unwrap_or("");
    let pr_url = pr["html_url"].as_str().unwrap_or("").to_string();
    let search_text = format!("{pr_title} {pr_body}");
    let Some(task_id) = extract_co_id(&search_text) else {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "skipped": true})),
        ));
    };

    let path = {
        let storage = state.core.storage.lock();
        let uc = storage.universe_conn(universe);
        drop(storage);
        let uc_guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        find_task_path_by_id(&uc_guard, universe, task_id)
    };

    let Some(entry_path) = path else {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "not_found": true})),
        ));
    };

    match action {
        "opened" | "reopened" => {
            let extra = serde_json::json!({ "pr_url": pr_url });
            apply_status_transition(
                &state,
                universe,
                &entry_path,
                "review",
                "github.pr",
                Some(&extra),
            )?;
            info!("delivery: CO-{task_id} → review (PR opened: {pr_url})");
            Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "task_id": task_id,
                    "new_status": "review",
                    "pr_url": pr_url,
                })),
            ))
        }
        "closed" => {
            let merged = pr["merged"].as_bool().unwrap_or(false);
            if !merged {
                return Ok((
                    StatusCode::OK,
                    Json(
                        serde_json::json!({"ok": true, "skipped": true, "reason": "pr closed without merge"}),
                    ),
                ));
            }
            apply_status_transition(&state, universe, &entry_path, "done", "github.merge", None)?;
            info!("delivery: CO-{task_id} → done (PR merged: {pr_url})");
            Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "task_id": task_id,
                    "new_status": "done",
                })),
            ))
        }
        _ => Ok((
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "skipped": true})),
        )),
    }
}

// ---------------------------------------------------------------------------
// Lead-time metrics
// ---------------------------------------------------------------------------

/// Row returned by the metrics endpoint.
#[derive(Debug, serde::Serialize)]
pub struct StatusDuration {
    pub status: String,
    /// Average seconds spent in this status before transitioning out.
    pub avg_seconds: f64,
    /// Number of transitions that left this status (sample size).
    pub count: i64,
}

/// `GET /api/v1/universes/:slug/delivery/metrics`
///
/// Returns per-column lead time averages computed from `task_status_log`.
/// Only considers tasks that have at least exited the reported status.
pub async fn delivery_metrics(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let storage = state.core.storage.lock();
    let _universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

    // Lead time per column: average seconds between entering and leaving each status.
    // We join consecutive rows (entry into status_to) with the next row (exit from same status).
    let rows: Vec<StatusDuration> = {
        let conn = storage.conn();
        let mut stmt = conn
            .prepare(
                "SELECT
               a.status_to   AS status,
               AVG(CAST((julianday(b.triggered_at) - julianday(a.triggered_at)) * 86400 AS REAL))
                             AS avg_seconds,
               COUNT(*)      AS cnt
             FROM task_status_log a
             JOIN task_status_log b
               ON b.universe_key = a.universe_key
              AND b.entry_path   = a.entry_path
              AND b.triggered_at > a.triggered_at
              AND NOT EXISTS (
                SELECT 1 FROM task_status_log c
                WHERE c.universe_key = a.universe_key
                  AND c.entry_path   = a.entry_path
                  AND c.triggered_at > a.triggered_at
                  AND c.triggered_at < b.triggered_at
              )
             WHERE a.universe_key = ?1
             GROUP BY a.status_to
             ORDER BY AVG(julianday(b.triggered_at) - julianday(a.triggered_at)) DESC",
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;

        stmt.query_map(rusqlite::params![slug], |row| {
            Ok(StatusDuration {
                status: row.get(0)?,
                avg_seconds: row.get(1)?,
                count: row.get(2)?,
            })
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect()
    };

    // Also provide total task count that reached "done".
    let done_count: i64 = {
        let conn = storage.conn();
        conn.query_row(
            "SELECT COUNT(DISTINCT entry_path) FROM task_status_log \
             WHERE universe_key = ?1 AND status_to = 'done'",
            rusqlite::params![slug],
            |row| row.get(0),
        )
        .unwrap_or(0)
    };

    Ok(Json(serde_json::json!({
        "universe_key": slug,
        "done_count": done_count,
        "lead_time_per_status": rows,
    })))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Routes for the delivery pipeline (GitHub webhook + lead-time metrics).
pub fn router() -> Router<AppState> {
    Router::new().route("/delivery/github", post(github_webhook))
}

/// Per-universe delivery metrics (nested under /universes/:slug).
pub fn universe_router() -> Router<AppState> {
    Router::new().route("/{slug}/delivery/metrics", get(delivery_metrics))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_co_id_from_branch_name() {
        assert_eq!(extract_co_id("feat/CO-398-delivery-pipeline"), Some(398));
        assert_eq!(extract_co_id("fix/co-42-some-bug"), Some(42));
        assert_eq!(extract_co_id("refs/heads/feat/CO-1234-foo"), Some(1234));
        assert_eq!(extract_co_id("main"), None);
        assert_eq!(extract_co_id("feat/no-number"), None);
    }

    #[test]
    fn extract_co_id_from_pr_text() {
        assert_eq!(extract_co_id("Fix CO-398: delivery pipeline"), Some(398));
        assert_eq!(extract_co_id("Closes #123, refs CO-42"), Some(42));
    }
}
