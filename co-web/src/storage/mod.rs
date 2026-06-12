use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;

use crate::universe_pool::UniversePool;

/// A single event received from a Vercel Log Drain.
pub struct LogDrainEvent {
    pub event_id: String,
    pub universe_id: String,
    pub received_at: String,
    pub source: String,
    pub level: String,
    pub message: String,
    pub host: String,
    pub path: String,
}

// --- Seed content (template universe legal + intro pages) ---
//
// Content lives as plain `.md` files under `co-web/seed/template/` so it can be
// edited as markdown rather than as Rust string literals. Files are embedded at
// compile time via `include_str!`. Frontmatter (slug/title/order/tags) becomes
// the entry metadata; the body becomes the entry body.

const SEED_TEMPLATE_INDEX_MD: &str = include_str!("../../seed/template/index.md");
const SEED_SOBRE_MD: &str = include_str!("../../seed/template/sobre.md");
const SEED_TERMOS_MD: &str = include_str!("../../seed/template/termos.md");
const SEED_PRIVACIDADE_MD: &str = include_str!("../../seed/template/privacidade.md");
const SEED_DADOS_RASTREADOS_MD: &str = include_str!("../../seed/template/dados-rastreados.md");
const SEED_LINHAS_DO_TEMPO_MD: &str = include_str!("../../seed/template/linhas-do-tempo.md");
const SEED_CO_PLATAFORMA_MD: &str = include_str!("../../seed/template/co-plataforma.md");
const SEED_GUIA_MD: &str = include_str!("../../seed/template/guia.md");
const SEED_SEGURANCA_MD: &str = include_str!("../../seed/co/public/seguranca.md");
const SEED_SEGURANCA_DEPS_MD: &str = include_str!("../../seed/co/public/seguranca-dependencias.md");
const SEED_SEGURANCA_CENARIOS_MD: &str = include_str!("../../seed/co/public/seguranca-cenarios.md");
const SEED_SEGURANCA_VAPID_MD: &str = include_str!("../../seed/co/public/seguranca-vapid.md");
const SEED_SEGURANCA_CRIPTO_MD: &str =
    include_str!("../../seed/co/public/seguranca-criptografia.md");
const SEED_LICENSA_MD: &str = include_str!("../../seed/co/public/licensa.md");
const SEED_RENDERERS_MD: &str = include_str!("../../seed/co/public/renderers.md");
const SEED_INFRA_MD: &str = include_str!("../../seed/co/public/infra.md");
const SEED_INFRA_CO_MD: &str = include_str!("../../seed/co/public/infra-co.md");
// Cross-repo infra pages (infra-rfq-gateway, infra-yggdrasil, infra-quilomboaraucaria)
// removed: their content belongs in the respective universe's content, not in CO's
// public seed. Users get those docs by subscribing to the universe.
const SEED_TX_LOG_MD: &str = include_str!("../../seed/co/public/transaction-log.md");
const SEED_CONTA_MD: &str = include_str!("../../seed/co/public/conta-e-mensagens.md");
const SEED_CO_PUBLIC_INDEX_MD: &str = include_str!("../../seed/co/public/index.md");
const SEED_CO_CHANGELOG_MD: &str = include_str!("../../seed/co/CHANGELOG.md");
const SEED_CO_INDEX_MD: &str = include_str!("../../seed/co/index.md");
const SEED_YGGDRASIL_INDEX_MD: &str = include_str!("../../seed/yggdrasil/index.md");

// Timeline universes — three sibling universes (`tempo`, `humanity`, `universo`)
// each backed by a JSON event manifest + a markdown index/front page. Loaded
// at compile time and seeded once on first boot.
const SEED_TIMELINE_TEMPO_JSON: &str = include_str!("../../seed/timeline/tempo.json");
const SEED_TIMELINE_HUMANITY_JSON: &str = include_str!("../../seed/timeline/humanity.json");
const SEED_TIMELINE_UNIVERSO_JSON: &str = include_str!("../../seed/timeline/universo.json");
const SEED_TIMELINE_TEMPO_INDEX_MD: &str = include_str!("../../seed/timeline/tempo-index.md");
const SEED_TIMELINE_HUMANITY_INDEX_MD: &str = include_str!("../../seed/timeline/humanity-index.md");
const SEED_TIMELINE_UNIVERSO_INDEX_MD: &str = include_str!("../../seed/timeline/universo-index.md");

/// CO-170 Phase B: a row pulled from a source universe ready to be re-inserted
/// into a destination universe under a (possibly transformed) path. Replaces
/// the prior 9-tuple that tripped clippy's `type_complexity` lint.
struct MoveRow {
    path: String,
    entry_type: String,
    title: Option<String>,
    frontmatter_json: String,
    payload: String,
    body: String,
    body_hash: String,
    created_at: Option<String>,
    updated_at: Option<String>,
}

pub struct Storage {
    conn: Connection,
    pub universe_pool: Arc<UniversePool>,
    pub data_dir: PathBuf,
}

impl Storage {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        std::fs::create_dir_all(data_dir.as_ref()).expect("Failed to create data directory");

        // CO-77: rename co.db → meta.db on first boot after upgrade (atomic on POSIX)
        let meta_path = data_dir.as_ref().join("meta.db");
        let old_path = data_dir.as_ref().join("co.db");
        if !meta_path.exists() && old_path.exists() {
            std::fs::rename(&old_path, &meta_path).expect("rename co.db -> meta.db");
            tracing::info!("CO-77: renamed co.db -> meta.db");
        }

        let db_path = if meta_path.exists() {
            meta_path
        } else {
            // Fresh install — create meta.db directly
            data_dir.as_ref().join("meta.db")
        };
        let conn = Connection::open(&db_path).expect("Failed to open meta.db");

        // Enable WAL mode for concurrent reads
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .expect("Failed to enable WAL mode");
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("Failed to enable foreign keys");

        let universe_pool = Arc::new(UniversePool::new(data_dir.as_ref(), 1000));

        let mut storage = Self {
            conn,
            universe_pool,
            data_dir: data_dir.as_ref().to_path_buf(),
        };
        storage.run_migrations();
        storage.maybe_migrate_entries_to_universe_dbs();
        storage
    }

    /// Returns the root directory for a universe's .md files.
    pub fn universe_root(&self, universe_key: &str) -> PathBuf {
        self.data_dir.join("universes").join(universe_key)
    }

    /// Access the underlying meta.db connection (for auth, users, universes, quilombo).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Get or open a connection to a universe's data.db.
    pub fn universe_conn(&self, universe_key: &str) -> Arc<std::sync::Mutex<Connection>> {
        self.universe_pool.get_or_open(universe_key)
    }

    /// CO-406: fallible variant of [`universe_conn`] for the request path. If
    /// the universe's pool open fails (disk full, I/O error, corrupt `-shm`),
    /// it returns the [`PoolError`] instead of panicking, so the handler can
    /// degrade to a 503 while every other universe keeps serving.
    ///
    /// [`universe_conn`]: Self::universe_conn
    /// [`PoolError`]: crate::universe_pool::PoolError
    pub fn try_universe_conn(
        &self,
        universe_key: &str,
    ) -> Result<Arc<std::sync::Mutex<Connection>>, crate::universe_pool::PoolError> {
        self.universe_pool.try_get_or_open(universe_key)
    }

    /// CO-406: shared handle to the universe pool, for admin reopen + status.
    pub fn universe_pool(&self) -> &Arc<UniversePool> {
        &self.universe_pool
    }
}

pub mod agent_sessions;
pub(crate) mod api_tokens;
pub mod backup;
pub(crate) mod changelog;
pub(crate) mod chat;
pub(crate) mod clone_ops;
pub(crate) mod dashboard;
pub(crate) mod data_migrate;
pub(crate) mod invitations;
pub(crate) mod log_drain;
pub mod migrations;
pub mod notifications;
pub(crate) mod onboarding;
pub(crate) mod projects;
pub mod push_subscriptions;
pub(crate) mod quilombo_bridge;
pub(crate) mod recompute;
pub(crate) mod release_notes;
pub(crate) mod schema;
pub(crate) mod seed;
pub mod snapshot;
pub(crate) mod subscriptions;
pub(crate) mod tasks;
pub mod tool;
pub(crate) mod universe;
pub mod usage_sessions;
pub(crate) mod users;

pub use agent_sessions::{AgentSession, NewAgentSession};
pub use onboarding::derive_usuario_from_email;
pub use tool::ToolRecord;
pub use usage_sessions::{
    NewUsageSession, UsageDimension, UsageGroup, UsageProjectRow, UsageTotals,
};

pub use invitations::Invitation;

pub use schema::parse_datetime;
pub use schema::seed_data;
