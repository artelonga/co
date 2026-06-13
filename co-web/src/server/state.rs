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
    pub storage: Arc<parking_lot::Mutex<Storage>>,
    pub storage_trait: Arc<dyn crate::infra::storage::Storage>,
    pub config: crate::config::WebConfig,
    /// CO-434: non-secret server config, populated once at boot from `secrets`.
    /// Subsystems read tunables here instead of calling `std::env::var` at the
    /// point of use.
    pub server_config: Arc<crate::CoServerConfig>,
    pub auth_store: Mutex<AuthStore>,
    pub event_bus: crate::events::Bus,
    /// CO-295: runtime secrets provider (env-var in prod, static in tests).
    pub secrets: Arc<dyn crate::infra::secrets::SecretsProvider>,
    /// CO-296: auth provider for JWT issuance and verification.
    pub auth_provider: Arc<dyn crate::infra::auth::AuthProvider>,
    /// CO-294: blob storage backend (local FS by default; R2 via blob-r2 feature).
    pub blob_backend: Arc<crate::infra::blob::BlobBackend>,
    /// CO-328: AI router — dispatches queries to Ollama or Claude Code.
    pub ai_router: Arc<crate::infra::ai::AiRouter>,
    /// CO-380: universal EDA bus — every state-changing route publishes here.
    pub eda_bus: Arc<dyn crate::eda::EdaBus>,
    /// CO-380: LiveTimeline fan-out channel — drives the `/api/v1/events` WebSocket.
    pub timeline_tx: crate::eda::subscribers::timeline::TimelineSender,
    /// CO-426: in-memory registry of active co-auto launchers (TTL). Heartbeats
    /// land here; the Gestão usage panel reads it via `/api/v1/usage/active`.
    pub active_launchers: Arc<crate::observability::ActiveLaunchers>,
    /// CO-431: universo backend seam — handlers open universos through this
    /// factory instead of hardcoding `UniversoLocal::abrir`. Default is the
    /// filesystem factory; alternative backends plug in via
    /// [`with_universo_factory`](CoreState::with_universo_factory).
    pub universo_factory: Arc<dyn co::universo::UniversoFactory>,
}

impl CoreState {
    /// Convenience constructor: wraps `storage` in an `Arc<Mutex>` and wires
    /// up `storage_trait` (SqliteStorage sharing the same inner Arc).
    pub fn from_storage(
        storage: Storage,
        config: crate::config::WebConfig,
        auth_store: AuthStore,
    ) -> Self {
        Self::from_storage_with_secrets(
            storage,
            config,
            auth_store,
            Arc::new(crate::infra::secrets::EnvSecretsProvider),
        )
    }

    /// Like [`from_storage`] but injects a custom `SecretsProvider` — use in
    /// tests to supply a `StaticSecretsProvider` instead of reading env vars.
    pub fn from_storage_with_secrets(
        storage: Storage,
        config: crate::config::WebConfig,
        auth_store: AuthStore,
        secrets: Arc<dyn crate::infra::secrets::SecretsProvider>,
    ) -> Self {
        Self::from_storage_full(
            storage,
            config,
            auth_store,
            secrets,
            crate::infra::blob::BlobBackend::local(),
        )
    }

    /// Full constructor — injects both a `SecretsProvider` and a `BlobBackend`.
    ///
    /// Used at boot time (via `blob_backend_from_config`) when a non-local
    /// backend is configured. Tests and other call-sites that do not care
    /// about blob backend can use [`from_storage_with_secrets`] which
    /// defaults to `BlobBackend::local()`.
    pub fn from_storage_full(
        storage: Storage,
        config: crate::config::WebConfig,
        auth_store: AuthStore,
        secrets: Arc<dyn crate::infra::secrets::SecretsProvider>,
        blob_backend: crate::infra::blob::BlobBackend,
    ) -> Self {
        let storage = Arc::new(parking_lot::Mutex::new(storage));
        let storage_trait: Arc<dyn crate::infra::storage::Storage> = Arc::new(
            crate::infra::storage::SqliteStorage::from_arc(Arc::clone(&storage)),
        );
        let auth_provider: Arc<dyn crate::infra::auth::AuthProvider> = Arc::new(
            crate::infra::auth::LocalJwtProvider::new(Arc::clone(&secrets)),
        );
        // CO-434: build the server config once from the injected provider.
        let server_config = Arc::new(crate::CoServerConfig::from_secrets(&*secrets));
        let ai_router = Arc::new(crate::infra::ai::AiRouter::from_env());
        let eda_bus: Arc<dyn crate::eda::EdaBus> = crate::eda::build_bus(&server_config);
        // Phase 1: create the timeline channel without spawning (safe in non-async contexts).
        // Phase 2 (start) is called from start_server_inner after the runtime is up.
        let timeline_tx = crate::eda::subscribers::timeline::new_channel();
        Self {
            storage,
            storage_trait,
            config,
            server_config,
            auth_store: Mutex::new(auth_store),
            event_bus: crate::events::Bus::new(),
            secrets,
            auth_provider,
            blob_backend: Arc::new(blob_backend),
            ai_router,
            eda_bus,
            timeline_tx,
            active_launchers: Arc::new(crate::observability::ActiveLaunchers::new()),
            universo_factory: Arc::new(crate::content::universo::UniversoLocalFactory),
        }
    }

    /// Replace the universo backend (CO-431). Builder-style so tests and
    /// embedders can plug a non-filesystem `UniversoFactory` without touching
    /// handlers or routes.
    pub fn with_universo_factory(
        mut self,
        factory: Arc<dyn co::universo::UniversoFactory>,
    ) -> Self {
        self.universo_factory = factory;
        self
    }
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
    pub rate_limiter: Mutex<crate::rate_limit::InProcessRateLimiter>,
    pub experiment: Mutex<ExperimentStore>,
    /// CO-292: pluggable worker executor — wraps CO-223's WorkerSupervisor for
    /// periodic workers and adds one-off enqueue/cancel/status operations.
    /// Default impl is InProcessExecutor; future impls (NATS, Temporal) plug in
    /// without touching domain code.
    pub worker_supervisor: std::sync::Arc<dyn crate::infra::workers::WorkerExecutor>,
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

impl axum::extract::FromRef<AppState> for Arc<crate::infra::ai::AiRouter> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.core.ai_router)
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
