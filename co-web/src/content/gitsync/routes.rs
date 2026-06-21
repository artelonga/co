//! CO-89: HTTP routes for git-backed universes.
//!
//! * `POST /api/v1/universes/{slug}/git-source` — set/clear the git source
//!   (owner only).
//! * `POST /api/v1/universes/{slug}/git-sync` — ingest `git log` now (owner
//!   only). Long-running git work runs on a blocking thread; the storage lock is
//!   never held across it.
//! * `POST /api/v1/universes/{slug}/recompute-analytics` — recompute every
//!   profile's analytics struct (owner only).
//! * `GET  /api/v1/universes/{slug}/gantt` — Mermaid gantt of in-flight tasks.
//! * `GET  /api/v1/universes/{slug}/commit-timeline` — Mermaid timeline of
//!   recent commits.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
use crate::error::AppError;
use crate::server::AppState;

use super::{analytics, ingest::SyncStats, mermaid};

#[derive(Debug, Deserialize)]
pub struct SetGitSourceRequest {
    /// Remote (or local `file://`) git URL. Empty/absent ⇒ clear (markdown-only).
    pub git_source: Option<String>,
    /// Branch to ingest. Defaults to the existing value (or `main`).
    pub git_branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GitConfigResponse {
    pub git_source: Option<String>,
    pub git_branch: Option<String>,
    pub git_last_synced_sha: Option<String>,
    pub git_last_synced_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MermaidResponse {
    pub mermaid: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct RecomputeResponse {
    pub profiles_updated: usize,
}

/// Resolve the caller and assert they own `slug`. Returns the universe.
fn require_owner(
    state: &AppState,
    slug: &str,
    caller: &UserId,
) -> Result<crate::models::Universe, AppError> {
    let storage = state.0.core.storage.lock();
    let universe = storage
        .get_universe(slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
    if universe.owner_id != caller.0 {
        return Err(AppError::Forbidden(
            "Only the universe owner can manage git-sync".into(),
        ));
    }
    Ok(universe)
}

/// POST /{slug}/git-source — owner sets or clears the git source.
pub async fn set_git_source(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    caller: UserId,
    Json(body): Json<SetGitSourceRequest>,
) -> Result<Json<GitConfigResponse>, AppError> {
    require_owner(&state, &slug, &caller)?;

    let source = body
        .git_source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let branch = body
        .git_branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let storage = state.0.core.storage.lock();
    storage
        .set_universe_git_source(&slug, source, branch)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let u = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
    Ok(Json(GitConfigResponse {
        git_source: u.git_source,
        git_branch: u.git_branch,
        git_last_synced_sha: u.git_last_synced_sha,
        git_last_synced_at: u.git_last_synced_at,
    }))
}

/// POST /{slug}/git-sync — ingest the universe's git history now.
pub async fn git_sync(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    caller: UserId,
) -> Result<Json<SyncStats>, AppError> {
    let universe = require_owner(&state, &slug, &caller)?;

    let remote = universe.git_source.clone().ok_or_else(|| {
        AppError::BadRequest(format!(
            "Universe '{slug}' has no git_source — set one via POST /git-source first"
        ))
    })?;
    let branch = universe.git_branch.clone().unwrap_or_else(|| "main".into());
    let since_sha = universe.git_last_synced_sha.clone();

    // Refuse early if a private network remote would just block on git's
    // credential prompt (CO-408 parity).
    if crate::vcs::remote_needs_credentials(&remote) {
        return Err(AppError::BadRequest(format!(
            "git source '{remote}' needs credentials (set CO_GIT_TOKEN / CO_GIT_SSH_KEY_PATH)"
        )));
    }

    // Fetch the per-universe store handle + data dir under a brief lock, then
    // drop it: the git clone/log is slow and must not serialize other universes.
    let (store, dest) = {
        let storage = state.0.core.storage.lock();
        let store = storage
            .entry_store(&slug)
            .map_err(|e| AppError::Internal(format!("open universe store: {e}")))?;
        let dest = storage.data_dir.join("git_sources").join(&slug);
        (store, dest)
    };

    let now_ns = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_default();
    let slug_for_sync = slug.clone();
    let stats = tokio::task::spawn_blocking(move || {
        super::sync_universe_git(
            store.as_ref(),
            &slug_for_sync,
            &remote,
            &branch,
            &dest,
            since_sha.as_deref(),
            now_ns,
        )
    })
    .await
    .map_err(|e| AppError::Internal(format!("git-sync task panicked: {e}")))?
    .map_err(|e| AppError::Internal(format!("git-sync failed: {e}")))?;

    // Advance the incremental cursor.
    if let Some(head) = &stats.head_sha {
        let now_iso = chrono::Utc::now().to_rfc3339();
        let storage = state.0.core.storage.lock();
        let _ = storage.set_universe_git_synced(&slug, head, &now_iso);
    }

    // Telemetry: feed the admin pipeline dashboard (CO-88).
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "pipeline.git_sync",
            universe: slug.clone(),
            list: Some("commit".into()),
            key: stats.head_sha.clone(),
            actor: Some(caller.0.clone()),
            session_id: None,
            extra: Some(serde_json::json!({
                "commits_ingested": stats.commits_ingested,
                "profiles_written": stats.profiles_written,
            })),
        },
    );

    Ok(Json(stats))
}

/// POST /{slug}/recompute-analytics — recompute every profile's analytics.
pub async fn recompute_analytics(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    caller: UserId,
) -> Result<Json<RecomputeResponse>, AppError> {
    require_owner(&state, &slug, &caller)?;

    let store = {
        let storage = state.0.core.storage.lock();
        storage
            .entry_store(&slug)
            .map_err(|e| AppError::Internal(format!("open universe store: {e}")))?
    };
    let now_ns = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_default();
    let updated = tokio::task::spawn_blocking(move || {
        analytics::recompute_all_profiles(store.as_ref(), now_ns)
    })
    .await
    .map_err(|e| AppError::Internal(format!("recompute task panicked: {e}")))?
    .map_err(|e| AppError::Internal(format!("recompute failed: {e}")))?;

    Ok(Json(RecomputeResponse {
        profiles_updated: updated,
    }))
}

/// GET /{slug}/gantt — Mermaid gantt of in-flight tasks (swimlane by assignee).
pub async fn gantt(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<MermaidResponse>, AppError> {
    let store = {
        let storage = state.0.core.storage.lock();
        storage
            .entry_store(&slug)
            .map_err(|e| AppError::Internal(format!("open universe store: {e}")))?
    };
    let tasks = store
        .list("task", &serde_json::json!({}), None)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let title = format!("{slug} — in-flight tasks");
    let mermaid = mermaid::tasks_to_gantt(&tasks, &title);
    Ok(Json(MermaidResponse {
        mermaid,
        count: tasks.len(),
    }))
}

/// GET /{slug}/commit-timeline — Mermaid timeline of recent commits.
pub async fn commit_timeline(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<MermaidResponse>, AppError> {
    let store = {
        let storage = state.0.core.storage.lock();
        storage
            .entry_store(&slug)
            .map_err(|e| AppError::Internal(format!("open universe store: {e}")))?
    };
    // Newest 200 commits keeps the diagram legible.
    let commits = store
        .list("commit", &serde_json::json!({}), Some(200))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let title = format!("{slug} — recent commits");
    let mermaid = mermaid::commits_to_timeline(&commits, &title);
    Ok(Json(MermaidResponse {
        mermaid,
        count: commits.len(),
    }))
}

/// Router for the git-backed-universe endpoints. All routes require auth (a
/// session JWT or an API token); ownership is enforced per-handler.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{slug}/git-source", post(set_git_source))
        .route("/{slug}/git-sync", post(git_sync))
        .route("/{slug}/recompute-analytics", post(recompute_analytics))
        .route("/{slug}/gantt", get(gantt))
        .route("/{slug}/commit-timeline", get(commit_timeline))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::require_auth_with_token,
        ))
}
