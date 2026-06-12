//! CO-426: Usage ingestion + launcher registry + fleet query endpoints.
//!
//! POST /api/v1/usage/sessions   — co-auto reports token/cost for a session (CO-425 producer)
//! POST /api/v1/usage/heartbeat  — a launcher registers/refreshes an active session (TTL)
//! GET  /api/v1/usage/summary     — aggregates by universe|model|machine|task over a window
//! GET  /api/v1/usage/active      — launchers active right now
//! GET  /api/v1/usage/projects    — fleet roll-up: usage + board + last deploy per universe
//!
//! Auth, two tiers (CO-426 review hardening):
//! - **Writes** (`POST /sessions`, `POST /heartbeat`) require any valid vault
//!   token or JWT via `require_auth_with_token` (mounted in `router.rs`) — the
//!   launchers reporting usage authenticate with their token.
//! - **Reads** (`GET /summary`, `/active`, `/projects`) additionally require the
//!   `AdminUser` extractor (`tier == "admin"` in the JWT) → **403** otherwise.
//!   This is fleet-wide cost/inventory data and must be API-gated like every
//!   other Gestão data router, not merely hidden behind the admin-only page
//!   (page-gating does not protect the endpoint from a direct token request).
//!   The CO-427 downshift consumer reads `/summary` with the operator's admin
//!   session token.
//!
//! Real-time: `POST /sessions` publishes a `DomainEvent::UsageReported` (bridged
//! to `AnalyticsEvent::Usage`) and `POST /heartbeat` pushes an
//! `AnalyticsEvent::LauncherSnapshot` straight to the analytics buffer — so the
//! existing `/api/v1/analytics/stream` WebSocket drives the panel with no polling.

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::observability::{AnalyticsEvent, LauncherInfo, buffer};
use crate::platform::error::AppError;
use crate::server::AppState;
use crate::storage::{NewUsageSession, UsageDimension, UsageGroup, UsageProjectRow, UsageTotals};

// ---------------------------------------------------------------------------
// Window helpers
// ---------------------------------------------------------------------------

/// Seconds in each named window. Unknown strings fall back to `day`.
fn window_secs(window: &str) -> i64 {
    match window {
        "5h" => 5 * 3600,
        "week" => 7 * 86_400,
        _ => 86_400, // "day"
    }
}

const WINDOW_5H_SECS: i64 = 5 * 3600;
const WINDOW_WEEK_SECS: i64 = 7 * 86_400;

/// Read an optional soft-limit marker from the environment. These are
/// configurable consumption milestones (not official Anthropic limits, which
/// aren't exposed by API), shown as the denominator of the consumption bars.
fn soft_limit(var: &str) -> Option<i64> {
    std::env::var(var).ok().and_then(|v| v.trim().parse().ok())
}

/// Fleet usage *reads* are admin-only (cost + live launcher inventory). Verifies
/// the bearer/cookie token through the same `auth_provider` the write-path
/// middleware uses — so it honours the test `StaticSecretsProvider` and the prod
/// `JWT_SECRET` alike — then requires `tier == "admin"`. Returns 401 (no/invalid
/// token) or 403 (authenticated but not admin). API-gating, not page-gating: the
/// endpoint must refuse a direct token request, not merely be hidden from the UI.
async fn require_admin(state: &AppState, headers: &axum::http::HeaderMap) -> Result<(), AppError> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .or_else(|| crate::auth::extract_session_cookie(headers))
        .ok_or_else(|| AppError::Unauthorized("missing or malformed authorization".into()))?;
    let claims = state
        .core
        .auth_provider
        .verify_token(&crate::infra::auth::Token(token))
        .await
        .map_err(|_| AppError::Unauthorized("invalid or expired token".into()))?;
    if claims.tier != "admin" {
        return Err(AppError::Forbidden("admin access required".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// POST /usage/sessions — ingest a usage report
// ---------------------------------------------------------------------------

/// Inner `usage` object — mirrors co-auto's `SessionUsage` (CO-425). Every field
/// has a default so a partial/best-effort payload never 400s on a missing token.
#[derive(Debug, Default, Deserialize)]
pub struct UsageInner {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_creation_input_tokens: i64,
    #[serde(default)]
    pub cache_read_input_tokens: i64,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct PostUsageBody {
    pub task_key: String,
    pub universe_key: String,
    #[serde(default)]
    pub machine: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub usage: UsageInner,
    pub started_at: i64,
    pub ended_at: i64,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub co_auto_version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PostUsageResponse {
    pub id: i64,
}

/// Shorten a model id to its family (`claude-opus-4-…` → `opus`); echoes
/// co-auto's `short_model_name` so the server stays robust if a full id arrives.
fn short_model(model: &str) -> String {
    for family in ["opus", "sonnet", "haiku", "fable", "mythos"] {
        if model.contains(family) {
            return family.to_string();
        }
    }
    model.to_string()
}

async fn post_session(
    State(state): State<AppState>,
    Json(body): Json<PostUsageBody>,
) -> Result<(StatusCode, Json<PostUsageResponse>), AppError> {
    if body.task_key.is_empty() {
        return Err(AppError::BadRequest("task_key required".into()));
    }
    if body.universe_key.is_empty() {
        return Err(AppError::BadRequest("universe_key required".into()));
    }

    let machine = if body.machine.is_empty() {
        "unknown".to_string()
    } else {
        body.machine.clone()
    };
    // Prefer the explicit model; else derive from the usage models list.
    let model = if !body.model.is_empty() {
        short_model(&body.model)
    } else {
        body.usage
            .models
            .first()
            .map(|m| short_model(m))
            .unwrap_or_else(|| "?".to_string())
    };
    let outcome = if body.outcome.is_empty() {
        "unknown".to_string()
    } else {
        body.outcome.clone()
    };

    let tokens_in = body.usage.input_tokens;
    let tokens_out = body.usage.output_tokens;
    let tokens_cache_read = body.usage.cache_read_input_tokens;
    let tokens_cache_write = body.usage.cache_creation_input_tokens;
    let total_tokens = tokens_in + tokens_out + tokens_cache_read + tokens_cache_write;

    let new_session = NewUsageSession {
        task_key: body.task_key.clone(),
        universe_key: body.universe_key.clone(),
        machine: machine.clone(),
        model: model.clone(),
        tokens_in,
        tokens_out,
        tokens_cache_read,
        tokens_cache_write,
        cost_usd: body.usage.total_cost_usd,
        started_at: body.started_at,
        ended_at: body.ended_at,
        outcome: outcome.clone(),
        reported_at: chrono::Utc::now().timestamp(),
    };

    let id = {
        let storage = state.core.storage.lock();
        storage
            .insert_usage_session(&new_session)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };

    // Bridge to the analytics buffer via the event bus (server/mod.rs subscriber
    // converts DomainEvent → AnalyticsEvent and broadcasts). No polling.
    state
        .core
        .event_bus
        .publish(crate::events::DomainEvent::UsageReported {
            task_key: body.task_key,
            universe_key: body.universe_key,
            machine,
            model,
            total_tokens,
            cost_usd: body.usage.total_cost_usd,
            outcome,
        });

    Ok((StatusCode::CREATED, Json(PostUsageResponse { id })))
}

// ---------------------------------------------------------------------------
// POST /usage/heartbeat — register/refresh an active launcher
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HeartbeatBody {
    pub task_key: String,
    pub universe_key: String,
    pub machine: String,
    #[serde(default)]
    pub model: String,
    pub started_at: i64,
    #[serde(default)]
    pub progress: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatResponse {
    pub ok: bool,
    pub active: usize,
}

async fn post_heartbeat(
    State(state): State<AppState>,
    Json(body): Json<HeartbeatBody>,
) -> Result<Json<HeartbeatResponse>, AppError> {
    if body.task_key.is_empty() {
        return Err(AppError::BadRequest("task_key required".into()));
    }
    if body.machine.is_empty() {
        return Err(AppError::BadRequest("machine required".into()));
    }

    let model = if body.model.is_empty() {
        "?".to_string()
    } else {
        short_model(&body.model)
    };

    state.core.active_launchers.heartbeat(LauncherInfo {
        task_key: body.task_key,
        universe_key: body.universe_key,
        machine: body.machine,
        model,
        started_at: body.started_at,
        // `heartbeat()` stamps last_seen to now; this placeholder is overwritten.
        last_seen: chrono::Utc::now(),
        progress: body.progress,
    });

    // Broadcast the full TTL-filtered snapshot so the panel replaces its list —
    // a crashed machine drops off as soon as any launcher next beats.
    let launchers = state.core.active_launchers.active();
    let active = launchers.len();
    if let Some(buf) = buffer() {
        buf.push(AnalyticsEvent::LauncherSnapshot {
            launchers,
            ts: chrono::Utc::now(),
        });
    }

    Ok(Json(HeartbeatResponse { ok: true, active }))
}

// ---------------------------------------------------------------------------
// GET /usage/summary — aggregates for the dashboard + downshift policy
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SummaryQuery {
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SummaryResponse {
    pub window: String,
    pub by: String,
    pub since: i64,
    pub groups: Vec<UsageGroup>,
    pub totals: UsageTotals,
    /// Always-present rolling totals for the consumption bars, independent of
    /// the requested `window`.
    pub window_5h: UsageTotals,
    pub window_week: UsageTotals,
    /// Configurable soft-limit markers (env `CO_USAGE_SOFT_LIMIT_5H` / `_WEEK`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_limit_5h: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_limit_week: Option<i64>,
}

async fn get_summary(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<SummaryQuery>,
) -> Result<Json<SummaryResponse>, AppError> {
    require_admin(&state, &headers).await?;
    let window = q.window.unwrap_or_else(|| "day".to_string());
    let by_str = q.by.unwrap_or_else(|| "universe".to_string());
    let by = UsageDimension::parse(&by_str)
        .ok_or_else(|| AppError::BadRequest(format!("invalid by='{by_str}'")))?;

    let now = chrono::Utc::now().timestamp();
    let since = now - window_secs(&window);

    let (groups, totals, window_5h, window_week) = {
        let storage = state.core.storage.lock();
        let groups = storage
            .usage_summary(since, by)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let totals = storage
            .usage_window_totals(since)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let window_5h = storage
            .usage_window_totals(now - WINDOW_5H_SECS)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let window_week = storage
            .usage_window_totals(now - WINDOW_WEEK_SECS)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        (groups, totals, window_5h, window_week)
    };

    Ok(Json(SummaryResponse {
        window,
        by: by_str,
        since,
        groups,
        totals,
        window_5h,
        window_week,
        soft_limit_5h: soft_limit("CO_USAGE_SOFT_LIMIT_5H"),
        soft_limit_week: soft_limit("CO_USAGE_SOFT_LIMIT_WEEK"),
    }))
}

// ---------------------------------------------------------------------------
// GET /usage/active — launchers active now
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ActiveResponse {
    pub launchers: Vec<LauncherInfo>,
    pub total: usize,
}

async fn get_active(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ActiveResponse>, AppError> {
    require_admin(&state, &headers).await?;
    let launchers = state.core.active_launchers.active();
    let total = launchers.len();
    Ok(Json(ActiveResponse { launchers, total }))
}

// ---------------------------------------------------------------------------
// GET /usage/projects — fleet roll-up
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ProjectsQuery {
    #[serde(default)]
    pub window: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectsResponse {
    pub window: String,
    pub projects: Vec<UsageProjectRow>,
}

async fn get_projects(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<ProjectsQuery>,
) -> Result<Json<ProjectsResponse>, AppError> {
    require_admin(&state, &headers).await?;
    let window = q.window.unwrap_or_else(|| "week".to_string());
    let since = chrono::Utc::now().timestamp() - window_secs(&window);
    let projects = {
        let storage = state.core.storage.lock();
        storage
            .usage_projects(since)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    Ok(Json(ProjectsResponse { window, projects }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// All usage endpoints. Mounted under `/api/v1` behind `require_auth_with_token`.
pub fn authed_router() -> Router<AppState> {
    Router::new()
        .route("/usage/sessions", post(post_session))
        .route("/usage/heartbeat", post(post_heartbeat))
        .route("/usage/summary", get(get_summary))
        .route("/usage/active", get(get_active))
        .route("/usage/projects", get(get_projects))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::json;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::auth::sign_jwt;
    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::server::{
        AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
        build_router,
    };
    use crate::storage::Storage;

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
            port: 3000,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
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
        }
    }

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        let config = test_config(dir);
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
        let secrets =
            crate::infra::secrets::StaticSecretsProvider::new([("JWT_SECRET", "test-jwt-secret")]);
        let state: AppState = AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage_with_secrets(
                storage, config, auth_store, secrets,
            )),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
                chat_presence: Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        build_router(state, None)
    }

    fn test_bearer() -> String {
        let (token, _) =
            sign_jwt("test-user", "test@example.com", "admin", "test-jwt-secret").unwrap();
        format!("Bearer {token}")
    }

    /// The canonical payload co-auto POSTs (CO-425 `post_usage_to_co`).
    fn usage_payload(task: &str, universe: &str, started: i64) -> serde_json::Value {
        json!({
            "task_key": task,
            "universe_key": universe,
            "machine": "macbook",
            "model": "opus",
            "usage": {
                "input_tokens": 12_000,
                "output_tokens": 4_000,
                "cache_creation_input_tokens": 1_000,
                "cache_read_input_tokens": 30_000,
                "models": ["claude-opus-4-8"],
                "total_cost_usd": 1.23
            },
            "started_at": started,
            "ended_at": started + 120,
            "outcome": "success",
            "co_auto_version": "0.1.0"
        })
    }

    async fn post_json(app: &axum::Router, uri: &str, body: serde_json::Value) -> StatusCode {
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", test_bearer())
            .body(Body::from(body.to_string()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn sessions_requires_auth() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/usage/sessions")
            .header("content-type", "application/json")
            .body(Body::from(usage_payload("CO-426", "co", 1).to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// CO-426 review hardening: the read endpoints expose fleet-wide cost +
    /// live launcher inventory, so a merely-authenticated (non-admin) caller
    /// must be refused — page-gating the Gestão panel does not protect the API.
    #[tokio::test]
    async fn reads_require_admin() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        // A valid JWT but tier=player (not admin).
        let (token, _) = sign_jwt(
            "player-user",
            "player@example.com",
            "player",
            "test-jwt-secret",
        )
        .unwrap();
        let bearer = format!("Bearer {token}");
        for uri in [
            "/api/v1/usage/summary?window=week&by=model",
            "/api/v1/usage/active",
            "/api/v1/usage/projects?window=week",
        ] {
            let req = Request::builder()
                .method("GET")
                .uri(uri)
                .header("authorization", &bearer)
                .body(Body::empty())
                .unwrap();
            let status = app.clone().oneshot(req).await.unwrap().status();
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "non-admin must be 403 on {uri}"
            );
        }
    }

    #[tokio::test]
    async fn post_session_and_summary() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let now = chrono::Utc::now().timestamp();

        let status = post_json(
            &app,
            "/api/v1/usage/sessions",
            usage_payload("CO-426", "co", now),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let status = post_json(
            &app,
            "/api/v1/usage/sessions",
            usage_payload("CO-426", "co", now),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        // Summary by model over the week window includes both sessions.
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/usage/summary?window=week&by=model")
            .header("authorization", test_bearer())
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["by"], "model");
        let groups = v["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["key"], "opus");
        assert_eq!(groups[0]["sessions"].as_i64().unwrap(), 2);
        // total = 2 * (12000 + 4000 + 1000 + 30000)
        assert_eq!(v["totals"]["total_tokens"].as_i64().unwrap(), 2 * 47_000);
        assert_eq!(v["window_5h"]["sessions"].as_i64().unwrap(), 2);
    }

    #[tokio::test]
    async fn summary_rejects_bad_dimension() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/usage/summary?by=bogus")
            .header("authorization", test_bearer())
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn heartbeat_then_active() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let now = chrono::Utc::now().timestamp();

        let hb = json!({
            "task_key": "CO-426",
            "universe_key": "co",
            "machine": "macbook",
            "model": "sonnet",
            "started_at": now,
            "progress": "writing tests"
        });
        let status = post_json(&app, "/api/v1/usage/heartbeat", hb).await;
        assert_eq!(status, StatusCode::OK);

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/usage/active")
            .header("authorization", test_bearer())
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["total"].as_i64().unwrap(), 1);
        assert_eq!(v["launchers"][0]["machine"], "macbook");
        assert_eq!(v["launchers"][0]["model"], "sonnet");
        assert_eq!(v["launchers"][0]["progress"], "writing tests");
    }

    #[tokio::test]
    async fn projects_rollup_route() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let now = chrono::Utc::now().timestamp();
        post_json(
            &app,
            "/api/v1/usage/sessions",
            usage_payload("CO-426", "co", now),
        )
        .await;

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/usage/projects?window=week")
            .header("authorization", test_bearer())
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let projects = v["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["universe_key"], "co");
        assert_eq!(projects[0]["sessions"].as_i64().unwrap(), 1);
    }
}
