//! CO-275: Agent session endpoints — co-auto POSTs after each run; kanban reads.
//!
//! POST /api/v1/agent/sessions              — record a session (vault token or JWT)
//! GET  /api/v1/agent/sessions?task_id=...  — list sessions for a task
//! GET  /api/v1/agent/sessions/latest?task_id=... — most recent session

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::platform::error::AppError;
use crate::server::AppState;
use crate::storage::{AgentSession, NewAgentSession};

// ---------------------------------------------------------------------------
// Request / response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PostSessionBody {
    pub task_id: String,
    pub universe_key: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub duration_ms: i64,
    pub exit_code: i32,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    /// JSON object: `{"Read": 8, "Edit": 5}`
    pub tool_calls: Option<serde_json::Value>,
    /// JSON array: `["spa-conventions"]`
    pub skills_loaded: Option<serde_json::Value>,
    pub context_chars: Option<i64>,
    pub final_commit_sha: Option<String>,
    pub pr_number: Option<i64>,
    pub model: Option<String>,
    pub co_auto_version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PostSessionResponse {
    pub id: i64,
}

#[derive(Debug, Serialize)]
pub struct ListSessionsResponse {
    pub sessions: Vec<AgentSession>,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub struct TaskIdQuery {
    pub task_id: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn post_session(
    State(state): State<AppState>,
    Json(body): Json<PostSessionBody>,
) -> Result<(StatusCode, Json<PostSessionResponse>), AppError> {
    if body.task_id.is_empty() {
        return Err(AppError::BadRequest("task_id required".into()));
    }
    if body.universe_key.is_empty() {
        return Err(AppError::BadRequest("universe_key required".into()));
    }

    let tool_calls_str = body.tool_calls.as_ref().map(|v| v.to_string());
    let skills_str = body.skills_loaded.as_ref().map(|v| v.to_string());

    let new_session = NewAgentSession {
        task_id: body.task_id.clone(),
        universe_key: body.universe_key.clone(),
        started_at: body.started_at,
        finished_at: body.finished_at,
        duration_ms: body.duration_ms,
        exit_code: body.exit_code,
        tokens_in: body.tokens_in,
        tokens_out: body.tokens_out,
        tool_calls: tool_calls_str,
        skills_loaded: skills_str,
        context_chars: body.context_chars,
        final_commit_sha: body.final_commit_sha.clone(),
        pr_number: body.pr_number,
        model: body.model.clone(),
        co_auto_version: body.co_auto_version.clone(),
        raw_log_blob_sha: None,
    };

    let id = {
        let storage = state.core.storage.lock();
        storage
            .insert_agent_session(&new_session)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };

    // Parse tool_calls for the event bus
    let tool_calls_map: std::collections::HashMap<String, u32> = body
        .tool_calls
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let skills_vec: Vec<String> = body
        .skills_loaded
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    state
        .core
        .event_bus
        .publish(crate::events::DomainEvent::AgentSessionComplete {
            task_id: body.task_id,
            universe_key: body.universe_key,
            duration_ms: body.duration_ms as u64,
            tokens_in: body.tokens_in.map(|n| n as u64),
            tokens_out: body.tokens_out.map(|n| n as u64),
            tool_calls: tool_calls_map,
            skills_loaded: skills_vec,
            context_chars: body.context_chars.unwrap_or(0) as u64,
            exit_code: body.exit_code,
            final_commit_sha: body.final_commit_sha,
            pr_number: body.pr_number.map(|n| n as u32),
        });

    Ok((StatusCode::CREATED, Json(PostSessionResponse { id })))
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(q): Query<TaskIdQuery>,
) -> Result<Json<ListSessionsResponse>, AppError> {
    let sessions = {
        let storage = state.core.storage.lock();
        storage
            .list_agent_sessions(&q.task_id)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    let total = sessions.len();
    Ok(Json(ListSessionsResponse { sessions, total }))
}

async fn latest_session(
    State(state): State<AppState>,
    Query(q): Query<TaskIdQuery>,
) -> Result<Json<Option<AgentSession>>, AppError> {
    let session = {
        let storage = state.core.storage.lock();
        storage
            .latest_agent_session(&q.task_id)
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    Ok(Json(session))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Router for agent session endpoints.
/// POST requires vault-token or JWT auth; GET is public (kanban reads).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/agent/sessions", get(list_sessions))
        .route("/agent/sessions/latest", get(latest_session))
}

/// Auth-gated router — POST (write) requires vault token or JWT.
pub fn authed_router() -> Router<AppState> {
    Router::new().route("/agent/sessions", post(post_session))
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
                worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
            }),
        });
        build_router(state, None)
    }

    fn test_bearer() -> String {
        let (token, _) =
            sign_jwt("test-user", "test@example.com", "player", "test-jwt-secret").unwrap();
        format!("Bearer {token}")
    }

    #[tokio::test]
    async fn post_and_get_session() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let body = json!({
            "task_id": "CO-272",
            "universe_key": "co",
            "started_at": 1_700_000_000_i64,
            "finished_at": 1_700_000_900_i64,
            "duration_ms": 900_000_i64,
            "exit_code": 0,
            "tokens_in": 5000,
            "tokens_out": 3000,
            "tool_calls": {"Read": 8, "Edit": 5},
            "skills_loaded": ["spa-conventions"],
            "context_chars": 12_000,
            "final_commit_sha": "abc1234",
            "pr_number": 89,
            "model": "sonnet",
            "co_auto_version": "0.1.0"
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/agent/sessions")
            .header("content-type", "application/json")
            .header("authorization", test_bearer())
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "POST should return 201");

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed["id"].as_i64().unwrap() > 0);

        // GET latest
        let req2 = Request::builder()
            .method("GET")
            .uri("/api/v1/agent/sessions/latest?task_id=CO-272")
            .body(Body::empty())
            .unwrap();
        let resp2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();
        let session: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();
        assert_eq!(session["task_id"].as_str().unwrap(), "CO-272");
        assert_eq!(session["exit_code"].as_i64().unwrap(), 0);
        assert_eq!(session["final_commit_sha"].as_str().unwrap(), "abc1234");

        // GET list
        let req3 = Request::builder()
            .method("GET")
            .uri("/api/v1/agent/sessions?task_id=CO-272")
            .body(Body::empty())
            .unwrap();
        let resp3 = app.oneshot(req3).await.unwrap();
        assert_eq!(resp3.status(), StatusCode::OK);

        let bytes3 = resp3.into_body().collect().await.unwrap().to_bytes();
        let list: serde_json::Value = serde_json::from_slice(&bytes3).unwrap();
        assert_eq!(list["total"].as_i64().unwrap(), 1);
        assert_eq!(list["sessions"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn latest_returns_null_when_no_sessions() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/agent/sessions/latest?task_id=CO-NONEXISTENT")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(val.is_null(), "missing task should return null");
    }
}
