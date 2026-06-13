//! CO-329: `/analytics` real-time telemetry dashboard.
//!
//! GET  /analytics               — HTML page, auth-gated, noindex.
//! GET  /api/v1/analytics/stream — WebSocket stream of AnalyticsEvents.
//!
//! Auth (both endpoints): JWT from `Authorization: Bearer` or `session` cookie.
//! Auth failures on the WS endpoint are returned as HTTP 401 BEFORE the
//! WebSocket upgrade so they are testable via `tower::ServiceExt::oneshot`.

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::time::{Duration, interval};

use crate::auth::{decode_user_id, extract_session_cookie, jwt_secret};
use crate::observability::{AnalyticsEvent, WorkerInfo, buffer};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PING_INTERVAL: Duration = Duration::from_secs(30);
const SILENCE_TIMEOUT: Duration = Duration::from_secs(45);
/// Snapshot sent on connect: last N events from the ring buffer.
const SNAPSHOT_LIMIT: usize = 200;

// ---------------------------------------------------------------------------
// GET /analytics — HTML page
// ---------------------------------------------------------------------------

pub async fn serve_analytics_page(headers: HeaderMap) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(&headers));

    match token {
        None => return StatusCode::UNAUTHORIZED.into_response(),
        Some(t) => {
            if decode_user_id(&t, &jwt_secret()).is_err() {
                return StatusCode::UNAUTHORIZED.into_response();
            }
        }
    }

    axum::http::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-robots-tag", "noindex")
        .body(axum::body::Body::from(ANALYTICS_PAGE_HTML))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        .into_response()
}

const ANALYTICS_PAGE_HTML: &str = include_str!("../../static/variants/a/analytics.html");

// ---------------------------------------------------------------------------
// WS /api/v1/analytics/stream — real-time event stream
// ---------------------------------------------------------------------------

pub async fn analytics_ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(&headers));

    match token {
        None => return StatusCode::UNAUTHORIZED.into_response(),
        Some(t) => {
            if decode_user_id(&t, &jwt_secret()).is_err() {
                return StatusCode::UNAUTHORIZED.into_response();
            }
        }
    }

    ws.on_upgrade(move |socket| handle_analytics_ws(socket, state))
}

async fn handle_analytics_ws(socket: WebSocket, state: AppState) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Subscribe before reading snapshot so no events are missed.
    let rx = buffer().map(|b| b.subscribe());

    // Send snapshot of recent events.
    if let Some(buf) = buffer() {
        let snap: Vec<AnalyticsEvent> = buf
            .snapshot()
            .into_iter()
            .rev()
            .take(SNAPSHOT_LIMIT)
            .rev()
            .collect();
        let msg = serde_json::json!({ "type": "snapshot", "events": snap });
        let json = serde_json::to_string(&msg).unwrap_or_default();
        if ws_tx.send(Message::Text(json.into())).await.is_err() {
            return;
        }
    }

    let mut rx = match rx {
        Some(r) => r,
        None => return,
    };

    let last_activity = std::sync::Arc::new(std::sync::Mutex::new(tokio::time::Instant::now()));

    // Writer task — forwards buffer events + keep-alive pings.
    let executor = std::sync::Arc::clone(&state.integrations.worker_supervisor);
    let last_w = std::sync::Arc::clone(&last_activity);
    let mut writer = tokio::spawn(async move {
        let mut ping_timer = interval(PING_INTERVAL);
        ping_timer.tick().await;
        let mut worker_timer = interval(Duration::from_secs(5));
        worker_timer.tick().await;

        loop {
            tokio::select! {
                evt = rx.recv() => {
                    match evt {
                        Ok(event) => {
                            let json = serde_json::to_string(&event).unwrap_or_default();
                            if ws_tx.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // Buffer lagged — client is too slow; close gracefully.
                            break;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }

                _ = worker_timer.tick() => {
                    let workers: Vec<WorkerInfo> = executor
                        .statuses()
                        .into_iter()
                        .map(WorkerInfo::from)
                        .collect();
                    let ts = chrono::Utc::now();
                    let event = AnalyticsEvent::WorkerSnapshot { workers, ts };
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    if ws_tx.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }

                _ = ping_timer.tick() => {
                    let elapsed = last_w.lock().unwrap().elapsed();
                    if elapsed > SILENCE_TIMEOUT {
                        break;
                    }
                    if ws_tx.send(Message::Ping(vec![].into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Reader task — only needs to update the last-activity timer.
    let last_r = std::sync::Arc::clone(&last_activity);
    let mut reader = tokio::spawn(async move {
        while let Some(msg) = ws_rx.next().await {
            match msg {
                Ok(Message::Pong(_) | Message::Text(_)) => {
                    *last_r.lock().unwrap() = tokio::time::Instant::now();
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = &mut writer => reader.abort(),
        _ = &mut reader => writer.abort(),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/analytics/stream", get(analytics_ws_handler))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxumStatus};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
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
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
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
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: std::sync::Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        crate::server::build_router(state, None)
    }

    async fn spawn_test_server(app: axum::Router) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        port
    }

    async fn ws_try_no_auth(port: u16, path: &str) -> Option<u16> {
        let url = format!("ws://127.0.0.1:{port}{path}");
        let host = format!("127.0.0.1:{port}");
        let req = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(&url)
            .header("Host", &host)
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .body(())
            .unwrap();
        match tokio_tungstenite::connect_async(req).await {
            Ok((mut ws, _)) => {
                ws.close(None).await.ok();
                None
            }
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Some(resp.status().as_u16()),
            Err(_) => None,
        }
    }

    #[tokio::test]
    async fn analytics_page_requires_auth() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/analytics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn analytics_ws_requires_auth() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let port = spawn_test_server(app).await;
        let status = ws_try_no_auth(port, "/api/v1/analytics/stream").await;
        assert_eq!(status, Some(401), "expected 401, got {status:?}");
    }
}
