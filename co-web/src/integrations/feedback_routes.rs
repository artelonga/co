//! CO-333 + CO-336: Feedback system — Yggdrasil-compatible + per-entry locus + traceability.
//!
//! POST /api/v1/feedback                              — universe-wide (Yggdrasil compat)
//! POST /api/v1/feedback/{universe}/{*entry_path}     — per-entry locus
//! GET  /api/v1/feedback/{universe}                   — list; owner: all, anon: open sugestao
//! GET  /api/v1/feedback/{universe}/entry/{*path}     — per-entry list (anon-safe)
//! GET  /api/v1/feedback/{universe}/public            — public mural (addressed + wont-fix)
//! GET  /api/v1/feedback/{universe}/{id}              — single item
//! GET  /api/v1/feedback/all/public                   — cross-universe public mural
//! PATCH /api/v1/feedback/{id}                        — update feedback (owner-only)

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/feedback", post(submit_universe_wide))
        // Cross-universe public mural — literal prefix avoids param conflicts
        .route("/feedback/all/public", get(list_all_public_feedback))
        // Universe list + patch update (key is universe_key for GET, feedback_id for PATCH)
        .route(
            "/feedback/{key}",
            get(list_universe_feedback).patch(update_feedback),
        )
        // Public mural — literal "public" segment
        .route("/feedback/{universe_key}/public", get(list_public_mural))
        // Single item — prefixed with "item/" to avoid wildcard conflict
        .route("/feedback/{universe_key}/item/{id}", get(get_feedback_item))
        // Per-entry listing (literal "entry/" segment to differentiate from wildcard POST)
        .route(
            "/feedback/{universe_key}/entry/{*entry_path}",
            get(list_entry_feedback),
        )
        // Per-entry submission (wildcard catches all entry paths including multi-segment)
        .route(
            "/feedback/{universe_key}/{*entry_path}",
            post(submit_per_entry),
        )
}

// ---------------------------------------------------------------------------
// Rate limiter: 10 submissions / hour per IP
// ---------------------------------------------------------------------------

static FEEDBACK_RATE: OnceLock<parking_lot::Mutex<HashMap<String, VecDeque<Instant>>>> =
    OnceLock::new();

fn check_rate(ip: &str) -> Result<(), u64> {
    let store = FEEDBACK_RATE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()));
    let mut store = store.lock();
    let now = Instant::now();
    let window = store.entry(ip.to_string()).or_default();
    let cutoff = now - Duration::from_secs(3600);
    while window.front().is_some_and(|t| *t < cutoff) {
        window.pop_front();
    }
    if window.len() >= 3 {
        let oldest = *window.front().unwrap();
        let retry_after = (oldest + Duration::from_secs(3600))
            .saturating_duration_since(now)
            .as_secs()
            .max(1);
        return Err(retry_after);
    }
    window.push_back(now);
    Ok(())
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// Returns true for known probe/smoke/scanner paths.
/// Normalises by stripping a leading '/' so both URL-path and body entry_path work.
fn is_probe_path(path: &str) -> bool {
    let p = path.trim_start_matches('/');
    p.starts_with('_')
        || matches!(
            p,
            "probe" | "smoke" | "selftest" | "telemetry-check" | "analytics-smoke" | "healthcheck"
        )
}

/// Build a minimal `{"error": "<code>"}` response.
fn feedback_err(error: &'static str, status: StatusCode) -> Response {
    (status, axum::Json(serde_json::json!({ "error": error }))).into_response()
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SubmitFeedbackBody {
    /// Yggdrasil compat: universe key in body (for POST /feedback without URL params).
    pub universe: Option<String>,
    pub kind: String,
    #[serde(alias = "body")]
    pub message: String,
    pub name: Option<String>,
    pub email: Option<String>,
    /// Optional entry_path when submitting from chat (CO-332 integration).
    pub entry_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FeedbackCreated {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackItem {
    pub id: String,
    pub universe_key: String,
    pub entry_path: Option<String>,
    pub kind: String,
    pub message: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub user_sub: Option<String>,
    pub anonymous: bool,
    pub created_at: i64,
    pub status: String,
    // CO-336 traceability fields
    pub linked_ref: Option<String>,
    pub linked_summary: Option<String>,
    pub owner_response: Option<String>,
    pub public_visible: bool,
}

/// CO-336: Body for PATCH /feedback/{id} — all fields optional.
#[derive(Debug, Deserialize)]
pub struct UpdateFeedbackBody {
    pub status: Option<String>,
    pub linked_ref: Option<String>,
    pub linked_summary: Option<String>,
    pub owner_response: Option<String>,
    pub public_visible: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct FeedbackList {
    pub items: Vec<FeedbackItem>,
    pub total: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Input for a new feedback row (avoids too-many-arguments clippy lint).
pub struct FeedbackCreate<'a> {
    pub universe_key: &'a str,
    pub entry_path: Option<&'a str>,
    pub kind: &'a str,
    pub message: &'a str,
    pub name: Option<&'a str>,
    pub email: Option<&'a str>,
    pub user_sub: Option<&'a str>,
}

fn validate_kind(kind: &str) -> Result<(), AppError> {
    match kind {
        "feedback" | "duvida" | "sugestao" => Ok(()),
        _ => Err(AppError::BadRequest(format!(
            "kind must be 'feedback', 'duvida', or 'sugestao'; got '{kind}'"
        ))),
    }
}

/// Returns true when the given status is a terminal state.
fn is_terminal_status(status: &str) -> bool {
    matches!(status, "addressed" | "wont-fix" | "duplicate")
}

/// Validates that the new status is a legal transition from the current status.
fn validate_status_transition(current: &str, new: &str) -> Result<(), AppError> {
    match new {
        "open" | "reviewed" | "addressed" | "wont-fix" | "duplicate" => {}
        _ => {
            return Err(AppError::BadRequest(format!(
                "status must be one of: open, reviewed, addressed, wont-fix, duplicate; got '{new}'"
            )));
        }
    }
    if is_terminal_status(current) {
        return Err(AppError::BadRequest(format!(
            "status '{current}' is terminal and cannot be changed"
        )));
    }
    Ok(())
}

fn universe_owner(state: &AppState, universe_key: &str) -> Result<String, AppError> {
    state
        .core
        .storage
        .lock()
        .get_universe(universe_key)
        .map(|u| u.owner_id)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))
}

pub fn insert_feedback(state: &AppState, create: FeedbackCreate<'_>) -> Result<String, AppError> {
    let id = nanoid::nanoid!(21);
    let now = chrono::Utc::now().timestamp();
    let anonymous = i64::from(create.user_sub.is_none());
    state
        .core
        .storage
        .lock()
        .conn()
        .execute(
            "INSERT INTO feedback \
             (id, universe_key, entry_path, kind, message, name, email, user_sub, anonymous, created_at, status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'open')",
            rusqlite::params![
                id, create.universe_key, create.entry_path, create.kind, create.message,
                create.name, create.email, create.user_sub, anonymous, now
            ],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(id)
}

fn notify_owner(state: &AppState, universe_key: &str, entry_path: Option<&str>, id: &str) {
    let Ok(owner_id) = universe_owner(state, universe_key) else {
        return;
    };
    let storage = state.core.storage.lock();
    let _ = storage.create_notification(
        &owner_id,
        "feedback_received",
        Some(universe_key),
        None,
        "system",
        id,
        "feedback.received",
        serde_json::json!({"entry_path": entry_path, "feedback_id": id}),
    );
}

/// Fire-and-forget federation forward (CO_FEEDBACK_FORWARD_URL env var).
fn maybe_forward(
    universe_key: String,
    entry_path: Option<String>,
    kind: String,
    message: String,
    name: Option<String>,
    email: Option<String>,
) {
    let Some(url) = crate::infra::secrets::global().get("CO_FEEDBACK_FORWARD_URL") else {
        return;
    };
    tokio::spawn(async move {
        let body = serde_json::json!({
            "universe": universe_key,
            "entry_path": entry_path,
            "kind": kind,
            "message": message,
            "name": name,
            "email": email,
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        let _ = client.post(&url).json(&body).send().await;
    });
}

/// CO-336: Fire-and-forget background task to fetch a GitHub PR/commit title
/// and store it in linked_summary (only if not already set).
fn maybe_fetch_github_title(
    state: AppState,
    id: String,
    linked_ref: String,
    summary_already_set: bool,
) {
    if summary_already_set || !linked_ref.starts_with("https://github.com/") {
        return;
    }
    tokio::spawn(async move {
        if let Some(title) = fetch_github_title(&linked_ref).await {
            let storage = state.core.storage.lock();
            let _ = storage.set_feedback_linked_summary(&id, &title);
        }
    });
}

async fn fetch_github_title(url: &str) -> Option<String> {
    let api_url = github_url_to_api_url(url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("co-platform/1.0")
        .build()
        .ok()?;
    let resp = client
        .get(&api_url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("title")
        .and_then(|v| v.as_str())
        .or_else(|| {
            json.pointer("/commit/message")
                .and_then(|v| v.as_str())
                .and_then(|m| m.lines().next())
        })
        .map(String::from)
}

fn github_url_to_api_url(url: &str) -> Option<String> {
    let path = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = path.splitn(4, '/').collect();
    if parts.len() < 4 {
        return None;
    }
    match parts[2] {
        "pull" => Some(format!(
            "https://api.github.com/repos/{}/{}/pulls/{}",
            parts[0], parts[1], parts[3]
        )),
        "commit" => Some(format!(
            "https://api.github.com/repos/{}/{}/commits/{}",
            parts[0], parts[1], parts[3]
        )),
        _ => None,
    }
}

const SELECT_COLS: &str = "id, universe_key, entry_path, kind, message, name, email, user_sub, \
     anonymous, created_at, status, linked_ref, linked_summary, owner_response, public_visible";

fn read_items(
    state: &AppState,
    sql: &str,
    p1: &str,
    p2: Option<&str>,
) -> Result<Vec<FeedbackItem>, AppError> {
    let storage = state.core.storage.lock();
    let conn = storage.conn();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let items: Vec<FeedbackItem> = if let Some(p2v) = p2 {
        stmt.query_map(rusqlite::params![p1, p2v], row_to_item)
    } else {
        stmt.query_map(rusqlite::params![p1], row_to_item)
    }
    .map_err(|e| AppError::Internal(e.to_string()))?
    .filter_map(|r| r.ok())
    .collect();
    Ok(items)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedbackItem> {
    Ok(FeedbackItem {
        id: row.get(0)?,
        universe_key: row.get(1)?,
        entry_path: row.get(2)?,
        kind: row.get(3)?,
        message: row.get(4)?,
        name: row.get(5)?,
        email: row.get(6)?,
        user_sub: row.get(7)?,
        anonymous: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        status: row.get(10)?,
        linked_ref: row.get(11)?,
        linked_summary: row.get(12)?,
        owner_response: row.get(13)?,
        public_visible: row.get::<_, i64>(14)? != 0,
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/feedback — Yggdrasil-compatible universe-wide submission.
pub async fn submit_universe_wide(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<SubmitFeedbackBody>,
) -> Result<Response, AppError> {
    // 1. Probe path check — before body-length so scanners don't get a different hint.
    if let Some(ref ep) = body.entry_path
        && is_probe_path(ep)
    {
        return Ok(feedback_err("probe_path_blocked", StatusCode::BAD_REQUEST));
    }
    // 2. Body length check.
    if body.message.trim().len() < 5 {
        return Ok(feedback_err("body_too_short", StatusCode::BAD_REQUEST));
    }
    // 3. Rate limit: 3 submissions per IP per hour.
    let ip = client_ip(&headers);
    if let Err(retry_after) = check_rate(&ip) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({
                "error": "rate_limited",
                "retry_after_s": retry_after,
            })),
        )
            .into_response());
    }

    let universe_key = body
        .universe
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("'universe' field required".into()))?;
    validate_kind(&body.kind)?;

    // Verify universe exists
    universe_owner(&state, universe_key)?;

    let user_sub = crate::auth::resolve_user_id(&state, &headers);
    let entry_path = body.entry_path.as_deref();

    let id = insert_feedback(
        &state,
        FeedbackCreate {
            universe_key,
            entry_path,
            kind: &body.kind,
            message: body.message.trim(),
            name: body.name.as_deref(),
            email: body.email.as_deref(),
            user_sub: user_sub.as_deref(),
        },
    )?;

    maybe_forward(
        universe_key.to_string(),
        entry_path.map(String::from),
        body.kind.clone(),
        body.message.trim().to_string(),
        body.name.clone(),
        body.email.clone(),
    );
    notify_owner(&state, universe_key, entry_path, &id);

    Ok((StatusCode::CREATED, axum::Json(FeedbackCreated { id })).into_response())
}

/// POST /api/v1/feedback/{universe}/{*entry_path} — per-entry locus.
pub async fn submit_per_entry(
    State(state): State<AppState>,
    Path((universe_key, entry_path)): Path<(String, String)>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<SubmitFeedbackBody>,
) -> Result<Response, AppError> {
    // 1. Probe path check (entry_path from URL, no leading slash).
    if is_probe_path(&entry_path) {
        return Ok(feedback_err("probe_path_blocked", StatusCode::BAD_REQUEST));
    }
    // 2. Body length check.
    if body.message.trim().len() < 5 {
        return Ok(feedback_err("body_too_short", StatusCode::BAD_REQUEST));
    }
    // 3. Rate limit: 3 submissions per IP per hour.
    let ip = client_ip(&headers);
    if let Err(retry_after) = check_rate(&ip) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({
                "error": "rate_limited",
                "retry_after_s": retry_after,
            })),
        )
            .into_response());
    }

    validate_kind(&body.kind)?;

    // Verify universe exists
    universe_owner(&state, &universe_key)?;

    let user_sub = crate::auth::resolve_user_id(&state, &headers);

    let id = insert_feedback(
        &state,
        FeedbackCreate {
            universe_key: &universe_key,
            entry_path: Some(&entry_path),
            kind: &body.kind,
            message: body.message.trim(),
            name: body.name.as_deref(),
            email: body.email.as_deref(),
            user_sub: user_sub.as_deref(),
        },
    )?;

    maybe_forward(
        universe_key.clone(),
        Some(entry_path.clone()),
        body.kind.clone(),
        body.message.trim().to_string(),
        body.name.clone(),
        body.email.clone(),
    );
    notify_owner(&state, &universe_key, Some(&entry_path), &id);

    Ok((StatusCode::CREATED, axum::Json(FeedbackCreated { id })).into_response())
}

/// GET /api/v1/feedback/{universe} — list feedback for the universe.
/// Owner sees all; anonymous sees only open sugestao.
pub async fn list_universe_feedback(
    State(state): State<AppState>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let owner_id = universe_owner(&state, &key)?;
    let caller = crate::auth::resolve_user_id(&state, &headers);
    let is_owner = caller.as_deref() == Some(owner_id.as_str());

    let items = if is_owner {
        read_items(
            &state,
            &format!(
                "SELECT {SELECT_COLS} FROM feedback \
                 WHERE universe_key = ?1 ORDER BY created_at DESC"
            ),
            &key,
            None,
        )?
    } else {
        read_items(
            &state,
            &format!(
                "SELECT {SELECT_COLS} FROM feedback \
                 WHERE universe_key = ?1 AND kind = 'sugestao' AND status = 'open' \
                 ORDER BY created_at DESC"
            ),
            &key,
            None,
        )?
    };

    let total = items.len();
    Ok(axum::Json(FeedbackList { items, total }))
}

/// GET /api/v1/feedback/{universe}/entry/{*path} — per-entry list (anon-safe).
/// CO-336: Anon callers now also see public_visible=1 items (not just sugestao/open).
pub async fn list_entry_feedback(
    State(state): State<AppState>,
    Path((universe_key, entry_path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let owner_id = universe_owner(&state, &universe_key)?;
    let caller = crate::auth::resolve_user_id(&state, &headers);
    let is_owner = caller.as_deref() == Some(owner_id.as_str());

    let items = if is_owner {
        read_items(
            &state,
            &format!(
                "SELECT {SELECT_COLS} FROM feedback \
                 WHERE universe_key = ?1 AND entry_path = ?2 ORDER BY created_at DESC"
            ),
            &universe_key,
            Some(&entry_path),
        )?
    } else {
        read_items(
            &state,
            &format!(
                "SELECT {SELECT_COLS} FROM feedback \
                 WHERE universe_key = ?1 AND entry_path = ?2 \
                 AND (public_visible = 1 OR (kind = 'sugestao' AND status = 'open')) \
                 ORDER BY created_at DESC"
            ),
            &universe_key,
            Some(&entry_path),
        )?
    };

    let total = items.len();
    Ok(axum::Json(FeedbackList { items, total }))
}

/// GET /api/v1/feedback/{universe}/public — public mural.
/// Lists addressed + wont-fix items where public_visible=1.
/// Anon-accessible (no auth required).
pub async fn list_public_mural(
    State(state): State<AppState>,
    Path(universe_key): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Verify universe exists
    universe_owner(&state, &universe_key)?;

    let items = read_items(
        &state,
        &format!(
            "SELECT {SELECT_COLS} FROM feedback \
             WHERE universe_key = ?1 AND public_visible = 1 \
             AND status IN ('addressed', 'wont-fix') \
             ORDER BY created_at DESC"
        ),
        &universe_key,
        None,
    )?;

    let total = items.len();
    Ok(axum::Json(FeedbackList { items, total }))
}

/// GET /api/v1/feedback/all/public — cross-universe public mural.
/// Lists all addressed + wont-fix public_visible items across all universes.
pub async fn list_all_public_feedback(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let storage = state.core.storage.lock();
    let conn = storage.conn();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SELECT_COLS} FROM feedback \
             WHERE public_visible = 1 AND status IN ('addressed', 'wont-fix') \
             ORDER BY created_at DESC"
        ))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let items: Vec<FeedbackItem> = stmt
        .query_map([], row_to_item)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    drop(storage);

    let total = items.len();
    Ok(axum::Json(FeedbackList { items, total }))
}

/// GET /api/v1/feedback/{universe_key}/{id} — get single feedback item.
/// Owner sees all; anon sees only public_visible=1.
pub async fn get_feedback_item(
    State(state): State<AppState>,
    Path((universe_key, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let owner_id = universe_owner(&state, &universe_key)?;
    let caller = crate::auth::resolve_user_id(&state, &headers);
    let is_owner = caller.as_deref() == Some(owner_id.as_str());

    let storage = state.core.storage.lock();
    let conn = storage.conn();
    let item = conn
        .query_row(
            &format!(
                "SELECT {SELECT_COLS} FROM feedback \
                 WHERE universe_key = ?1 AND id = ?2"
            ),
            rusqlite::params![universe_key, id],
            row_to_item,
        )
        .map_err(|_| AppError::NotFound(format!("Feedback '{id}' not found")))?;
    drop(storage);

    if !is_owner && !item.public_visible {
        return Err(AppError::NotFound(format!("Feedback '{id}' not found")));
    }

    Ok(axum::Json(item))
}

/// PATCH /api/v1/feedback/{id} — update feedback (owner-only).
/// CO-336: extended from UpdateStatusBody to UpdateFeedbackBody.
pub async fn update_feedback(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UpdateFeedbackBody>,
) -> Result<impl IntoResponse, AppError> {
    // Load the existing item to get current status and universe_key
    let (current_status, universe_key): (String, String) = state
        .core
        .storage
        .lock()
        .conn()
        .query_row(
            "SELECT status, universe_key FROM feedback WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| AppError::NotFound(format!("Feedback '{id}' not found")))?;

    let owner_id = universe_owner(&state, &universe_key)?;
    let caller = crate::auth::resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    if caller != owner_id {
        return Err(AppError::Forbidden(
            "Only the universe owner can update feedback".into(),
        ));
    }

    // Validate status transition if status is being changed
    if let Some(ref new_status) = body.status {
        validate_status_transition(&current_status, new_status)?;

        // wont-fix requires an owner_response in the body OR already set in DB
        if new_status == "wont-fix" {
            let has_response_in_body = body
                .owner_response
                .as_deref()
                .is_some_and(|r| !r.trim().is_empty());
            if !has_response_in_body {
                // Check if there's already a response in DB
                let existing_response: Option<String> = state
                    .core
                    .storage
                    .lock()
                    .conn()
                    .query_row(
                        "SELECT owner_response FROM feedback WHERE id = ?1",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                if existing_response
                    .as_deref()
                    .map(|r| r.trim().is_empty())
                    .unwrap_or(true)
                {
                    return Err(AppError::BadRequest(
                        "owner_response is required when setting status to 'wont-fix'".into(),
                    ));
                }
            }
        }
    }

    // Build dynamic SET clause — only update provided fields
    let mut set_clauses: Vec<String> = Vec::new();
    let mut sql_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut param_idx = 1usize;

    if let Some(ref s) = body.status {
        set_clauses.push(format!("status = ?{param_idx}"));
        sql_params.push(Box::new(s.clone()));
        param_idx += 1;

        // Terminal states auto-set public_visible=1
        if is_terminal_status(s) && body.public_visible != Some(false) {
            set_clauses.push(format!("public_visible = ?{param_idx}"));
            sql_params.push(Box::new(1i64));
            param_idx += 1;
        }
    }

    if let Some(ref lr) = body.linked_ref {
        set_clauses.push(format!("linked_ref = ?{param_idx}"));
        sql_params.push(Box::new(lr.clone()));
        param_idx += 1;
    }

    if let Some(ref ls) = body.linked_summary {
        set_clauses.push(format!("linked_summary = ?{param_idx}"));
        sql_params.push(Box::new(ls.clone()));
        param_idx += 1;
    }

    if let Some(ref or_) = body.owner_response {
        set_clauses.push(format!("owner_response = ?{param_idx}"));
        sql_params.push(Box::new(or_.clone()));
        param_idx += 1;
    }

    // Only set public_visible explicitly if we didn't already auto-set it from status
    let auto_set_public = body
        .status
        .as_deref()
        .is_some_and(|s| is_terminal_status(s) && body.public_visible != Some(false));
    if !auto_set_public && let Some(pv) = body.public_visible {
        set_clauses.push(format!("public_visible = ?{param_idx}"));
        sql_params.push(Box::new(if pv { 1i64 } else { 0i64 }));
        param_idx += 1;
    }

    if set_clauses.is_empty() {
        // Nothing to update — return OK
        return Ok(StatusCode::OK);
    }

    let set_sql = set_clauses.join(", ");
    let sql = format!("UPDATE feedback SET {set_sql} WHERE id = ?{param_idx}");
    sql_params.push(Box::new(id.clone()));

    {
        let storage = state.core.storage.lock();
        let conn = storage.conn();
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            sql_params.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_refs.as_slice())
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // CO-336: fire-and-forget GitHub title fetch
    if let Some(lr) = body.linked_ref {
        maybe_fetch_github_title(state, id, lr, body.linked_summary.is_some());
    }

    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use rusqlite::params;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::server::{
        AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
    };
    use crate::storage::Storage;

    fn isolate_env() {
        // CO-477: share the canonical "test-jwt-secret" with every other co-web
        // test so concurrent JWT_SECRET writes can never produce a value
        // mismatch (no test removes JWT_SECRET). The signing key below must
        // match this value.
        unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    }

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
            port: 3000,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "universo".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "test".into(),
            wae_api_key: None,
            wae_endpoint: None,
            cookie_domain: None,
            bypass_rate_limit: false,
        }
    }

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        let config = test_config(dir);
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("open game storage"));
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let state = AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
                chat_presence: Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        crate::server::build_router(state, None)
    }

    fn insert_user(dir: &std::path::Path, email: &str) -> String {
        let storage = Storage::new(dir.to_str().unwrap());
        let id = format!("usr_{}", nanoid::nanoid!(8));
        let usuario = email.split('@').next().unwrap_or("u").to_lowercase();
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
                 VALUES (?1, ?2, ?3, 'player', ?4, ?5)",
                params![id, email, email, now, usuario],
            )
            .unwrap();
        id
    }

    fn insert_universe(dir: &std::path::Path, key: &str, owner_id: &str) {
        let storage = Storage::new(dir.to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, visibility) \
                 VALUES (?1, ?2, '', ?3, ?4, 'private')",
                params![key, key, owner_id, now],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, owner_id, now],
            )
            .unwrap();
        storage.ensure_default_room(key).unwrap();
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    // -----------------------------------------------------------------------
    // 1. POST /api/v1/feedback — universe-wide (Yggdrasil compat)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_submit_feedback_universe_wide_201() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb1@example.com");
        insert_universe(dir.path(), "fb_uni1", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.0.1")
                    .body(Body::from(
                        r#"{"universe":"fb_uni1","kind":"feedback","message":"Great content!"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let json = body_json(resp.into_body()).await;
        assert!(json["id"].is_string());
    }

    // -----------------------------------------------------------------------
    // 2. POST /api/v1/feedback/{universe}/{*path} — per-entry locus
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_submit_feedback_per_entry_201() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb2@example.com");
        insert_universe(dir.path(), "fb_uni2", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback/fb_uni2/content/about.md")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.0.2")
                    .body(Body::from(
                        r#"{"kind":"duvida","message":"What does this mean?"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let json = body_json(resp.into_body()).await;
        assert!(json["id"].is_string());
    }

    // -----------------------------------------------------------------------
    // 3. GET /api/v1/feedback/{universe} — list (no auth = only sugestao/open)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_universe_feedback_anon() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb3@example.com");
        insert_universe(dir.path(), "fb_uni3", &owner_id);

        // Insert one feedback, one sugestao
        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('id1', 'fb_uni3', 'feedback', 'private comment', 1, 1000, 'open')",
                    [],
                )
                .unwrap();
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('id2', 'fb_uni3', 'sugestao', 'public suggestion', 1, 2000, 'open')",
                    [],
                )
                .unwrap();
        }

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/feedback/fb_uni3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        // Anon should only see sugestao/open
        assert_eq!(json["total"], 1);
        assert_eq!(json["items"][0]["kind"], "sugestao");
    }

    // Helper: create HS256 JWT signed with the test JWT_SECRET.
    fn test_jwt(user_id: &str) -> String {
        use jsonwebtoken::{Algorithm, EncodingKey, Header};
        #[derive(serde::Serialize)]
        struct C {
            sub: String,
            exp: i64,
            iat: i64,
            email: String,
            tier: String,
            usuario: String,
        }
        jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &C {
                sub: user_id.to_string(),
                exp: chrono::Utc::now().timestamp() + 3600,
                iat: chrono::Utc::now().timestamp(),
                email: String::new(),
                tier: "player".into(),
                usuario: user_id.to_string(),
            },
            &EncodingKey::from_secret(b"test-jwt-secret"), // CO-477: canonical test secret
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // 4. PATCH /api/v1/feedback/{id} — owner updates status
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_status_owner_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb4@example.com");
        insert_universe(dir.path(), "fb_uni4", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('fb_id4', 'fb_uni4', 'feedback', 'hello', 1, 1000, 'open')",
                    [],
                )
                .unwrap();
        }

        let token = test_jwt(&owner_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/feedback/fb_id4")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"status":"reviewed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let storage2 = Storage::new(dir.path().to_str().unwrap());
        let status: String = storage2
            .conn()
            .query_row(
                "SELECT status FROM feedback WHERE id = 'fb_id4'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "reviewed");
    }

    // -----------------------------------------------------------------------
    // 5. PATCH /api/v1/feedback/{id} — non-owner gets 403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_status_non_owner_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb5@example.com");
        let other_id = insert_user(dir.path(), "other_fb5@example.com");
        insert_universe(dir.path(), "fb_uni5", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('fb_id5', 'fb_uni5', 'feedback', 'hello', 1, 1000, 'open')",
                    [],
                )
                .unwrap();
        }

        let token = test_jwt(&other_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/feedback/fb_id5")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"status":"reviewed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // 6. POST — unknown universe returns 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_submit_unknown_universe_404() {
        isolate_env();
        let dir = tempdir().unwrap();

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.0.6")
                    .body(Body::from(
                        r#"{"universe":"nonexistent","kind":"feedback","message":"hello"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // CO-336 new tests
    // -----------------------------------------------------------------------

    // 7. PATCH with linked_ref — owner can link a PR/commit ref
    #[tokio::test]
    async fn test_patch_linked_ref_owner_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb7@example.com");
        insert_universe(dir.path(), "fb_uni7", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('fb_id7', 'fb_uni7', 'feedback', 'hello', 1, 1000, 'open')",
                    [],
                )
                .unwrap();
        }

        let token = test_jwt(&owner_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/feedback/fb_id7")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        r#"{"linked_ref":"commit:abc123","linked_summary":"Fix the bug","owner_response":"Done!"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let storage2 = Storage::new(dir.path().to_str().unwrap());
        let (linked_ref, linked_summary, owner_response): (String, String, String) = storage2
            .conn()
            .query_row(
                "SELECT linked_ref, linked_summary, owner_response FROM feedback WHERE id = 'fb_id7'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(linked_ref, "commit:abc123");
        assert_eq!(linked_summary, "Fix the bug");
        assert_eq!(owner_response, "Done!");
    }

    // 8. PATCH with wont-fix requires owner_response
    #[tokio::test]
    async fn test_patch_wont_fix_requires_response_400() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb8@example.com");
        insert_universe(dir.path(), "fb_uni8", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('fb_id8', 'fb_uni8', 'feedback', 'hello', 1, 1000, 'open')",
                    [],
                )
                .unwrap();
        }

        let token = test_jwt(&owner_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/feedback/fb_id8")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"status":"wont-fix"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // 9. addressed auto-sets public_visible=1
    #[tokio::test]
    async fn test_addressed_auto_public_visible() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb9@example.com");
        insert_universe(dir.path(), "fb_uni9", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status, public_visible) \
                     VALUES ('fb_id9', 'fb_uni9', 'feedback', 'hello', 1, 1000, 'open', 0)",
                    [],
                )
                .unwrap();
        }

        let token = test_jwt(&owner_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/feedback/fb_id9")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"status":"addressed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let storage2 = Storage::new(dir.path().to_str().unwrap());
        let (status, public_visible): (String, i64) = storage2
            .conn()
            .query_row(
                "SELECT status, public_visible FROM feedback WHERE id = 'fb_id9'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "addressed");
        assert_eq!(public_visible, 1);
    }

    // 10. State machine: terminal status cannot be changed
    #[tokio::test]
    async fn test_state_machine_terminal_reject() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb10@example.com");
        insert_universe(dir.path(), "fb_uni10", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('fb_id10', 'fb_uni10', 'feedback', 'hello', 1, 1000, 'addressed')",
                    [],
                )
                .unwrap();
        }

        let token = test_jwt(&owner_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/feedback/fb_id10")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"status":"open"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // 11. Public mural lists addressed + wont-fix
    #[tokio::test]
    async fn test_public_mural_shows_addressed() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb11@example.com");
        insert_universe(dir.path(), "fb_uni11", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            // public addressed item
            storage.conn().execute(
                "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status, public_visible) \
                 VALUES ('id11a', 'fb_uni11', 'feedback', 'fixed!', 1, 1000, 'addressed', 1)",
                [],
            ).unwrap();
            // private open item
            storage.conn().execute(
                "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status, public_visible) \
                 VALUES ('id11b', 'fb_uni11', 'feedback', 'private', 1, 2000, 'open', 0)",
                [],
            ).unwrap();
        }

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/feedback/fb_uni11/public")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["total"], 1);
        assert_eq!(json["items"][0]["id"], "id11a");
        assert_eq!(json["items"][0]["status"], "addressed");
    }

    // 12. Public mural accessible anonymously (no auth token)
    #[tokio::test]
    async fn test_public_mural_anon_accessible() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb12@example.com");
        insert_universe(dir.path(), "fb_uni12", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/feedback/fb_uni12/public")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should not return 401/403 — empty mural is fine
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["total"], 0);
    }

    // 13. Per-entry feedback strip shows public history for anon
    #[tokio::test]
    async fn test_per_entry_shows_public_history() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb13@example.com");
        insert_universe(dir.path(), "fb_uni13", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            // public_visible=1 addressed
            storage.conn().execute(
                "INSERT INTO feedback (id, universe_key, entry_path, kind, message, anonymous, created_at, status, public_visible) \
                 VALUES ('id13a', 'fb_uni13', 'docs/page.md', 'feedback', 'great fix', 1, 1000, 'addressed', 1)",
                [],
            ).unwrap();
            // private open
            storage.conn().execute(
                "INSERT INTO feedback (id, universe_key, entry_path, kind, message, anonymous, created_at, status, public_visible) \
                 VALUES ('id13b', 'fb_uni13', 'docs/page.md', 'feedback', 'secret', 1, 2000, 'open', 0)",
                [],
            ).unwrap();
        }

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/feedback/fb_uni13/entry/docs/page.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        // anon should see only public_visible=1 item
        assert_eq!(json["total"], 1);
        assert_eq!(json["items"][0]["id"], "id13a");
    }

    // -----------------------------------------------------------------------
    // CO-339: body validation + probe-path blocklist + rate limit
    // -----------------------------------------------------------------------

    // 14. Empty body returns 400 body_too_short
    #[tokio::test]
    async fn test_body_too_short_400() {
        isolate_env();
        let dir = tempdir().unwrap();

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.1.1")
                    .body(Body::from(r#"{"kind":"feedback","body":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["error"], "body_too_short");
    }

    // 15. Probe path returns 400 probe_path_blocked (even if body < 5 chars)
    #[tokio::test]
    async fn test_probe_path_blocked_400() {
        isolate_env();
        let dir = tempdir().unwrap();

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.1.2")
                    .body(Body::from(
                        r#"{"kind":"feedback","body":"ok","entry_path":"/_probe"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["error"], "probe_path_blocked");
    }

    // 16. Various probe paths are blocked
    #[tokio::test]
    async fn test_probe_path_variants_blocked() {
        isolate_env();
        let dir = tempdir().unwrap();

        for (i, probe_path) in [
            "/_smoke",
            "/_selftest",
            "/probe",
            "/smoke",
            "/selftest",
            "/telemetry-check",
            "/analytics-smoke",
            "/healthcheck",
        ]
        .iter()
        .enumerate()
        {
            let app = build_test_router(dir.path());
            let body = serde_json::json!({
                "kind": "feedback",
                "body": "hello world this is a long enough message",
                "entry_path": probe_path,
            });
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/feedback")
                        .header("content-type", "application/json")
                        .header("x-forwarded-for", format!("10.0.1.{}", 20 + i))
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(
                resp.status(),
                StatusCode::BAD_REQUEST,
                "probe path '{probe_path}' should be blocked"
            );
            let json = body_json(resp.into_body()).await;
            assert_eq!(json["error"], "probe_path_blocked");
        }
    }

    // 17. Valid submission with /yuri entry_path succeeds with 201
    #[tokio::test]
    async fn test_valid_feedback_201() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb17@example.com");
        insert_universe(dir.path(), "fb_uni17", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.1.50")
                    .body(Body::from(
                        r#"{"kind":"feedback","body":"Found a bug in mbya terms","universe":"fb_uni17","entry_path":"/yuri"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let json = body_json(resp.into_body()).await;
        assert!(json["id"].is_string());
    }

    // 18. 4th submission from same IP within 1 hour returns 429
    #[tokio::test]
    async fn test_rate_limit_429() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb18@example.com");
        insert_universe(dir.path(), "fb_uni18", &owner_id);

        // 3 submissions succeed
        for i in 0..3u8 {
            let app = build_test_router(dir.path());
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/feedback")
                        .header("content-type", "application/json")
                        .header("x-forwarded-for", "10.0.1.99")
                        .body(Body::from(format!(
                            r#"{{"kind":"feedback","body":"Valid message number {}","universe":"fb_uni18"}}"#,
                            i
                        )))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::CREATED,
                "submission {i} should succeed"
            );
        }

        // 4th submission is rate-limited
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.1.99")
                    .body(Body::from(
                        r#"{"kind":"feedback","body":"This should be blocked","universe":"fb_uni18"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["error"], "rate_limited");
        assert!(json["retry_after_s"].as_u64().unwrap_or(0) > 0);
    }

    // 19. Probe-cleanup migration SQL marks short/empty rows wont-fix
    #[tokio::test]
    async fn test_probe_cleanup_migration_sql() {
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_probe_mig@example.com");
        insert_universe(dir.path(), "probe_mig_uni", &owner_id);

        let storage = Storage::new(dir.path().to_str().unwrap());
        let two_min_ago = chrono::Utc::now().timestamp() - 120;

        // Insert an empty-body probe row
        storage
            .conn()
            .execute(
                "INSERT INTO feedback \
                 (id, universe_key, entry_path, kind, message, anonymous, created_at, status) \
                 VALUES ('pmig1', 'probe_mig_uni', '/_smoke', 'feedback', '', 1, ?1, 'open')",
                rusqlite::params![two_min_ago],
            )
            .unwrap();

        // Insert a short-body probe row ('ok' = 2 chars < 5)
        storage
            .conn()
            .execute(
                "INSERT INTO feedback \
                 (id, universe_key, entry_path, kind, message, anonymous, created_at, status) \
                 VALUES ('pmig2', 'probe_mig_uni', '/probe', 'feedback', 'ok', 1, ?1, 'open')",
                rusqlite::params![two_min_ago],
            )
            .unwrap();

        // Insert a valid row that should NOT be touched
        storage
            .conn()
            .execute(
                "INSERT INTO feedback \
                 (id, universe_key, kind, message, anonymous, created_at, status) \
                 VALUES ('pmig3', 'probe_mig_uni', 'feedback', 'valid feedback message', 1, ?1, 'open')",
                rusqlite::params![two_min_ago],
            )
            .unwrap();

        // Run the cleanup SQL (same as v57 migration)
        storage
            .conn()
            .execute(
                "UPDATE feedback \
                    SET status = 'wont-fix', \
                        owner_response = 'auto-resolved: probe traffic (CO-339)' \
                  WHERE status = 'open' \
                    AND (message IS NULL OR TRIM(message) = '' OR LENGTH(TRIM(message)) < 5) \
                    AND created_at < (strftime('%s','now') - 60)",
                [],
            )
            .unwrap();

        let status1: String = storage
            .conn()
            .query_row("SELECT status FROM feedback WHERE id = 'pmig1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            status1, "wont-fix",
            "empty-body probe row should be wont-fix"
        );

        let status2: String = storage
            .conn()
            .query_row("SELECT status FROM feedback WHERE id = 'pmig2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            status2, "wont-fix",
            "short-body probe row should be wont-fix"
        );

        let status3: String = storage
            .conn()
            .query_row("SELECT status FROM feedback WHERE id = 'pmig3'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status3, "open", "valid feedback row must not be touched");
    }
}
