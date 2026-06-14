//! CO-453 — Public API telemetry + analytics endpoints (CO-278-E).
//!
//! A documented, read-only telemetry/analytics surface for the public API
//! (CO-278). Every route is gated by [`Scoped<TelemetryRead>`](crate::auth::extractors::Scoped)
//! (`telemetry:read`, CO-448): a least-privilege token carrying the capability
//! reads telemetry without being admin-pleno, and a full JWT/session also
//! passes. A token lacking `telemetry:read` (e.g. `entries:read`-only) → 403.
//!
//! All data is reused from existing tables/aggregations — **no new schema**:
//! - `agent_sessions` (CO-275) → `/agent/sessions[/{id}]`
//! - `deployment_snapshots` (CO-273) → `/deployments`
//! - `usage_sessions` (CO-426) aggregations → `/metrics/throughput`, `/metrics/token-budget`
//! - `release_notes` (CO-334) → `/metrics/release-cadence`
//!
//! Surface lives under `/api/v1/telemetry/*` to avoid colliding with the
//! pre-existing anon, task-scoped `/api/v1/agent/sessions` kanban reads (CO-275).

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::auth::extractors::{Scoped, TelemetryRead};
use crate::platform::error::AppError;
use crate::server::AppState;
use crate::storage::{AgentSession, UsageDimension, UsageGroup, UsageTotals};

/// `CO_AUTO_SOFT_LIMIT_5H_TOKENS` — the rolling 5h soft token limit (CO-427).
/// When set, it is the budget the `/metrics/token-budget` endpoint reports
/// spend against; the default window is therefore 5h.
const BUDGET_WINDOW_SECONDS: i64 = 5 * 3600;
/// Default throughput window when `?since=` is omitted.
const DEFAULT_THROUGHPUT_WINDOW_SECONDS: i64 = 24 * 3600;
/// Default / maximum page size for paginated listings.
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Clamp a caller-supplied `limit` into `[1, MAX_LIMIT]`, defaulting when absent.
fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

// ===========================================================================
// GET /api/v1/telemetry/agent/sessions
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct SessionsQuery {
    pub model: Option<String>,
    /// Epoch seconds — sessions started at/after this are included.
    pub since: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SessionsResponse {
    pub sessions: Vec<AgentSession>,
    pub total: usize,
    pub limit: i64,
    pub offset: i64,
}

async fn list_sessions(
    _cap: Scoped<TelemetryRead>,
    State(state): State<AppState>,
    Query(q): Query<SessionsQuery>,
) -> Result<Json<SessionsResponse>, AppError> {
    let limit = clamp_limit(q.limit);
    let offset = q.offset.unwrap_or(0).max(0);
    let sessions = {
        let storage = state.core.storage.lock();
        storage
            .list_agent_sessions_filtered(q.model.as_deref(), q.since, limit, offset)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    let total = sessions.len();
    Ok(Json(SessionsResponse {
        sessions,
        total,
        limit,
        offset,
    }))
}

// ===========================================================================
// GET /api/v1/telemetry/agent/sessions/{id}
// ===========================================================================

async fn get_session(
    _cap: Scoped<TelemetryRead>,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<AgentSession>, AppError> {
    let session = {
        let storage = state.core.storage.lock();
        storage
            .get_agent_session(id)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    session
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("agent session {id} not found")))
}

// ===========================================================================
// GET /api/v1/telemetry/deployments
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct DeploymentsQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct DeploymentsResponse {
    pub units: Vec<crate::admin::deployment_dashboard::DeploymentSnapshot>,
    pub total: usize,
}

async fn list_deployments(
    _cap: Scoped<TelemetryRead>,
    State(state): State<AppState>,
    Query(q): Query<DeploymentsQuery>,
) -> Json<DeploymentsResponse> {
    // `deployment_snapshots` holds the latest snapshot per unit (UNIQUE unit),
    // so this is already "the latest deploy per app". `?limit=` caps how many
    // units are returned.
    let mut units = {
        let storage = state.core.storage.lock();
        crate::admin::deployment_dashboard::load_snapshots(&storage)
    };
    if let Some(limit) = q.limit {
        let n = limit.clamp(0, units.len() as i64) as usize;
        units.truncate(n);
    }
    let total = units.len();
    Json(DeploymentsResponse { units, total })
}

// ===========================================================================
// GET /api/v1/telemetry/metrics/throughput
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct WindowQuery {
    /// Epoch seconds — window start. Defaults to `now - window`.
    pub since: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ThroughputResponse {
    pub since: i64,
    pub window_seconds: i64,
    pub totals: UsageTotals,
    pub tokens_per_hour: f64,
    pub sessions_per_hour: f64,
    pub by_model: Vec<UsageGroup>,
}

async fn throughput(
    _cap: Scoped<TelemetryRead>,
    State(state): State<AppState>,
    Query(q): Query<WindowQuery>,
) -> Result<Json<ThroughputResponse>, AppError> {
    let now = now_secs();
    let since = q.since.unwrap_or(now - DEFAULT_THROUGHPUT_WINDOW_SECONDS);
    let window_seconds = (now - since).max(1);
    let (totals, by_model) = {
        let storage = state.core.storage.lock();
        let totals = storage
            .usage_window_totals(since)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let by_model = storage
            .usage_summary(since, UsageDimension::Model)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        (totals, by_model)
    };
    let hours = window_seconds as f64 / 3600.0;
    Ok(Json(ThroughputResponse {
        since,
        window_seconds,
        tokens_per_hour: totals.total_tokens as f64 / hours,
        sessions_per_hour: totals.sessions as f64 / hours,
        totals,
        by_model,
    }))
}

// ===========================================================================
// GET /api/v1/telemetry/metrics/token-budget
// ===========================================================================

#[derive(Debug, Serialize)]
pub struct TokenBudgetResponse {
    pub since: i64,
    pub window_seconds: i64,
    pub spent_tokens: i64,
    pub budget_tokens: Option<i64>,
    pub remaining_tokens: Option<i64>,
    pub pct_used: Option<f64>,
}

fn budget_tokens() -> Option<i64> {
    std::env::var("CO_AUTO_SOFT_LIMIT_5H_TOKENS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|&n| n > 0)
}

async fn token_budget(
    _cap: Scoped<TelemetryRead>,
    State(state): State<AppState>,
    Query(q): Query<WindowQuery>,
) -> Result<Json<TokenBudgetResponse>, AppError> {
    let now = now_secs();
    let since = q.since.unwrap_or(now - BUDGET_WINDOW_SECONDS);
    let window_seconds = (now - since).max(1);
    let totals = {
        let storage = state.core.storage.lock();
        storage
            .usage_window_totals(since)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    let spent = totals.total_tokens;
    let budget = budget_tokens();
    let remaining = budget.map(|b| b - spent);
    let pct_used = budget.map(|b| spent as f64 / b as f64 * 100.0);
    Ok(Json(TokenBudgetResponse {
        since,
        window_seconds,
        spent_tokens: spent,
        budget_tokens: budget,
        remaining_tokens: remaining,
        pct_used,
    }))
}

// ===========================================================================
// GET /api/v1/telemetry/metrics/release-cadence
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct ReleaseCadenceQuery {
    /// ISO date "YYYY-MM-DD" — only releases on/after this date.
    pub since: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ReleaseEntry {
    pub repo: String,
    pub version: String,
    pub date: String,
    pub theme: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RepoLatest {
    pub repo: String,
    pub latest_version: String,
    pub latest_date: String,
}

#[derive(Debug, Serialize)]
pub struct ReleaseCadenceResponse {
    pub since: Option<String>,
    pub total_releases: usize,
    pub repos: Vec<RepoLatest>,
    pub releases: Vec<ReleaseEntry>,
}

async fn release_cadence(
    _cap: Scoped<TelemetryRead>,
    State(state): State<AppState>,
    Query(q): Query<ReleaseCadenceQuery>,
) -> Json<ReleaseCadenceResponse> {
    let limit = clamp_limit(q.limit);
    let (releases, repos) = {
        let storage = state.core.storage.lock();
        let feed = storage.query_release_feed(None, q.since.as_deref(), 1, limit as u32);
        let repos = storage.query_release_repos();
        (feed, repos)
    };
    let releases: Vec<ReleaseEntry> = releases
        .into_iter()
        .map(|r| ReleaseEntry {
            repo: r.repo,
            version: r.version,
            date: r.date,
            theme: r.theme,
        })
        .collect();
    let repos: Vec<RepoLatest> = repos
        .into_iter()
        .map(|r| RepoLatest {
            repo: r.repo,
            latest_version: r.latest_version,
            latest_date: r.latest_date,
        })
        .collect();
    Json(ReleaseCadenceResponse {
        since: q.since,
        total_releases: releases.len(),
        repos,
        releases,
    })
}

// ===========================================================================
// Router
// ===========================================================================

/// Telemetry public API, nested at `/api/v1/telemetry` (see [`module docs`](self)).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/agent/sessions", get(list_sessions))
        .route("/agent/sessions/{id}", get(get_session))
        .route("/deployments", get(list_deployments))
        .route("/metrics/throughput", get(throughput))
        .route("/metrics/token-budget", get(token_budget))
        .route("/metrics/release-cadence", get(release_cadence))
}

// ===========================================================================
// Tests (in-process, tower::ServiceExt — no port binding)
// ===========================================================================

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tempfile::{TempDir, tempdir};
    use tower::ServiceExt;

    use crate::server::AppState;
    use crate::storage::{NewAgentSession, NewUsageSession, Storage};

    const SECRET: &str = "test-jwt-secret";

    fn make_state() -> (AppState, TempDir) {
        unsafe { std::env::set_var("JWT_SECRET", SECRET) };
        use crate::config::WebConfig;
        use crate::experiment::ExperimentStore;
        use crate::server::{
            AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
        };

        let dir = tempdir().unwrap();
        let storage = Storage::new(dir.path());
        let config = WebConfig {
            port: 0,
            data_dir: dir.path().to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        };
        let experiment = ExperimentStore::new(dir.path());
        let auth_store = crate::auth::AuthStore::new(dir.path()).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&dir.path().join("game_test.db")).expect("game db"),
        );
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
        (state, dir)
    }

    fn sample_session(task: &str, model: &str, started: i64) -> NewAgentSession {
        NewAgentSession {
            task_id: task.into(),
            universe_key: "co".into(),
            started_at: started,
            finished_at: started + 60,
            duration_ms: 60_000,
            exit_code: 0,
            tokens_in: Some(1000),
            tokens_out: Some(500),
            tool_calls: None,
            skills_loaded: None,
            context_chars: Some(12_000),
            final_commit_sha: Some("abc1234".into()),
            pr_number: Some(7),
            model: Some(model.into()),
            co_auto_version: Some("0.1.0".into()),
            raw_log_blob_sha: None,
        }
    }

    fn sample_usage(task: &str, model: &str, started: i64) -> NewUsageSession {
        NewUsageSession {
            task_key: task.into(),
            universe_key: "co".into(),
            machine: "m1".into(),
            model: model.into(),
            tokens_in: 1000,
            tokens_out: 500,
            tokens_cache_read: 200,
            tokens_cache_write: 100,
            cost_usd: Some(0.25),
            started_at: started,
            ended_at: started + 60,
            outcome: "success".into(),
            reported_at: started + 61,
            tool_uses: None,
            pr_url: None,
            turns: 4,
        }
    }

    /// Mint a least-privilege API token from a capability/bundle list.
    fn scoped_token(state: &AppState, user_id: &str, scopes: &[&str]) -> String {
        let requested: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
        let resolved = crate::auth::capabilities::resolve_scopes(&requested).unwrap();
        let storage = state.core.storage.lock();
        storage
            .create_api_token_with_scopes(user_id, "test", &resolved)
            .unwrap()
            .token
            .expect("raw token")
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .nest("/api/v1/telemetry", super::router())
            .with_state(state)
    }

    fn get_with(uri: &str, token: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri(uri);
        if let Some(t) = token {
            b = b.header("Authorization", format!("Bearer {t}"));
        }
        b.body(Body::empty()).unwrap()
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn make_user(state: &AppState, email: &str) -> String {
        state
            .core
            .storage
            .lock()
            .create_user(email, "T")
            .unwrap()
            .id
    }

    // ---- scope gate (acceptance: entries:read → 403, telemetry:read → 200) ----

    #[tokio::test]
    async fn entries_read_token_is_forbidden() {
        let (state, _dir) = make_state();
        let uid = make_user(&state, "e@x.com");
        let token = scoped_token(&state, &uid, &["entries:read"]);
        let app = app(state);

        for uri in [
            "/api/v1/telemetry/agent/sessions",
            "/api/v1/telemetry/deployments",
            "/api/v1/telemetry/metrics/throughput",
            "/api/v1/telemetry/metrics/token-budget",
            "/api/v1/telemetry/metrics/release-cadence",
        ] {
            let resp = app
                .clone()
                .oneshot(get_with(uri, Some(&token)))
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::FORBIDDEN,
                "{uri} must 403 without telemetry:read"
            );
        }
    }

    #[tokio::test]
    async fn telemetry_read_token_is_allowed() {
        let (state, _dir) = make_state();
        let uid = make_user(&state, "t@x.com");
        let token = scoped_token(&state, &uid, &["telemetry:read"]);
        let app = app(state);

        for uri in [
            "/api/v1/telemetry/agent/sessions",
            "/api/v1/telemetry/deployments",
            "/api/v1/telemetry/metrics/throughput",
            "/api/v1/telemetry/metrics/token-budget",
            "/api/v1/telemetry/metrics/release-cadence",
        ] {
            let resp = app
                .clone()
                .oneshot(get_with(uri, Some(&token)))
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "{uri} must 200 with telemetry:read"
            );
        }
    }

    #[tokio::test]
    async fn missing_token_is_unauthorized() {
        let (state, _dir) = make_state();
        let app = app(state);
        let resp = app
            .oneshot(get_with("/api/v1/telemetry/agent/sessions", None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn jwt_session_passes_gate() {
        let (state, _dir) = make_state();
        let (jwt, _) = crate::auth::sign_jwt("u1", "u@x.com", "free", SECRET).unwrap();
        let app = app(state);
        let resp = app
            .oneshot(get_with("/api/v1/telemetry/deployments", Some(&jwt)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ---- per-route behavior ----

    #[tokio::test]
    async fn agent_sessions_paginates_and_filters_by_model() {
        let (state, _dir) = make_state();
        let uid = make_user(&state, "s@x.com");
        let token = scoped_token(&state, &uid, &["telemetry:read"]);
        {
            let storage = state.core.storage.lock();
            storage
                .insert_agent_session(&sample_session("CO-1", "opus", 1000))
                .unwrap();
            storage
                .insert_agent_session(&sample_session("CO-2", "sonnet", 2000))
                .unwrap();
            storage
                .insert_agent_session(&sample_session("CO-3", "opus", 3000))
                .unwrap();
        }
        let app = app(state);

        // All three, newest first.
        let resp = app
            .clone()
            .oneshot(get_with("/api/v1/telemetry/agent/sessions", Some(&token)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["total"], 3);
        assert_eq!(json["sessions"][0]["task_id"], "CO-3");

        // Filter by model=opus → two.
        let resp = app
            .clone()
            .oneshot(get_with(
                "/api/v1/telemetry/agent/sessions?model=opus",
                Some(&token),
            ))
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["total"], 2);

        // since filter excludes the oldest.
        let resp = app
            .clone()
            .oneshot(get_with(
                "/api/v1/telemetry/agent/sessions?since=1500",
                Some(&token),
            ))
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["total"], 2);

        // limit=1 paginates.
        let resp = app
            .oneshot(get_with(
                "/api/v1/telemetry/agent/sessions?limit=1",
                Some(&token),
            ))
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["total"], 1);
        assert_eq!(json["limit"], 1);
    }

    #[tokio::test]
    async fn single_session_by_id_and_404() {
        let (state, _dir) = make_state();
        let uid = make_user(&state, "i@x.com");
        let token = scoped_token(&state, &uid, &["telemetry:read"]);
        let id = {
            let storage = state.core.storage.lock();
            storage
                .insert_agent_session(&sample_session("CO-1", "opus", 1000))
                .unwrap()
        };
        let app = app(state);

        let resp = app
            .clone()
            .oneshot(get_with(
                &format!("/api/v1/telemetry/agent/sessions/{id}"),
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["task_id"], "CO-1");

        let resp = app
            .oneshot(get_with(
                "/api/v1/telemetry/agent/sessions/99999",
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn throughput_aggregates_usage() {
        let (state, _dir) = make_state();
        let uid = make_user(&state, "th@x.com");
        let token = scoped_token(&state, &uid, &["telemetry:read"]);
        {
            let storage = state.core.storage.lock();
            storage
                .insert_usage_session(&sample_usage("CO-1", "opus", 1000))
                .unwrap();
            storage
                .insert_usage_session(&sample_usage("CO-2", "opus", 2000))
                .unwrap();
            storage
                .insert_usage_session(&sample_usage("CO-3", "sonnet", 3000))
                .unwrap();
        }
        let app = app(state);

        let resp = app
            .oneshot(get_with(
                "/api/v1/telemetry/metrics/throughput?since=0",
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["totals"]["sessions"], 3);
        // 3 × (1000+500+200+100) tokens.
        assert_eq!(json["totals"]["total_tokens"], 3 * 1800);
        // by_model has opus (2) and sonnet (1).
        assert_eq!(json["by_model"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn token_budget_reports_spend_against_env_budget() {
        let (state, _dir) = make_state();
        let uid = make_user(&state, "tb@x.com");
        let token = scoped_token(&state, &uid, &["telemetry:read"]);
        {
            let storage = state.core.storage.lock();
            storage
                .insert_usage_session(&sample_usage("CO-1", "opus", 1000))
                .unwrap();
        }
        unsafe { std::env::set_var("CO_AUTO_SOFT_LIMIT_5H_TOKENS", "10000") };
        let app = app(state);

        let resp = app
            .oneshot(get_with(
                "/api/v1/telemetry/metrics/token-budget?since=0",
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["spent_tokens"], 1800);
        assert_eq!(json["budget_tokens"], 10000);
        assert_eq!(json["remaining_tokens"], 10000 - 1800);
        unsafe { std::env::remove_var("CO_AUTO_SOFT_LIMIT_5H_TOKENS") };
    }

    #[tokio::test]
    async fn release_cadence_lists_releases() {
        let (state, _dir) = make_state();
        let uid = make_user(&state, "rc@x.com");
        let token = scoped_token(&state, &uid, &["telemetry:read"]);
        {
            let storage = state.core.storage.lock();
            storage
                .upsert_release_notes_batch(&[
                    crate::storage::ReleaseNoteRow {
                        repo: "co".into(),
                        version: "2.40.0".into(),
                        date: "2026-06-01".into(),
                        theme: Some("public API".into()),
                        body_md: "notes".into(),
                        body_text: "notes".into(),
                        ingested_at: 100,
                    },
                    crate::storage::ReleaseNoteRow {
                        repo: "co".into(),
                        version: "2.41.0".into(),
                        date: "2026-06-10".into(),
                        theme: None,
                        body_md: "more".into(),
                        body_text: "more".into(),
                        ingested_at: 200,
                    },
                ])
                .unwrap();
        }
        let app = app(state);

        let resp = app
            .oneshot(get_with(
                "/api/v1/telemetry/metrics/release-cadence",
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["total_releases"], 2);
        // Newest date first.
        assert_eq!(json["releases"][0]["version"], "2.41.0");
        assert_eq!(json["repos"].as_array().unwrap().len(), 1);
    }
}
