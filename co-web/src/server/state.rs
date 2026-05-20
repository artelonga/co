use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::auth::AuthStore;
use crate::error::AppError;
use crate::experiment::ExperimentStore;
use crate::storage::Storage;

// ---------------------------------------------------------------------------
// Sub-state structs
// ---------------------------------------------------------------------------

pub struct CoreState {
    pub storage: parking_lot::Mutex<Storage>,
    pub config: crate::config::WebConfig,
    pub auth_store: Mutex<AuthStore>,
    pub event_bus: crate::events::Bus,
}

pub struct RealtimeState {
    /// CRDT document rooms — keyed by `"slug:doc_path"`.
    pub doc_rooms: crate::ws::DocRoomManager,
    /// CO-151: protobuf SyncDelta rooms — keyed by universe_key.
    pub sync_rooms: crate::sync_ws::SyncRoomManager,
    /// CO-194: per-room broadcast channels for chat WebSocket fan-out.
    pub chat_rooms_broadcast:
        Mutex<HashMap<String, tokio::sync::broadcast::Sender<crate::chat::ChatEvent>>>,
    /// CO-194: per-room presence refcounts (room_id → user_id → connection count).
    pub chat_presence: Mutex<HashMap<String, HashMap<String, u32>>>,
}

pub struct IndexState {
    /// CO-79: in-process LRU caching layer (manifest, theme CSS, query results).
    pub cache: Arc<crate::cache::CacheLayer>,
    /// CO-164: shared embedding model (all-MiniLM-L6-v2).
    pub embeddings: Arc<crate::embedding::EmbeddingService>,
    /// CO-164: channel to send embedding jobs to the background worker.
    pub embedding_tx: crate::embedding_worker::EmbeddingSender,
}

pub struct IntegrationsState {
    pub mail: Arc<dyn co::MailProvider>,
    /// CO-178: in-process MaxMind GeoLite2 database for country+city enrichment.
    pub geo: Arc<crate::geo::GeoDb>,
    pub plugin_registry: game_core::plugin::PluginRegistry,
    pub game_storage: Arc<game_core::storage::Storage>,
    /// CO-118: Workers Analytics Engine emitter (no-op when env vars absent).
    pub wae: Arc<crate::wae::WaeEmitter>,
    /// CO-166: EC P-256 key pair for ES256 JWT signing and JWKS endpoint.
    pub jwt_key: Arc<crate::auth::JwtKey>,
    /// CO-80: token-bucket rate limiter shared across request handlers.
    pub rate_limiter: Mutex<crate::rate_limit::RateLimiter>,
    pub experiment: Mutex<ExperimentStore>,
    /// CO-223: unified worker lifecycle supervisor — tracks last-tick timestamps
    /// and exposes `/api/v1/admin/workers/status`.
    pub worker_supervisor: crate::worker_supervisor::WorkerSupervisor,
}

// ---------------------------------------------------------------------------
// Composite state
// ---------------------------------------------------------------------------

pub struct AppStateInner {
    pub core: Arc<CoreState>,
    pub realtime: Arc<RealtimeState>,
    pub index: Arc<IndexState>,
    pub integrations: Arc<IntegrationsState>,
}

/// Newtype wrapper around `Arc<AppStateInner>`.
///
/// Using a newtype (instead of a bare type alias) lets us implement
/// `axum::extract::FromRef<AppState>` for each sub-state without running
/// into the orphan rule: `AppState` is a *local* type, so the trait impl
/// is always anchored to this crate.
///
/// `AppState` is `Clone + Send + Sync` because the inner `Arc` is.
#[derive(Clone)]
pub struct AppState(pub Arc<AppStateInner>);

impl AppState {
    pub fn new(inner: AppStateInner) -> Self {
        Self(Arc::new(inner))
    }
}

impl std::ops::Deref for AppState {
    type Target = AppStateInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// axum FromRef — narrow sub-state extraction
// ---------------------------------------------------------------------------

impl axum::extract::FromRef<AppState> for Arc<CoreState> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.core)
    }
}

impl axum::extract::FromRef<AppState> for Arc<RealtimeState> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.realtime)
    }
}

impl axum::extract::FromRef<AppState> for Arc<IndexState> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.index)
    }
}

impl axum::extract::FromRef<AppState> for Arc<IntegrationsState> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.integrations)
    }
}

// ---------------------------------------------------------------------------
// Helper functions (unchanged public API)
// ---------------------------------------------------------------------------

pub fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, Storage> {
    state.0.core.storage.lock()
}

pub fn lock_experiment(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, ExperimentStore>, AppError> {
    state
        .0
        .integrations
        .experiment
        .lock()
        .map_err(|_| AppError::Internal("Experiment lock failed".into()))
}

pub fn lock_auth(state: &AppState) -> Result<std::sync::MutexGuard<'_, AuthStore>, AppError> {
    state
        .0
        .core
        .auth_store
        .lock()
        .map_err(|_| AppError::Internal("Auth store lock failed".into()))
}
