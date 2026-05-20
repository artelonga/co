//! CO-144 Phase C — Process model: deterministic source→sink chain for
//! repeatable atomic changes.
//!
//! First implementation: `alterar-pagina-na-web`. Wires all 7 chain steps:
//!   1. Trigger   — POST /preview (caller initiates)
//!   2. Source    — server reads current entry frontmatter
//!   3. Review    — preview row stored with diff; preview URL/diff returned
//!   4. Approval  — POST /approve/<run_id> (caller confirms)
//!   5. Sink      — frontmatter write + universe content_version bump + CHANGELOG.md append
//!   6. Telemetry — telemetry_events row + run completion record
//!   7. Rollback  — POST /revert (restore prior state, mark prior run reverted)

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::UserId;
use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    pub universe: String,
    pub page_path: String,
    pub field: String,
    pub new_value: serde_json::Value,
    /// Optional bump level: "patch" (default), "minor", "major"
    #[serde(default)]
    pub bump_level: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PreviewResponse {
    pub run_id: String,
    pub state: String,
    pub from_value: serde_json::Value,
    pub to_value: serde_json::Value,
    pub current_version: String,
    pub proposed_version: String,
    pub bump_level: String,
}

#[derive(Debug, Serialize)]
pub struct RunResponse {
    pub run_id: String,
    pub state: String,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub changelog_entry: Option<String>,
    pub deploy_status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RevertRequest {
    pub universe: String,
    /// Target version to restore to. `prior` means the version immediately
    /// before the current one (uses the most recent completed run).
    pub target_version: String,
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    pub universe: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunRow {
    pub run_id: String,
    pub process_name: String,
    pub universe_key: String,
    pub state: String,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub actor_id: Option<String>,
    pub payload: serde_json::Value,
    pub parent_run_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Semver helper (patch bump)
// ---------------------------------------------------------------------------

fn next_semver(current: &str, level: &str) -> String {
    let parts: Vec<&str> = current.split('.').collect();
    let (mut major, mut minor, mut patch) = (
        parts
            .first()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0),
        parts
            .get(1)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0),
        parts
            .get(2)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0),
    );
    match level {
        "major" => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        "minor" => {
            minor += 1;
            patch = 0;
        }
        _ => patch += 1,
    }
    format!("{major}.{minor}.{patch}")
}

// ---------------------------------------------------------------------------
// Storage helpers (work directly against meta.db connection inside the lock)
// ---------------------------------------------------------------------------

fn read_content_version(
    conn: &rusqlite::Connection,
    universe_key: &str,
) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT COALESCE(content_version, '0.0.0') FROM universes WHERE key = ?1",
        rusqlite::params![universe_key],
        |row| row.get(0),
    )
}

fn write_content_version(
    conn: &rusqlite::Connection,
    universe_key: &str,
    version: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE universes SET content_version = ?1 WHERE key = ?2",
        rusqlite::params![version, universe_key],
    )
}

struct ChangelogEntry<'a> {
    universe_root: &'a std::path::Path,
    version: &'a str,
    actor: Option<&'a str>,
    field: &'a str,
    from_v: &'a serde_json::Value,
    to_v: &'a serde_json::Value,
    page: &'a str,
    revert: bool,
}

fn append_changelog(args: ChangelogEntry<'_>) -> std::io::Result<String> {
    let ChangelogEntry {
        universe_root,
        version,
        actor,
        field,
        from_v,
        to_v,
        page,
        revert,
    } = args;
    let date = chrono::Utc::now().format("%Y-%m-%d");
    let actor = actor.unwrap_or("unknown");
    let from = render_value(from_v);
    let to = render_value(to_v);
    let header = if revert {
        format!("\n## [{version}] — {date} (Reverted)\n")
    } else {
        format!("\n## [{version}] — {date}\n")
    };
    let entry = format!(
        "{header}\n### Changed\n- `{page}` — campo `{field}`: {from} → {to} (por @{actor})\n"
    );
    let path = universe_root.join("CHANGELOG.md");
    std::fs::create_dir_all(universe_root)?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut new_content = if existing.trim().is_empty() {
        format!(
            "# Changelog\n\nAll notable changes to this universe.\n{}",
            entry
        )
    } else {
        // Insert the new entry right after the header (above prior versions).
        match existing.find("\n## ") {
            Some(idx) => {
                let (head, tail) = existing.split_at(idx);
                format!("{head}{entry}{tail}")
            }
            None => format!("{existing}{entry}"),
        }
    };
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    std::fs::write(&path, &new_content)?;
    Ok(entry)
}

fn render_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

fn random_run_id() -> String {
    format!("run_{}", uuid::Uuid::new_v4().simple())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Step 1 + 2 + 3: Trigger arrives, source is read, review row is stored.
pub async fn preview_alterar_pagina(
    State(state): State<AppState>,
    user_id: UserId,
    Json(body): Json<PreviewRequest>,
) -> Result<Json<PreviewResponse>, AppError> {
    let bump_level = body
        .bump_level
        .as_deref()
        .filter(|s| matches!(*s, "patch" | "minor" | "major"))
        .unwrap_or("patch")
        .to_string();

    // Step 2: Source — read the current entry from filesystem (source of truth).
    let storage = state.core.storage.lock();

    // Access check.
    let access = storage.check_universe_access(Some(&user_id.0), &body.universe);
    if !matches!(access, crate::models::UniverseAccess::ReadWrite) {
        return Err(AppError::Forbidden(
            "write access required to preview a process".into(),
        ));
    }

    let universe_root = storage.universe_root(&body.universe);
    drop(storage); // release before filesystem reads

    let entry = co::entry::read_entry(&universe_root, &body.page_path)
        .map_err(|e| AppError::NotFound(format!("entry not found: {e}")))?;

    let from_value = entry
        .frontmatter
        .get(&body.field)
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // Step 3: Review — record the diff, compute proposed version.
    let storage = state.core.storage.lock();
    let current_version = read_content_version(storage.conn(), &body.universe)
        .map_err(|e| AppError::Internal(format!("read content_version: {e}")))?;
    let proposed_version = next_semver(&current_version, &bump_level);

    let run_id = random_run_id();
    let payload = serde_json::json!({
        "page_path": body.page_path,
        "field": body.field,
        "from_value": from_value,
        "to_value": body.new_value,
        "current_version": current_version,
        "proposed_version": proposed_version,
        "bump_level": bump_level,
    });
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO process_runs \
             (run_id, process_name, universe_key, state, payload, created_at, actor_id, parent_run_id) \
             VALUES (?1, 'alterar-pagina-na-web', ?2, 'preview', ?3, ?4, ?5, NULL)",
            rusqlite::params![run_id, body.universe, payload.to_string(), now, user_id.0],
        )
        .map_err(|e| AppError::Internal(format!("insert preview run: {e}")))?;
    drop(storage);

    Ok(Json(PreviewResponse {
        run_id,
        state: "preview".into(),
        from_value,
        to_value: body.new_value,
        current_version,
        proposed_version,
        bump_level,
    }))
}

/// Step 4 + 5 + 6: Approval, Sink (frontmatter write + bump + CHANGELOG),
/// Telemetry. Step 7 (rollback) is on a separate endpoint.
pub async fn approve_alterar_pagina(
    State(state): State<AppState>,
    user_id: UserId,
    Path(run_id): Path<String>,
) -> Result<Json<RunResponse>, AppError> {
    let storage = state.core.storage.lock();
    let row: Option<(String, String, String, String)> = storage
        .conn()
        .query_row(
            "SELECT state, universe_key, payload, COALESCE(actor_id, '') \
             FROM process_runs WHERE run_id = ?1 AND process_name = 'alterar-pagina-na-web'",
            rusqlite::params![run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();
    drop(storage);

    let (state_str, universe_key, payload_str, _payload_actor) =
        row.ok_or_else(|| AppError::NotFound(format!("run {run_id} not found")))?;
    if state_str != "preview" {
        return Err(AppError::BadRequest(format!(
            "run is in state '{state_str}', cannot approve"
        )));
    }

    let payload: serde_json::Value = serde_json::from_str(&payload_str)
        .map_err(|e| AppError::Internal(format!("payload parse: {e}")))?;

    let page_path = payload
        .get("page_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("payload missing page_path".into()))?;
    let field = payload
        .get("field")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("payload missing field".into()))?;
    let from_value = payload.get("from_value").cloned().unwrap_or_default();
    let to_value = payload.get("to_value").cloned().unwrap_or_default();
    let current_version = payload
        .get("current_version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();
    let proposed_version = payload
        .get("proposed_version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("payload missing proposed_version".into()))?
        .to_string();

    // Re-acquire and run the full sink+telemetry inside one critical section.
    let storage = state.core.storage.lock();

    // Re-check access.
    let access = storage.check_universe_access(Some(&user_id.0), &universe_key);
    if !matches!(access, crate::models::UniverseAccess::ReadWrite) {
        return Err(AppError::Forbidden("write access required".into()));
    }

    let universe_root = storage.universe_root(&universe_key);

    // Step 5.1: Read entry, mutate frontmatter, write back. Re-read at this
    // moment because someone may have edited between preview and approve.
    let mut entry = co::entry::read_entry(&universe_root, page_path)
        .map_err(|e| AppError::NotFound(format!("entry not found: {e}")))?;
    if let Some(obj) = entry.frontmatter.as_object_mut() {
        obj.insert(field.to_string(), to_value.clone());
    }
    co::entry::write_entry(&universe_root, &entry)
        .map_err(|e| AppError::Internal(format!("write entry: {e}")))?;

    // Step 5.2: Bump universe.content_version.
    write_content_version(storage.conn(), &universe_key, &proposed_version)
        .map_err(|e| AppError::Internal(format!("write content_version: {e}")))?;

    // Step 5.3 sink → CHANGELOG.md append.
    let changelog_entry = append_changelog(ChangelogEntry {
        universe_root: &universe_root,
        version: &proposed_version,
        actor: Some(&user_id.0),
        field,
        from_v: &from_value,
        to_v: &to_value,
        page: page_path,
        revert: false,
    })
    .map_err(|e| AppError::Internal(format!("CHANGELOG write: {e}")))?;

    // Step 5.4 sink → deploy: simulated for now (target adapters are CO-134/135 etc).
    let deploy_status = "simulated".to_string();

    // Step 6: Telemetry — write to telemetry_events + mark run completed.
    let now = chrono::Utc::now().to_rfc3339();
    let _ = storage.conn().execute(
        "INSERT INTO telemetry_events \
         (event_type, event_name, user_id, universe_key, payload, timestamp) \
         VALUES ('process', 'alterar-pagina-na-web.completed', ?1, ?2, ?3, ?4)",
        rusqlite::params![
            user_id.0,
            universe_key,
            serde_json::json!({
                "run_id": run_id,
                "page": page_path,
                "field": field,
                "from_value": from_value,
                "to_value": to_value,
                "from_version": current_version,
                "to_version": proposed_version,
                "deploy_status": deploy_status,
            })
            .to_string(),
            now
        ],
    );

    storage
        .conn()
        .execute(
            "UPDATE process_runs SET state = 'completed', completed_at = ?1 WHERE run_id = ?2",
            rusqlite::params![now, run_id],
        )
        .map_err(|e| AppError::Internal(format!("update run state: {e}")))?;
    drop(storage);

    // CO-156: emit entry.upsert telemetry for the process sink write
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "entry.upsert",
            universe: universe_key.clone(),
            list: Some("alterar-pagina-na-web".to_string()),
            key: Some(page_path.to_string()),
            actor: Some(user_id.0.clone()),
            session_id: None,
            extra: Some(serde_json::json!({ "run_id": run_id, "field": field })),
        },
    );

    Ok(Json(RunResponse {
        run_id,
        state: "completed".into(),
        from_version: Some(current_version),
        to_version: Some(proposed_version),
        changelog_entry: Some(changelog_entry),
        deploy_status: Some(deploy_status),
    }))
}

/// Step 7: Rollback — restore prior state by inverting the most recent run.
/// `target_version` of `"prior"` means "the version before the current one".
pub async fn revert_alterar_pagina(
    State(state): State<AppState>,
    user_id: UserId,
    Json(body): Json<RevertRequest>,
) -> Result<Json<RunResponse>, AppError> {
    let storage = state.core.storage.lock();

    let access = storage.check_universe_access(Some(&user_id.0), &body.universe);
    if !matches!(access, crate::models::UniverseAccess::ReadWrite) {
        return Err(AppError::Forbidden("write access required".into()));
    }

    // Resolve "prior" to the most recent completed run's from_version.
    let target_version = if body.target_version == "prior" {
        let prior: String = storage
            .conn()
            .query_row(
                "SELECT json_extract(payload, '$.current_version') \
                 FROM process_runs \
                 WHERE process_name = 'alterar-pagina-na-web' \
                 AND universe_key = ?1 AND state = 'completed' \
                 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![body.universe],
                |r| r.get(0),
            )
            .map_err(|e| AppError::NotFound(format!("no prior completed run: {e}")))?;
        prior
    } else {
        body.target_version.clone()
    };

    // Find the most recent completed run that produced the *current* version
    // we're reverting from — its payload tells us what to restore.
    let current_version = read_content_version(storage.conn(), &body.universe)
        .map_err(|e| AppError::Internal(format!("read content_version: {e}")))?;

    let run_to_invert: Option<(String, String)> = storage
        .conn()
        .query_row(
            "SELECT run_id, payload FROM process_runs \
             WHERE process_name = 'alterar-pagina-na-web' \
             AND universe_key = ?1 \
             AND state = 'completed' \
             AND json_extract(payload, '$.proposed_version') = ?2 \
             ORDER BY created_at DESC LIMIT 1",
            rusqlite::params![body.universe, current_version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();

    let (parent_run_id, parent_payload_str) = run_to_invert.ok_or_else(|| {
        AppError::NotFound(format!(
            "no completed run produced current version {current_version}"
        ))
    })?;

    let parent_payload: serde_json::Value = serde_json::from_str(&parent_payload_str)
        .map_err(|e| AppError::Internal(format!("parent payload parse: {e}")))?;
    let page_path = parent_payload
        .get("page_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("parent payload missing page_path".into()))?;
    let field = parent_payload
        .get("field")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("parent payload missing field".into()))?;
    let to_restore = parent_payload
        .get("from_value")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let was_value = parent_payload
        .get("to_value")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let universe_root = storage.universe_root(&body.universe);

    // Inverse sink: restore frontmatter field → from_value.
    let mut entry = co::entry::read_entry(&universe_root, page_path)
        .map_err(|e| AppError::NotFound(format!("entry not found: {e}")))?;
    if let Some(obj) = entry.frontmatter.as_object_mut() {
        obj.insert(field.to_string(), to_restore.clone());
    }
    co::entry::write_entry(&universe_root, &entry)
        .map_err(|e| AppError::Internal(format!("write entry: {e}")))?;

    // Restore version.
    write_content_version(storage.conn(), &body.universe, &target_version)
        .map_err(|e| AppError::Internal(format!("write content_version: {e}")))?;

    // CHANGELOG: revert entry.
    let changelog_entry = append_changelog(ChangelogEntry {
        universe_root: &universe_root,
        version: &target_version,
        actor: Some(&user_id.0),
        field,
        from_v: &was_value, // reverting FROM the current value
        to_v: &to_restore,  // back TO the prior value
        page: page_path,
        revert: true,
    })
    .map_err(|e| AppError::Internal(format!("CHANGELOG write: {e}")))?;

    // Telemetry + run rows.
    let now = chrono::Utc::now().to_rfc3339();
    let revert_run_id = random_run_id();
    let revert_payload = serde_json::json!({
        "page_path": page_path,
        "field": field,
        "from_value": was_value,
        "to_value": to_restore,
        "current_version": current_version,
        "proposed_version": target_version,
        "revert_of": parent_run_id,
    });
    storage
        .conn()
        .execute(
            "INSERT INTO process_runs \
             (run_id, process_name, universe_key, state, payload, created_at, completed_at, actor_id, parent_run_id) \
             VALUES (?1, 'alterar-pagina-na-web', ?2, 'completed', ?3, ?4, ?4, ?5, ?6)",
            rusqlite::params![
                revert_run_id,
                body.universe,
                revert_payload.to_string(),
                now,
                user_id.0,
                parent_run_id
            ],
        )
        .map_err(|e| AppError::Internal(format!("insert revert run: {e}")))?;

    // Mark the parent run as reverted.
    let _ = storage.conn().execute(
        "UPDATE process_runs SET state = 'reverted' WHERE run_id = ?1",
        rusqlite::params![parent_run_id],
    );

    let _ = storage.conn().execute(
        "INSERT INTO telemetry_events \
         (event_type, event_name, user_id, universe_key, payload, timestamp) \
         VALUES ('process', 'alterar-pagina-na-web.reverted', ?1, ?2, ?3, ?4)",
        rusqlite::params![user_id.0, body.universe, revert_payload.to_string(), now],
    );

    let universe_key_clone = body.universe.clone();
    let page_path_owned = page_path.to_string();
    let revert_run_id_clone = revert_run_id.clone();
    let actor_clone = user_id.0.clone();

    drop(storage);

    // CO-156: emit entry.delete telemetry for the revert (the prior write is being undone)
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "entry.delete",
            universe: universe_key_clone,
            list: Some("alterar-pagina-na-web".to_string()),
            key: Some(page_path_owned),
            actor: Some(actor_clone),
            session_id: None,
            extra: Some(
                serde_json::json!({ "revert_run_id": revert_run_id_clone, "parent_run_id": parent_run_id }),
            ),
        },
    );

    Ok(Json(RunResponse {
        run_id: revert_run_id,
        state: "completed".into(),
        from_version: Some(current_version),
        to_version: Some(target_version),
        changelog_entry: Some(changelog_entry),
        deploy_status: Some("simulated".into()),
    }))
}

/// GET /runs?universe=<key>&limit=20 — list recent runs of this process.
pub async fn list_runs(
    State(state): State<AppState>,
    Query(q): Query<RunsQuery>,
) -> Result<Json<Vec<RunRow>>, AppError> {
    let storage = state.core.storage.lock();
    let limit = q.limit.unwrap_or(20).clamp(1, 200);

    fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
        Ok(RunRow {
            run_id: r.get(0)?,
            process_name: r.get(1)?,
            universe_key: r.get(2)?,
            state: r.get(3)?,
            created_at: r.get(4)?,
            completed_at: r.get::<_, Option<String>>(5)?,
            actor_id: r.get::<_, Option<String>>(6)?,
            payload: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
            parent_run_id: r.get::<_, Option<String>>(8)?,
        })
    }

    let rows: Vec<RunRow> = if let Some(uk) = &q.universe {
        let mut stmt = storage
            .conn()
            .prepare(
                "SELECT run_id, process_name, universe_key, state, created_at, \
                 completed_at, actor_id, payload, parent_run_id \
                 FROM process_runs \
                 WHERE process_name = 'alterar-pagina-na-web' AND universe_key = ?1 \
                 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| AppError::Internal(format!("prepare runs query: {e}")))?;
        stmt.query_map(rusqlite::params![uk, limit], map_row)
            .map_err(|e| AppError::Internal(format!("query runs: {e}")))?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        let mut stmt = storage
            .conn()
            .prepare(
                "SELECT run_id, process_name, universe_key, state, created_at, \
                 completed_at, actor_id, payload, parent_run_id \
                 FROM process_runs \
                 WHERE process_name = 'alterar-pagina-na-web' \
                 ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|e| AppError::Internal(format!("prepare runs query: {e}")))?;
        stmt.query_map(rusqlite::params![limit], map_row)
            .map_err(|e| AppError::Internal(format!("query runs: {e}")))?
            .filter_map(|r| r.ok())
            .collect()
    };
    drop(storage);

    Ok(Json(rows))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/alterar-pagina-na-web/preview",
            post(preview_alterar_pagina),
        )
        .route(
            "/alterar-pagina-na-web/approve/{run_id}",
            post(approve_alterar_pagina),
        )
        .route("/alterar-pagina-na-web/revert", post(revert_alterar_pagina))
        .route("/alterar-pagina-na-web/runs", get(list_runs))
}

// Bind unused imports defensively.
#[allow(dead_code)]
fn _bind() {
    let _ = std::any::type_name::<Arc<()>>();
    let _ = StatusCode::OK;
    let _: Box<dyn IntoResponse> = Box::new(StatusCode::OK);
}
