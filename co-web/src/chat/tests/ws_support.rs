//! WS-specific test helpers.

use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::http::Request as TRequest;

use crate::config::WebConfig;
use crate::experiment::ExperimentStore;
use crate::server::{AppState, AppStateInner, build_router};
use crate::storage::Storage;

pub type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 0,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: false,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: dir.join("universes").to_string_lossy().to_string(),
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

pub fn make_state(dir: &std::path::Path) -> AppState {
    let config = test_config(dir);
    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = crate::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage =
        Arc::new(game_core::storage::Storage::open(&game_db_path).expect("open test game storage"));
    let (embedding_tx, _rx) = crate::embedding_worker::channel();
    Arc::new(AppStateInner {
        storage: parking_lot::Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config,
        auth_store: Mutex::new(auth_store),
        mail,
        game_storage,
        plugin_registry: game_core::plugin::PluginRegistry::new(),
        doc_rooms: crate::ws::new_room_manager(),
        sync_rooms: crate::sync_ws::new_sync_room_manager(),
        cache: crate::cache::CacheLayer::new(),
        rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
        wae: crate::wae::WaeEmitter::new(None, None),
        jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
        embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
        embedding_tx,
        chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
        chat_presence: Mutex::new(std::collections::HashMap::new()),
        geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
        event_bus: crate::events::Bus::new(),
        worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
    })
}

/// Spin up a full axum server bound to 127.0.0.1:0 and return the port.
pub async fn spawn_server(state: AppState) -> u16 {
    let app = build_router(state, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}

/// Connect via WS with Bearer auth header; panics on failure.
pub async fn ws_connect(url: &str, token: &str) -> WsStream {
    // Parse host from the ws:// URL so the Host header is correct.
    let host = url
        .trim_start_matches("ws://")
        .split('/')
        .next()
        .unwrap_or("127.0.0.1");
    let req = TRequest::builder()
        .uri(url)
        .header("Host", host)
        .header("Authorization", format!("Bearer {token}"))
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .body(())
        .unwrap();
    let (ws, _): (WsStream, _) = connect_async(req).await.unwrap();
    ws
}

/// Try a WS connect and return the HTTP status when the server rejects it.
/// Returns `None` if the connection succeeded (shouldn't happen in auth-gate tests).
pub async fn ws_try_connect(port: u16, path: &str, token: Option<&str>) -> Option<u16> {
    let url = format!("ws://127.0.0.1:{port}{path}");
    let host = format!("127.0.0.1:{port}");
    let mut builder = TRequest::builder()
        .uri(&url)
        .header("Host", &host)
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket");
    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {t}"));
    }
    let req = builder.body(()).unwrap();
    match connect_async(req).await {
        Ok((mut ws, _)) => {
            ws.close(None).await.ok();
            None // connected — not what we wanted
        }
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Some(resp.status().as_u16()),
        Err(_) => None,
    }
}
