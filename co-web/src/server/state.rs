use std::sync::{Arc, Mutex};

use crate::auth::AuthStore;
use crate::error::AppError;
use crate::experiment::ExperimentStore;
use crate::storage::Storage;

pub struct AppStateInner {
    pub storage: parking_lot::Mutex<Storage>,
    pub experiment: Mutex<ExperimentStore>,
    pub config: crate::config::WebConfig,
    pub auth_store: Mutex<AuthStore>,
    pub mail: Arc<dyn co::MailProvider>,
    pub game_storage: Arc<game_core::storage::Storage>,
    pub plugin_registry: game_core::plugin::PluginRegistry,
    /// CRDT document rooms — keyed by `"slug:doc_path"`.
    pub doc_rooms: crate::ws::DocRoomManager,
    /// CO-151: protobuf SyncDelta rooms — keyed by universe_key.
    pub sync_rooms: crate::sync_ws::SyncRoomManager,
    /// CO-79: in-process LRU caching layer (manifest, theme CSS, query results).
    pub cache: std::sync::Arc<crate::cache::CacheLayer>,
    /// CO-80: token-bucket rate limiter shared across request handlers.
    pub rate_limiter: Mutex<crate::rate_limit::RateLimiter>,
    /// CO-118: Workers Analytics Engine emitter (no-op when env vars absent).
    pub wae: Arc<crate::wae::WaeEmitter>,
    /// CO-166: EC P-256 key pair for ES256 JWT signing and JWKS endpoint.
    pub jwt_key: Arc<crate::auth::JwtKey>,
    /// CO-164: shared embedding model (all-MiniLM-L6-v2).
    pub embeddings: Arc<crate::embedding::EmbeddingService>,
    /// CO-164: channel to send embedding jobs to the background worker.
    pub embedding_tx: crate::embedding_worker::EmbeddingSender,
    /// CO-194: per-room broadcast channels for chat WebSocket fan-out.
    pub chat_rooms_broadcast: std::sync::Mutex<
        std::collections::HashMap<
            String,
            tokio::sync::broadcast::Sender<crate::chat_ws::ChatEvent>,
        >,
    >,
    /// CO-194: per-room presence refcounts (room_id → user_id → connection count).
    pub chat_presence:
        std::sync::Mutex<std::collections::HashMap<String, std::collections::HashMap<String, u32>>>,
    /// CO-178: in-process MaxMind GeoLite2 database for country+city enrichment.
    pub geo: std::sync::Arc<crate::geo::GeoDb>,
    /// CO-220: in-process domain event bus for decoupled cross-feature signaling.
    pub event_bus: crate::events::Bus,
    /// CO-223: unified worker lifecycle supervisor — tracks last-tick timestamps
    /// and exposes `/api/v1/admin/workers/status`.
    pub worker_supervisor: crate::worker_supervisor::WorkerSupervisor,
}

pub type AppState = Arc<AppStateInner>;

pub fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, Storage> {
    state.storage.lock()
}

pub fn lock_experiment(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, ExperimentStore>, AppError> {
    state
        .experiment
        .lock()
        .map_err(|_| AppError::Internal("Experiment lock failed".into()))
}

pub fn lock_auth(state: &AppState) -> Result<std::sync::MutexGuard<'_, AuthStore>, AppError> {
    state
        .auth_store
        .lock()
        .map_err(|_| AppError::Internal("Auth store lock failed".into()))
}
