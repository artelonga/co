//! CO-328: AI provider endpoints.
//!
//! POST /api/v1/ai/query  — run a prompt against Ollama or Claude Code.
//! GET  /api/v1/ai/status — availability of each provider.
//!
//! Both endpoints require a valid session JWT (`require_auth` middleware).
//! If neither provider is installed the query endpoint returns 503 with an
//! install hint so the caller knows exactly what to set up.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::infra::ai::{AiRouter, ProviderStatus};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    pub provider: String,
    pub prompt: String,
}

#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub provider: String,
    pub response: String,
}

#[derive(Debug, Serialize)]
pub struct AiStatusResponse {
    pub ollama: ProviderStatus,
    pub claude: ProviderStatus,
    /// Reserved for future active-session tracking.
    pub active_sessions: Vec<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/ai/query
///
/// Body: `{ "provider": "ollama"|"claude", "prompt": "..." }`
///
/// Returns the AI response or 503 with an install hint when the provider is
/// unavailable.
async fn query_handler(
    State(router): State<Arc<AiRouter>>,
    axum::Json(req): axum::Json<QueryRequest>,
) -> Response {
    let provider = req.provider.as_str();
    if !matches!(provider, "ollama" | "claude") {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": format!(
                    "Unknown provider '{}'. Supported: 'ollama', 'claude'.",
                    provider
                ),
            })),
        )
            .into_response();
    }

    match router.query(provider, &req.prompt).await {
        Ok(resp) => {
            // CO-327: fire a desktop notification when a claude session finishes.
            if provider == "claude" {
                crate::desktop_notify::send(
                    "CO — IA",
                    "Claude terminou de responder",
                    "http://localhost:3000/",
                );
            }

            // CO-329: emit an analytics event.
            if let Some(buf) = crate::observability::buffer() {
                buf.push(crate::observability::AnalyticsEvent::Domain {
                    kind: format!("ai.query.{provider}"),
                    universe_key: None,
                    detail: format!("{} chars", resp.text.len()),
                    ts: chrono::Utc::now(),
                });
            }

            (
                StatusCode::OK,
                axum::Json(QueryResponse {
                    provider: resp.provider,
                    response: resp.text,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": e.to_string(),
                "hint": install_hint(provider),
            })),
        )
            .into_response(),
    }
}

/// GET /api/v1/ai/status
///
/// Returns current availability of Ollama and Claude Code.
async fn status_handler(State(router): State<Arc<AiRouter>>) -> axum::Json<AiStatusResponse> {
    let (ollama, claude) = tokio::join!(router.ollama_status(), router.claude_status());
    axum::Json(AiStatusResponse {
        ollama,
        claude,
        active_sessions: vec![],
    })
}

fn install_hint(provider: &str) -> String {
    match provider {
        "ollama" => {
            "Install Ollama: https://ollama.com — then run: ollama pull qwen2.5-coder:7b".into()
        }
        "claude" => "Install Claude Code CLI: https://claude.ai/code".into(),
        _ => "Provider not found".into(),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Production router — requires auth.
///
/// Nest under `/api/v1` in `build_router`.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/ai/query", post(query_handler))
        .route("/ai/status", get(status_handler))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::require_auth,
        ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::ai::MockProvider;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxumStatus};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tower::ServiceExt;

    // Build a minimal test router with mock providers (no auth).
    fn mock_router(
        ollama: Arc<dyn crate::infra::ai::AiProvider>,
        claude: Arc<dyn crate::infra::ai::AiProvider>,
    ) -> axum::Router {
        let ai = Arc::new(AiRouter::new(ollama, claude));
        axum::Router::new()
            .route("/api/v1/ai/query", post(query_handler))
            .route("/api/v1/ai/status", get(status_handler))
            .with_state(ai)
    }

    // Build the full app for auth-gate tests.
    fn build_full_app(dir: &std::path::Path) -> axum::Router {
        let config = crate::config::WebConfig {
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
            co_env: "test".into(),
            wae_api_key: None,
            wae_endpoint: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        };
        let storage = crate::storage::Storage::new(&config.data_dir);
        let experiment = crate::experiment::ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = std::sync::Arc::new(
            game_core::storage::Storage::open(&game_db_path).expect("game storage"),
        );
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let state = crate::server::AppState::new(crate::server::AppStateInner {
            core: std::sync::Arc::new(crate::server::CoreState::from_storage(
                storage, config, auth_store,
            )),
            realtime: std::sync::Arc::new(crate::server::RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: std::sync::Arc::new(crate::server::IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: std::sync::Arc::new(crate::server::IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: std::sync::Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: std::sync::Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        crate::server::build_router(state, None)
    }

    // --- Auth gate tests (full app) ---

    #[tokio::test]
    async fn query_requires_auth() {
        let dir = tempdir().unwrap();
        let app = build_full_app(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ai/query")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"provider":"ollama","prompt":"hi"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn status_requires_auth() {
        let dir = tempdir().unwrap();
        let app = build_full_app(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/ai/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    // --- Logic tests (mock router, no auth) ---

    #[tokio::test]
    async fn query_unknown_provider_returns_400() {
        let app = mock_router(MockProvider::available("x"), MockProvider::available("y"));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ai/query")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"provider":"gpt4","prompt":"hi"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::BAD_REQUEST);
    }

    #[tokio::test]
    async fn query_ollama_success_returns_200() {
        let app = mock_router(
            MockProvider::available("ollama says hi"),
            MockProvider::unavailable(),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ai/query")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"provider":"ollama","prompt":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["response"], "ollama says hi");
        assert_eq!(json["provider"], "mock");
    }

    #[tokio::test]
    async fn query_claude_success_returns_200() {
        let app = mock_router(
            MockProvider::unavailable(),
            MockProvider::available("claude says hi"),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ai/query")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"provider":"claude","prompt":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["response"], "claude says hi");
    }

    #[tokio::test]
    async fn query_unavailable_provider_returns_503_with_hint() {
        let app = mock_router(MockProvider::unavailable(), MockProvider::unavailable());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ai/query")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"provider":"ollama","prompt":"hi"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["hint"].as_str().unwrap_or("").contains("ollama.com"),
            "hint should mention ollama.com"
        );
    }

    #[tokio::test]
    async fn status_returns_provider_info() {
        let app = mock_router(MockProvider::available("x"), MockProvider::unavailable());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/ai/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["ollama"]["available"].as_bool().unwrap_or(false));
        assert!(!json["claude"]["available"].as_bool().unwrap_or(true));
        assert!(json["active_sessions"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn status_neither_installed_both_unavailable() {
        let app = mock_router(MockProvider::unavailable(), MockProvider::unavailable());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/ai/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["ollama"]["available"].as_bool().unwrap_or(true));
        assert!(!json["claude"]["available"].as_bool().unwrap_or(true));
    }
}
