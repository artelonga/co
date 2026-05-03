use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::entry_index::{EntryRow, make_entry};
use crate::models::*;
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

const SEED_TEMPLATE_INDEX_MD: &str = include_str!("../seed/template/index.md");
const SEED_SOBRE_MD: &str = include_str!("../seed/template/sobre.md");
const SEED_TERMOS_MD: &str = include_str!("../seed/template/termos.md");
const SEED_PRIVACIDADE_MD: &str = include_str!("../seed/template/privacidade.md");
const SEED_DADOS_RASTREADOS_MD: &str = include_str!("../seed/template/dados-rastreados.md");
const SEED_LINHAS_DO_TEMPO_MD: &str = include_str!("../seed/template/linhas-do-tempo.md");

// Timeline universes — three sibling universes (`tempo`, `humanity`, `universo`)
// each backed by a JSON event manifest + a markdown index/front page. Loaded
// at compile time and seeded once on first boot.
const SEED_TIMELINE_TEMPO_JSON: &str = include_str!("../seed/timeline/tempo.json");
const SEED_TIMELINE_HUMANITY_JSON: &str = include_str!("../seed/timeline/humanity.json");
const SEED_TIMELINE_UNIVERSO_JSON: &str = include_str!("../seed/timeline/universo.json");
const SEED_TIMELINE_TEMPO_INDEX_MD: &str = include_str!("../seed/timeline/tempo-index.md");
const SEED_TIMELINE_HUMANITY_INDEX_MD: &str = include_str!("../seed/timeline/humanity-index.md");
const SEED_TIMELINE_UNIVERSO_INDEX_MD: &str = include_str!("../seed/timeline/universo-index.md");

/// Split a `.md` file with YAML frontmatter into `(frontmatter_yaml, body)`.
/// If no frontmatter is present, returns `("", whole_input)`.
fn split_frontmatter(md: &str) -> (&str, &str) {
    let s = md
        .strip_prefix("---\n")
        .or_else(|| md.strip_prefix("---\r\n"));
    let Some(rest) = s else { return ("", md) };
    if let Some(end) = rest.find("\n---\n") {
        return (&rest[..end], rest[end + 5..].trim_start_matches('\n'));
    }
    if let Some(end) = rest.find("\r\n---\r\n") {
        return (
            &rest[..end],
            rest[end + 7..].trim_start_matches(['\n', '\r']),
        );
    }
    ("", md)
}

/// Convert the YAML frontmatter of a seed page into a `serde_json::Value` and
/// stamp `created`/`modified` to the supplied timestamp (so seeds always show
/// "now" rather than the file's original creation date).
fn seed_page_frontmatter(md: &str, now_str: &str) -> serde_json::Value {
    let (fm_yaml, _) = split_frontmatter(md);
    let mut fm: serde_json::Value = serde_yaml::from_str::<serde_yaml::Value>(fm_yaml)
        .ok()
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = fm.as_object_mut() {
        obj.insert("created".into(), json!(now_str));
        obj.insert("modified".into(), json!(now_str));
    }
    fm
}

fn seed_page_body(md: &str) -> &str {
    let (_, body) = split_frontmatter(md);
    body
}

/// Recursively collect file paths under `dir`. Returns absolute PathBufs in
/// dir-order. Caller filters by extension. No symlink-following; ignores
/// errors (skipped silently).
fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read.flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(walkdir(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// Idempotent ALTER TABLE ADD COLUMN: checks `pragma_table_info` before issuing
/// the DDL so repeated calls (and partially-applied migrations) are safe.
/// Returns `true` if the column was added, `false` if it already existed.
/// CO-137: replaces bare `ALTER TABLE … ADD COLUMN` in migrations v17–v22 to
/// prevent "duplicate column name" panics on re-run after partial application.
fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_def: &str,
) -> rusqlite::Result<bool> {
    let exists: bool = conn
        .query_row(
            &format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"),
            params![column],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {column_def};"
        ))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Idempotent CREATE TABLE: queries `sqlite_master` before issuing the DDL.
/// Returns `true` if the table was created, `false` if it already existed.
///
/// Sibling of `ensure_column`. Surfaced after the third partial-apply incident
/// (CO-77 entries, CO-137 parent_key, CO-121 feature_flags). The standalone
/// `CREATE TABLE IF NOT EXISTS` SQL is already idempotent, so this helper
/// exists primarily to give callers a single, consistent surface for migrations
/// and to make it trivial to add observability (e.g. tracing) at the call site.
fn ensure_table(conn: &Connection, name: &str, body_sql: &str) -> rusqlite::Result<bool> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !exists {
        conn.execute_batch(body_sql)?;
        Ok(true)
    } else {
        Ok(false)
    }
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

    fn run_migrations(&mut self) {
        // Create schema_version table
        self.conn
            .execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
            .expect("Failed to create schema_version table");

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 1 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS projects (
                    key TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    next_id INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS tasks (
                    project_key TEXT NOT NULL,
                    id INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT 'todo',
                    priority TEXT NOT NULL DEFAULT 'medium',
                    due_date TEXT,
                    parent INTEGER,
                    labels TEXT NOT NULL DEFAULT '[]',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (project_key, id),
                    FOREIGN KEY (project_key) REFERENCES projects(key)
                );

                CREATE TABLE IF NOT EXISTS comments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_key TEXT NOT NULL,
                    task_id INTEGER NOT NULL,
                    author TEXT NOT NULL DEFAULT 'Anonymous',
                    body TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (project_key, task_id) REFERENCES tasks(project_key, id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS activity_log (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_key TEXT NOT NULL,
                    task_id INTEGER,
                    action TEXT NOT NULL,
                    field TEXT,
                    old_value TEXT,
                    new_value TEXT,
                    actor TEXT NOT NULL DEFAULT 'system',
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (project_key) REFERENCES projects(key)
                );

                INSERT INTO schema_version (version) VALUES (1);
                ",
                )
                .expect("Failed to run migration v1");
        }

        if current_version < 2 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS users (
                    id TEXT PRIMARY KEY,
                    email TEXT UNIQUE NOT NULL,
                    display_name TEXT NOT NULL DEFAULT '',
                    tier TEXT NOT NULL DEFAULT 'player',
                    created_at TEXT NOT NULL
                );
                INSERT INTO schema_version (version) VALUES (2);
                ",
                )
                .expect("Failed to run migration v2");
        }

        // Quilombo community tables (v3, v4)
        crate::quilombo_storage::run_quilombo_migrations(&self.conn);

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 5 {
            self.conn
                .execute_batch(
                    "ALTER TABLE tasks ADD COLUMN assignee TEXT;
                     INSERT INTO schema_version (version) VALUES (5);",
                )
                .expect("Failed to run migration v5");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 6 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS universes (
                    key TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    owner_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (owner_id) REFERENCES users(id)
                );
                INSERT INTO schema_version (version) VALUES (6);
                ",
                )
                .expect("Failed to run migration v6");
        }

        if current_version < 7 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS universe_members (
                    universe_key TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    role TEXT NOT NULL DEFAULT 'member',
                    joined_at TEXT NOT NULL,
                    PRIMARY KEY (universe_key, user_id),
                    FOREIGN KEY (universe_key) REFERENCES universes(key) ON DELETE CASCADE,
                    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
                );
                INSERT INTO schema_version (version) VALUES (7);
                ",
                )
                .expect("Failed to run migration v7");
        }

        if current_version < 8 {
            self.conn
                .execute_batch(
                    "ALTER TABLE projects ADD COLUMN universe_key TEXT REFERENCES universes(key);
                     INSERT INTO schema_version (version) VALUES (8);",
                )
                .expect("Failed to run migration v8");
        }

        if current_version < 9 {
            // Recreate universe_members without the FK on user_id so that
            // quilombo users (stored in quilombo_usuarios, not users) can be members.
            self.conn
                .execute_batch(
                    "
                CREATE TABLE universe_members_new (
                    universe_key TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    role TEXT NOT NULL DEFAULT 'member',
                    joined_at TEXT NOT NULL,
                    PRIMARY KEY (universe_key, user_id),
                    FOREIGN KEY (universe_key) REFERENCES universes(key) ON DELETE CASCADE
                );
                INSERT INTO universe_members_new SELECT * FROM universe_members;
                DROP TABLE universe_members;
                ALTER TABLE universe_members_new RENAME TO universe_members;
                INSERT INTO schema_version (version) VALUES (9);
                ",
                )
                .expect("Failed to run migration v9");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 10 {
            // Rebuild universes without FK on owner_id (support anonymous/system owners)
            // and add is_template + is_public columns.
            self.conn
                .execute_batch(
                    "
                PRAGMA foreign_keys = OFF;
                DROP TABLE IF EXISTS universes_new;
                CREATE TABLE universes_new (
                    key TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    owner_id TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL,
                    is_template INTEGER NOT NULL DEFAULT 0,
                    is_public INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO universes_new (key, name, description, owner_id, created_at, is_template, is_public)
                    SELECT key, name, description, owner_id, created_at, 0, 0 FROM universes;
                DROP TABLE universes;
                ALTER TABLE universes_new RENAME TO universes;
                PRAGMA foreign_keys = ON;
                INSERT INTO schema_version (version) VALUES (10);
                ",
                )
                .expect("Failed to run migration v10");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 11 {
            self.conn
                .execute_batch(
                    "ALTER TABLE universes ADD COLUMN content_count INTEGER NOT NULL DEFAULT 0;
                     INSERT INTO schema_version (version) VALUES (11);",
                )
                .expect("Failed to run migration v11");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 12 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS entries (
                    path TEXT NOT NULL,
                    universe_key TEXT NOT NULL,
                    entry_type TEXT NOT NULL,
                    title TEXT,
                    frontmatter_json TEXT NOT NULL,
                    body TEXT NOT NULL DEFAULT '',
                    body_hash TEXT NOT NULL,
                    created_at TEXT,
                    updated_at TEXT,
                    PRIMARY KEY (universe_key, path)
                );
                CREATE INDEX IF NOT EXISTS idx_entries_type ON entries(universe_key, entry_type);
                CREATE INDEX IF NOT EXISTS idx_entries_updated ON entries(universe_key, updated_at);
                CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
                    universe_key UNINDEXED,
                    path UNINDEXED,
                    title,
                    body
                );
                INSERT INTO schema_version (version) VALUES (12);
                ",
                )
                .expect("Failed to run migration v12");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 13 {
            // Create default universe
            let _ = self.conn.execute_batch(
                "INSERT OR IGNORE INTO universes (key, name, description, owner_id, created_at, is_template, is_public, content_count) \
                 VALUES ('default', 'Default', 'Default universe', 'system', datetime('now'), 0, 0, 0);",
            );

            // Check if projects table exists
            let projects_exist: bool = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='projects'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;

            if projects_exist {
                self.migrate_old_data_to_entries();
            }

            // Drop old tables (disable FK checks to avoid constraint errors on drop order)
            self.conn
                .execute_batch(
                    "PRAGMA foreign_keys = OFF; \
                     DROP TABLE IF EXISTS comments; \
                     DROP TABLE IF EXISTS activity_log; \
                     DROP TABLE IF EXISTS tasks; \
                     DROP TABLE IF EXISTS projects; \
                     PRAGMA foreign_keys = ON; \
                     INSERT INTO schema_version (version) VALUES (13);",
                )
                .expect("Failed migration v13");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 14 {
            // Add form config columns to universes (presentation layer — CO-24).
            // SQLite ALTER TABLE ADD COLUMN requires a default for NOT NULL columns.
            self.conn
                .execute_batch(
                    "ALTER TABLE universes ADD COLUMN theme_preset TEXT NOT NULL DEFAULT 'scholarly-light';
                     ALTER TABLE universes ADD COLUMN layout TEXT NOT NULL DEFAULT 'board';
                     ALTER TABLE universes ADD COLUMN font_headline TEXT;
                     ALTER TABLE universes ADD COLUMN font_body TEXT;
                     ALTER TABLE universes ADD COLUMN custom_tokens TEXT;
                     UPDATE universes SET theme_preset = 'scholarly-light', layout = 'board' WHERE is_template = 1;
                     INSERT INTO schema_version (version) VALUES (14);",
                )
                .expect("Failed to run migration v14");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 15 {
            // API tokens for vault REST API / Obsidian plugin auth (CO-35).
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS api_tokens (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    name TEXT NOT NULL DEFAULT '',
                    token TEXT UNIQUE NOT NULL,
                    created_at TEXT NOT NULL,
                    expires_at TEXT NOT NULL,
                    last_used_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
                CREATE INDEX IF NOT EXISTS idx_api_tokens_token ON api_tokens(token);
                INSERT INTO schema_version (version) VALUES (15);
                ",
                )
                .expect("Failed to run migration v15");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 16 {
            // CO-46: telemetry_events table — privacy-respecting event tracking.
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS telemetry_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    timestamp TEXT NOT NULL,
                    visitor_token TEXT,
                    user_id TEXT,
                    session_id TEXT,
                    event_type TEXT NOT NULL,
                    event_name TEXT NOT NULL,
                    universe_key TEXT,
                    path TEXT,
                    properties TEXT,
                    duration_ms INTEGER,
                    ip_hash TEXT,
                    ua_device TEXT,
                    ua_browser TEXT,
                    ua_os TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_telemetry_time ON telemetry_events(timestamp);
                CREATE INDEX IF NOT EXISTS idx_telemetry_event ON telemetry_events(event_type, event_name);
                CREATE INDEX IF NOT EXISTS idx_telemetry_user ON telemetry_events(user_id);
                INSERT INTO schema_version (version) VALUES (16);
                ",
                )
                .expect("Failed to run migration v16");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 17 {
            // CO-44: password_hash column for UAT user (password-based login).
            ensure_column(&self.conn, "users", "password_hash", "TEXT")
                .expect("migration v17: password_hash");
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (17)",
                    [],
                )
                .expect("migration v17: version insert");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 18 {
            // CO-38: requires_login flag on universes — gates Yggdrasil and future
            // login-only universes from anonymous access.
            ensure_column(
                &self.conn,
                "universes",
                "requires_login",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v18: requires_login");
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (18)",
                    [],
                )
                .expect("migration v18: version insert");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 19 {
            // CO-45: uat_mutations table — records every write op on UAT for change promotion.
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS uat_mutations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    timestamp TEXT NOT NULL,
                    user_id TEXT,
                    action TEXT NOT NULL,
                    target TEXT NOT NULL,
                    before_value TEXT,
                    after_value TEXT,
                    metadata TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_uat_mutations_ts ON uat_mutations(timestamp);
                CREATE INDEX IF NOT EXISTS idx_uat_mutations_action ON uat_mutations(action);
                INSERT INTO schema_version (version) VALUES (19);
                ",
                )
                .expect("Failed to run migration v19");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 20 {
            // CO-49: deterministic access model.
            // Add visibility column + populate from existing flags.
            // Add subscriptions table for public-subscribable universes.
            ensure_column(
                &self.conn,
                "universes",
                "visibility",
                "TEXT NOT NULL DEFAULT 'private'",
            )
            .expect("migration v20: visibility");
            self.conn
                .execute_batch(
                    "
                UPDATE universes SET visibility = 'template' WHERE is_template = 1;
                UPDATE universes SET visibility = 'requires_login'
                    WHERE is_template = 0 AND requires_login = 1;
                UPDATE universes SET visibility = 'public-subscribable'
                    WHERE is_template = 0 AND requires_login = 0 AND is_public = 1;

                CREATE TABLE IF NOT EXISTS subscriptions (
                    user_id TEXT NOT NULL,
                    universe_key TEXT NOT NULL,
                    subscribed_at TEXT NOT NULL,
                    PRIMARY KEY (user_id, universe_key),
                    FOREIGN KEY (universe_key) REFERENCES universes(key) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_subscriptions_user ON subscriptions(user_id);
                CREATE INDEX IF NOT EXISTS idx_subscriptions_universe ON subscriptions(universe_key);
                ",
                )
                .expect("migration v20: subscriptions + UPDATE");
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (20)",
                    [],
                )
                .expect("migration v20: version insert");
        }

        if current_version < 21 {
            // CO-50: git-backed universes — each universe may be linked to a git repo.
            for (col, def) in [
                ("git_repo", "TEXT"),
                ("git_path", "TEXT"),
                ("git_branch", "TEXT NOT NULL DEFAULT 'main'"),
                ("git_commit_hash", "TEXT"),
                ("git_synced_at", "TEXT"),
                ("git_sync_error", "TEXT"),
            ] {
                ensure_column(&self.conn, "universes", col, def)
                    .expect("migration v21: git columns");
            }
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (21)",
                    [],
                )
                .expect("migration v21: version insert");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 22 {
            // CO-98: hierarchical universes — parent_key links a universe to a parent
            // (NULL = top-level). No FK so deletes don't cascade; orphaned children
            // surface as top-level in the sidebar (acceptable degradation).
            ensure_column(&self.conn, "universes", "parent_key", "TEXT")
                .expect("migration v22: parent_key");
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_universes_parent_key ON universes(parent_key);",
                )
                .expect("migration v22: parent_key index");
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (22)",
                    [],
                )
                .expect("migration v22: version insert");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 23 {
            // CO-64: drop git-sync columns — dead since Vault API replaced git-clone-on-server
            for col in [
                "git_repo",
                "git_path",
                "git_branch",
                "git_commit_hash",
                "git_synced_at",
                "git_sync_error",
            ] {
                // DROP INDEX first (SQLite requires this before dropping an indexed column)
                let _ = self
                    .conn
                    .execute_batch(&format!("DROP INDEX IF EXISTS idx_universes_{col};"));
                let _ = self
                    .conn
                    .execute_batch(&format!("ALTER TABLE universes DROP COLUMN {col};"));
            }
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (23)",
                    [],
                )
                .expect("migration v23: version insert");
        }

        // CO-137 unconditional backfill: ensure parent_key exists even when
        // schema_version already shows 22 but the ALTER TABLE never ran (the
        // exact prod failure mode this ticket investigates). ensure_column is a
        // no-op if the column is already present, so this is always safe.
        ensure_column(&self.conn, "universes", "parent_key", "TEXT")
            .expect("CO-137 backfill: parent_key column");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_universes_parent_key ON universes(parent_key);",
            )
            .expect("CO-137 backfill: parent_key index");

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 24 {
            // CO-71: add generic JSON payload column (validated at write, indexed by manifest)
            // and per-universe manifest_version for migration tracking.
            ensure_column(
                &self.conn,
                "entries",
                "payload",
                "TEXT NOT NULL DEFAULT '{}'",
            )
            .expect("migration v24: entries.payload column");

            // Backfill payload from frontmatter_json for existing rows.
            self.conn
                .execute_batch(
                    "UPDATE entries SET payload = frontmatter_json WHERE payload = '{}';",
                )
                .expect("migration v24: payload backfill");

            ensure_column(
                &self.conn,
                "universes",
                "manifest_version",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v24: universes.manifest_version column");

            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (24)",
                    [],
                )
                .expect("migration v24: version insert");
        }

        // CO-71 unconditional backfill.
        ensure_column(
            &self.conn,
            "entries",
            "payload",
            "TEXT NOT NULL DEFAULT '{}'",
        )
        .expect("CO-71 backfill: payload column");
        ensure_column(
            &self.conn,
            "universes",
            "manifest_version",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .expect("CO-71 backfill: manifest_version column");

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 25 {
            // CO-72: job queue table + universe doc-gen error columns.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS jobs (
                         id           TEXT PRIMARY KEY,
                         universe_key TEXT NOT NULL,
                         kind         TEXT NOT NULL,
                         payload      TEXT NOT NULL,
                         status       TEXT NOT NULL DEFAULT 'pending',
                         attempts     INTEGER NOT NULL DEFAULT 0,
                         dedupe_key   TEXT,
                         created_at   TEXT NOT NULL,
                         run_at       TEXT NOT NULL,
                         started_at   TEXT,
                         completed_at TEXT,
                         error        TEXT
                     );
                     CREATE INDEX IF NOT EXISTS idx_jobs_status_run_at
                         ON jobs(status, run_at);
                     CREATE INDEX IF NOT EXISTS idx_jobs_universe_key
                         ON jobs(universe_key);
                     CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_dedupe
                         ON jobs(dedupe_key) WHERE dedupe_key IS NOT NULL;",
                )
                .expect("migration v25: jobs table");
            ensure_column(&self.conn, "universes", "doc_gen_error", "TEXT")
                .expect("migration v25: doc_gen_error");
            ensure_column(&self.conn, "universes", "doc_gen_error_at", "TEXT")
                .expect("migration v25: doc_gen_error_at");
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (25)",
                    [],
                )
                .expect("migration v25: version insert");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 26 {
            // CO-77: project→universe routing index for legacy routes.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS project_universe_index (
                         project_key  TEXT PRIMARY KEY,
                         universe_key TEXT NOT NULL
                     );",
                )
                .expect("migration v26: project_universe_index");
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (26)",
                    [],
                )
                .expect("migration v26: version insert");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 27 {
            // CO-121: A/B testing primitives — feature_flags, ab_assignments, ab_exposures.
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS feature_flags (
                    flag_key    TEXT PRIMARY KEY,
                    description TEXT NOT NULL,
                    variants    TEXT NOT NULL,
                    salt        TEXT NOT NULL,
                    enabled     INTEGER NOT NULL DEFAULT 0,
                    created_at  TEXT NOT NULL,
                    updated_at  TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS ab_assignments (
                    user_id     TEXT NOT NULL,
                    flag_key    TEXT NOT NULL,
                    variant     TEXT NOT NULL,
                    assigned_at TEXT NOT NULL,
                    PRIMARY KEY (user_id, flag_key)
                );

                CREATE TABLE IF NOT EXISTS ab_exposures (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    user_id     TEXT NOT NULL,
                    flag_key    TEXT NOT NULL,
                    variant     TEXT NOT NULL,
                    universe_id TEXT,
                    exposed_at  TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_exposures_flag_time
                    ON ab_exposures(flag_key, exposed_at);

                INSERT INTO schema_version (version) VALUES (27);
                ",
                )
                .expect("Failed to run migration v27");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 28 {
            // CO-124: Vercel Log Drain — per-universe drain secret + incoming event store.
            ensure_column(
                &self.conn,
                "universes",
                "log_drain_secret",
                "TEXT NOT NULL DEFAULT ''",
            )
            .expect("migration v28: universes.log_drain_secret column");

            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS log_drain_events (
                    event_id     TEXT PRIMARY KEY,
                    universe_id  TEXT NOT NULL,
                    received_at  TEXT NOT NULL,
                    source       TEXT NOT NULL DEFAULT '',
                    level        TEXT NOT NULL DEFAULT 'info',
                    message      TEXT NOT NULL DEFAULT '',
                    host         TEXT NOT NULL DEFAULT '',
                    path         TEXT NOT NULL DEFAULT ''
                );
                CREATE INDEX IF NOT EXISTS idx_log_drain_events_universe
                    ON log_drain_events(universe_id, received_at);

                INSERT OR IGNORE INTO schema_version (version) VALUES (28);
                ",
                )
                .expect("Failed to run migration v28");
        }

        // CO-77 unconditional backfill: entries + entries_fts on meta.db for
        // the startup migration to per-universe DBs. Uses ensure_table so the
        // call site is consistent with the migration-drift class fixes.
        ensure_table(
            &self.conn,
            "entries",
            "CREATE TABLE entries (
                path              TEXT NOT NULL,
                universe_key      TEXT NOT NULL,
                entry_type        TEXT NOT NULL,
                title             TEXT,
                frontmatter_json  TEXT NOT NULL DEFAULT '{}',
                body              TEXT NOT NULL DEFAULT '',
                body_hash         TEXT NOT NULL DEFAULT '',
                created_at        TEXT,
                updated_at        TEXT,
                PRIMARY KEY (universe_key, path)
            );",
        )
        .ok();
        ensure_table(
            &self.conn,
            "entries_fts",
            "CREATE TABLE entries_fts (content TEXT);",
        )
        .ok();

        // CO-121 unconditional backfill: ensure A/B testing tables exist on
        // meta.db regardless of schema_version state. Surfaced on prod
        // (1.33.0, 2026-05-01) as `no such table: feature_flags` on every
        // boot — same partial-apply failure mode CO-137 documented.
        ensure_table(
            &self.conn,
            "feature_flags",
            "CREATE TABLE feature_flags (
                flag_key    TEXT PRIMARY KEY,
                description TEXT NOT NULL,
                variants    TEXT NOT NULL,
                salt        TEXT NOT NULL,
                enabled     INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );",
        )
        .expect("CO-121 backfill: feature_flags");
        ensure_table(
            &self.conn,
            "ab_assignments",
            "CREATE TABLE ab_assignments (
                user_id     TEXT NOT NULL,
                flag_key    TEXT NOT NULL,
                variant     TEXT NOT NULL,
                assigned_at TEXT NOT NULL,
                PRIMARY KEY (user_id, flag_key)
            );",
        )
        .expect("CO-121 backfill: ab_assignments");
        ensure_table(
            &self.conn,
            "ab_exposures",
            "CREATE TABLE ab_exposures (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id     TEXT NOT NULL,
                flag_key    TEXT NOT NULL,
                variant     TEXT NOT NULL,
                universe_id TEXT,
                exposed_at  TEXT NOT NULL
            );",
        )
        .expect("CO-121 backfill: ab_exposures");
        // Index on ab_exposures (idempotent via IF NOT EXISTS — no helper
        // needed since indexes don't carry a sqlite_master 'table' type).
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_exposures_flag_time \
                 ON ab_exposures(flag_key, exposed_at);",
            )
            .expect("CO-121 backfill: idx_exposures_flag_time");

        // CO-144 Phase C: per-universe semver content version. Bumped by the
        // alterar-pagina-na-web process sink step. Default '0.0.0' for any
        // pre-existing universe; processes apply patch/minor/major bumps.
        ensure_column(
            &self.conn,
            "universes",
            "content_version",
            "TEXT NOT NULL DEFAULT '0.0.0'",
        )
        .expect("CO-144 backfill: universes.content_version column");

        // CO-144 Phase C: process_runs — every preview/approval/revert run of
        // a process (e.g. alterar-pagina-na-web) gets a row. State machine:
        //   preview → approved → completed
        //   preview → rejected (terminal)
        //   completed → reverted (a different run with type=revert pointing back)
        ensure_table(
            &self.conn,
            "process_runs",
            "CREATE TABLE process_runs (
                run_id        TEXT PRIMARY KEY,
                process_name  TEXT NOT NULL,
                universe_key  TEXT NOT NULL,
                state         TEXT NOT NULL,
                payload       TEXT NOT NULL,
                created_at    TEXT NOT NULL,
                completed_at  TEXT,
                actor_id      TEXT,
                parent_run_id TEXT
            );",
        )
        .expect("CO-144 backfill: process_runs");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_process_runs_universe_time \
                 ON process_runs(universe_key, created_at DESC);",
            )
            .expect("CO-144 backfill: idx_process_runs_universe_time");
    }

    /// Migrate data from old projects/tasks/comments tables into the entries table + .md files.
    fn migrate_old_data_to_entries(&mut self) {
        // Collect projects
        struct OldProject {
            key: String,
            name: String,
            description: String,
            next_id: i64,
            created_at: String,
            archived: i64,
            universe_key: Option<String>,
        }

        let old_projects: Vec<OldProject> = {
            let mut stmt = match self.conn.prepare(
                "SELECT key, name, description, next_id, created_at, archived, universe_key FROM projects",
            ) {
                Ok(s) => s,
                Err(_) => return,
            };
            match stmt.query_map([], |row| {
                Ok(OldProject {
                    key: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    next_id: row.get::<_, i64>(3)?,
                    created_at: row.get::<_, String>(4)?,
                    archived: row.get::<_, i64>(5)?,
                    universe_key: row.get::<_, Option<String>>(6)?,
                })
            }) {
                Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                Err(_) => return,
            }
        };

        for proj in &old_projects {
            let universe_key = proj
                .universe_key
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let universe_root = self.data_dir.join("universes").join(&universe_key);
            let path = format!("projects/{}/_project.md", proj.key);
            let fm = json!({
                "type": "project",
                "key": proj.key,
                "title": proj.name,
                "status": "active",
                "next_id": proj.next_id,
                "created": proj.created_at,
                "modified": proj.created_at,
                "archived": proj.archived != 0,
                "tags": []
            });
            let entry = make_entry(&path, fm.clone(), &proj.description);
            let _ = co::entry::write_entry(&universe_root, &entry);
            let _ = upsert_entry_row(&self.conn, &universe_key, &entry);

            // Collect tasks for this project
            struct OldTask {
                id: i64,
                title: String,
                description: String,
                status: String,
                priority: String,
                due_date: Option<String>,
                parent: Option<i64>,
                labels: String,
                created_at: String,
                updated_at: String,
                archived: i64,
                assignee: Option<String>,
            }

            let old_tasks: Vec<OldTask> = {
                let mut stmt = match self.conn.prepare(
                    "SELECT id, title, description, status, priority, due_date, parent, labels, \
                     created_at, updated_at, archived, assignee FROM tasks WHERE project_key = ?1",
                ) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                match stmt.query_map(params![proj.key], |row| {
                    Ok(OldTask {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        description: row.get(2)?,
                        status: row.get(3)?,
                        priority: row.get(4)?,
                        due_date: row.get(5)?,
                        parent: row.get(6)?,
                        labels: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        archived: row.get::<_, i64>(10)?,
                        assignee: row.get(11)?,
                    })
                }) {
                    Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                    Err(_) => continue,
                }
            };

            for task in &old_tasks {
                let task_path = format!("projects/{}/{}.md", proj.key, task.id);
                let labels: Vec<String> = serde_json::from_str(&task.labels).unwrap_or_default();
                let task_fm = json!({
                    "type": "task",
                    "id": task.id,
                    "title": task.title,
                    "status": task.status,
                    "priority": task.priority,
                    "due": task.due_date,
                    "parent": task.parent,
                    "tags": labels,
                    "created": task.created_at,
                    "modified": task.updated_at,
                    "archived": task.archived != 0,
                    "assignee": task.assignee,
                    "project": proj.key
                });
                let task_entry = make_entry(&task_path, task_fm, &task.description);
                let _ = co::entry::write_entry(&universe_root, &task_entry);
                let _ = upsert_entry_row(&self.conn, &universe_key, &task_entry);

                // Collect comments for this task
                struct OldComment {
                    id: i64,
                    author: String,
                    body: String,
                    created_at: String,
                }

                let old_comments: Vec<OldComment> = {
                    let mut stmt = match self.conn.prepare(
                        "SELECT id, author, body, created_at FROM comments \
                         WHERE project_key = ?1 AND task_id = ?2",
                    ) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    match stmt.query_map(params![proj.key, task.id], |row| {
                        Ok(OldComment {
                            id: row.get(0)?,
                            author: row.get(1)?,
                            body: row.get(2)?,
                            created_at: row.get(3)?,
                        })
                    }) {
                        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                        Err(_) => continue,
                    }
                };

                for comment in &old_comments {
                    let comment_path = format!(
                        "projects/{}/comments/{}-{}.md",
                        proj.key, task.id, comment.id
                    );
                    let comment_fm = json!({
                        "type": "comment",
                        "id": comment.id,
                        "task": task.id,
                        "project": proj.key,
                        "author": comment.author,
                        "created": comment.created_at,
                        "modified": comment.created_at,
                        "tags": []
                    });
                    let comment_entry = make_entry(&comment_path, comment_fm, &comment.body);
                    let _ = co::entry::write_entry(&universe_root, &comment_entry);
                    let _ = upsert_entry_row(&self.conn, &universe_key, &comment_entry);
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // CO-77: startup migration — move entries from meta.db to per-universe DBs
    // -------------------------------------------------------------------------

    /// On first boot after CO-77, migrate all rows in `entries` (meta.db) to
    /// the appropriate per-universe `data.db`. This is a one-shot migration:
    /// once `entries` in meta.db is empty it never runs again.
    fn maybe_migrate_entries_to_universe_dbs(&self) {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
            .unwrap_or(0);
        if count == 0 {
            return;
        }

        tracing::info!(
            "CO-77: migrating {} entries from meta.db to per-universe DBs",
            count
        );

        // Collect distinct universe keys
        let universe_keys: Vec<String> = {
            let mut stmt = match self
                .conn
                .prepare("SELECT DISTINCT universe_key FROM entries")
            {
                Ok(s) => s,
                Err(_) => return,
            };
            stmt.query_map([], |r| r.get(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };

        for uk in &universe_keys {
            let uc = self.universe_pool.get_or_open(uk);
            let uc_guard = uc.lock().expect("universe conn lock");

            type EntryRow9 = (
                String,
                String,
                String,
                Option<String>,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
            );
            // Collect rows for this universe
            let rows: Vec<EntryRow9> = {
                let mut stmt = match self.conn.prepare(
                    "SELECT path, universe_key, entry_type, title, \
                     frontmatter_json, body, body_hash, created_at, updated_at \
                     FROM entries WHERE universe_key = ?1",
                ) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                stmt.query_map(params![uk], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                })
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
            };

            for (path, uk2, entry_type, title, fm_json, body, body_hash, created_at, updated_at) in
                &rows
            {
                let _ = uc_guard.execute(
                    "INSERT OR IGNORE INTO entries \
                     (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        path, uk2, entry_type, title, fm_json, body, body_hash,
                        created_at, updated_at
                    ],
                );
                // Populate project_universe_index for project entries
                if entry_type == "project"
                    && let Ok(fm) = serde_json::from_str::<serde_json::Value>(fm_json)
                    && let Some(proj_key) = fm.get("key").and_then(|v| v.as_str())
                {
                    let _ = self.conn.execute(
                        "INSERT OR IGNORE INTO project_universe_index \
                         (project_key, universe_key) VALUES (?1, ?2)",
                        params![proj_key, uk],
                    );
                }
            }
        }

        // Clear entries from meta.db — now only in per-universe DBs
        let _ = self.conn.execute_batch("DELETE FROM entries;");
        tracing::info!("CO-77: entries migrated to per-universe DBs");
    }

    // -------------------------------------------------------------------------
    // CO-77: new helper methods
    // -------------------------------------------------------------------------

    /// Backup a universe's data.db to the given path using SQLite's Backup API.
    pub fn backup_universe(&self, universe_key: &str, dest_path: &Path) -> anyhow::Result<()> {
        use rusqlite::backup::Backup;
        let uc = self.universe_pool.get_or_open(universe_key);
        let src = uc.lock().expect("universe conn lock");
        let mut dest = Connection::open(dest_path)?;
        let backup = Backup::new(&src, &mut dest)?;
        backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
        Ok(())
    }

    /// Return the size of a universe's data.db in bytes, or 0 if not yet created.
    pub fn universe_db_size(&self, universe_key: &str) -> u64 {
        std::fs::metadata(self.universe_pool.db_path(universe_key))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Search entries across multiple universes (cross-universe aggregator).
    pub fn search_entries_across_universes(
        &self,
        universe_keys: &[&str],
        query: &str,
    ) -> Vec<crate::entry_index::EntryRow> {
        let mut results = Vec::new();
        for &key in universe_keys {
            let uc = self.universe_pool.get_or_open(key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let index = crate::entry_index::EntryIndex::new(&uc_guard);
            if let Ok(entries) = index.search(key, query) {
                results.extend(entries);
            }
        }
        results
    }

    pub fn schema_version(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    // --- CO-124: Vercel Log Drain ---

    /// Return the per-universe HMAC secret used to validate Vercel drain signatures.
    /// Returns `None` when the universe does not exist.
    pub fn get_log_drain_secret(&self, universe_id: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT log_drain_secret FROM universes WHERE key = ?1",
                params![universe_id],
                |row| row.get(0),
            )
            .optional()
    }

    /// Set the per-universe Vercel Log Drain secret.
    pub fn set_log_drain_secret(&self, universe_id: &str, secret: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE universes SET log_drain_secret = ?1 WHERE key = ?2",
            params![secret, universe_id],
        )?;
        Ok(())
    }

    /// Insert a single Vercel Log Drain event, ignoring duplicates (idempotent by event_id).
    pub fn insert_log_drain_event(&self, ev: &LogDrainEvent) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO log_drain_events \
             (event_id, universe_id, received_at, source, level, message, host, path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ev.event_id,
                ev.universe_id,
                ev.received_at,
                ev.source,
                ev.level,
                ev.message,
                ev.host,
                ev.path
            ],
        )?;
        Ok(())
    }

    // --- CO-45: UAT mutation log ---

    /// Record a write operation in the uat_mutations table.
    /// Only call this when `CO_ENV=uat`.
    pub fn log_uat_mutation(
        &self,
        action: &str,
        target: &str,
        before_value: Option<&str>,
        after_value: Option<&str>,
        user_id: Option<&str>,
        metadata: Option<&str>,
    ) -> rusqlite::Result<()> {
        let ts = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO uat_mutations \
             (timestamp, user_id, action, target, before_value, after_value, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ts,
                user_id,
                action,
                target,
                before_value,
                after_value,
                metadata
            ],
        )?;
        Ok(())
    }

    /// Return all mutations with id > since_id, ordered ascending.
    pub fn get_uat_mutations_since(&self, since_id: i64) -> Vec<crate::models::UatMutation> {
        let mut stmt = match self.conn.prepare(
            "SELECT id, timestamp, user_id, action, target, before_value, after_value, metadata \
             FROM uat_mutations WHERE id > ?1 ORDER BY id ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![since_id], |row| {
            Ok(crate::models::UatMutation {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                user_id: row.get(2)?,
                action: row.get(3)?,
                target: row.get(4)?,
                before_value: row.get(5)?,
                after_value: row.get(6)?,
                metadata: row.get(7)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Return the maximum mutation id (watermark for snapshot creation).
    /// Returns 0 if no mutations exist.
    pub fn get_uat_mutations_watermark(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(id), 0) FROM uat_mutations",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    // --- Projects ---

    pub fn list_projects(&self) -> Vec<Project> {
        // CO-77: fan out to all universes listed in the project_universe_index
        let universe_keys: Vec<String> = {
            let mut stmt = match self
                .conn
                .prepare("SELECT DISTINCT universe_key FROM project_universe_index")
            {
                Ok(s) => s,
                Err(_) => return vec![],
            };
            stmt.query_map([], |r| r.get(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };
        let mut result = Vec::new();
        for uk in &universe_keys {
            result.extend(self.list_projects_for_universe(uk));
        }
        result.sort_by(|a, b| a.key.cmp(&b.key));
        result
    }

    pub fn list_projects_for_universe(&self, universe_key: &str) -> Vec<Project> {
        let uc = self.universe_pool.get_or_open(universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let mut stmt = match uc_guard.prepare(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE universe_key = ?1 AND entry_type = 'project' ORDER BY path",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![universe_key], entry_row_from_sql)
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .filter_map(|row| entry_row_to_project(&row))
            .collect()
    }

    pub fn get_project(&self, key: &str) -> Option<Project> {
        let upper_key = key.to_uppercase();
        let path = format!("projects/{}/_project.md", upper_key);

        // CO-77: look up universe via index, then query per-universe DB
        let universe_key = self.get_project_universe_key(&upper_key)?;
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let result = uc_guard.query_row(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE path = ?1 AND entry_type = 'project'",
            params![path],
            entry_row_from_sql,
        );
        match result {
            Ok(row) => entry_row_to_project(&row),
            Err(_) => None,
        }
    }

    pub fn create_project(&mut self, create: CreateProject) -> anyhow::Result<Project> {
        let upper_key = create.key.to_uppercase();
        if self.get_project(&upper_key).is_some() {
            anyhow::bail!("Project with key '{}' already exists", upper_key);
        }

        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let universe_key = create
            .universe_key
            .clone()
            .unwrap_or_else(|| "default".to_string());

        let path = format!("projects/{}/_project.md", upper_key);
        let fm = json!({
            "type": "project",
            "key": upper_key,
            "title": create.name,
            "status": "active",
            "next_id": 1,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "tags": []
        });

        let entry = make_entry(&path, fm, &create.description);
        let universe_root = self.universe_root(&universe_key);
        co::entry::write_entry(&universe_root, &entry)?;
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            upsert_entry_row(&uc_guard, &universe_key, &entry)?;
        }
        // Register in routing index
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) VALUES (?1, ?2)",
            params![upper_key, universe_key],
        );

        Ok(Project {
            name: create.name,
            key: upper_key,
            description: create.description,
            created_at: now,
            next_id: 1,
            archived: false,
        })
    }

    pub fn delete_project(&mut self, key: &str) -> anyhow::Result<()> {
        let upper_key = key.to_uppercase();
        if self.get_project(&upper_key).is_none() {
            anyhow::bail!("Project '{}' not found", upper_key);
        }

        // Find the universe_key
        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());
        let universe_root = self.universe_root(&universe_key);

        // Find all entries under this project
        let prefix = format!("projects/{}/", upper_key);
        let entry_paths: Vec<String> = {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let mut stmt = uc_guard
                .prepare("SELECT path FROM entries WHERE universe_key = ?1 AND path LIKE ?2")?;
            let like_pattern = format!("{}%", prefix);
            stmt.query_map(params![universe_key, like_pattern], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            for entry_path in &entry_paths {
                let _ = co::entry::delete_entry(&universe_root, entry_path);
                let _ = uc_guard.execute(
                    "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
                    params![universe_key, entry_path],
                );
            }
        }

        // Remove from routing index
        let _ = self.conn.execute(
            "DELETE FROM project_universe_index WHERE project_key = ?1",
            params![upper_key],
        );

        Ok(())
    }

    // --- Tasks ---

    pub fn list_tasks(&self, project_key: &str) -> Vec<Task> {
        self.list_tasks_filtered(project_key, Some(false))
    }

    pub fn list_tasks_filtered(&self, project_key: &str, archived: Option<bool>) -> Vec<Task> {
        self.list_tasks_paginated(project_key, archived, 500, 0)
    }

    pub fn list_tasks_paginated(
        &self,
        project_key: &str,
        archived: Option<bool>,
        limit: u64,
        offset: u64,
    ) -> Vec<Task> {
        let upper_key = project_key.to_uppercase();
        let limit = limit.min(500);

        // CO-77: look up universe, then query per-universe DB
        let universe_key = match self.get_project_universe_key(&upper_key) {
            Some(uk) => uk,
            None => return vec![],
        };
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");

        let sql: String;
        let rows: Vec<EntryRow>;

        match archived {
            Some(archived_val) => {
                let archived_int = if archived_val { 1 } else { 0 };
                sql = format!(
                    "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                     created_at, updated_at FROM entries \
                     WHERE entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?1 \
                     AND json_extract(frontmatter_json, '$.archived') = {} \
                     ORDER BY CAST(json_extract(frontmatter_json, '$.id') AS INTEGER) \
                     LIMIT ?2 OFFSET ?3",
                    archived_int
                );
                let mut stmt = match uc_guard.prepare(&sql) {
                    Ok(s) => s,
                    Err(_) => return vec![],
                };
                rows = stmt
                    .query_map(
                        params![upper_key, limit as i64, offset as i64],
                        entry_row_from_sql,
                    )
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r.ok())
                    .collect();
            }
            None => {
                sql = "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                       created_at, updated_at FROM entries \
                       WHERE entry_type = 'task' \
                       AND json_extract(frontmatter_json, '$.project') = ?1 \
                       ORDER BY CAST(json_extract(frontmatter_json, '$.id') AS INTEGER) \
                       LIMIT ?2 OFFSET ?3"
                    .to_string();
                let mut stmt = match uc_guard.prepare(&sql) {
                    Ok(s) => s,
                    Err(_) => return vec![],
                };
                rows = stmt
                    .query_map(
                        params![upper_key, limit as i64, offset as i64],
                        entry_row_from_sql,
                    )
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r.ok())
                    .collect();
            }
        }

        rows.into_iter()
            .filter_map(|row| entry_row_to_task(&row))
            .collect()
    }

    pub fn get_task(&self, project_key: &str, id: u64) -> Option<Task> {
        let upper_key = project_key.to_uppercase();
        let path = format!("projects/{}/{}.md", upper_key, id);
        // CO-77: look up universe, then query per-universe DB
        let universe_key = self.get_project_universe_key(&upper_key)?;
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let result = uc_guard.query_row(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries WHERE path = ?1 AND entry_type = 'task'",
            params![path],
            entry_row_from_sql,
        );
        match result {
            Ok(row) => entry_row_to_task(&row),
            Err(_) => None,
        }
    }

    pub fn create_task(&mut self, project_key: &str, create: CreateTask) -> anyhow::Result<Task> {
        let upper_key = project_key.to_uppercase();
        let project = self
            .get_project(&upper_key)
            .ok_or_else(|| anyhow::anyhow!("Project '{}' not found", upper_key))?;

        let id = project.next_id;
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());

        // Increment next_id in project entry
        self.increment_project_next_id(&upper_key, &universe_key, id + 1);

        let path = format!("projects/{}/{}.md", upper_key, id);
        let fm = json!({
            "type": "task",
            "id": id,
            "title": create.title,
            "status": create.status.to_string(),
            "priority": create.priority.to_string(),
            "due": create.due_date.map(|d| d.to_string()),
            "parent": create.parent,
            "tags": create.labels,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "assignee": create.assignee,
            "project": upper_key
        });

        let entry = make_entry(&path, fm, &create.description);
        let universe_root = self.universe_root(&universe_key);
        co::entry::write_entry(&universe_root, &entry)?;
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            upsert_entry_row(&uc_guard, &universe_key, &entry)?;
        }

        Ok(Task {
            id,
            key: format!("{}-{}", upper_key, id),
            project_key: upper_key,
            title: create.title,
            status: create.status,
            priority: create.priority,
            due_date: create.due_date,
            parent: create.parent,
            labels: create.labels,
            created_at: now,
            updated_at: now,
            description: create.description,
            archived: false,
            assignee: create.assignee,
        })
    }

    pub fn update_task(
        &mut self,
        project_key: &str,
        id: u64,
        update: UpdateTask,
    ) -> anyhow::Result<Task> {
        let mut task = self
            .get_task(project_key, id)
            .ok_or_else(|| anyhow::anyhow!("Task {}-{} not found", project_key, id))?;

        if let Some(title) = update.title {
            task.title = title;
        }
        if let Some(description) = update.description {
            task.description = description;
        }
        if let Some(status) = update.status {
            task.status = status;
        }
        if let Some(priority) = update.priority {
            task.priority = priority;
        }
        if let Some(due_date) = update.due_date {
            task.due_date = Some(due_date);
        }
        if let Some(parent) = update.parent {
            task.parent = Some(parent);
        }
        if let Some(labels) = update.labels {
            task.labels = labels;
        }
        if let Some(archived) = update.archived {
            task.archived = archived;
        }
        if update.assignee.is_some() {
            task.assignee = update.assignee;
        }

        task.updated_at = Utc::now();

        let universe_key = self
            .get_project_universe_key(&task.project_key)
            .unwrap_or_else(|| "default".to_string());

        let path = format!("projects/{}/{}.md", task.project_key, id);
        let fm = json!({
            "type": "task",
            "id": id,
            "title": task.title,
            "status": task.status.to_string(),
            "priority": task.priority.to_string(),
            "due": task.due_date.map(|d| d.to_string()),
            "parent": task.parent,
            "tags": task.labels,
            "created": task.created_at.to_rfc3339(),
            "modified": task.updated_at.to_rfc3339(),
            "archived": task.archived,
            "assignee": task.assignee,
            "project": task.project_key
        });

        let entry = make_entry(&path, fm, &task.description);
        let universe_root = self.universe_root(&universe_key);
        co::entry::write_entry(&universe_root, &entry)?;
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            upsert_entry_row(&uc_guard, &universe_key, &entry)?;
        }

        Ok(task)
    }

    pub fn delete_task(&mut self, project_key: &str, id: u64) -> anyhow::Result<()> {
        let upper_key = project_key.to_uppercase();
        self.get_task(&upper_key, id)
            .ok_or_else(|| anyhow::anyhow!("Task {}-{} not found", upper_key, id))?;

        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());
        let universe_root = self.universe_root(&universe_key);
        let path = format!("projects/{}/{}.md", upper_key, id);

        let _ = co::entry::delete_entry(&universe_root, &path);
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let _ = uc_guard.execute(
                "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
                params![universe_key, path],
            );
        }

        Ok(())
    }

    // --- Bulk Operations ---

    pub fn bulk_update_tasks(
        &mut self,
        project_key: &str,
        bulk: BulkUpdateTasks,
    ) -> anyhow::Result<Vec<Task>> {
        let upper_key = project_key.to_uppercase();
        for &task_id in &bulk.task_ids {
            let update = UpdateTask {
                title: None,
                description: None,
                status: bulk.status.clone(),
                priority: None,
                due_date: None,
                parent: None,
                labels: None,
                archived: bulk.archived,
                assignee: None,
            };
            let _ = self.update_task(&upper_key, task_id, update);
        }

        let mut result = Vec::new();
        for &task_id in &bulk.task_ids {
            if let Some(task) = self.get_task(&upper_key, task_id) {
                result.push(task);
            }
        }
        Ok(result)
    }

    pub fn bulk_delete_tasks(
        &mut self,
        project_key: &str,
        bulk: BulkDeleteTasks,
    ) -> anyhow::Result<()> {
        let upper_key = project_key.to_uppercase();
        for &task_id in &bulk.task_ids {
            let _ = self.delete_task(&upper_key, task_id);
        }
        Ok(())
    }

    // --- Comments ---

    pub fn list_comments(&self, project_key: &str, task_id: u64) -> Vec<Comment> {
        let upper_key = project_key.to_uppercase();
        let path_prefix = format!("projects/{}/comments/{}-", upper_key, task_id);

        // CO-77: look up universe, then query per-universe DB
        let universe_key = match self.get_project_universe_key(&upper_key) {
            Some(uk) => uk,
            None => return vec![],
        };
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let mut stmt = match uc_guard.prepare(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE entry_type = 'comment' AND path LIKE ?1 \
             ORDER BY created_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let like_pattern = format!("{}%", path_prefix);
        stmt.query_map(params![like_pattern], entry_row_from_sql)
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .filter_map(|row| entry_row_to_comment(&row, &upper_key, task_id))
            .collect()
    }

    pub fn create_comment(
        &mut self,
        project_key: &str,
        task_id: u64,
        create: CreateComment,
    ) -> anyhow::Result<Comment> {
        let upper_key = project_key.to_uppercase();

        // Verify task exists
        self.get_task(&upper_key, task_id)
            .ok_or_else(|| anyhow::anyhow!("Task {}-{} not found", upper_key, task_id))?;

        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());

        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Allocate id via COUNT in per-universe DB
        let id: u64 = {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let count: i64 = uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE entry_type = 'comment' \
                     AND path LIKE ?1",
                    params![format!("projects/{}/comments/{}-%%", upper_key, task_id)],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            (count + 1) as u64
        };

        let path = format!("projects/{}/comments/{}-{}.md", upper_key, task_id, id);
        let fm = json!({
            "type": "comment",
            "id": id,
            "task": task_id,
            "project": upper_key,
            "author": create.author,
            "created": now_str,
            "modified": now_str,
            "tags": []
        });

        let entry = make_entry(&path, fm, &create.body);
        let universe_root = self.universe_root(&universe_key);
        co::entry::write_entry(&universe_root, &entry)?;
        {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            upsert_entry_row(&uc_guard, &universe_key, &entry)?;
        }

        Ok(Comment {
            id,
            project_key: upper_key,
            task_id,
            author: create.author,
            body: create.body,
            created_at: now,
        })
    }

    // --- Activity Log (graceful fallback — table may not exist) ---

    pub fn list_activity(&self, project_key: &str, limit: u64) -> Vec<ActivityEntry> {
        let upper_key = project_key.to_uppercase();
        let mut stmt = match self.conn.prepare(
            "SELECT id, project_key, task_id, action, field, old_value, new_value, actor, created_at \
             FROM activity_log WHERE project_key = ?1 ORDER BY created_at DESC LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match stmt.query_map(params![upper_key, limit as i64], |row| {
            Ok(ActivityEntry {
                id: row.get::<_, i64>(0)? as u64,
                project_key: row.get(1)?,
                task_id: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                action: row.get(3)?,
                field: row.get(4)?,
                old_value: row.get(5)?,
                new_value: row.get(6)?,
                actor: row.get(7)?,
                created_at: parse_datetime(&row.get::<_, String>(8)?),
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    // --- Dashboard ---

    pub fn get_dashboard(&self, project_key: &str) -> DashboardData {
        let upper_key = project_key.to_uppercase();

        let status_counts = self.get_status_counts(&upper_key);

        // CO-77: look up universe for per-universe queries
        let universe_key = self
            .get_project_universe_key(&upper_key)
            .unwrap_or_else(|| "default".to_string());

        let today_str = chrono::Utc::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        let overdue_count: i64 = {
            let uc = self.universe_pool.get_or_open(&universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?1 \
                     AND json_extract(frontmatter_json, '$.archived') = 0 \
                     AND json_extract(frontmatter_json, '$.status') != 'done' \
                     AND json_extract(frontmatter_json, '$.due') IS NOT NULL \
                     AND json_extract(frontmatter_json, '$.due') < ?2",
                    params![upper_key, today_str],
                    |row| row.get(0),
                )
                .unwrap_or(0)
        };

        let upcoming_tasks =
            self.query_tasks_entries(&upper_key, Some(false), Some("!= 'done'"), true, Some(10));
        let recently_updated = self.query_tasks_recent(&upper_key, 10);
        let velocity = self.get_velocity(&upper_key);
        let burndown = self.get_burndown(&upper_key);
        let label_distribution = self.get_label_distribution(&upper_key);
        let overdue_tasks_detail = self.get_overdue_tasks_detail(&upper_key);

        DashboardData {
            status_counts,
            overdue_count: overdue_count as u64,
            upcoming_tasks,
            recently_updated,
            velocity,
            burndown,
            label_distribution,
            overdue_tasks_detail,
        }
    }

    fn get_velocity(&self, project_key: &str) -> Vec<WeeklyVelocity> {
        let mut stmt = match self.conn.prepare(
            "SELECT strftime('%Y-W%W', created_at) as week, COUNT(*) as count \
             FROM activity_log \
             WHERE project_key = ?1 \
               AND action = 'field_changed' \
               AND field = 'status' \
               AND new_value = 'done' \
               AND date(created_at) >= date('now', '-56 days') \
             GROUP BY week \
             ORDER BY week ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match stmt.query_map(params![project_key], |row| {
            Ok(WeeklyVelocity {
                week: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    fn get_burndown(&self, project_key: &str) -> Vec<BurndownPoint> {
        // CO-77: look up universe for per-universe queries
        let universe_key = self
            .get_project_universe_key(project_key)
            .unwrap_or_else(|| "default".to_string());
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");

        let today = chrono::Utc::now().date_naive();
        let mut result = Vec::with_capacity(8);

        for week_offset in (0i64..8).rev() {
            let week_end = today - chrono::Duration::weeks(week_offset);
            let week_label = week_end.format("%Y-W%V").to_string();
            let week_end_str = week_end.to_string();

            let total_created: i64 = uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?1 \
                     AND json_extract(frontmatter_json, '$.archived') = 0 \
                     AND date(created_at) <= ?2",
                    params![project_key, week_end_str],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            // For done count we fall back to activity_log (graceful)
            let total_done: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM activity_log \
                     WHERE project_key = ?1 \
                       AND action = 'field_changed' \
                       AND field = 'status' \
                       AND new_value = 'done' \
                       AND date(created_at) <= ?2",
                    params![project_key, week_end_str],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            result.push(BurndownPoint {
                date: week_label,
                remaining: (total_created - total_done).max(0),
                completed: total_done as u64,
            });
        }

        result
    }

    fn get_label_distribution(&self, project_key: &str) -> Vec<LabelCount> {
        let mut stmt = match self.conn.prepare(
            "SELECT frontmatter_json FROM entries \
             WHERE entry_type = 'task' \
             AND json_extract(frontmatter_json, '$.project') = ?1 \
             AND json_extract(frontmatter_json, '$.archived') = 0",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let fm_strings: Vec<String> = match stmt.query_map(params![project_key], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => return vec![],
        };

        let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for fm_str in fm_strings {
            let fm: serde_json::Value =
                serde_json::from_str(&fm_str).unwrap_or(serde_json::Value::Null);
            if let Some(tags) = fm.get("tags").and_then(|v| v.as_array()) {
                for tag in tags {
                    if let Some(t) = tag.as_str()
                        && !t.is_empty()
                    {
                        *counts.entry(t.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut result: Vec<LabelCount> = counts
            .into_iter()
            .map(|(label, count)| LabelCount { label, count })
            .collect();
        result.sort_by(|a, b| b.count.cmp(&a.count));
        result.truncate(10);
        result
    }

    fn get_overdue_tasks_detail(&self, project_key: &str) -> Vec<OverdueTaskDetail> {
        let today = chrono::Utc::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let mut stmt = match self.conn.prepare(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE entry_type = 'task' \
             AND json_extract(frontmatter_json, '$.project') = ?1 \
             AND json_extract(frontmatter_json, '$.archived') = 0 \
             AND json_extract(frontmatter_json, '$.status') != 'done' \
             AND json_extract(frontmatter_json, '$.due') IS NOT NULL \
             AND json_extract(frontmatter_json, '$.due') < ?2 \
             ORDER BY json_extract(frontmatter_json, '$.due') ASC LIMIT 20",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let rows: Vec<EntryRow> =
            match stmt.query_map(params![project_key, today_str], entry_row_from_sql) {
                Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                Err(_) => return vec![],
            };

        rows.into_iter()
            .filter_map(|row| {
                let task = entry_row_to_task(&row)?;
                let due = task.due_date?;
                let days_overdue = (today - due).num_days();
                Some(OverdueTaskDetail {
                    id: task.id,
                    key: task.key,
                    title: task.title,
                    due_date: due.to_string(),
                    days_overdue,
                    priority: task.priority.to_string(),
                })
            })
            .collect()
    }

    fn get_status_counts(&self, project_key: &str) -> StatusCounts {
        // CO-77: look up universe for per-universe queries
        let universe_key = self
            .get_project_universe_key(project_key)
            .unwrap_or_else(|| "default".to_string());
        let uc = self.universe_pool.get_or_open(&universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");

        let count = |status: &str| -> u64 {
            uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?1 \
                     AND json_extract(frontmatter_json, '$.archived') = 0 \
                     AND json_extract(frontmatter_json, '$.status') = ?2",
                    params![project_key, status],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0) as u64
        };

        let todo = count("todo");
        let in_progress = count("in_progress");
        let in_review = count("in_review");
        let done = count("done");

        StatusCounts {
            todo,
            in_progress,
            in_review,
            done,
            total: todo + in_progress + in_review + done,
        }
    }

    /// Query tasks with due date within the next 7 days (upcoming).
    fn query_tasks_entries(
        &self,
        project_key: &str,
        archived: Option<bool>,
        status_condition: Option<&str>,
        upcoming_only: bool,
        limit: Option<u64>,
    ) -> Vec<Task> {
        let archived_filter = match archived {
            Some(true) => "AND json_extract(frontmatter_json, '$.archived') = 1".to_string(),
            Some(false) => "AND json_extract(frontmatter_json, '$.archived') = 0".to_string(),
            None => String::new(),
        };
        let status_filter = status_condition
            .map(|c| format!("AND json_extract(frontmatter_json, '$.status') {}", c))
            .unwrap_or_default();
        let upcoming_filter = if upcoming_only {
            "AND json_extract(frontmatter_json, '$.due') IS NOT NULL \
             AND json_extract(frontmatter_json, '$.due') BETWEEN date('now') AND date('now', '+7 days')"
                .to_string()
        } else {
            String::new()
        };
        let limit_clause = limit.map(|l| format!("LIMIT {}", l)).unwrap_or_default();

        let sql = format!(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries \
             WHERE entry_type = 'task' \
             AND json_extract(frontmatter_json, '$.project') = ?1 \
             {} {} {} \
             ORDER BY json_extract(frontmatter_json, '$.due') ASC {}",
            archived_filter, status_filter, upcoming_filter, limit_clause
        );

        let mut stmt = match self.conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match stmt.query_map(params![project_key], entry_row_from_sql) {
            Ok(rows) => rows
                .filter_map(|r| r.ok())
                .filter_map(|row| entry_row_to_task(&row))
                .collect(),
            Err(_) => vec![],
        }
    }

    fn query_tasks_recent(&self, project_key: &str, limit: u64) -> Vec<Task> {
        let sql = "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                   created_at, updated_at FROM entries \
                   WHERE entry_type = 'task' \
                   AND json_extract(frontmatter_json, '$.project') = ?1 \
                   AND json_extract(frontmatter_json, '$.archived') = 0 \
                   ORDER BY updated_at DESC \
                   LIMIT ?2";

        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match stmt.query_map(params![project_key, limit as i64], entry_row_from_sql) {
            Ok(rows) => rows
                .filter_map(|r| r.ok())
                .filter_map(|row| entry_row_to_task(&row))
                .collect(),
            Err(_) => vec![],
        }
    }

    // --- Users ---

    pub fn create_user(
        &mut self,
        email: &str,
        display_name: &str,
    ) -> anyhow::Result<crate::models::User> {
        let id = format!("usr_{}", nanoid::nanoid!(10));
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT INTO users (id, email, display_name, tier, created_at) VALUES (?1, ?2, ?3, 'player', ?4)",
            params![id, email, display_name, now_str],
        )?;
        Ok(crate::models::User {
            id,
            email: email.to_string(),
            display_name: display_name.to_string(),
            tier: "player".to_string(),
            created_at: now,
        })
    }

    pub fn get_user_by_email(&self, email: &str) -> Option<crate::models::User> {
        self.conn
            .query_row(
                "SELECT id, email, display_name, tier, created_at FROM users WHERE email = ?1",
                params![email],
                |row| {
                    Ok(crate::models::User {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        tier: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                    })
                },
            )
            .ok()
    }

    pub fn get_user_by_id(&self, id: &str) -> Option<crate::models::User> {
        self.conn
            .query_row(
                "SELECT id, email, display_name, tier, created_at FROM users WHERE id = ?1",
                params![id],
                |row| {
                    Ok(crate::models::User {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        tier: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                    })
                },
            )
            .ok()
    }

    // --- UAT-specific methods (CO-44) ---

    /// Get user by email along with their stored Argon2 password hash.
    /// Returns `None` if the user does not exist or has no password set.
    pub fn get_user_by_email_with_hash(
        &self,
        email: &str,
    ) -> Option<(crate::models::User, Option<String>)> {
        self.conn
            .query_row(
                "SELECT id, email, display_name, tier, created_at, password_hash \
                 FROM users WHERE email = ?1",
                params![email],
                |row| {
                    Ok((
                        crate::models::User {
                            id: row.get(0)?,
                            email: row.get(1)?,
                            display_name: row.get(2)?,
                            tier: row.get(3)?,
                            created_at: parse_datetime(&row.get::<_, String>(4)?),
                        },
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .ok()
    }

    /// Seed the UAT `yuri@uat.local` user with an Argon2-hashed password.
    ///
    /// Idempotent: if the user already exists their password hash is updated
    /// to the supplied value (so a fresh hash is applied on each UAT startup).
    pub fn seed_uat_user(&mut self, password_hash: &str) -> anyhow::Result<()> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM users WHERE email = 'yuri@uat.local'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if exists {
            self.conn.execute(
                "UPDATE users SET password_hash = ?1, tier = 'admin' WHERE email = 'yuri@uat.local'",
                params![password_hash],
            )?;
            tracing::info!("UAT: updated yuri@uat.local password hash");
        } else {
            let now = Utc::now().to_rfc3339();
            self.conn.execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, password_hash) \
                 VALUES ('usr_yuri_uat', 'yuri@uat.local', 'yuri', 'admin', ?1, ?2)",
                params![now, password_hash],
            )?;
            tracing::info!("UAT: seeded user yuri@uat.local (tier=admin)");
        }
        Ok(())
    }

    /// Seed an admin user from env vars `CO_SEED_ADMIN_EMAIL` + `CO_SEED_ADMIN_PASSWORD_HASH`.
    ///
    /// Idempotent with drift detection:
    /// - User missing → insert (tier=admin).
    /// - User exists, hash unchanged → no-op.
    /// - User exists, hash differs → update hash + tier.
    pub fn seed_admin_user_from_env(
        &mut self,
        email: &str,
        password_hash: &str,
    ) -> anyhow::Result<()> {
        if !password_hash.starts_with("$argon2id$") {
            tracing::warn!(
                "CO_SEED_ADMIN_PASSWORD_HASH does not look like an Argon2id hash \
                 (expected '$argon2id$v=19$m=...$...'). Check your configuration."
            );
        }

        let email = email.trim().to_lowercase();

        let existing = self
            .conn
            .query_row(
                "SELECT id, password_hash FROM users WHERE email = ?1",
                params![email],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .ok();

        match existing {
            Some((_, ref existing_hash)) if existing_hash.as_deref() == Some(password_hash) => {
                tracing::info!("admin user already seeded: {email} (hash unchanged)");
            }
            Some((user_id, _)) => {
                // CO-90: tier is billing-only; do not write 'admin' here. The
                // seeded user gets privileged access via per-universe ownership,
                // not a global tier bypass.
                self.conn.execute(
                    "UPDATE users SET password_hash = ?1 WHERE id = ?2",
                    params![password_hash, user_id],
                )?;
                tracing::info!("seeded user updated: {email} (hash refreshed)");
            }
            None => {
                let id = format!(
                    "usr_{}",
                    &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
                );
                let now = Utc::now().to_rfc3339();
                // CO-90: tier='user' (billing default). Authority over system
                // universes comes from owner_id, not tier.
                self.conn.execute(
                    "INSERT INTO users (id, email, display_name, tier, created_at, password_hash) \
                     VALUES (?1, ?2, ?2, 'user', ?3, ?4)",
                    params![id, email, now, password_hash],
                )?;
                tracing::info!("seeded user created: {email}");
            }
        }
        Ok(())
    }

    /// CO-90 (preview): make the seeded admin user a member of every existing
    /// system-owned universe so the SPA shows them in the user's sidebar after
    /// login. Idempotent (`INSERT OR IGNORE`). Skips universes that don't
    /// exist yet — call this AFTER all universe seeds.
    ///
    /// Without this, a freshly-seeded admin logs in to an empty dashboard
    /// because `list_universes_for_user` only returns owned + member +
    /// subscribed universes.
    pub fn ensure_admin_universe_memberships(&mut self, email: &str) -> anyhow::Result<()> {
        let email = email.trim().to_lowercase();
        let user_id: String = match self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![email],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!(
                    "ensure_admin_universe_memberships: no user for {email} (seed first)"
                );
                return Ok(());
            }
        };

        // System universes the seeded admin should see immediately.
        // Order is irrelevant; `INSERT OR IGNORE` handles repeats.
        // co-dev and co-experience removed (CO-142 Phase C — deprecated).
        let system_keys = [
            "template",
            "quilomboaraucaria",
            "yggdrasil",
            "dados",
            // Admin content universes (seeded by seed_admin_content_universes).
            "artelonga",
            "rfq",
            "co",
        ];
        let now = Utc::now().to_rfc3339();
        let mut added = 0usize;
        for key in system_keys {
            // Skip universes that don't exist (haven't been seeded yet).
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM universes WHERE key = ?1",
                    params![key],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if !exists {
                continue;
            }
            let n = self.conn.execute(
                "INSERT OR IGNORE INTO universe_members \
                 (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'admin', ?3)",
                params![key, user_id, now],
            )?;
            if n > 0 {
                added += 1;
            }
        }
        if added > 0 {
            tracing::info!(
                "ensure_admin_universe_memberships: added {email} to {added} universe(s) as admin"
            );
        }
        Ok(())
    }

    /// Re-home a known list of personal universes to the admin's CURRENT
    /// user_id, regardless of whether the prior owner is still in `users`.
    ///
    /// `rescue_orphan_universes` only catches truly dangling `owner_id` values
    /// (no row in `users`). But a more common failure mode is two valid users:
    /// the prior admin (still in `users` from a stale bootstrap), and the
    /// current admin (re-seeded). The universes still point at the prior
    /// admin's id, so the new admin can't see them.
    ///
    /// This function is targeted — it only touches the well-known personal
    /// universes named in the bootstrap script. Idempotent. Returns the number
    /// of universes re-homed.
    /// Free up slug claims held by `*@co.local` legacy/test users so real
    /// users can claim those slugs as their `username`. Renames any colliding
    /// `*@co.local` user's username to `legacy-<original-slug>` (e.g.
    /// `yuri` → `legacy-yuri`). Idempotent — already-renamed rows are no-ops.
    ///
    /// 2026-05-02 — closes the unique-index conflict that blocked
    /// `ensure_admin_username` from claiming `yuri` for the admin while the
    /// legacy `yuri@co.local` test user held it.
    pub fn free_legacy_co_local_usernames(&mut self) -> anyhow::Result<usize> {
        // Only touches usernames currently NOT prefixed with 'legacy-' to
        // remain idempotent across re-runs.
        let updated = self.conn.execute(
            "UPDATE users SET username = 'legacy-' || username \
             WHERE email LIKE '%@co.local' \
               AND username IS NOT NULL \
               AND username != '' \
               AND username NOT LIKE 'legacy-%'",
            [],
        )?;
        if updated > 0 {
            tracing::info!(
                "free_legacy_co_local_usernames: renamed {updated} legacy @co.local username(s)"
            );
        }
        Ok(updated)
    }

    /// Ensure the admin user has a username derived from their email prefix
    /// (e.g. `yuri@artelonga.com.br` → `yuri`) when currently empty.
    ///
    /// 2026-05-02 — addresses user directive "always use slug as user name by
    /// default". The unique index on `users.username` means we may collide
    /// with an existing user; on conflict we log and skip rather than break
    /// the boot path. `free_legacy_co_local_usernames` runs first to clear
    /// the common case of legacy `*@co.local` test users holding the slug.
    pub fn ensure_admin_username(&mut self, admin_email: &str) -> anyhow::Result<bool> {
        let email = admin_email.trim().to_lowercase();
        let prefix: String = email
            .split('@')
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if prefix.is_empty() {
            return Ok(false);
        }

        let current: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT username FROM users WHERE email = ?1",
                params![email],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok();
        let Some(current) = current else {
            return Ok(false);
        }; // user not seeded yet

        if current.as_deref().is_some_and(|s| !s.is_empty()) {
            return Ok(false); // already set, leave alone
        }

        // Try to claim the prefix as username. Unique index may collide with
        // a legacy/UAT user holding the same slug.
        match self.conn.execute(
            "UPDATE users SET username = ?1 WHERE email = ?2",
            params![prefix, email],
        ) {
            Ok(n) if n > 0 => {
                tracing::info!("ensure_admin_username: set username='{prefix}' for {email}");
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(e) => {
                tracing::warn!(
                    "ensure_admin_username: could not set username='{prefix}' for {email} \
                     (likely unique-index conflict with another user holding the same slug): {e}"
                );
                Ok(false)
            }
        }
    }

    pub fn ensure_admin_owns_personal_universes(
        &mut self,
        admin_email: &str,
        keys: &[&str],
    ) -> anyhow::Result<usize> {
        let admin_email = admin_email.trim().to_lowercase();
        let admin_user_id: String = match self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![admin_email],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => return Ok(0),
        };
        let now = Utc::now().to_rfc3339();
        let mut rehomed = 0usize;
        for key in keys {
            let current_owner: Option<String> = self
                .conn
                .query_row(
                    "SELECT owner_id FROM universes WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            let Some(current_owner) = current_owner else {
                continue; // universe doesn't exist
            };
            if current_owner == admin_user_id {
                // Still ensure membership row exists (defensive).
                self.conn.execute(
                    "INSERT OR IGNORE INTO universe_members \
                     (universe_key, user_id, role, joined_at) \
                     VALUES (?1, ?2, 'owner', ?3)",
                    params![key, admin_user_id, now],
                )?;
                continue;
            }
            self.conn.execute(
                "UPDATE universes SET owner_id = ?1 WHERE key = ?2",
                params![admin_user_id, key],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO universe_members \
                 (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, admin_user_id, now],
            )?;
            tracing::info!(
                "ensure_admin_owns_personal_universes: re-homed {key} from {current_owner} to {admin_email}"
            );
            rehomed += 1;
        }
        Ok(rehomed)
    }

    /// Delete anonymous-clone universes (key prefix `u-` or `anon-`) whose
    /// owner has been re-homed to the supplied admin user. These are the
    /// "Meu Co" clones that pile up on the admin's sidebar after a previous
    /// version of `rescue_orphan_universes` grabbed them.
    ///
    /// Only deletes universes whose owner currently equals the admin's user
    /// id — does NOT touch anon clones that genuinely belong to someone else,
    /// or that retain their original `anon-...` owner_id.
    ///
    /// Idempotent. Returns the number of universes removed.
    pub fn cleanup_admin_anon_clutter(&mut self, admin_email: &str) -> anyhow::Result<usize> {
        let admin_email = admin_email.trim().to_lowercase();
        let admin_user_id: String = match self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![admin_email],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => return Ok(0),
        };
        let keys: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT key FROM universes \
                 WHERE owner_id = ?1 \
                   AND (key LIKE 'u-%' OR key LIKE 'anon-%')",
            )?;
            stmt.query_map(params![admin_user_id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };
        let mut removed = 0usize;
        for key in &keys {
            let _ = self
                .conn
                .execute("DELETE FROM entries WHERE universe_key = ?1", params![key]);
            let _ = self.conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                params![key],
            );
            let n = self
                .conn
                .execute("DELETE FROM universes WHERE key = ?1", params![key])?;
            if n > 0 {
                let universe_dir = self.data_dir.join("universes").join(key);
                if universe_dir.exists() {
                    let _ = std::fs::remove_dir_all(&universe_dir);
                }
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!(
                "cleanup_admin_anon_clutter: removed {removed} stale anon-clone universe(s) from {admin_email}"
            );
        }
        Ok(removed)
    }

    /// Re-home universes whose `owner_id` no longer maps to any user — typically
    /// the result of a data wipe or migration that re-seeded the admin user
    /// with a different uuid, leaving prior universes orphaned.
    ///
    /// For every universe with a dangling `owner_id`, set the owner to the
    /// supplied admin user_id and ensure they're a member with role='owner'.
    /// Idempotent. Returns the number of universes re-homed.
    pub fn rescue_orphan_universes(&mut self, admin_email: &str) -> anyhow::Result<usize> {
        let admin_email = admin_email.trim().to_lowercase();
        let admin_user_id: String = match self.conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            params![admin_email],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => return Ok(0),
        };

        // Find universes whose owner_id has no row in users — but skip the
        // 'system' sentinel (template, quilomboaraucaria, yggdrasil, etc.)
        // AND skip anonymous-clone universes (key starts with `anon-` or
        // `u-`). Those should be deleted by `cleanup_anon_universes`, not
        // re-homed to the admin (otherwise the admin's sidebar fills up
        // with hundreds of orphaned "Meu Co" clones from past visitors).
        let orphan_keys: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT u.key FROM universes u \
                 LEFT JOIN users usr ON usr.id = u.owner_id \
                 WHERE usr.id IS NULL \
                   AND u.owner_id != 'system' \
                   AND u.key NOT LIKE 'anon-%' \
                   AND u.key NOT LIKE 'u-%'",
            )?;
            stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        let now = Utc::now().to_rfc3339();
        let mut rescued = 0usize;
        for key in &orphan_keys {
            self.conn.execute(
                "UPDATE universes SET owner_id = ?1 WHERE key = ?2",
                params![admin_user_id, key],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO universe_members \
                 (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, admin_user_id, now],
            )?;
            tracing::info!("rescue_orphan_universes: re-homed {key} to {admin_email}");
            rescued += 1;
        }
        Ok(rescued)
    }

    /// Remove all anonymous universes (keys starting with `anon-`) from the
    /// database and from the filesystem. Called on UAT startup so each session
    /// starts from a clean slate.
    ///
    /// Returns the number of universes removed.
    pub fn cleanup_anon_universes(&mut self) -> usize {
        let anon_keys: Vec<String> = {
            let mut stmt = match self
                .conn
                .prepare("SELECT key FROM universes WHERE key LIKE 'anon-%'")
            {
                Ok(s) => s,
                Err(_) => return 0,
            };
            stmt.query_map([], |row| row.get(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };

        let count = anon_keys.len();
        for key in &anon_keys {
            let _ = self
                .conn
                .execute("DELETE FROM entries WHERE universe_key = ?1", params![key]);
            let _ = self.conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                params![key],
            );
            let _ = self
                .conn
                .execute("DELETE FROM universes WHERE key = ?1", params![key]);
            let universe_dir = self.data_dir.join("universes").join(key);
            if universe_dir.exists() {
                let _ = std::fs::remove_dir_all(&universe_dir);
            }
        }

        if count > 0 {
            tracing::info!("UAT: cleaned up {} anonymous universe(s)", count);
        }
        count
    }

    /// Collect all users (with password hashes) for backup before a DB reset.
    pub fn get_all_users_with_hashes(&self) -> Vec<(crate::models::User, Option<String>)> {
        let mut stmt = match self
            .conn
            .prepare("SELECT id, email, display_name, tier, created_at, password_hash FROM users")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |row| {
            Ok((
                crate::models::User {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    display_name: row.get(2)?,
                    tier: row.get(3)?,
                    created_at: parse_datetime(&row.get::<_, String>(4)?),
                },
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Re-insert users (with hashes) after a DB reset. Uses INSERT OR IGNORE to
    /// avoid duplicates if migrations re-ran and the yuri seed already ran.
    pub fn restore_users_with_hashes(&mut self, users: &[(crate::models::User, Option<String>)]) {
        for (user, hash) in users {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO users \
                 (id, email, display_name, tier, created_at, password_hash) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    user.id,
                    user.email,
                    user.display_name,
                    user.tier,
                    user.created_at.to_rfc3339(),
                    hash.as_deref(),
                ],
            );
        }
    }

    // --- Universes ---

    pub fn create_universe(
        &mut self,
        create: crate::models::CreateUniverse,
        owner_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        if self.get_universe(&create.key).is_some() {
            anyhow::bail!("Universe '{}' already exists", create.key);
        }
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT INTO universes (key, name, description, owner_id, created_at, visibility) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'private')",
            params![
                create.key,
                create.name,
                create.description,
                owner_id,
                now_str
            ],
        )?;
        // Owner is automatically a member
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) VALUES (?1, ?2, 'owner', ?3)",
            params![create.key, owner_id, now_str],
        )?;

        // Create ONE empty project named "Co" — private boards start empty.
        // Project key is unique per universe to avoid cross-universe task leaks.
        let proj_key = format!(
            "{}P",
            create
                .key
                .to_uppercase()
                .chars()
                .take(4)
                .collect::<String>()
        );
        let proj_path = format!("projects/{}/_project.md", proj_key);
        let proj_fm = json!({
            "type": "project",
            "key": proj_key,
            "title": "Bem-vindo ao Co",
            "status": "active",
            "next_id": 1,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "tags": []
        });
        let proj_entry = make_entry(&proj_path, proj_fm, "");
        let universe_root = self.universe_root(&create.key);
        let _ = co::entry::write_entry(&universe_root, &proj_entry);
        let _ = upsert_entry_row(&self.conn, &create.key, &proj_entry);

        let _ = self.conn.execute(
            "UPDATE universes SET content_count = 1 WHERE key = ?1",
            params![create.key],
        );

        Ok(crate::models::Universe {
            key: create.key,
            name: create.name,
            description: create.description,
            owner_id: owner_id.to_string(),
            created_at: now,
            is_template: false,
            is_public: false,
            content_count: 1,
            requires_login: false,
            visibility: "private".into(),
            parent_key: None,
        })
    }

    pub fn get_universe(&self, key: &str) -> Option<crate::models::Universe> {
        // CO-98 hardening: query the stable schema first (always present at
        // schema_v ≥ 17), then opportunistically fetch `parent_key` in a
        // second query. If migration v22 never landed on this DB (or the
        // column is otherwise absent), the second query returns None and the
        // function still produces a valid Universe — instead of returning
        // 404 to the caller as if the universe didn't exist at all.
        let mut universe = self
            .conn
            .query_row(
                "SELECT key, name, description, owner_id, created_at, is_template, is_public, content_count, \
                 COALESCE(requires_login, 0), COALESCE(visibility, 'private') \
                 FROM universes WHERE key = ?1",
                params![key],
                |row| {
                    Ok(crate::models::Universe {
                        key: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        owner_id: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                        is_template: row.get::<_, i64>(5)? != 0,
                        is_public: row.get::<_, i64>(6)? != 0,
                        content_count: row.get::<_, i64>(7).unwrap_or(0),
                        requires_login: row.get::<_, i64>(8).unwrap_or(0) != 0,
                        visibility: row.get::<_, String>(9).unwrap_or_else(|_| "private".into()),
                        parent_key: None,
                    })
                },
            )
            .ok()?;
        universe.parent_key = self
            .conn
            .query_row(
                "SELECT parent_key FROM universes WHERE key = ?1",
                params![key],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        Some(universe)
    }

    pub fn list_universes_for_user(&self, user_id: &str) -> Vec<crate::models::Universe> {
        // Owned + member + subscribed universes, deduplicated.
        // Also include `owner_id = user_id` directly as a defensive fallback —
        // `create_universe` inserts an owner row in `universe_members`, but
        // historic data or a partial migration could leave that row missing,
        // which would silently hide the user's own universe from their sidebar.
        let mut stmt = self
            .conn
            .prepare(
                "SELECT u.key, u.name, u.description, u.owner_id, u.created_at, \
                 u.is_template, u.is_public, u.content_count, \
                 COALESCE(u.requires_login, 0), COALESCE(u.visibility, 'private') \
                 FROM universes u \
                 WHERE u.owner_id = ?1 \
                    OR u.key IN ( \
                      SELECT universe_key FROM universe_members WHERE user_id = ?1 \
                      UNION \
                      SELECT universe_key FROM subscriptions WHERE user_id = ?1 \
                    ) \
                 ORDER BY u.created_at ASC",
            )
            .expect("Failed to prepare list_universes_for_user");
        // CO-98 hardening: same two-query split as get_universe — the bulk
        // SELECT stays on the stable schema, then we opportunistically fetch
        // parent_key per row. Slightly more SQL round-trips but resilient to
        // a partially-applied migration.
        let universes: Vec<crate::models::Universe> = stmt
            .query_map(params![user_id], |row| {
                Ok(crate::models::Universe {
                    key: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    owner_id: row.get(3)?,
                    created_at: parse_datetime(&row.get::<_, String>(4)?),
                    is_template: row.get::<_, i64>(5)? != 0,
                    is_public: row.get::<_, i64>(6)? != 0,
                    content_count: row.get::<_, i64>(7).unwrap_or(0),
                    requires_login: row.get::<_, i64>(8).unwrap_or(0) != 0,
                    visibility: row.get::<_, String>(9).unwrap_or_else(|_| "private".into()),
                    parent_key: None,
                })
            })
            .expect("Failed to list universes for user")
            .filter_map(|r| r.ok())
            .collect();
        universes
            .into_iter()
            .map(|mut u| {
                u.parent_key = self
                    .conn
                    .query_row(
                        "SELECT parent_key FROM universes WHERE key = ?1",
                        params![u.key],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten();
                u
            })
            .collect()
    }

    // --- Universe Members ---

    pub fn is_universe_member(&self, universe_key: &str, user_id: &str) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
                params![universe_key, user_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    pub fn list_universe_members(&self, universe_key: &str) -> Vec<crate::models::UniverseMember> {
        let mut stmt = self.conn.prepare(
            "SELECT um.universe_key, um.user_id, um.role, um.joined_at, u.email, u.display_name \
             FROM universe_members um \
             LEFT JOIN users u ON um.user_id = u.id \
             WHERE um.universe_key = ?1 \
             ORDER BY um.joined_at ASC",
        ).expect("Failed to prepare list_universe_members");
        stmt.query_map(params![universe_key], |row| {
            Ok(crate::models::UniverseMember {
                universe_key: row.get(0)?,
                user_id: row.get(1)?,
                role: row.get(2)?,
                joined_at: parse_datetime(&row.get::<_, String>(3)?),
                email: row.get(4)?,
                display_name: row.get(5)?,
            })
        })
        .expect("Failed to list universe members")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn add_universe_member(
        &mut self,
        universe_key: &str,
        user_id: &str,
        role: &str,
    ) -> anyhow::Result<crate::models::UniverseMember> {
        if self.get_universe(universe_key).is_none() {
            anyhow::bail!("Universe '{}' not found", universe_key);
        }
        // Note: user_id may refer to a quilombo user (not in the users table) — no FK check.
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) VALUES (?1, ?2, ?3, ?4)",
            params![universe_key, user_id, role, now_str],
        )?;
        let member = self.conn.query_row(
            "SELECT um.universe_key, um.user_id, um.role, um.joined_at, u.email, u.display_name \
             FROM universe_members um LEFT JOIN users u ON um.user_id = u.id \
             WHERE um.universe_key = ?1 AND um.user_id = ?2",
            params![universe_key, user_id],
            |row| {
                Ok(crate::models::UniverseMember {
                    universe_key: row.get(0)?,
                    user_id: row.get(1)?,
                    role: row.get(2)?,
                    joined_at: parse_datetime(&row.get::<_, String>(3)?),
                    email: row.get(4)?,
                    display_name: row.get(5)?,
                })
            },
        )?;
        Ok(member)
    }

    pub fn remove_universe_member(
        &mut self,
        universe_key: &str,
        user_id: &str,
    ) -> anyhow::Result<()> {
        // Prevent removing the owner
        let universe = self
            .get_universe(universe_key)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", universe_key))?;
        if universe.owner_id == user_id {
            anyhow::bail!("Cannot remove the owner from a universe");
        }
        self.conn.execute(
            "DELETE FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
            params![universe_key, user_id],
        )?;
        Ok(())
    }

    // --- Universe member role helper ---

    /// Return the role of a user in a universe, or None if not a member.
    pub fn universe_member_role(&self, universe_key: &str, user_id: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT role FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
                params![universe_key, user_id],
                |row| row.get(0),
            )
            .ok()
    }

    // --- CO-49: Subscriptions ---

    /// Subscribe a user to a public-subscribable universe.
    pub fn subscribe_universe(&mut self, user_id: &str, universe_key: &str) -> anyhow::Result<()> {
        let universe = self
            .get_universe(universe_key)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", universe_key))?;
        if universe.visibility != "public-subscribable" {
            anyhow::bail!("Universe '{}' is not public-subscribable", universe_key);
        }
        let now_str = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO subscriptions (user_id, universe_key, subscribed_at) \
             VALUES (?1, ?2, ?3)",
            params![user_id, universe_key, now_str],
        )?;
        Ok(())
    }

    /// Unsubscribe a user from a universe.
    pub fn unsubscribe_universe(
        &mut self,
        user_id: &str,
        universe_key: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM subscriptions WHERE user_id = ?1 AND universe_key = ?2",
            params![user_id, universe_key],
        )?;
        Ok(())
    }

    /// Check whether a user is subscribed to a universe.
    pub fn is_subscribed(&self, user_id: &str, universe_key: &str) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM subscriptions WHERE user_id = ?1 AND universe_key = ?2",
                params![user_id, universe_key],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    /// List subscribers for a universe.
    pub fn list_universe_subscribers(
        &self,
        universe_key: &str,
    ) -> Vec<crate::models::Subscription> {
        let mut stmt = match self.conn.prepare(
            "SELECT user_id, universe_key, subscribed_at FROM subscriptions \
             WHERE universe_key = ?1 ORDER BY subscribed_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![universe_key], |row| {
            Ok(crate::models::Subscription {
                user_id: row.get(0)?,
                universe_key: row.get(1)?,
                subscribed_at: parse_datetime(&row.get::<_, String>(2)?),
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Search public-subscribable universes by name/description.
    pub fn search_public_universes(&self, query: &str) -> Vec<crate::models::Universe> {
        let pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = match self.conn.prepare(
            "SELECT key, name, description, owner_id, created_at, is_template, is_public, content_count, \
             COALESCE(requires_login, 0), COALESCE(visibility, 'private') \
             FROM universes \
             WHERE visibility = 'public-subscribable' \
             AND (LOWER(name) LIKE ?1 OR LOWER(description) LIKE ?1 OR LOWER(key) LIKE ?1) \
             ORDER BY content_count DESC LIMIT 50",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![pattern], |row| {
            Ok(crate::models::Universe {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                owner_id: row.get(3)?,
                created_at: parse_datetime(&row.get::<_, String>(4)?),
                is_template: row.get::<_, i64>(5)? != 0,
                is_public: row.get::<_, i64>(6)? != 0,
                content_count: row.get::<_, i64>(7).unwrap_or(0),
                requires_login: row.get::<_, i64>(8).unwrap_or(0) != 0,
                visibility: row
                    .get::<_, String>(9)
                    .unwrap_or_else(|_| "public-subscribable".into()),
                // CO-98: search results don't carry parent_key (not load-bearing
                // for the search UX); leave as None for resilience.
                parent_key: None,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    // --- CO-49: Deterministic access check ---

    /// Check the access level for a user (or anonymous) on a given universe.
    ///
    /// Implements the 7-step deterministic check from the CO-49 spec:
    /// 1. template → READ for everyone
    /// 2. owner → READ+WRITE
    /// 3. member with write role (owner/admin/editor) → READ+WRITE
    /// 4. member with read role (viewer/member) → READ
    /// 5. subscribed → READ
    /// 6. public-subscribable → metadata only
    /// 7. otherwise → Denied (404)
    pub fn check_universe_access(
        &self,
        user_id: Option<&str>,
        universe_key: &str,
    ) -> crate::models::UniverseAccess {
        use crate::models::UniverseAccess;

        let universe = match self.get_universe(universe_key) {
            Some(u) => u,
            None => return UniverseAccess::Denied,
        };

        // 1. Template / public-static universes are readable by everyone.
        // `public-static` is the visibility for the seeded read-only public
        // universes (template, timeline trio, etc.) — content is curated and
        // never login-gated.
        if universe.visibility == "template" || universe.visibility == "public-static" {
            return UniverseAccess::ReadOnly;
        }

        // 2–5 require a known user_id.
        if let Some(uid) = user_id {
            // 2. Owner gets full access.
            if universe.owner_id == uid {
                return UniverseAccess::ReadWrite;
            }

            // 3–4. Members: check role.
            if let Some(role) = self.universe_member_role(universe_key, uid) {
                let write_roles = ["owner", "admin", "editor"];
                if write_roles.contains(&role.as_str()) {
                    return UniverseAccess::ReadWrite;
                }
                return UniverseAccess::ReadOnly;
            }

            // 5. Subscribed users get read access.
            if self.is_subscribed(uid, universe_key) {
                return UniverseAccess::ReadOnly;
            }
        }

        // 6. Public-subscribable: show metadata to anyone (for discovery).
        if universe.visibility == "public-subscribable" {
            return UniverseAccess::MetadataOnly;
        }

        // requires_login: any logged-in user gets read access;
        // anonymous users get LoginRequired (401).
        if universe.visibility == "requires_login" || universe.requires_login {
            if user_id.is_some() {
                return UniverseAccess::ReadOnly;
            }
            return UniverseAccess::LoginRequired;
        }

        // 7. Everything else is denied.
        UniverseAccess::Denied
    }

    // --- Usage gate / content count ---

    /// Return the universe_key for a given project key, or None if not found.
    ///
    /// CO-77: primary lookup is `project_universe_index` in meta.db.
    /// Falls back to scanning open universe connections if not indexed yet.
    pub fn get_project_universe_key(&self, project_key: &str) -> Option<String> {
        let upper = project_key.to_uppercase();

        // Fast path: project_universe_index
        if let Ok(uk) = self.conn.query_row(
            "SELECT universe_key FROM project_universe_index WHERE project_key = ?1",
            params![upper],
            |row| row.get::<_, String>(0),
        ) {
            return Some(uk);
        }

        // Fallback: check meta.db entries (pre-CO-77 rows)
        let path = format!("projects/{}/_project.md", upper);
        self.conn
            .query_row(
                "SELECT universe_key FROM entries WHERE entry_type = 'project' AND path = ?1",
                params![path],
                |row| row.get(0),
            )
            .ok()
    }

    /// Increment content_count for a universe and return the new value.
    pub fn increment_universe_content_count(&mut self, universe_key: &str) -> i64 {
        self.conn
            .execute(
                "UPDATE universes SET content_count = content_count + 1 WHERE key = ?1",
                params![universe_key],
            )
            .ok();
        self.conn
            .query_row(
                "SELECT content_count FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Decrement content_count for a universe by `by`, flooring at 0.
    pub fn decrement_universe_content_count(&mut self, universe_key: &str, by: i64) {
        self.conn
            .execute(
                "UPDATE universes SET content_count = MAX(0, content_count - ?1) WHERE key = ?2",
                params![by, universe_key],
            )
            .ok();
    }

    /// CO-80: Sum of content_count across all universes owned by `user_id`.
    /// Used for tier storage quota checks.
    pub fn count_user_entries(&self, user_id: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(content_count), 0) FROM universes \
                 WHERE owner_id = ?1 AND is_template = 0",
                params![user_id],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// CO-80: Count non-template universes owned by `user_id`.
    /// Used for tier universe count quota checks.
    pub fn count_user_universes(&self, user_id: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM universes WHERE owner_id = ?1 AND is_template = 0",
                params![user_id],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Count comments for a specific task.
    pub fn count_task_comments(&self, project_key: &str, task_id: u64) -> i64 {
        let upper = project_key.to_uppercase();
        let like_pattern = format!("projects/{}/comments/{}-%%", upper, task_id);
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE entry_type = 'comment' AND path LIKE ?1",
                params![like_pattern],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Count all tasks and their comments for a project (used for delete_project decrement).
    pub fn count_project_content(&self, project_key: &str) -> i64 {
        let upper = project_key.to_uppercase();
        let tasks: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries \
                 WHERE entry_type = 'task' \
                 AND json_extract(frontmatter_json, '$.project') = ?1",
                params![upper],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let comments: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries \
                 WHERE entry_type = 'comment' \
                 AND json_extract(frontmatter_json, '$.project') = ?1",
                params![upper],
                |row| row.get(0),
            )
            .unwrap_or(0);
        tasks + comments
    }

    /// Claim an anonymous universe: transfer ownership to a real user.
    /// `anon_id` must match the universe's current owner_id (must start with "anon-").
    pub fn claim_universe(
        &mut self,
        slug: &str,
        user_id: &str,
        anon_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        let universe = self
            .get_universe(slug)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", slug))?;

        if !universe.owner_id.starts_with("anon-") {
            anyhow::bail!("Universe '{}' is not an anonymous universe", slug);
        }
        if universe.owner_id != anon_id {
            anyhow::bail!("Owner cookie does not match universe owner");
        }

        let now_str = Utc::now().to_rfc3339();

        // Transfer ownership
        self.conn.execute(
            "UPDATE universes SET owner_id = ?1 WHERE key = ?2",
            params![user_id, slug],
        )?;

        // Replace anon member with real user
        self.conn.execute(
            "DELETE FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
            params![slug, anon_id],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', ?3)",
            params![slug, user_id, now_str],
        )?;

        self.get_universe(slug)
            .ok_or_else(|| anyhow::anyhow!("Universe not found after claim"))
    }

    // --- Universe form config (CO-24) ---

    /// Get the presentation config for a universe (theme, layout, fonts, tokens).
    pub fn get_universe_form_config(&self, key: &str) -> Option<crate::models::UniverseFormConfig> {
        self.conn
            .query_row(
                "SELECT theme_preset, layout, font_headline, font_body, custom_tokens \
                 FROM universes WHERE key = ?1",
                params![key],
                |row| {
                    let tokens_str: Option<String> = row.get(4)?;
                    Ok(crate::models::UniverseFormConfig {
                        theme_preset: row.get::<_, String>(0)?,
                        layout: row.get::<_, String>(1)?,
                        font_headline: row.get(2)?,
                        font_body: row.get(3)?,
                        custom_tokens: tokens_str.and_then(|s| serde_json::from_str(&s).ok()),
                    })
                },
            )
            .ok()
    }

    /// Apply a partial update to the form config and sync `.universo.yaml`.
    pub fn update_universe_form_config(
        &mut self,
        key: &str,
        update: crate::models::UpdateUniverseFormConfig,
    ) -> anyhow::Result<crate::models::UniverseFormConfig> {
        let mut config = self
            .get_universe_form_config(key)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", key))?;

        if let Some(tp) = update.theme_preset {
            config.theme_preset = tp;
        }
        if let Some(l) = update.layout {
            config.layout = l;
        }
        // Empty string clears the font; a value sets it.
        if let Some(fh) = update.font_headline {
            config.font_headline = if fh.is_empty() { None } else { Some(fh) };
        }
        if let Some(fb) = update.font_body {
            config.font_body = if fb.is_empty() { None } else { Some(fb) };
        }
        // An explicit `null` in JSON becomes None here and clears the tokens.
        config.custom_tokens = update.custom_tokens;

        let tokens_str = config.custom_tokens.as_ref().map(|v| v.to_string());

        self.conn.execute(
            "UPDATE universes SET theme_preset = ?1, layout = ?2, \
             font_headline = ?3, font_body = ?4, custom_tokens = ?5 \
             WHERE key = ?6",
            params![
                config.theme_preset,
                config.layout,
                config.font_headline,
                config.font_body,
                tokens_str,
                key,
            ],
        )?;

        // Sync to .universo.yaml (best-effort; never fails the request).
        let _ = self.write_universo_yaml(key, &config);

        Ok(config)
    }

    /// Write form config to `.universo.yaml` at the universe vault root.
    fn write_universo_yaml(
        &self,
        universe_key: &str,
        config: &crate::models::UniverseFormConfig,
    ) -> anyhow::Result<()> {
        let root = self.universe_root(universe_key);
        std::fs::create_dir_all(&root)?;
        let yaml_path = root.join(".universo.yaml");

        let mut yaml = format!(
            "theme_preset: {}\nlayout: {}\n",
            config.theme_preset, config.layout
        );
        if let Some(fh) = &config.font_headline {
            yaml.push_str(&format!("font_headline: {fh}\n"));
        }
        if let Some(fb) = &config.font_body {
            yaml.push_str(&format!("font_body: {fb}\n"));
        }
        if let Some(tokens) = &config.custom_tokens {
            yaml.push_str(&format!("custom_tokens: {tokens}\n"));
        }

        std::fs::write(yaml_path, yaml)?;
        Ok(())
    }

    // --- Check if data exists ---

    pub fn has_data(&self) -> bool {
        // Primary: project_universe_index has rows → projects exist → real data.
        // This is fast and correct for both fresh installs and seeded test DBs.
        let idx: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM project_universe_index", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        if idx > 0 {
            return true;
        }
        // Secondary: user-created universes beyond 'default' and 'template'.
        // Guards the CO-77 first-boot edge case where project_universe_index is
        // momentarily empty on a pre-CO-77 prod DB before
        // maybe_migrate_entries_to_universe_dbs populates it (prod incident 2026-05-01).
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM universes WHERE key NOT IN ('default', 'template')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0i64)
            > 0
    }

    // --- Template universe ---

    /// Returns true if a template universe already exists (seed already ran).
    pub fn template_exists(&self) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM universes WHERE is_template = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    /// Seed the template universe with interactive tutorial tasks.
    /// Safe to call multiple times — checks if project entry already exists.
    pub fn seed_template_universe(&mut self) {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Template universe with Modern theme (default) + board layout.
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              visibility, theme_preset, layout) \
             VALUES ('template', 'Co', \
             'Aprenda a usar o Co — arraste, crie e explore', \
             'system', ?1, 1, 1, 'template', 'modern', 'board')",
            params![now_str],
        );
        // Ensure form config YAML is written for the template.
        if let Some(config) = self.get_universe_form_config("template") {
            let _ = self.write_universo_yaml("template", &config);
        }

        // Check if project entry already exists (query per-universe DB — CO-77).
        let proj_path = "projects/CO/_project.md";
        let template_uc = self.universe_pool.get_or_open("template");
        let already_seeded: bool = {
            let uc_guard = template_uc.lock().expect("template universe conn lock");
            uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE universe_key = 'template' AND path = ?1",
                    params![proj_path],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0
        };

        if already_seeded {
            return;
        }

        // Create project entry
        let proj_fm = json!({
            "type": "project",
            "key": "CO",
            "title": "Bem-vindo ao Co",
            "status": "active",
            "next_id": 10,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "tags": ["onboarding"]
        });
        let proj_entry = make_entry(
            proj_path,
            proj_fm,
            "Cada cartão é uma ideia. Arraste, crie, explore.",
        );
        let universe_root = self.universe_root("template");
        let _ = co::entry::write_entry(&universe_root, &proj_entry);
        {
            let uc_guard = template_uc.lock().expect("template universe conn lock");
            let _ = upsert_entry_row(&uc_guard, "template", &proj_entry);
        }
        // Register in project_universe_index so get_project() works
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) VALUES ('CO', 'template')",
            [],
        );

        // Onboarding tasks — game tutorial tone, curiosity-driven
        // Content always in Portuguese. UI labels translate via i18n.
        struct SeedTask {
            id: i64,
            title: &'static str,
            description: &'static str,
            status: &'static str,
            priority: &'static str,
            labels: Vec<&'static str>,
            due_days: Option<i64>,
            parent: Option<i64>,
        }

        let tasks = [
            // --- Act 1: First contact ---
            SeedTask {
                id: 1,
                title: "Mova este cartão para Concluído",
                description: "Você acabou de chegar. Que tal começar com algo simples?\n\nArraste este cartão direto para a coluna **Concluído**.\n\nPronto — você já terminou sua primeira tarefa no Co. Cada coluna representa um estado. Mova os cartões conforme avança.",
                status: "todo",
                priority: "high",
                labels: vec!["inicio"],
                due_days: None,
                parent: None,
            },
            // --- Act 2: Make it yours ---
            SeedTask {
                id: 2,
                title: "Crie algo seu",
                description: "Clique em **+ Nova Tarefa** e escreva o que vier à mente.\n\nPode ser uma ideia, um lembrete, um projeto. A descrição aceita **Markdown** — negrito, listas, links, código.\n\nCada tarefa vira um arquivo `.md` que você pode abrir no Obsidian, editar no VS Code, ou versionar no Git.",
                status: "todo",
                priority: "high",
                labels: vec!["inicio"],
                due_days: None,
                parent: None,
            },
            SeedTask {
                id: 3,
                title: "Quebre em partes menores",
                description: "Toda grande ideia começa com um passo pequeno.\n\nAbra uma tarefa e escolha um **pai** no campo \"Tarefa Pai\". A subtarefa aparece aninhada no Kanban — clique no triângulo para expandir.\n\nVocê pode criar quantos níveis quiser.",
                status: "todo",
                priority: "medium",
                labels: vec!["inicio"],
                due_days: None,
                parent: Some(2),
            },
            // --- Act 3: Discover ---
            SeedTask {
                id: 4,
                title: "Escolha um visual",
                description: "Cada universo tem sua identidade. Use o seletor de tema no cabeçalho para experimentar:\n\n- **Scholarly** — editorial acadêmico, tons de cobre\n- **Relic** — cinema escuro, rosa e ouro\n- **Cyberpunk** — neon sobre noite\n- **Garden** — verde orgânico\n- **Matrix** — fósforo sobre preto\n- **Terminal** — minimalismo absoluto\n\nSão 12 temas. Cada um transforma completamente a interface.",
                status: "todo",
                priority: "medium",
                labels: vec!["explorar"],
                due_days: None,
                parent: None,
            },
            SeedTask {
                id: 5,
                title: "Veja de outro ângulo",
                description: "Os mesmos dados, apresentados de formas diferentes. Alterne entre as abas:\n\n- **Kanban** — visão espacial, arraste entre colunas\n- **Tabela** — lista ordenável, filtros rápidos\n- **Painel** — visão geral, métricas\n- **Conteúdo** — seus textos como artigos\n\nO Conteúdo é especial: cada descrição de tarefa é um texto Markdown completo. Escreva documentação, notas, artigos — tudo organizado no mesmo lugar.",
                status: "todo",
                priority: "medium",
                labels: vec!["explorar"],
                due_days: None,
                parent: None,
            },
            // --- Act 4: Understand the system ---
            SeedTask {
                id: 6,
                title: "Entenda o que é Conteúdo",
                description: "No CO, **tudo é conteúdo**. Uma tarefa é um arquivo Markdown com metadados (título, status, prioridade) no cabeçalho.\n\n```yaml\n---\ntype: task\ntitle: Minha tarefa\nstatus: todo\ntags: [projeto, ideia]\n---\n```\n\nIsso significa que seu quadro de tarefas é também um banco de dados de textos. Abra a aba **Conteúdo** para ver seus cartões como artigos.\n\nVocê pode sincronizar com o **Obsidian**, editar no seu editor favorito, ou acessar via API. O conteúdo é seu — em Markdown, sempre portátil.",
                status: "todo",
                priority: "low",
                labels: vec!["explorar", "conteudo"],
                due_days: None,
                parent: None,
            },
            SeedTask {
                id: 7,
                title: "Troque o idioma da interface",
                description: "A interface do CO funciona em **Português** e **English**.\n\nClique no botão de idioma no cabeçalho. Os rótulos da interface mudam, mas o conteúdo (seus textos, descrições, títulos) permanece como você escreveu.\n\nIsso porque conteúdo é seu — a interface é só a moldura.",
                status: "todo",
                priority: "low",
                labels: vec!["explorar"],
                due_days: None,
                parent: None,
            },
            // --- Act 5: Join ---
            SeedTask {
                id: 8,
                title: "Faça parte",
                description: "Tudo o que você fez até agora está salvo neste navegador.\n\nQuando criar uma conta, seu universo ganha um endereço permanente que você pode compartilhar. Outras pessoas podem colaborar em tempo real — com cursores visíveis e edição simultânea.\n\nSeu conteúdo continua sendo Markdown. Seu universo continua sendo seu.\n\n**Crie uma conta gratuita** para salvar, compartilhar e colaborar.",
                status: "todo",
                priority: "critical",
                labels: vec!["acao"],
                due_days: None,
                parent: None,
            },
            // --- Bonus: hidden depth ---
            SeedTask {
                id: 9,
                title: "Conecte com o Obsidian",
                description: "Se você usa Obsidian, pode sincronizar este universo como um vault.\n\nCada tarefa vira uma nota `.md` com frontmatter YAML. Subtarefas viram `[[wikilinks]]`. Tags viram #tags.\n\nInstale o plugin **CO Universe Sync** no Obsidian e conecte com sua conta. Seus dados fluem entre o CO e o Obsidian sem atrito.\n\nDataview queries funcionam nativamente:\n\n```dataview\nTABLE status, priority\nFROM \"projects\"\nWHERE type = \"task\" AND status != \"done\"\nSORT priority DESC\n```",
                status: "todo",
                priority: "low",
                labels: vec!["avancado", "obsidian"],
                due_days: None,
                parent: None,
            },
        ];

        for t in &tasks {
            let created_at = (now - chrono::Duration::days(30 - t.id * 3)).to_rfc3339();
            let updated_at = (now - chrono::Duration::days(5)).to_rfc3339();
            let due_date: Option<String> = t.due_days.map(|d| {
                (now + chrono::Duration::days(d))
                    .format("%Y-%m-%d")
                    .to_string()
            });
            let task_path = format!("projects/CO/{}.md", t.id);
            let labels: Vec<serde_json::Value> = t.labels.iter().map(|l| json!(l)).collect();
            let task_fm = json!({
                "type": "task",
                "id": t.id,
                "title": t.title,
                "status": t.status,
                "priority": t.priority,
                "due": due_date,
                "parent": t.parent,
                "tags": labels,
                "created": created_at,
                "modified": updated_at,
                "archived": false,
                "assignee": null,
                "project": "CO"
            });
            let task_entry = make_entry(&task_path, task_fm, t.description);
            let _ = co::entry::write_entry(&universe_root, &task_entry);
            {
                let uc_guard = template_uc.lock().expect("template universe conn lock");
                let _ = upsert_entry_row(&uc_guard, "template", &task_entry);
            }
        }

        // Seed/refresh the template's content pages (intro + legal).
        // Extracted so it can also run unconditionally on each startup.
        self.reseed_template_content_pages();
    }

    /// Force the template universe's `theme_preset` to a known value.
    ///
    /// Earlier migrations defaulted `theme_preset` to `'scholarly-light'` and
    /// updated the existing template row to match — even though the seed code
    /// today uses `'modern'`. Because the row is then `INSERT OR IGNORE`d on
    /// every boot, the migration value is sticky. This setter overrides it on
    /// every startup so the template page is consistently rendered with the
    /// product's intended default look.
    pub fn ensure_template_theme_preset(&self, preset: &str) {
        let _ = self.conn.execute(
            "UPDATE universes SET theme_preset = ?1 WHERE key = 'template'",
            params![preset],
        );
    }

    /// Always-overwrite seed of the template universe's content pages from the
    /// embedded `seed/template/*.md` files.
    ///
    /// Called both from `seed_template_universe()` (first-boot path) and on
    /// every server startup, so the binary's bundled legal/intro content is
    /// the source of truth — even when the database already exists from a
    /// prior version. `upsert_entry_row` does an `INSERT OR REPLACE`, so this
    /// is safe to call repeatedly.
    pub fn reseed_template_content_pages(&mut self) {
        if !self.template_exists() {
            return; // template universe not seeded yet — first-boot path will handle it
        }
        let now_str = Utc::now().to_rfc3339();
        let universe_root = self.universe_root("template");

        for (path, md) in [
            ("index.md", SEED_TEMPLATE_INDEX_MD),
            ("content/sobre.md", SEED_SOBRE_MD),
            ("content/termos.md", SEED_TERMOS_MD),
            ("content/privacidade.md", SEED_PRIVACIDADE_MD),
            ("content/dados-rastreados.md", SEED_DADOS_RASTREADOS_MD),
            ("content/linhas-do-tempo.md", SEED_LINHAS_DO_TEMPO_MD),
        ] {
            let entry = make_entry(
                path,
                seed_page_frontmatter(md, &now_str),
                seed_page_body(md),
            );
            if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                tracing::warn!("Failed to write {path} file: {e}");
            }
            let template_uc = self.universe_pool.get_or_open("template");
            let uc_guard = template_uc.lock().expect("template universe conn lock");
            if let Err(e) = upsert_entry_row(&uc_guard, "template", &entry) {
                tracing::warn!("Failed to upsert {path} page: {e}");
            }
        }
    }

    // --- Quilombo Araucária universe ---

    /// Returns true if the quilomboaraucaria universe already exists.
    pub fn quilombo_universe_exists(&self) -> bool {
        self.get_universe("quilomboaraucaria").is_some()
    }

    /// Seed the `quilomboaraucaria` public universe (CO-41).
    ///
    /// Creates the universe with is_public=1, visibility='public-subscribable',
    /// owner_id='system', and the quilombo theme preset.
    /// Safe to call multiple times — idempotent via INSERT OR IGNORE.
    pub fn seed_quilombo_universe(&mut self) {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Custom tokens for the quilombo-blog design system
        let custom_tokens = serde_json::json!({
            "--bg": "#f5f0e8",
            "--card-bg": "#faf6ef",
            "--border": "#c8b48e",
            "--accent": "#2d4a22",
            "--text-muted": "#8b6914"
        });
        let tokens_str = custom_tokens.to_string();

        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              visibility, theme_preset, layout, font_headline, font_body, custom_tokens, content_count) \
             VALUES ('quilomboaraucaria', 'Quilombo Araucária', \
             'Comunidade quilombola do Paraná — publicações, eventos e missões', \
             'system', ?1, 0, 1, 'public-subscribable', 'quilombo', 'board', \
             'Playfair Display', 'Inter', ?2, 0)",
            params![now_str, tokens_str],
        );

        // Sync .universo.yaml
        if let Some(config) = self.get_universe_form_config("quilomboaraucaria") {
            let _ = self.write_universo_yaml("quilomboaraucaria", &config);
        }
    }

    /// Import markdown files from the quilomboaraucaria content repo into the universe.
    #[allow(dead_code)]
    fn import_quilombo_content(&mut self) {
        // Look for the repo in common locations
        let candidates = [
            std::path::PathBuf::from("/app/seed-co/quilomboaraucaria"),
            std::path::PathBuf::from("quilomboaraucaria"),
            dirs::home_dir()
                .unwrap_or_default()
                .join("projects/quilomboaraucaria"),
        ];
        let repo = candidates.iter().find(|p| p.join("schema.yaml").exists());
        let repo = match repo {
            Some(r) => r.clone(),
            None => {
                tracing::info!("quilomboaraucaria content repo not found, skipping import");
                return;
            }
        };
        tracing::info!(
            "Importing quilomboaraucaria content from {}",
            repo.display()
        );

        let now = Utc::now().to_rfc3339();
        let universe_root = self.universe_root("quilomboaraucaria");

        // Map folder → entry type
        let folder_types = [
            ("eventos", "event"),
            ("relatos", "post"),
            ("jardim", "page"),
            ("membros", "member"),
            ("quadro", "task"),
        ];

        let mut count: i64 = 0;
        for (folder, entry_type) in &folder_types {
            let dir = repo.join(folder);
            if !dir.is_dir() {
                continue;
            }
            let entries = std::fs::read_dir(&dir);
            if entries.is_err() {
                continue;
            }

            for entry in entries.unwrap().flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                // Parse frontmatter
                let parsed = co::entry::Entry::parse_frontmatter(&content);
                let (mut fm, body) = match parsed {
                    Some((f, b)) => (f, b),
                    None => {
                        // No frontmatter — treat entire content as body
                        let fm = json!({"type": entry_type});
                        (fm, content.clone())
                    }
                };

                // Ensure type field
                if let Some(obj) = fm.as_object_mut() {
                    if !obj.contains_key("type") {
                        obj.insert("type".to_string(), json!(entry_type));
                    }
                    // Map Portuguese fields to standard
                    if let Some(titulo) = obj.remove("titulo") {
                        obj.entry("title".to_string()).or_insert(titulo);
                    }
                    if !obj.contains_key("created") {
                        obj.insert("created".to_string(), json!(now));
                    }
                    if !obj.contains_key("modified") {
                        obj.insert("modified".to_string(), json!(now));
                    }
                }

                let title = fm
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                let entry_path = format!("content/{}/{}.md", folder, filename);

                let entry = make_entry(&entry_path, fm, &body);
                let _ = co::entry::write_entry(&universe_root, &entry);
                let _ = upsert_entry_row(&self.conn, "quilomboaraucaria", &entry);
                count += 1;

                if !title.is_empty() {
                    tracing::debug!("  imported: {}/{} — {}", folder, filename, title);
                }
            }
        }

        // Update content count
        let _ = self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = 'quilomboaraucaria'",
            params![count],
        );

        tracing::info!("quilomboaraucaria: imported {} entries", count);
    }

    /// Stats for the quilomboaraucaria universe.
    ///
    /// Entry counts come from the SQLite index; totalUsuarios, versaoApp and
    /// ultimaSync are read from the `meta/stats.md` metadata entry written by
    /// the importer on each run. Returns a zeroed struct if the universe has no
    /// content yet.
    pub fn quilombo_stats(&self) -> crate::models::QuilomboStats {
        let count_by_type = |entry_type: &str| -> i64 {
            self.conn
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE universe_key = 'quilomboaraucaria' AND entry_type = ?1",
                    params![entry_type],
                    |row| row.get(0),
                )
                .unwrap_or(0)
        };

        let total_publicacoes = count_by_type("post");
        let total_eventos = count_by_type("event");
        let total_missoes = count_by_type("mission");

        // Read metadata from meta/stats.md entry
        let meta: Option<(i64, String, String)> = self
            .conn
            .query_row(
                "SELECT frontmatter_json FROM entries \
                 WHERE universe_key = 'quilomboaraucaria' AND path = 'meta/stats.md'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok())
            .and_then(|fm| {
                let usuarios = fm.get("total_usuarios")?.as_i64()?;
                let versao = fm.get("versao_app")?.as_str()?.to_string();
                let sync = fm.get("ultima_sync")?.as_str()?.to_string();
                Some((usuarios, versao, sync))
            });

        let (total_usuarios, versao_app, ultima_sync) = match meta {
            Some((u, v, s)) => (u, v, Some(s)),
            None => (0, "—".to_string(), None),
        };

        crate::models::QuilomboStats {
            total_usuarios,
            total_publicacoes,
            total_eventos,
            total_missoes,
            versao_app,
            ultima_sync,
        }
    }

    // --- Yggdrasil universe (CO-38) ---

    /// Returns true if the yggdrasil universe already exists.
    pub fn yggdrasil_universe_exists(&self) -> bool {
        self.get_universe("yggdrasil").is_some()
    }

    /// Seed the `yggdrasil` special universe — the minigames hub (CO-38).
    ///
    /// `is_public=1`, `requires_login=1`, `owner='system'`, Relic Dark theme,
    /// layout='gaming'. Safe to call multiple times — idempotent via INSERT OR IGNORE.
    pub fn seed_yggdrasil_universe(&mut self) {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              requires_login, visibility, theme_preset, layout, font_headline, font_body, content_count) \
             VALUES ('yggdrasil', 'Yggdrasil', \
             'Hub de minijogos — perfis de jogadores e rankings globais', \
             'system', ?1, 0, 1, 1, 'requires_login', 'relic', 'gaming', \
             'Newsreader', 'Manrope', 0)",
            params![now_str],
        );
        tracing::info!("Yggdrasil universe seeded (key=yggdrasil, requires_login=true)");
    }

    // --- CO Dev universe (CO-53 / CO-140) ---

    /// Seed the `co-dev` private universe — the CO platform development board.
    ///
    /// Owned by 'system', private, scholarly-dark, board layout.
    /// `ensure_admin_universe_memberships` makes Yuri a member so it appears
    /// in his sidebar. Idempotent via INSERT OR IGNORE.
    /// Ensure admin-owned content universes exist — idempotent, runs every boot.
    ///
    /// Creates artelonga, rfq, and co universes owned by any admin-tier user so
    /// they appear in the sidebar without manual API calls.  Content is pushed
    /// separately via the Vault API or `co push`; this only guarantees the DB row.
    pub fn seed_admin_content_universes(&mut self) {
        let now = Utc::now().to_rfc3339();

        for (key, name, desc, vis) in [
            (
                "artelonga",
                "ArteLonga",
                "Arte Longa — conteúdo público, portfólio e presença digital",
                "public-subscribable",
            ),
            (
                "rfq",
                "RFQ Gateway",
                "Plataforma de cotações e registro de negociações",
                "private",
            ),
            (
                "co",
                "Co Platform",
                "Board público do Co — roadmap, releases e decisões",
                "public-subscribable",
            ),
            // CO-141: meaning-topology universes. Each language plane is its
            // own universe key; `concepts` is the language-agnostic anchor
            // plane; `mbya` is the Arandu Mbyá Guarani lexicon (separate from
            // the shallow `guarani-mbya` cross-language plane).
            (
                "mbya",
                "Arandu — Mbyá Guarani",
                "Lexicon and learning content for Mbyá Guarani (Arandu project)",
                "public-subscribable",
            ),
            // Topologia universes are private for now — they're under
            // active authoring with non-native draft entries that need
            // review before being open to anonymous readers. Flip to
            // public-subscribable when seed_status passes review.
            (
                "concepts",
                "Concepts (topologia)",
                "Language-agnostic meaning anchors — the meta plane onto which language-specific terms project",
                "private",
            ),
            (
                "guarani-mbya",
                "Guarani Mbyá (topologia)",
                "Mbyá Guarani term plane — shallow cross-language anchor layer above the Arandu lexicon",
                "private",
            ),
            (
                "portuguese",
                "Portuguese (topologia)",
                "Portuguese term plane in the meaning-topology",
                "private",
            ),
            (
                "yoruba",
                "Yoruba (topologia)",
                "Yoruba term plane in the meaning-topology",
                "private",
            ),
            (
                "languages",
                "Languages catalog (topologia)",
                "Centralized queryable index of every language plane — code, family, Glottolog/SAPhon authority links, geographic centroid, speaker estimate, and cross-ref to the term plane",
                "private",
            ),
        ] {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, is_template, is_public, \
                  visibility, theme_preset, layout, content_count) \
                 VALUES (?1, ?2, ?3, 'system', ?4, 0, 0, ?5, 'scholarly-light', 'board', 0)",
                rusqlite::params![key, name, desc, now, vis],
            );
            // CO-143 follow-up (2026-05-02): reconcile visibility to declared
            // intent on every boot. INSERT OR IGNORE above doesn't update
            // existing rows, so a row created with an old default (e.g.
            // 'private' before the public-subscribable convention) would
            // silently stay wrong forever. Surfaced as: artelonga returning
            // 404 to anonymous despite the seed declaring public-subscribable.
            //
            // Only updates when the stored visibility doesn't match.
            // is_public bit kept in sync (0 for private, 1 otherwise) so
            // legacy callers checking that flag also see the intended state.
            let is_public_bit: i64 = if vis == "private" { 0 } else { 1 };
            let _ = self.conn.execute(
                "UPDATE universes SET visibility = ?2, is_public = ?3 \
                 WHERE key = ?1 AND (visibility != ?2 OR is_public != ?3)",
                rusqlite::params![key, vis, is_public_bit],
            );
            // Assign every admin user as owner of these universes.
            // membership is wired by ensure_admin_universe_memberships at startup.
        }
    }

    pub fn seed_co_dev_universe(&mut self) {
        let now = Utc::now().to_rfc3339();
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              requires_login, visibility, theme_preset, layout, content_count) \
             VALUES ('co-dev', 'CO Dev', \
             'CO platform development board — all tickets, sprints, and architecture', \
             'system', ?1, 0, 0, 1, 'requires_login', 'scholarly-dark', 'board', 0)",
            params![now],
        );
        // co-dev membership is handled by ensure_admin_universe_memberships at startup.
    }

    /// Ingest CO-*.md ticket files from `/app/seed-co/` (or any source dir) into
    /// the `co` universe's entries table at path `tasks/<filename>`.
    ///
    /// Mirrors `reseed_template_content_pages` shape: idempotent upsert, runs
    /// on every boot. Closes the "co has 0 entries" gap user-reported on
    /// 2026-05-02 — Phase E of CO-142 populated `/data/co/` for the dev_board
    /// admin scan, but the SPA's `/co/co` board reads from the per-universe
    /// `entries` table. This function bridges that.
    pub fn seed_co_universe_tasks(&mut self, source_dir: &std::path::Path) {
        if !source_dir.exists() {
            tracing::warn!(
                "seed_co_universe_tasks: source dir {} does not exist — skipped",
                source_dir.display()
            );
            return;
        }
        if self.get_universe("co").is_none() {
            tracing::warn!("seed_co_universe_tasks: 'co' universe row missing — skipped");
            return;
        }

        let universe_root = self.universe_root("co");
        let now_str = Utc::now().to_rfc3339();
        let mut upserted = 0usize;
        let mut skipped = 0usize;

        // Recursively walk source_dir. Path layout in the `co` universe:
        //   - top-level *.md  → tasks/<filename>   (legacy compat with 1.34.3)
        //   - subdir/<f>.md   → <subdir>/<f>       (CO-144: e.g. processos/)
        //
        // Files >2 levels deep are flattened to <leaf-subdir>/<filename> for
        // safety against pathological nesting; rare in practice.
        fn walk(
            dir: &std::path::Path,
            base: &std::path::Path,
        ) -> Vec<(std::path::PathBuf, String)> {
            let mut out = Vec::new();
            let Ok(read) = std::fs::read_dir(dir) else {
                return out;
            };
            for entry in read.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    out.extend(walk(&p, base));
                    continue;
                }
                if p.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let rel = match p.strip_prefix(base) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                // Compute entry path: depth-0 → tasks/<filename>; deeper → relative.
                let entry_path = if rel
                    .parent()
                    .map(|p| p.as_os_str().is_empty())
                    .unwrap_or(true)
                {
                    format!("tasks/{}", rel.display())
                } else {
                    rel.display().to_string()
                };
                out.push((p, entry_path));
            }
            out
        }

        let candidates = walk(source_dir, source_dir);

        for (path, entry_path) in candidates {
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => {
                    skipped += 1;
                    continue;
                }
            };
            let raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };

            let entry = make_entry(
                &entry_path,
                seed_page_frontmatter(&raw, &now_str),
                seed_page_body(&raw),
            );
            if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                tracing::warn!("seed_co_universe_tasks: write {filename}: {e}");
                skipped += 1;
                continue;
            }
            let co_uc = self.universe_pool.get_or_open("co");
            let uc_guard = co_uc.lock().expect("co universe conn lock");
            if let Err(e) = upsert_entry_row(&uc_guard, "co", &entry) {
                tracing::warn!("seed_co_universe_tasks: upsert {filename}: {e}");
                skipped += 1;
                continue;
            }
            upserted += 1;
        }

        tracing::info!(
            "seed_co_universe_tasks: upserted {upserted} task(s) from {} (skipped {skipped})",
            source_dir.display()
        );
    }

    /// Prune filesystem dirs under `/data/universes/` that no longer have a
    /// corresponding row in the `universes` table.
    ///
    /// Surfaces post-CO-142 cleanup: Phase C and Phase D hard-deleted DB rows
    /// (co-dev, co-experience, qa-dev, quilombo-blog{,-2,-3}, prodtest*) but
    /// the filesystem dirs persisted, accumulating cruft. This runs on every
    /// boot, after any seed/delete passes, so any orphaned dir is collected.
    ///
    /// Safety:
    /// - Only operates on dirs directly under `/data/universes/`
    /// - Skips a dir only if a row with that exact key exists (so anonymous
    ///   clones with hash-keys, plus all live system universes, are kept)
    /// - Idempotent — deleting an already-deleted dir is a no-op
    /// - Does NOT touch /data/co/, /data/meta.db, or any other top-level state
    pub fn prune_orphan_universe_dirs(&self) {
        // 2026-05-02 critical fix: the previous implementation iterated ALL
        // top-level dirs under /data/universes/ and deleted any whose name
        // didn't match a `universes.key` row. That was wrong — UniversePool
        // (CO-77) shards per-universe data.db files at:
        //
        //     /data/universes/<2-hex>/<2-hex>/<key>/data.db
        //
        // The 2-hex shard-prefix dirs (e.g. `68`, `b5`, `0e`) are NOT universe
        // keys — they're hash-prefix directories holding multiple per-universe
        // DB files. Deleting them wipes real universe data.
        //
        // This replacement is narrow on purpose: only deletes dirs whose key
        // matches the EXACT list of known-deprecated keys. Wider cleanup is
        // a manual ops task, not an unattended boot-time pass.
        const KNOWN_DEPRECATED_DIRS: &[&str] = &[
            // CO-142 Phase C deletions
            "co-dev",
            "co-experience",
            // CO-142 Phase D deletions
            "qa-dev",
            "quilombo-blog",
            "quilombo-blog-2",
            "quilombo-blog-3",
            // Test/anon residue without DB rows
            "prodtest1776629312",
            "anon-test-1775647138",
            "local-6zc952",
            "local-myks0v",
            "u-mruim7",
            "u-wd4zk2",
        ];

        let universes_root = self.data_dir.join("universes");
        let mut pruned = 0usize;
        for key in KNOWN_DEPRECATED_DIRS {
            let path = universes_root.join(key);
            if !path.exists() {
                continue;
            }
            // Defensive: only delete if NO `universes` row holds this key.
            let exists: i64 = self
                .conn
                .query_row(
                    "SELECT 1 FROM universes WHERE key = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if exists != 0 {
                continue;
            }
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!("prune_orphan_universe_dirs: failed to remove {}: {e}", key);
                continue;
            }
            pruned += 1;
            tracing::info!(
                "prune_orphan_universe_dirs: removed deprecated dir '{}'",
                key
            );
        }
        if pruned > 0 {
            tracing::info!("prune_orphan_universe_dirs: pruned {pruned} known-deprecated dir(s)");
        }
    }

    /// Re-ingest entries from filesystem .md files into per-universe `data.db`.
    ///
    /// 2026-05-02 recovery: the previous prune_orphan_universe_dirs deleted
    /// shard-prefix dirs (e.g. /data/universes/68/), which contained per-
    /// universe `data.db` files. The flat .md content survived (lives at
    /// /data/universes/<key>/), but the SQLite shards got recreated empty.
    /// This function walks /data/universes/<key>/**/*.md for every system
    /// universe and upserts each entry into its per-universe DB.
    ///
    /// Idempotent — uses upsert. Safe to run on every boot. Skipped for
    /// universes whose entry count is non-zero (already populated).
    pub fn rebuild_entries_from_filesystem(&mut self, keys: &[&str]) {
        let now_str = Utc::now().to_rfc3339();
        for key in keys {
            let universe_root = self.data_dir.join("universes").join(key);
            if !universe_root.exists() {
                continue;
            }
            // Skip if per-universe DB already has entries (avoids redundant work).
            let already_has: i64 = {
                let uc = self.universe_pool.get_or_open(key);
                let conn = uc.lock().expect("universe conn lock");
                conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
                    .unwrap_or(0)
            };
            if already_has > 0 {
                continue;
            }

            let mut count = 0usize;
            // Recursive walk of the universe's filesystem.
            let walk = walkdir(&universe_root);
            for fs_path in walk {
                let rel = match fs_path.strip_prefix(&universe_root) {
                    Ok(r) => r.to_path_buf(),
                    Err(_) => continue,
                };
                if rel.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let raw = match std::fs::read_to_string(&fs_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let entry_path = rel.to_string_lossy().to_string();
                let entry = make_entry(
                    &entry_path,
                    seed_page_frontmatter(&raw, &now_str),
                    seed_page_body(&raw),
                );
                let uc = self.universe_pool.get_or_open(key);
                let conn = uc.lock().expect("universe conn lock");
                if upsert_entry_row(&conn, key, &entry).is_ok() {
                    count += 1;
                }
            }
            if count > 0 {
                tracing::info!(
                    "rebuild_entries_from_filesystem: re-ingested {count} entr(ies) for '{key}' from filesystem"
                );
            }
        }
    }

    /// Phase C (CO-142): hard-delete the deprecated `co-dev` and `co-experience`
    /// universe rows and their membership records. Idempotent — DELETE WHERE
    /// is a no-op when the rows are already gone.
    pub fn delete_deprecated_universes(&mut self) {
        for key in ["co-dev", "co-experience"] {
            let _ = self.conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                params![key],
            );
            let deleted = self
                .conn
                .execute("DELETE FROM universes WHERE key = ?1", params![key])
                .unwrap_or(0);
            if deleted > 0 {
                tracing::info!("CO-142: deleted deprecated universe '{key}'");
            }
        }
    }

    /// Phase D (CO-142): hard-delete stale quilombo variant rows that have no
    /// documented purpose and accumulated via manual experiments. Idempotent.
    pub fn delete_stale_quilombo_variants(&mut self) {
        for key in [
            "quilombo-blog",
            "quilombo-blog-2",
            "quilombo-blog-3",
            "qa-dev",
        ] {
            let _ = self.conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                params![key],
            );
            let deleted = self
                .conn
                .execute("DELETE FROM universes WHERE key = ?1", params![key])
                .unwrap_or(0);
            if deleted > 0 {
                tracing::info!("CO-142: deleted stale quilombo variant '{key}'");
            }
        }
    }

    /// Phase B (CO-142): recompute `content_count` for every universe by counting
    /// rows in each per-universe entries DB. Runs on every boot so the column stays
    /// accurate even when seed paths bypass `increment_universe_content_count`.
    pub fn recompute_content_counts(&mut self) {
        let keys: Vec<String> = self
            .conn
            .prepare("SELECT key FROM universes")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get(0))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default();

        for key in &keys {
            let count: i64 = {
                let uc = self.universe_pool.get_or_open(key);
                let conn = uc.lock().expect("universe conn lock");
                conn.query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
                    .unwrap_or(0)
            };
            let _ = self.conn.execute(
                "UPDATE universes SET content_count = ?1 WHERE key = ?2",
                params![count, key],
            );
        }
        tracing::info!(
            "CO-142: recomputed content_count for {} universe(s)",
            keys.len()
        );
    }

    /// Returns true if the given timeline universe already exists.
    pub fn timeline_universe_exists(&self, key: &str) -> bool {
        self.get_universe(key).is_some()
    }

    /// Seed a single timeline universe from its JSON manifest + index markdown.
    ///
    /// The manifest defines: universe metadata (`key`, `name`, `description`,
    /// `theme_preset`, `layout`) and an array of `events` each with
    /// `slug`, `title`, `date_year`, and `description`. Events are written as
    /// `type: event` entries under `events/<slug>.md`. The index is written
    /// as `index.md` (rendered as the universe home page).
    ///
    /// Idempotent — `INSERT OR IGNORE` for the universe row, `upsert_entry_row`
    /// for each entry. Safe to re-call on every boot to refresh content.
    pub fn seed_timeline_universe(
        &mut self,
        manifest_json: &str,
        index_md: &str,
    ) -> anyhow::Result<()> {
        let manifest: serde_json::Value = serde_json::from_str(manifest_json)
            .map_err(|e| anyhow::anyhow!("timeline manifest parse: {e}"))?;
        let universe = manifest
            .get("universe")
            .ok_or_else(|| anyhow::anyhow!("timeline manifest missing `universe`"))?;
        let key = universe
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("timeline manifest missing universe.key"))?;
        let name = universe.get("name").and_then(|v| v.as_str()).unwrap_or(key);
        let description = universe
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let theme_preset = universe
            .get("theme_preset")
            .and_then(|v| v.as_str())
            .unwrap_or("modern");
        let layout = universe
            .get("layout")
            .and_then(|v| v.as_str())
            .unwrap_or("timeline");

        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Universe row — public read, no login required, system-owned.
        // parent_key='template' (CO-98) so the timeline trio appears nested
        // under the template universe in the SPA sidebar tree-build.
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              requires_login, visibility, theme_preset, layout, content_count, parent_key) \
             VALUES (?1, ?2, ?3, 'system', ?4, 0, 1, 0, 'public-static', ?5, ?6, 0, 'template')",
            params![key, name, description, now_str, theme_preset, layout],
        );
        // Existing rows from before CO-98 may have NULL parent_key — backfill
        // idempotently on every boot. Only updates rows that look like the
        // timeline trio (system-owned, public-static), so unrelated existing
        // rows are not touched.
        let _ = self.conn.execute(
            "UPDATE universes SET parent_key = 'template' \
             WHERE key = ?1 AND parent_key IS NULL \
             AND owner_id = 'system' AND visibility = 'public-static'",
            params![key],
        );

        let universe_root = self.universe_root(key);

        // index.md → page entry rendered as the front page by the SPA.
        let index_entry = make_entry(
            "index.md",
            json!({
                "type": "page",
                "slug": "index",
                "title": name,
                "tags": ["timeline", "index"],
                "created": now_str,
                "modified": now_str,
            }),
            index_md,
        );
        if let Err(e) = co::entry::write_entry(&universe_root, &index_entry) {
            tracing::warn!("seed_timeline_universe({key}): write index.md: {e}");
        }
        if let Err(e) = upsert_entry_row(&self.conn, key, &index_entry) {
            tracing::warn!("seed_timeline_universe({key}): upsert index.md: {e}");
        }

        // Each event → `events/<slug>.md` with `type: event`. Body is the
        // description; frontmatter carries `title`, `date_year`.
        let events = manifest
            .get("events")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("timeline manifest missing `events` array"))?;
        let mut count = 0usize;
        for event in events {
            let slug = event
                .get("slug")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("event missing slug"))?;
            let title = event.get("title").and_then(|v| v.as_str()).unwrap_or(slug);
            // `date_year` may exceed i64 range for far-future cosmic events
            // (e.g. heat death ~10^100). Read as f64 for storage; the SPA
            // parses it back as a number on the timeline.
            let date_year = event
                .get("date_year")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| anyhow::anyhow!("event {slug} missing or non-numeric date_year"))?;
            let description = event
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let path = format!("events/{}.md", slug);
            let fm = json!({
                "type": "event",
                "slug": slug,
                "title": title,
                "date_year": date_year,
                "description": description,
                "tags": ["timeline"],
                "created": now_str,
                "modified": now_str,
            });
            let entry = make_entry(&path, fm, description);
            if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                tracing::warn!("seed_timeline_universe({key}): write {path}: {e}");
            }
            if let Err(e) = upsert_entry_row(&self.conn, key, &entry) {
                tracing::warn!("seed_timeline_universe({key}): upsert {path}: {e}");
            } else {
                count += 1;
            }
        }

        // Update content_count to match what we wrote.
        let _ = self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = ?2",
            params![(count + 1) as i64, key],
        );

        tracing::info!(
            "Timeline universe '{}' seeded ({} events + index.md)",
            key,
            count
        );
        Ok(())
    }

    /// Seed all three timeline universes (`tempo`, `humanity`, `universo`).
    /// Each is independent — failure of one doesn't stop the others.
    pub fn seed_all_timeline_universes(&mut self) {
        for (label, manifest, index) in [
            (
                "tempo",
                SEED_TIMELINE_TEMPO_JSON,
                SEED_TIMELINE_TEMPO_INDEX_MD,
            ),
            (
                "humanity",
                SEED_TIMELINE_HUMANITY_JSON,
                SEED_TIMELINE_HUMANITY_INDEX_MD,
            ),
            (
                "universo",
                SEED_TIMELINE_UNIVERSO_JSON,
                SEED_TIMELINE_UNIVERSO_INDEX_MD,
            ),
        ] {
            if let Err(e) = self.seed_timeline_universe(manifest, index) {
                tracing::warn!("seed_timeline_universe({label}): {e}");
            }
        }
    }

    /// Returns true if the given project belongs to a template universe.
    pub fn is_project_in_template(&self, project_key: &str) -> bool {
        let universe_key = match self.get_project_universe_key(project_key) {
            Some(k) => k,
            None => return false,
        };

        let v: i64 = self
            .conn
            .query_row(
                "SELECT is_template FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .unwrap_or(0);
        v != 0
    }

    /// List projects for a universe that has `is_public = 1`.
    /// Returns Err if the universe doesn't exist or is not public.
    pub fn list_projects_for_public_universe(
        &self,
        universe_key: &str,
    ) -> anyhow::Result<Vec<crate::models::Project>> {
        let is_public: i64 = self
            .conn
            .query_row(
                "SELECT is_public FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("Universe '{}' not found", universe_key))?;

        if is_public == 0 {
            anyhow::bail!("Universe '{}' is not public", universe_key);
        }

        Ok(self.list_projects_for_universe(universe_key))
    }

    /// Copy entries (projects, tasks, pages) from source into an already-existing target universe.
    /// Returns the number of entries cloned.
    #[allow(dead_code)]
    fn clone_universe_internal(
        &mut self,
        source_key: &str,
        target_key: &str,
        now_str: &str,
    ) -> anyhow::Result<i64> {
        let mut count: i64 = 0;
        let target_root = self.universe_root(target_key);

        // Collect ALL entries from source per-universe DB (CO-77).
        let rows: Vec<EntryRow> = {
            let src_uc = self.universe_pool.get_or_open(source_key);
            let src_guard = src_uc.lock().expect("source universe conn lock");
            let mut stmt = src_guard.prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries WHERE universe_key = ?1",
            )?;
            stmt.query_map(params![source_key], entry_row_from_sql)?
                .filter_map(|r| r.ok())
                .collect()
        };

        let tgt_uc = self.universe_pool.get_or_open(target_key);

        for row in &rows {
            // Derive new project key for project entries
            let mut new_fm = row.frontmatter.clone();
            let mut new_path = row.path.clone();

            if row.entry_type == "project" {
                let old_key = row
                    .frontmatter
                    .get("key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let new_key = self.derive_unique_project_key(target_key, &old_key);
                new_path = format!("projects/{}/_project.md", new_key);
                if let Some(obj) = new_fm.as_object_mut() {
                    obj.insert("key".to_string(), json!(new_key));
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let entry = make_entry(&new_path, new_fm, &row.body);
                let _ = co::entry::write_entry(&target_root, &entry);
                {
                    let tgt_guard = tgt_uc.lock().expect("target universe conn lock");
                    let _ = upsert_entry_row(&tgt_guard, target_key, &entry);
                }
                // Register in routing index so get_project() can find it
                let _ = self.conn.execute(
                    "INSERT OR IGNORE INTO project_universe_index \
                     (project_key, universe_key) VALUES (?1, ?2)",
                    params![new_key, target_key],
                );
                count += 1;

                // Clone tasks for this project
                let old_key2 = old_key.clone();
                let task_rows: Vec<EntryRow> = {
                    let src_uc2 = self.universe_pool.get_or_open(source_key);
                    let src_guard2 = src_uc2.lock().expect("source universe conn lock");
                    let mut stmt2 = src_guard2.prepare(
                        "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                         created_at, updated_at FROM entries \
                         WHERE universe_key = ?1 AND entry_type = 'task' \
                         AND json_extract(frontmatter_json, '$.project') = ?2",
                    )?;
                    stmt2
                        .query_map(params![source_key, old_key2], entry_row_from_sql)?
                        .filter_map(|r| r.ok())
                        .collect()
                };
                for task_row in &task_rows {
                    let task_id = task_row
                        .frontmatter
                        .get("id")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let task_path = format!("projects/{}/{}.md", new_key, task_id);
                    let mut task_fm = task_row.frontmatter.clone();
                    if let Some(obj) = task_fm.as_object_mut() {
                        obj.insert("project".to_string(), json!(new_key));
                        obj.insert("created".to_string(), json!(now_str));
                        obj.insert("modified".to_string(), json!(now_str));
                    }
                    let entry = make_entry(&task_path, task_fm, &task_row.body);
                    let _ = co::entry::write_entry(&target_root, &entry);
                    {
                        let tgt_guard = tgt_uc.lock().expect("target universe conn lock");
                        let _ = upsert_entry_row(&tgt_guard, target_key, &entry);
                    }
                    count += 1;
                }
            } else if row.entry_type == "page" {
                if let Some(obj) = new_fm.as_object_mut() {
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let entry = make_entry(&new_path, new_fm, &row.body);
                let _ = co::entry::write_entry(&target_root, &entry);
                {
                    let tgt_guard = tgt_uc.lock().expect("target universe conn lock");
                    let _ = upsert_entry_row(&tgt_guard, target_key, &entry);
                }
                count += 1;
            }
        }

        // Inherit form config
        if let Some(config) = self.get_universe_form_config(source_key) {
            let tokens_str = config.custom_tokens.as_ref().map(|v| v.to_string());
            let _ = self.conn.execute(
                "UPDATE universes SET theme_preset = ?1, layout = ?2 WHERE key = ?3",
                params![config.theme_preset, config.layout, target_key],
            );
            if tokens_str.is_some() {
                let _ = self.conn.execute(
                    "UPDATE universes SET custom_tokens = ?1 WHERE key = ?2",
                    params![tokens_str, target_key],
                );
            }
        }

        Ok(count)
    }

    /// Clone a universe: copy all its projects and tasks into a new universe.
    /// The new universe is NOT a template and is private by default.
    pub fn clone_universe(
        &mut self,
        source_key: &str,
        new_key: &str,
        new_name: &str,
        description: &str,
        owner_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        if self.get_universe(new_key).is_some() {
            anyhow::bail!("Universe '{}' already exists", new_key);
        }
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Create the new universe
        self.conn.execute(
            "INSERT INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
            params![new_key, new_name, description, owner_id, now_str],
        )?;

        // Add owner as member
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members \
             (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', ?3)",
            params![new_key, owner_id, now_str],
        )?;

        let mut cloned_entries: i64 = 0;

        // CO-77: read from source per-universe DB, write to destination per-universe DB.
        let src_uc = self.universe_pool.get_or_open(source_key);
        let dst_uc = self.universe_pool.get_or_open(new_key);

        // Collect source project entries
        let source_project_rows: Vec<EntryRow> = {
            let src_guard = src_uc.lock().expect("src universe conn lock");
            let mut stmt = src_guard.prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries \
                 WHERE universe_key = ?1 AND entry_type = 'project'",
            )?;
            stmt.query_map(params![source_key], entry_row_from_sql)?
                .filter_map(|r| r.ok())
                .collect()
        };

        let new_universe_root = self.data_dir.join("universes").join(new_key);

        for proj_row in &source_project_rows {
            let old_pkey = proj_row
                .frontmatter
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let new_pkey = self.derive_unique_project_key(new_key, &old_pkey);

            // Build new project entry
            let new_proj_path = format!("projects/{}/_project.md", new_pkey);
            let mut new_proj_fm = proj_row.frontmatter.clone();
            if let Some(obj) = new_proj_fm.as_object_mut() {
                obj.insert("key".to_string(), json!(new_pkey));
                obj.insert("created".to_string(), json!(now_str));
                obj.insert("modified".to_string(), json!(now_str));
            }
            let new_proj_entry = make_entry(&new_proj_path, new_proj_fm, &proj_row.body);
            co::entry::write_entry(&new_universe_root, &new_proj_entry)?;
            {
                let dst_guard = dst_uc.lock().expect("dst universe conn lock");
                upsert_entry_row(&dst_guard, new_key, &new_proj_entry)?;
            }
            // Register project→universe mapping in routing index
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) VALUES (?1, ?2)",
                params![new_pkey, new_key],
            );
            cloned_entries += 1;

            // Collect source task entries
            let source_task_rows: Vec<EntryRow> = {
                let src_guard = src_uc.lock().expect("src universe conn lock");
                let mut stmt = src_guard.prepare(
                    "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                     created_at, updated_at FROM entries \
                     WHERE universe_key = ?1 AND entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?2 \
                     AND json_extract(frontmatter_json, '$.archived') = 0",
                )?;
                stmt.query_map(params![source_key, old_pkey], entry_row_from_sql)?
                    .filter_map(|r| r.ok())
                    .collect()
            };

            for task_row in &source_task_rows {
                let task_id = task_row
                    .frontmatter
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let new_task_path = format!("projects/{}/{}.md", new_pkey, task_id);
                let mut new_task_fm = task_row.frontmatter.clone();
                if let Some(obj) = new_task_fm.as_object_mut() {
                    obj.insert("project".to_string(), json!(new_pkey));
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let new_task_entry = make_entry(&new_task_path, new_task_fm, &task_row.body);
                co::entry::write_entry(&new_universe_root, &new_task_entry)?;
                {
                    let dst_guard = dst_uc.lock().expect("dst universe conn lock");
                    upsert_entry_row(&dst_guard, new_key, &new_task_entry)?;
                }
                cloned_entries += 1;
            }
        }

        // Clone page entries (content/about, terms, privacy, etc.)
        {
            let source_pages: Vec<EntryRow> = {
                let src_guard = src_uc.lock().expect("src universe conn lock");
                let mut stmt = src_guard.prepare(
                    "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                     created_at, updated_at FROM entries \
                     WHERE universe_key = ?1 AND entry_type = 'page'",
                )?;
                stmt.query_map(params![source_key], entry_row_from_sql)?
                    .filter_map(|r| r.ok())
                    .collect()
            };
            for page_row in &source_pages {
                let mut new_fm = page_row.frontmatter.clone();
                if let Some(obj) = new_fm.as_object_mut() {
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let new_page = make_entry(&page_row.path, new_fm, &page_row.body);
                let _ = co::entry::write_entry(&new_universe_root, &new_page);
                {
                    let dst_guard = dst_uc.lock().expect("dst universe conn lock");
                    let _ = upsert_entry_row(&dst_guard, new_key, &new_page);
                }
                cloned_entries += 1;
            }
        }

        // CO-95: copy any remaining entries that weren't picked up by the
        // project/task/page paths above (events, clips, untyped markdown,
        // doc.* generated entries, etc.). Bulk-insert with the new
        // universe_key; preserves path/title/frontmatter/body verbatim so
        // the duplicate is a true snapshot.
        let other_rows: Vec<EntryRow> = {
            let src_guard = src_uc.lock().expect("src universe conn lock");
            let mut stmt = src_guard.prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries \
                 WHERE universe_key = ?1 AND entry_type NOT IN ('project', 'task', 'page')",
            )?;
            stmt.query_map(params![source_key], entry_row_from_sql)?
                .filter_map(|r| r.ok())
                .collect()
        };
        let other_count = other_rows.len() as i64;
        {
            let dst_guard = dst_uc.lock().expect("dst universe conn lock");
            for row in &other_rows {
                let fm_json = serde_json::to_string(&row.frontmatter).unwrap_or_default();
                let _ = dst_guard.execute(
                    "INSERT OR IGNORE INTO entries \
                     (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        row.path,
                        new_key,
                        row.entry_type,
                        row.title,
                        fm_json,
                        row.body,
                        row.body_hash,
                        now_str,
                    ],
                );
            }
        }
        cloned_entries += other_count;

        // Set content_count
        self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = ?2",
            params![cloned_entries, new_key],
        )?;

        // Inherit form config (theme, layout, fonts, tokens) from the source universe.
        if let Some(source_config) = self.get_universe_form_config(source_key) {
            let tokens_str = source_config.custom_tokens.as_ref().map(|v| v.to_string());
            self.conn.execute(
                "UPDATE universes SET theme_preset = ?1, layout = ?2, \
                 font_headline = ?3, font_body = ?4, custom_tokens = ?5 \
                 WHERE key = ?6",
                params![
                    source_config.theme_preset,
                    source_config.layout,
                    source_config.font_headline,
                    source_config.font_body,
                    tokens_str,
                    new_key,
                ],
            )?;
            let _ = self.write_universo_yaml(new_key, &source_config);
        }

        Ok(crate::models::Universe {
            key: new_key.to_string(),
            name: new_name.to_string(),
            description: description.to_string(),
            owner_id: owner_id.to_string(),
            created_at: now,
            is_template: false,
            is_public: false,
            content_count: cloned_entries,
            requires_login: false,
            visibility: "private".into(),
            parent_key: None,
        })
    }

    /// Derive a unique project key for a clone, based on the universe key + original project key.
    fn derive_unique_project_key(&self, universe_key: &str, original_key: &str) -> String {
        // Take up to 4 alphanumeric chars from universe key (uppercase) + original key, max 10
        let prefix: String = universe_key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(4)
            .collect::<String>()
            .to_uppercase();
        let base: String = format!("{}{}", prefix, original_key)
            .chars()
            .take(10)
            .collect();

        // If unique, use as-is; otherwise append a number
        if self.get_project(&base).is_none() {
            return base;
        }
        for i in 2u32..=99 {
            let candidate: String = format!("{}{}", base, i).chars().take(10).collect();
            if self.get_project(&candidate).is_none() {
                return candidate;
            }
        }
        // Fallback: uuid-based suffix (shouldn't happen in practice)
        format!("{}{}", &base[..6.min(base.len())], nanoid::nanoid!(4))
            .chars()
            .take(10)
            .collect()
    }

    // --- List universe keys a user is a member of ---

    pub fn list_user_universes(&self, user_id: &str) -> Vec<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT universe_key FROM universe_members WHERE user_id = ?1")
            .expect("Failed to prepare list_user_universes");
        stmt.query_map(rusqlite::params![user_id], |row| row.get(0))
            .expect("Failed to query user universes")
            .filter_map(|r| r.ok())
            .collect()
    }

    /// List projects the user can see: those in their universes.
    pub fn list_projects_for_user(&self, user_id: &str) -> Vec<crate::models::Project> {
        let universe_keys = self.list_user_universes(user_id);
        let mut result = Vec::new();
        for uk in &universe_keys {
            result.extend(self.list_projects_for_universe(uk));
        }
        result
    }

    // --- Increment project next_id ---

    fn increment_project_next_id(
        &mut self,
        project_key: &str,
        universe_key: &str,
        new_next_id: u64,
    ) {
        let path = format!("projects/{}/_project.md", project_key);
        // CO-77: project entries live in the per-universe data.db, not meta.db.
        let uc = self.universe_pool.get_or_open(universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let result = uc_guard.query_row(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries WHERE path = ?1",
            params![path],
            entry_row_from_sql,
        );
        if let Ok(row) = result {
            let mut fm = row.frontmatter.clone();
            if let Some(obj) = fm.as_object_mut() {
                obj.insert("next_id".to_string(), json!(new_next_id));
            }
            let entry = make_entry(&path, fm, &row.body);
            let universe_root = self.universe_root(universe_key);
            let _ = co::entry::write_entry(&universe_root, &entry);
            let _ = upsert_entry_row(&uc_guard, universe_key, &entry);
        }
    }

    // -------------------------------------------------------------------------
    // WS / CRDT helpers
    // -------------------------------------------------------------------------

    /// Load the markdown body of an entry for CRDT initialisation.
    /// Returns `None` if the entry does not exist yet.
    pub fn get_entry_body(&self, universe_key: &str, path: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT body FROM entries WHERE universe_key = ?1 AND path = ?2",
                params![universe_key, path],
                |row| row.get::<_, String>(0),
            )
            .ok()
    }

    /// Persist the markdown body of an entry (CRDT idle / last-disconnect save).
    pub fn update_entry_body(
        &self,
        universe_key: &str,
        path: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        // Simple hash: polynomial rolling hash — used only for change detection.
        let hash = {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in body.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            format!("{h:016x}")
        };
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE entries SET body = ?1, body_hash = ?2, updated_at = ?3 \
             WHERE universe_key = ?4 AND path = ?5",
            params![body, hash, now, universe_key, path],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // API tokens (CO-35 — Vault REST API)
    // -------------------------------------------------------------------------

    /// Create a new long-lived API token (90 days) for the given user.
    pub fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
    ) -> anyhow::Result<crate::vault_routes::ApiToken> {
        let id = nanoid::nanoid!(21);
        let token = format!("co_{}", nanoid::nanoid!(40));
        let now = Utc::now();
        let expires_at = now + chrono::Duration::days(90);
        let now_str = now.to_rfc3339();
        let exp_str = expires_at.to_rfc3339();
        self.conn.execute(
            "INSERT INTO api_tokens (id, user_id, name, token, created_at, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, user_id, name, token, now_str, exp_str],
        )?;
        Ok(crate::vault_routes::ApiToken {
            id,
            user_id: user_id.to_string(),
            name: name.to_string(),
            token,
            created_at: now,
            expires_at,
            last_used_at: None,
        })
    }

    /// List API tokens for a user (token value redacted).
    pub fn list_api_tokens(
        &self,
        user_id: &str,
    ) -> anyhow::Result<Vec<crate::vault_routes::ApiToken>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_id, name, token, created_at, expires_at, last_used_at \
             FROM api_tokens WHERE user_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut tokens = vec![];
        for row in rows.filter_map(|r| r.ok()) {
            let (id, uid, name, token, created_str, expires_str, last_used_str) = row;
            let created_at = created_str
                .parse::<chrono::DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now());
            let expires_at = expires_str
                .parse::<chrono::DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now());
            let last_used_at = last_used_str
                .as_deref()
                .and_then(|s| s.parse::<chrono::DateTime<Utc>>().ok());
            tokens.push(crate::vault_routes::ApiToken {
                id,
                user_id: uid,
                name,
                token,
                created_at,
                expires_at,
                last_used_at,
            });
        }
        Ok(tokens)
    }

    /// Revoke an API token by id. Returns true if deleted.
    pub fn delete_api_token(&self, id: &str, user_id: &str) -> anyhow::Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM api_tokens WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    /// Look up a token by value; check expiry. Updates `last_used_at`.
    pub fn get_api_token_by_value(
        &self,
        token: &str,
    ) -> anyhow::Result<Option<crate::vault_routes::ApiToken>> {
        let result = self.conn.query_row(
            "SELECT id, user_id, name, token, created_at, expires_at, last_used_at \
             FROM api_tokens WHERE token = ?1",
            params![token],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        );
        match result {
            Ok((id, uid, name, tok, created_str, expires_str, last_used_str)) => {
                let created_at = created_str
                    .parse::<chrono::DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now());
                let expires_at = expires_str
                    .parse::<chrono::DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now());
                if expires_at < Utc::now() {
                    return Ok(None); // expired
                }
                let last_used_at = last_used_str
                    .as_deref()
                    .and_then(|s| s.parse::<chrono::DateTime<Utc>>().ok());
                // Update last_used_at
                let _ = self.conn.execute(
                    "UPDATE api_tokens SET last_used_at = ?1 WHERE id = ?2",
                    params![Utc::now().to_rfc3339(), id],
                );
                Ok(Some(crate::vault_routes::ApiToken {
                    id,
                    user_id: uid,
                    name,
                    token: tok,
                    created_at,
                    expires_at,
                    last_used_at,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// SQL helper — upsert a single entry into the entries table
// ---------------------------------------------------------------------------

fn upsert_entry_row(
    conn: &Connection,
    universe_key: &str,
    entry: &co::entry::Entry,
) -> anyhow::Result<()> {
    let fm_json = serde_json::to_string(&entry.frontmatter)?;
    let title: Option<&str> = entry.frontmatter.get("title").and_then(|v| v.as_str());
    let created_at = entry
        .frontmatter
        .get("created")
        .and_then(|v| v.as_str())
        .map(String::from);
    let updated_at = entry
        .frontmatter
        .get("modified")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| created_at.clone());

    conn.execute(
        "INSERT INTO entries (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(universe_key, path) DO UPDATE SET
           entry_type = excluded.entry_type,
           title = excluded.title,
           frontmatter_json = excluded.frontmatter_json,
           body = excluded.body,
           body_hash = excluded.body_hash,
           created_at = excluded.created_at,
           updated_at = excluded.updated_at",
        params![
            entry.path,
            universe_key,
            entry.entry_type,
            title,
            fm_json,
            entry.body,
            entry.body_hash,
            created_at,
            updated_at,
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn entry_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    let fm_str: String = row.get(4)?;
    let frontmatter: serde_json::Value =
        serde_json::from_str(&fm_str).unwrap_or(serde_json::Value::Object(Default::default()));
    Ok(EntryRow {
        path: row.get(0)?,
        universe_key: row.get(1)?,
        entry_type: row.get(2)?,
        title: row.get(3)?,
        frontmatter,
        body: row.get(5)?,
        body_hash: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn entry_row_to_project(row: &EntryRow) -> Option<Project> {
    let fm = &row.frontmatter;
    let key = fm.get("key").and_then(|v| v.as_str())?.to_string();
    let name = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let next_id = fm.get("next_id").and_then(|v| v.as_u64()).unwrap_or(1);
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);
    let archived = fm
        .get("archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Some(Project {
        key,
        name,
        description: row.body.clone(),
        created_at,
        next_id,
        archived,
    })
}

fn entry_row_to_task(row: &EntryRow) -> Option<Task> {
    let fm = &row.frontmatter;
    let id = fm.get("id").and_then(|v| v.as_u64())?;
    let project_key = fm
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status = parse_status(fm.get("status").and_then(|v| v.as_str()).unwrap_or("todo"));
    let priority = parse_priority(
        fm.get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("medium"),
    );
    let due_date = fm
        .get("due")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<NaiveDate>().ok());
    let parent = fm.get("parent").and_then(|v| v.as_u64());
    let labels: Vec<String> = fm
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);
    let updated_at = fm
        .get("modified")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or(created_at);
    let archived = fm
        .get("archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let assignee = fm
        .get("assignee")
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(Task {
        id,
        key: format!("{}-{}", project_key, id),
        project_key,
        title,
        status,
        priority,
        due_date,
        parent,
        labels,
        created_at,
        updated_at,
        description: row.body.clone(),
        archived,
        assignee,
    })
}

fn entry_row_to_comment(row: &EntryRow, project_key: &str, task_id: u64) -> Option<Comment> {
    let fm = &row.frontmatter;
    let id = fm.get("id").and_then(|v| v.as_u64())?;
    let author = fm
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("Anonymous")
        .to_string();
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);

    Some(Comment {
        id,
        project_key: project_key.to_string(),
        task_id,
        author,
        body: row.body.clone(),
        created_at,
    })
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

pub fn parse_datetime(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_status(s: &str) -> TaskStatus {
    match s {
        "todo" => TaskStatus::Todo,
        "in_progress" => TaskStatus::InProgress,
        "in_review" => TaskStatus::InReview,
        "done" => TaskStatus::Done,
        _ => TaskStatus::Todo,
    }
}

fn parse_priority(s: &str) -> Priority {
    match s {
        "low" => Priority::Low,
        "medium" => Priority::Medium,
        "high" => Priority::High,
        "critical" => Priority::Critical,
        _ => Priority::Medium,
    }
}

// --- Seed Data ---

pub fn seed_data(storage: &mut Storage) {
    use chrono::NaiveDate;

    let ds = CreateProject {
        name: "Design System".into(),
        key: "DS".into(),
        description: "Shared component library and design tokens".into(),
        ..Default::default()
    };
    storage.create_project(ds).unwrap();

    let api = CreateProject {
        name: "Backend API".into(),
        key: "API".into(),
        description: "Core REST API and data services".into(),
        ..Default::default()
    };
    storage.create_project(api).unwrap();

    // --- Design System tasks ---
    let ds_tasks = vec![
        CreateTask {
            title: "Define visual identity".into(),
            description: "Create logo, color palette, and typography for the design system.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 1),
            parent: None,
            labels: vec!["design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Build component showcase".into(),
            description: "Develop a web-based showcase of all available components and patterns."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 15),
            parent: None,
            labels: vec!["web".into(), "design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Organize first design review".into(),
            description:
                "Schedule review session, prepare demos, and gather feedback from stakeholders."
                    .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 20),
            parent: None,
            labels: vec!["review".into()],
            assignee: None,
        },
        CreateTask {
            title: "Produce component catalog".into(),
            description:
                "Document each component with usage examples, props, and accessibility notes."
                    .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 1),
            parent: None,
            labels: vec!["docs".into()],
            assignee: None,
        },
        CreateTask {
            title: "Set up documentation site".into(),
            description: "Deploy a static site with guidelines and a monthly content calendar."
                .into(),
            status: TaskStatus::Done,
            priority: Priority::Low,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 10),
            parent: None,
            labels: vec!["marketing".into()],
            assignee: None,
        },
        CreateTask {
            title: "Select color palette".into(),
            description: "Define primary and secondary colors aligned with the project identity."
                .into(),
            status: TaskStatus::InReview,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 25),
            parent: Some(1),
            labels: vec!["design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Design logo".into(),
            description: "Create 3 logo proposals for team vote.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 28),
            parent: Some(1),
            labels: vec!["design".into()],
            assignee: None,
        },
    ];

    for task in ds_tasks {
        storage.create_task("ds", task).unwrap();
    }

    // --- Backend API tasks ---
    let api_tasks = vec![
        CreateTask {
            title: "Database schema design".into(),
            description: "Design and document the relational schema for all core entities.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 30),
            parent: None,
            labels: vec!["database".into(), "urgent".into()],
            assignee: None,
        },
        CreateTask {
            title: "API documentation".into(),
            description: "Write OpenAPI specs and usage guides for every endpoint.".into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 15),
            parent: None,
            labels: vec!["docs".into()],
            assignee: None,
        },
        CreateTask {
            title: "Authentication module".into(),
            description: "Implement JWT-based auth with refresh tokens and role-based access."
                .into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 1),
            parent: None,
            labels: vec!["security".into(), "auth".into()],
            assignee: None,
        },
        CreateTask {
            title: "Rate limiting and throttling".into(),
            description: "Add per-endpoint rate limits and IP-based throttling to protect the API."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 7, 1),
            parent: None,
            labels: vec!["security".into()],
            assignee: None,
        },
        CreateTask {
            title: "CI/CD pipeline setup".into(),
            description:
                "Configure automated testing, linting, and deployment for the API service.".into(),
            status: TaskStatus::InReview,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 15),
            parent: None,
            labels: vec!["devops".into()],
            assignee: None,
        },
        CreateTask {
            title: "Write migration scripts".into(),
            description: "Create versioned SQL migrations for the initial schema.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 10),
            parent: Some(1),
            labels: vec!["database".into()],
            assignee: None,
        },
        CreateTask {
            title: "Integration test suite".into(),
            description: "Build end-to-end tests covering all critical API workflows.".into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 1),
            parent: Some(3),
            labels: vec!["testing".into()],
            assignee: None,
        },
        CreateTask {
            title: "Load testing workshop".into(),
            description:
                "Run load tests to identify bottlenecks and establish performance baselines.".into(),
            status: TaskStatus::Done,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 8),
            parent: Some(1),
            labels: vec!["testing".into(), "performance".into()],
            assignee: None,
        },
    ];

    for task in api_tasks {
        storage.create_task("api", task).unwrap();
    }

    // --- Platform ---
    let plt = CreateProject {
        name: "Platform".into(),
        key: "PLT".into(),
        description: "Unified platform for management and collaboration".into(),
        ..Default::default()
    };
    storage.create_project(plt).unwrap();

    let plt_tasks = vec![
        CreateTask {
            title: "Initial Launch".into(),
            description: "Launch epic: prepare and publish the first versions of the product."
                .into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 30),
            parent: None,
            labels: vec!["epic".into(), "launch".into()],
            assignee: None,
        },
        CreateTask {
            title: "Internal MVP".into(),
            description: "Minimum viable version for internal team use. Validate core \
                           workflows, identify critical bugs, and collect feedback before \
                           the public launch."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 15),
            parent: Some(1),
            labels: vec!["mvp".into()],
            assignee: None,
        },
        CreateTask {
            title: "Public MVP".into(),
            description: "First public version of the product. Incorporate fixes from the \
                           internal MVP, prepare onboarding, documentation, and production \
                           infrastructure."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 30),
            parent: Some(1),
            labels: vec!["mvp".into(), "public".into()],
            assignee: None,
        },
    ];

    for task in plt_tasks {
        storage.create_task("plt", task).unwrap();
    }
}

#[cfg(test)]
mod seed_md_tests {
    use super::*;

    #[test]
    fn split_frontmatter_extracts_yaml_and_body() {
        let md = "---
slug: foo
title: Bar
order: 3
---

# Heading

Body text.
";
        let (fm, body) = split_frontmatter(md);
        assert!(fm.contains("slug: foo"));
        assert!(fm.contains("title: Bar"));
        assert!(body.starts_with("# Heading"));
        assert!(body.contains("Body text."));
    }

    #[test]
    fn split_frontmatter_handles_no_frontmatter() {
        let md = "# Just markdown

No frontmatter.";
        let (fm, body) = split_frontmatter(md);
        assert_eq!(fm, "");
        assert_eq!(body, md);
    }

    #[test]
    fn seed_page_frontmatter_overrides_timestamps() {
        let md = "---
slug: x
title: T
order: 1
tags:
  - a
created: 2020-01-01T00:00:00Z
modified: 2020-01-01T00:00:00Z
---

body";
        let now = "2026-04-26T00:00:00+00:00";
        let fm = seed_page_frontmatter(md, now);
        assert_eq!(fm.get("slug").and_then(|v| v.as_str()), Some("x"));
        assert_eq!(fm.get("title").and_then(|v| v.as_str()), Some("T"));
        assert_eq!(fm.get("created").and_then(|v| v.as_str()), Some(now));
        assert_eq!(fm.get("modified").and_then(|v| v.as_str()), Some(now));
    }

    #[test]
    fn embedded_seed_md_files_parse() {
        for md in [
            SEED_TEMPLATE_INDEX_MD,
            SEED_SOBRE_MD,
            SEED_TERMOS_MD,
            SEED_PRIVACIDADE_MD,
            SEED_DADOS_RASTREADOS_MD,
            SEED_LINHAS_DO_TEMPO_MD,
        ] {
            let now = "2026-04-26T00:00:00+00:00";
            let fm = seed_page_frontmatter(md, now);
            assert!(
                fm.get("slug").and_then(|v| v.as_str()).is_some(),
                "missing slug"
            );
            assert!(
                fm.get("title").and_then(|v| v.as_str()).is_some(),
                "missing title"
            );
            let body = seed_page_body(md);
            assert!(
                body.starts_with("# "),
                "body should start with H1, got: {:?}",
                &body[..40.min(body.len())]
            );
        }
    }
}

#[cfg(test)]
mod ensure_column_tests {
    use rusqlite::Connection;

    use super::ensure_column;

    fn make_test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY);")
            .expect("create test table");
        conn
    }

    #[test]
    fn adds_missing_column() {
        let conn = make_test_conn();
        let added = ensure_column(&conn, "t", "foo", "TEXT").expect("ensure_column");
        assert!(added, "should report column was added");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('t') WHERE name = 'foo'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "column should exist after ensure_column");
    }

    #[test]
    fn no_op_if_column_exists() {
        let conn = make_test_conn();
        ensure_column(&conn, "t", "foo", "TEXT").expect("first call");
        let added = ensure_column(&conn, "t", "foo", "TEXT").expect("second call");
        assert!(!added, "should report no-op when column already exists");
    }

    #[test]
    fn idempotent_repeated_calls() {
        let conn = make_test_conn();
        for i in 0..5 {
            let added =
                ensure_column(&conn, "t", "bar", "INTEGER DEFAULT 0").expect("idempotent call");
            assert_eq!(added, i == 0, "only first call should add the column");
        }
    }

    #[test]
    fn partial_migration_recovery() {
        // Simulates the CO-137 scenario: schema_version shows v22 was applied
        // but the ALTER TABLE never ran. ensure_column should recover cleanly.
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE universes (key TEXT PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (22);",
        )
        .expect("setup");

        // Column doesn't exist yet (partial migration state)
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('universes') WHERE name = 'parent_key'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "precondition: parent_key should be missing");

        // ensure_column adds it without panic
        let added = ensure_column(&conn, "universes", "parent_key", "TEXT")
            .expect("ensure_column on partial migration");
        assert!(added, "should have added the missing column");

        // Verify it's now present
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('universes') WHERE name = 'parent_key'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "parent_key should exist after recovery");
    }
}
