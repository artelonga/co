//! CO-332: Public chat endpoint backed by a non-Claude LLM.
//!
//! POST /api/v1/chat/:slug           — SSE chat (anon-accessible for public universes)
//! GET  /api/v1/deployments/status   — curated Fly.io deployment snapshot (public)
//!
//! The LLM retrieves data exclusively through deterministic tool calls against the CO
//! storage layer — no RAG, no vector search, no hallucination on content.
//!
//! Provider constraint (CO-332): this endpoint NEVER uses Claude.
//! Claude is reserved for Yuri's authenticated dev tooling (co-auto).

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Router;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::infra::ai::{ChatMessage, ChatProvider, ChatTurn, ToolDef};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// SSE stream wrapper
// ---------------------------------------------------------------------------

struct SseStream(mpsc::Receiver<Result<Vec<u8>, Infallible>>);

impl Stream for SseStream {
    type Item = Result<Vec<u8>, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

fn sse_bytes(event: &str, data: &serde_json::Value) -> Vec<u8> {
    let json = serde_json::to_string(data).unwrap_or_else(|_| "{}".into());
    format!("event: {event}\ndata: {json}\n\n").into_bytes()
}

async fn send_sse(
    tx: &mpsc::Sender<Result<Vec<u8>, Infallible>>,
    event: &str,
    data: &serde_json::Value,
) {
    let _ = tx.send(Ok(sse_bytes(event, data))).await;
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub history: Vec<HistoryMessage>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
}

// ---------------------------------------------------------------------------
// Tool definitions (OpenAI function-calling format)
// ---------------------------------------------------------------------------

fn chat_tools() -> Vec<ToolDef> {
    vec![
        ToolDef::function(
            "search_entries",
            "Search for published content entries. Use when asked about specific topics or content types.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "description": "Content type filter (e.g. 'song', 'post', 'note', 'project'). Omit to search all types."
                    },
                    "q": {
                        "type": "string",
                        "description": "Full-text search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default 10, max 50)",
                        "default": 10
                    }
                },
                "required": []
            }),
        ),
        ToolDef::function(
            "get_entry",
            "Get the full content of a specific entry by its path.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The entry path (e.g. 'public/songs/grace-kelly.md')"
                    }
                },
                "required": ["path"]
            }),
        ),
        ToolDef::function(
            "list_types",
            "List all published content types available in this universe.",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        ),
        ToolDef::function(
            "get_recent",
            "Get recently added or updated published entries.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Number of recent entries to return (default 10, max 20)",
                        "default": 10
                    }
                },
                "required": []
            }),
        ),
        ToolDef::function(
            "get_deployment_status",
            "Get the current deployment status of the sister apps on yuri.artelonga.com.br.",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        ),
        // CO-333: feedback submission from chat
        ToolDef::function(
            "submit_feedback",
            "Submit feedback on behalf of the visitor. Use when the visitor explicitly wants to leave a comment (feedback), ask a question (duvida), or make a suggestion (sugestao). Extract the intent from the conversation.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["feedback", "duvida", "sugestao"],
                        "description": "Type: 'feedback' = general comment, 'duvida' = question, 'sugestao' = suggestion"
                    },
                    "message": {
                        "type": "string",
                        "description": "The feedback message, extracted or summarised from the visitor's words"
                    },
                    "entry_path": {
                        "type": "string",
                        "description": "Optional: path of the specific entry this feedback is about (e.g. 'public/songs/grace-kelly.md')"
                    }
                },
                "required": ["kind", "message"]
            }),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Tool execution — deterministic, no hallucination
// ---------------------------------------------------------------------------

/// CO-333: submit feedback on behalf of the chat visitor.
fn exec_submit_feedback(
    state: &AppState,
    universe_key: &str,
    args: &serde_json::Value,
) -> serde_json::Value {
    let kind = args["kind"].as_str().unwrap_or("feedback");
    let message = match args["message"].as_str() {
        Some(m) if !m.trim().is_empty() => m.trim(),
        _ => return serde_json::json!({"error": "message is required"}),
    };
    let entry_path = args["entry_path"].as_str();

    match crate::feedback_routes::insert_feedback(
        state,
        crate::feedback_routes::FeedbackCreate {
            universe_key,
            entry_path,
            kind,
            message,
            name: None,
            email: None,
            user_sub: None,
        },
    ) {
        Ok(id) => serde_json::json!({"ok": true, "id": id}),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    }
}

/// Apply the published-only filter for anonymous callers.
fn filter_published(
    state: &AppState,
    universe_key: &str,
    entries: Vec<crate::entry_index::EntryRow>,
) -> Vec<crate::entry_index::EntryRow> {
    let anon_published_only = state
        .core
        .storage_trait
        .get_universe(universe_key)
        .map(|u| u.anon_published_only)
        .unwrap_or(false);

    if !anon_published_only {
        return entries;
    }
    entries
        .into_iter()
        .filter(|e| {
            e.frontmatter
                .get("published")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .collect()
}

fn exec_search_entries(
    state: &AppState,
    universe_key: &str,
    args: &serde_json::Value,
) -> serde_json::Value {
    let entry_type = args["type"].as_str().unwrap_or("");
    let q = args["q"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(10).min(50) as usize;

    let entries = if let Some(query) = q {
        state
            .core
            .storage_trait
            .search_entries(universe_key, query)
            .unwrap_or_default()
    } else {
        state
            .core
            .storage_trait
            .list_entries(universe_key, entry_type, &serde_json::json!({}), None)
            .unwrap_or_default()
    };

    let entries = filter_published(state, universe_key, entries);
    let total = entries.len();
    let shown: Vec<serde_json::Value> = entries
        .into_iter()
        .take(limit)
        .map(|e| {
            serde_json::json!({
                "path": e.path,
                "title": e.title,
                "type": e.entry_type,
                "updated_at": e.updated_at,
            })
        })
        .collect();

    serde_json::json!({"total": total, "shown": shown.len(), "entries": shown})
}

fn exec_get_entry(
    state: &AppState,
    universe_key: &str,
    args: &serde_json::Value,
) -> serde_json::Value {
    let path = match args["path"].as_str() {
        Some(p) => p,
        None => return serde_json::json!({"error": "path is required"}),
    };

    match state.core.storage_trait.get_entry(universe_key, path) {
        Ok(Some(entry)) => {
            let anon_published_only = state
                .core
                .storage_trait
                .get_universe(universe_key)
                .map(|u| u.anon_published_only)
                .unwrap_or(false);
            if anon_published_only
                && !entry
                    .frontmatter
                    .get("published")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                return serde_json::json!({"error": "Entry not found"});
            }

            let body_preview = if entry.body.len() > 2000 {
                format!("{}... [truncated]", &entry.body[..2000])
            } else {
                entry.body.clone()
            };

            serde_json::json!({
                "path": entry.path,
                "title": entry.title,
                "type": entry.entry_type,
                "frontmatter": entry.frontmatter,
                "body": body_preview,
                "updated_at": entry.updated_at,
            })
        }
        Ok(None) => serde_json::json!({"error": "Entry not found"}),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    }
}

fn exec_list_types(state: &AppState, universe_key: &str) -> serde_json::Value {
    let entries = state
        .core
        .storage_trait
        .list_entries(universe_key, "", &serde_json::json!({}), None)
        .unwrap_or_default();
    let entries = filter_published(state, universe_key, entries);

    let mut types: std::collections::HashSet<String> = entries
        .iter()
        .map(|e| e.entry_type.clone())
        .filter(|t| !t.is_empty() && t != "_project")
        .collect();
    // Remove internal marker types
    types.remove("state");
    types.remove("branch");
    let mut types: Vec<String> = types.into_iter().collect();
    types.sort();

    serde_json::json!({"types": types})
}

fn exec_get_recent(
    state: &AppState,
    universe_key: &str,
    args: &serde_json::Value,
) -> serde_json::Value {
    let limit = args["limit"].as_u64().unwrap_or(10).min(20) as usize;

    let entries = state
        .core
        .storage_trait
        .list_entries(universe_key, "", &serde_json::json!({}), None)
        .unwrap_or_default();
    let mut entries = filter_published(state, universe_key, entries);

    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    entries.truncate(limit);

    let shown: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "path": e.path,
                "title": e.title,
                "type": e.entry_type,
                "updated_at": e.updated_at,
            })
        })
        .collect();

    serde_json::json!({"count": shown.len(), "entries": shown})
}

fn exec_get_deployment_status(state: &AppState) -> serde_json::Value {
    let storage = state.core.storage.lock();
    let result = storage
        .conn()
        .prepare(
            "SELECT unit, state, version, health_status, last_deploy_at, region \
             FROM deployment_snapshots ORDER BY unit",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(serde_json::json!({
                    "unit":        row.get::<_, String>(0).unwrap_or_default(),
                    "state":       row.get::<_, String>(1).unwrap_or_default(),
                    "version":     row.get::<_, String>(2).unwrap_or_default(),
                    "health":      row.get::<_, String>(3).unwrap_or_default(),
                    "deployed_at": row.get::<_, String>(4).unwrap_or_default(),
                    "region":      row.get::<_, String>(5).unwrap_or_default(),
                }))
            })
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        });

    match result {
        Ok(units) => serde_json::json!({"units": units}),
        Err(_) => serde_json::json!({"units": [], "note": "No deployment data available yet"}),
    }
}

/// Dispatch a tool call to its implementation.
fn execute_tool(
    state: &AppState,
    universe_key: &str,
    name: &str,
    arguments: &str,
) -> serde_json::Value {
    let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or(serde_json::json!({}));

    match name {
        "search_entries" => exec_search_entries(state, universe_key, &args),
        "get_entry" => exec_get_entry(state, universe_key, &args),
        "list_types" => exec_list_types(state, universe_key),
        "get_recent" => exec_get_recent(state, universe_key, &args),
        "get_deployment_status" => exec_get_deployment_status(state),
        // CO-333: feedback submission from chat assistant
        "submit_feedback" => exec_submit_feedback(state, universe_key, &args),
        _ => serde_json::json!({"error": format!("Unknown tool: {name}")}),
    }
}

// ---------------------------------------------------------------------------
// Chat loop (runs in spawned task)
// ---------------------------------------------------------------------------

async fn run_chat(
    state: AppState,
    provider: Arc<dyn ChatProvider>,
    slug: String,
    req: ChatRequest,
    tx: mpsc::Sender<Result<Vec<u8>, Infallible>>,
) {
    let start = std::time::Instant::now();

    let universe_name = state
        .core
        .storage_trait
        .get_universe(&slug)
        .map(|u| u.name)
        .unwrap_or_else(|| slug.clone());

    let system_prompt = format!(
        "You are a helpful assistant for {universe_name}. \
        Help visitors discover published content using the available tools. \
        ALWAYS use the provided tools to retrieve real data — never invent content or facts. \
        Respond concisely in the same language as the visitor's message (default: Portuguese)."
    );

    let tools = chat_tools();

    // Build message history
    let mut messages = vec![ChatMessage::system(system_prompt)];
    for h in &req.history {
        if matches!(h.role.as_str(), "user" | "assistant") {
            messages.push(ChatMessage {
                role: h.role.clone(),
                content: Some(h.content.clone()),
                tool_calls: None,
                tool_call_id: None,
            });
        }
    }
    messages.push(ChatMessage::user(&req.message));

    // First LLM call — get tool decisions or direct answer
    let turn = match provider.chat(&messages, &tools).await {
        Ok(t) => t,
        Err(e) => {
            send_sse(&tx, "error", &serde_json::json!({"message": e.to_string()})).await;
            return;
        }
    };

    let first_tool_name = turn.tool_calls.first().map(|tc| tc.function.name.clone());

    let final_content = if turn.tool_calls.is_empty() {
        // Direct response — no tool needed
        turn.content
    } else {
        execute_tools_and_get_response(
            state.clone(),
            provider.clone(),
            &slug,
            turn,
            &mut messages,
            &tx,
        )
        .await
    };

    // Stream response as word tokens
    if let Some(content) = final_content {
        for word in content.split(' ') {
            if !word.is_empty() {
                send_sse(
                    &tx,
                    "token",
                    &serde_json::json!({"text": format!("{word} ")}),
                )
                .await;
            }
        }
    }

    send_sse(&tx, "done", &serde_json::json!({})).await;

    // CO-329: log to analytics
    if let Some(buf) = crate::observability::buffer() {
        buf.push(crate::observability::AnalyticsEvent::Domain {
            kind: format!(
                "chat.query.{}",
                first_tool_name.as_deref().unwrap_or("direct")
            ),
            universe_key: Some(slug),
            detail: format!(
                "{} chars, {}ms",
                req.message.len(),
                start.elapsed().as_millis()
            ),
            ts: chrono::Utc::now(),
        });
    }
}

async fn execute_tools_and_get_response(
    state: AppState,
    provider: Arc<dyn ChatProvider>,
    slug: &str,
    turn: ChatTurn,
    messages: &mut Vec<ChatMessage>,
    tx: &mpsc::Sender<Result<Vec<u8>, Infallible>>,
) -> Option<String> {
    // Add assistant's tool-call message to history
    messages.push(ChatMessage {
        role: "assistant".into(),
        content: None,
        tool_calls: Some(turn.tool_calls.clone()),
        tool_call_id: None,
    });

    for tc in &turn.tool_calls {
        send_sse(
            tx,
            "tool_start",
            &serde_json::json!({
                "name": tc.function.name,
                "args": serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
                    .unwrap_or(serde_json::json!({}))
            }),
        )
        .await;

        let result = execute_tool(&state, slug, &tc.function.name, &tc.function.arguments);

        send_sse(
            tx,
            "tool_result",
            &serde_json::json!({"name": tc.function.name, "result": result}),
        )
        .await;

        messages.push(ChatMessage::tool_result(
            tc.id.clone(),
            serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()),
        ));
    }

    // Second LLM call — format tool results into natural-language reply
    match provider.chat(messages, &[]).await {
        Ok(t) => t.content,
        Err(e) => {
            send_sse(tx, "error", &serde_json::json!({"message": e.to_string()})).await;
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/chat/:slug
///
/// Anon-accessible for publicly visible universes. Responds with an SSE stream
/// of `token`, `tool_start`, `tool_result`, `done`, and `error` events.
pub async fn chat_handler(
    State(state): State<AppState>,
    Extension(provider): Extension<Arc<dyn ChatProvider>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<ChatRequest>,
) -> Response {
    // Guard: only publicly visible universes can use the chat endpoint.
    let universe = state.core.storage_trait.get_universe(&slug);
    let is_accessible = universe
        .as_ref()
        .map(|u| u.is_public || u.is_template || u.visibility == "public-subscribable")
        .unwrap_or(false);

    if !is_accessible {
        // Allow authenticated owners/members regardless
        if crate::auth::resolve_user_id(&state, &headers).is_none() {
            return (
                StatusCode::FORBIDDEN,
                "Universe not enabled for public chat",
            )
                .into_response();
        }
    }

    let (tx, rx) = mpsc::channel(64);
    let state_c = state.clone();
    let provider_c = Arc::clone(&provider);

    tokio::spawn(async move {
        run_chat(state_c, provider_c, slug, req, tx).await;
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(SseStream(rx)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// GET /api/v1/deployments/status
///
/// Returns the curated deployment snapshot for all sister apps.
/// Public endpoint — no auth required.
pub async fn deployment_status_handler(
    State(state): State<AppState>,
) -> axum::Json<serde_json::Value> {
    let result = exec_get_deployment_status(&state);
    axum::Json(serde_json::json!({
        "units": result["units"],
        "refreshed_at": chrono::Utc::now().to_rfc3339()
    }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(_state: AppState) -> Router<AppState> {
    let provider: Arc<dyn ChatProvider> = crate::infra::ai::build_chat_provider();

    // CO-332: assert non-Claude at startup so misconfiguration fails fast.
    assert_ne!(
        provider.kind(),
        "claude",
        "CO-332: /chat must never be backed by Claude"
    );

    Router::new()
        .route("/chat/{slug}", post(chat_handler))
        .route("/deployments/status", get(deployment_status_handler))
        .layer(axum::Extension(provider))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::ai::MockChatProvider;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as S};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn build_test_app(dir: &std::path::Path, provider: Arc<dyn ChatProvider>) -> axum::Router {
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
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let game_db_path = dir.join("game_test.db");
        let game_storage = std::sync::Arc::new(
            game_core::storage::Storage::open(&game_db_path).expect("game storage"),
        );
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let state = crate::server::AppState::new(crate::server::AppStateInner {
            core: Arc::new(crate::server::CoreState::from_storage(
                storage, config, auth_store,
            )),
            realtime: Arc::new(crate::server::RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(crate::server::IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(crate::server::IntegrationsState {
                mail,
                geo: Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: std::sync::Mutex::new(crate::experiment::ExperimentStore::new(dir)),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });

        // Minimal router for chat routes (no rate-limit overhead in tests)
        axum::Router::new()
            .route("/api/v1/chat/{slug}", post(chat_handler))
            .route("/api/v1/deployments/status", get(deployment_status_handler))
            .layer(axum::Extension(provider))
            .with_state(state)
    }

    #[test]
    fn chat_provider_in_router_is_not_claude() {
        // Verify the router factory never installs Claude as the chat backend.
        // We can't call `router()` directly in a unit test without full AppState,
        // but we can assert the factory produces a non-Claude kind.
        let p = crate::infra::ai::build_chat_provider();
        assert_ne!(p.kind(), "claude");
    }

    #[tokio::test]
    async fn deployment_status_returns_200() {
        let dir = tempdir().unwrap();
        let provider = MockChatProvider::mock("mock", "hello");
        let app = build_test_app(dir.path(), provider);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/deployments/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), S::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["refreshed_at"].is_string());
        assert!(json["units"].is_array());
    }

    #[tokio::test]
    async fn chat_returns_403_for_private_universe() {
        let dir = tempdir().unwrap();
        let provider = MockChatProvider::mock("mock", "hello");
        let app = build_test_app(dir.path(), provider);

        // "nonexistent-private" universe doesn't exist → treated as private
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/chat/nonexistent-private")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"message":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), S::FORBIDDEN);
    }

    #[tokio::test]
    async fn chat_streams_sse_for_public_universe() {
        use crate::models::CreateUniverse;

        let dir = tempdir().unwrap();
        let provider = MockChatProvider::mock("mock", "Olá! Posso ajudar.");
        let app = build_test_app(dir.path(), provider);

        // Create a public universe first
        {
            let config = crate::config::WebConfig {
                port: 3000,
                data_dir: dir.path().to_str().unwrap().to_string(),
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
            let mut storage = crate::storage::Storage::new(&config.data_dir);
            storage
                .create_universe(
                    CreateUniverse {
                        key: "yuri-test".into(),
                        name: "Yuri Test".into(),
                        description: String::new(),
                    },
                    "owner-1",
                )
                .unwrap();
            // Make it public
            storage
                .conn()
                .execute(
                    "UPDATE universes SET is_public=1 WHERE key=?1",
                    rusqlite::params!["yuri-test"],
                )
                .unwrap();
        }

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/chat/yuri-test")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"message":"olá"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), S::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.contains("text/event-stream"),
            "expected SSE content type, got: {ct}"
        );
    }
}
