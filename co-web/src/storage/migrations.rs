use super::Storage;
use super::schema::{ensure_column, ensure_table};

impl Storage {
    pub(super) fn run_migrations(&mut self) {
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

        if current_version < 29 {
            // 1.46.0 visibility consolidation: drop `requires_login` as a
            // distinct visibility — collapse into `public-subscribable`
            // (anonymous gets metadata-only, authed gets read+write).
            // Universes that need to be available to every authed user by
            // default are tagged with `default_for_new_users` and the
            // signup path auto-subscribes new users to them.
            ensure_column(
                &self.conn,
                "universes",
                "default_for_new_users",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v29: universes.default_for_new_users column");

            self.conn
                .execute_batch(
                    "
                    UPDATE universes
                       SET visibility = 'public-subscribable',
                           is_public = 1,
                           requires_login = 0,
                           default_for_new_users = 1
                     WHERE visibility = 'requires_login' OR requires_login = 1;

                    INSERT INTO schema_version (version) VALUES (29);
                    ",
                )
                .expect("Failed to run migration v29");
        }

        if current_version < 30 {
            // 1.60.0: subscribers can pin to a specific state ID. NULL =
            // head-following (default); non-NULL = "show me this universe
            // as of state X". The rewind-view behavior (serving entries
            // as of the pinned state) is NOT implemented yet — Phase 7.
            // This migration is just the data layer.
            ensure_column(&self.conn, "subscriptions", "pinned_state", "TEXT")
                .expect("migration v30: subscriptions.pinned_state column");

            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (30)", [])
                .expect("Failed to record migration v30");
        }

        if current_version < 31 {
            // 1.70.0 (Phase 8 step 1): content-addressed blob storage. Each
            // unique entry body becomes a row keyed by its sha256 hash.
            // States can reference these hashes to enable full-fidelity
            // rewind — serving entries with their historical bytes, not
            // just current ones. This step ships the storage layer only;
            // vault writes don't dual-write yet (step 2). Reads/writes from
            // application code go through put_blob / get_blob.
            self.conn
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS blobs (
                        hash       TEXT PRIMARY KEY,
                        bytes      BLOB NOT NULL,
                        size       INTEGER NOT NULL,
                        created_at TEXT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_blobs_size ON blobs(size);

                    INSERT INTO schema_version (version) VALUES (31);
                    ",
                )
                .expect("Failed to run migration v31");
        }

        if current_version < 32 {
            // CO-166 (1.76.0): OIDC OAuth2 server tables.
            self.conn
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS oauth_clients (
                        id TEXT PRIMARY KEY,
                        client_id TEXT UNIQUE NOT NULL,
                        client_secret TEXT NOT NULL,
                        name TEXT NOT NULL,
                        redirect_uris TEXT NOT NULL,
                        scopes TEXT NOT NULL,
                        created_at TEXT NOT NULL
                    );
                    CREATE TABLE IF NOT EXISTS oauth_auth_codes (
                        code TEXT PRIMARY KEY,
                        client_id TEXT NOT NULL,
                        user_id TEXT NOT NULL,
                        redirect_uri TEXT NOT NULL,
                        scope TEXT NOT NULL,
                        code_challenge TEXT NOT NULL,
                        code_challenge_method TEXT NOT NULL DEFAULT 'S256',
                        expires_at TEXT NOT NULL,
                        created_at TEXT NOT NULL
                    );
                    CREATE TABLE IF NOT EXISTS oauth_access_tokens (
                        token TEXT PRIMARY KEY,
                        client_id TEXT NOT NULL,
                        user_id TEXT NOT NULL,
                        scope TEXT NOT NULL,
                        expires_at TEXT NOT NULL,
                        created_at TEXT NOT NULL
                    );
                    INSERT INTO schema_version (version) VALUES (32);
                    ",
                )
                .expect("Failed to run migration v32");
        }

        if current_version < 33 {
            // CO-166 (1.76.0): link quilombo accounts to CO main accounts.
            ensure_column(&self.conn, "quilombo_usuarios", "linked_co_user_id", "TEXT")
                .expect("migration v33: quilombo_usuarios.linked_co_user_id column");
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (33)", [])
                .expect("Failed to record migration v33");
        }

        if current_version < 34 {
            // CO-167 (1.79.0): email for quilombo users — bridge to CO unified auth.
            ensure_column(&self.conn, "quilombo_usuarios", "email", "TEXT")
                .expect("migration v34: quilombo_usuarios.email");
            ensure_column(&self.conn, "quilombo_usuarios", "linked_co_user_id", "TEXT")
                .expect("migration v34: quilombo_usuarios.linked_co_user_id");
            self.conn
                .execute_batch(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_quilombo_usuarios_email
                         ON quilombo_usuarios(email) WHERE email IS NOT NULL;
                     INSERT INTO schema_version (version) VALUES (34);",
                )
                .expect("Failed to run migration v34");
        }

        if current_version < 35 {
            // CO-168 (1.80.0): outbound webhook system + notification queue.
            self.conn
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS webhooks (
                        id         TEXT PRIMARY KEY,
                        url        TEXT NOT NULL,
                        secret     TEXT NOT NULL,
                        events     TEXT NOT NULL,
                        enabled    INTEGER NOT NULL DEFAULT 1,
                        created_at TEXT NOT NULL
                    );
                    CREATE TABLE IF NOT EXISTS notifications (
                        id              TEXT PRIMARY KEY,
                        webhook_id      TEXT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
                        event_type      TEXT NOT NULL,
                        payload         TEXT NOT NULL,
                        status          TEXT NOT NULL DEFAULT 'pending',
                        attempts        INTEGER NOT NULL DEFAULT 0,
                        next_attempt_at TEXT NOT NULL,
                        sent_at         TEXT,
                        error           TEXT,
                        created_at      TEXT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_notifications_pending
                        ON notifications(status, next_attempt_at)
                        WHERE status IN ('pending', 'failed');
                    INSERT INTO schema_version (version) VALUES (35);
                    ",
                )
                .expect("Failed to run migration v35");
        }

        if current_version < 36 {
            // CO-169 (1.81.0): direct notification provider adapters.
            ensure_column(&self.conn, "quilombo_usuarios", "telefone", "TEXT")
                .expect("migration v36: quilombo_usuarios.telefone");
            ensure_column(&self.conn, "notifications", "channel", "TEXT")
                .expect("migration v36: notifications.channel");
            ensure_column(&self.conn, "notifications", "recipient", "TEXT")
                .expect("migration v36: notifications.recipient");
            self.conn
                .execute_batch(
                    "
                    INSERT OR IGNORE INTO webhooks (id, url, secret, events, enabled, created_at)
                        VALUES ('__direct__', 'direct://', '__direct__', '[]', 0, datetime('now'));
                    INSERT INTO schema_version (version) VALUES (36);
                    ",
                )
                .expect("Failed to run migration v36");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 37 {
            // CO-165 (1.82.0): username+email decoupling + recovery channel tables.
            //
            // Restart-safe: a previous run may have left users_old (rename done)
            // but users not yet recreated (CREATE TABLE failed mid-batch).
            // Detect both states and resume accordingly.
            let users_old_exists: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='users_old'",
                    [],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            let users_exists: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='users'",
                    [],
                    |_| Ok(true),
                )
                .unwrap_or(false);

            if users_old_exists && !users_exists {
                // Partial-migration recovery: rename already happened; finish from here.
                self.conn
                    .execute_batch(
                        "
                        CREATE TABLE users (
                            id            TEXT PRIMARY KEY,
                            email         TEXT UNIQUE,
                            display_name  TEXT NOT NULL DEFAULT '',
                            tier          TEXT NOT NULL DEFAULT 'player',
                            created_at    TEXT NOT NULL,
                            password_hash TEXT,
                            usuario       TEXT UNIQUE
                        );
                        INSERT INTO users (id, email, display_name, tier, created_at, password_hash)
                        SELECT id, email, display_name, tier, created_at, password_hash
                        FROM users_old;
                        -- Backfill usuario from email local-part. Multiple rows
                        -- can share a local-part (yuri@artelonga.com.br vs.
                        -- yuri@uat.local) which `usuario UNIQUE` would reject;
                        -- dedupe by suffixing `-2`, `-3`, ... after the
                        -- earliest-created row in each partition.
                        WITH numbered AS (
                            SELECT id,
                                   LOWER(SUBSTR(email, 1, INSTR(email, '@') - 1)) AS base,
                                   ROW_NUMBER() OVER (
                                       PARTITION BY LOWER(SUBSTR(email, 1, INSTR(email, '@') - 1))
                                       ORDER BY created_at, id
                                   ) AS rn
                            FROM users
                            WHERE email LIKE '%@%' AND usuario IS NULL
                        )
                        UPDATE users
                        SET usuario = (
                            SELECT CASE WHEN n.rn = 1 THEN n.base
                                        ELSE n.base || '-' || n.rn END
                            FROM numbered n WHERE n.id = users.id
                        )
                        WHERE EXISTS (SELECT 1 FROM numbered n WHERE n.id = users.id);
                        DROP TABLE users_old;
                        PRAGMA foreign_keys=ON;
                        ",
                    )
                    .expect("Failed partial-recovery of migration v37 (users)");
            } else {
                // Normal path: both tables clean.
                self.conn
                    .execute_batch(
                        "
                        PRAGMA foreign_keys=OFF;
                        DROP TABLE IF EXISTS users_old;
                        ALTER TABLE users RENAME TO users_old;
                        CREATE TABLE users (
                            id            TEXT PRIMARY KEY,
                            email         TEXT UNIQUE,
                            display_name  TEXT NOT NULL DEFAULT '',
                            tier          TEXT NOT NULL DEFAULT 'player',
                            created_at    TEXT NOT NULL,
                            password_hash TEXT,
                            usuario       TEXT UNIQUE
                        );
                        INSERT INTO users (id, email, display_name, tier, created_at, password_hash)
                        SELECT id, email, display_name, tier, created_at, password_hash
                        FROM users_old;
                        -- Backfill usuario from email local-part. Multiple rows
                        -- can share a local-part (yuri@artelonga.com.br vs.
                        -- yuri@uat.local) which `usuario UNIQUE` would reject;
                        -- dedupe by suffixing `-2`, `-3`, ... after the
                        -- earliest-created row in each partition.
                        WITH numbered AS (
                            SELECT id,
                                   LOWER(SUBSTR(email, 1, INSTR(email, '@') - 1)) AS base,
                                   ROW_NUMBER() OVER (
                                       PARTITION BY LOWER(SUBSTR(email, 1, INSTR(email, '@') - 1))
                                       ORDER BY created_at, id
                                   ) AS rn
                            FROM users
                            WHERE email LIKE '%@%' AND usuario IS NULL
                        )
                        UPDATE users
                        SET usuario = (
                            SELECT CASE WHEN n.rn = 1 THEN n.base
                                        ELSE n.base || '-' || n.rn END
                            FROM numbered n WHERE n.id = users.id
                        )
                        WHERE EXISTS (SELECT 1 FROM numbered n WHERE n.id = users.id);
                        DROP TABLE users_old;
                        PRAGMA foreign_keys=ON;
                        ",
                    )
                    .expect("Failed to run migration v37 (users table recreate)");
            }

            // Recovery tables (safe to run unconditionally — IF NOT EXISTS).
            self.conn
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS user_recovery_channels (
                        id                TEXT PRIMARY KEY,
                        user_id           TEXT NOT NULL,
                        channel_type      TEXT NOT NULL
                                          CHECK (channel_type IN ('email','whatsapp','sms')),
                        value_ciphertext  BLOB NOT NULL,
                        value_nonce       BLOB NOT NULL,
                        value_lookup_hash TEXT NOT NULL,
                        verified_at       TEXT,
                        created_at        TEXT NOT NULL,
                        last_used_at      TEXT,
                        lockout_until     TEXT,
                        FOREIGN KEY (user_id) REFERENCES users(id)
                    );
                    CREATE INDEX IF NOT EXISTS idx_urc_user
                        ON user_recovery_channels(user_id);
                    CREATE TABLE IF NOT EXISTS recovery_verifications (
                        id          TEXT PRIMARY KEY,
                        channel_id  TEXT NOT NULL,
                        user_id     TEXT NOT NULL,
                        purpose     TEXT NOT NULL
                                    CHECK (purpose IN ('add_channel','reset_password','change_email')),
                        code_hash   TEXT NOT NULL,
                        expires_at  TEXT NOT NULL,
                        consumed_at TEXT,
                        attempts    INTEGER NOT NULL DEFAULT 0,
                        created_at  TEXT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_rv_channel
                        ON recovery_verifications(channel_id, consumed_at);
                    CREATE INDEX IF NOT EXISTS idx_rv_user_purpose
                        ON recovery_verifications(user_id, purpose, consumed_at);
                    CREATE TABLE IF NOT EXISTS password_reset_tokens (
                        token_hash  TEXT PRIMARY KEY,
                        user_id     TEXT NOT NULL,
                        channel_id  TEXT NOT NULL,
                        expires_at  TEXT NOT NULL,
                        consumed_at TEXT,
                        created_at  TEXT NOT NULL
                    );
                    INSERT OR IGNORE INTO schema_version (version) VALUES (37);
                    ",
                )
                .expect("Failed to run migration v37 (recovery tables)");
        }

        if current_version < 38 {
            // CO-170: `hidden` flag on universes — soft-hide deprecated /
            // empty / placeholder universes from sidebar listings without
            // deleting the underlying content. Reversible via UPDATE.
            ensure_column(
                &self.conn,
                "universes",
                "hidden",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v38: universes.hidden");
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (38)",
                    [],
                )
                .expect("migration v38: version insert");
        }

        if current_version < 39 {
            // CO-177: Google OAuth linkage. `google_sub` stores Google's
            // stable user identifier (the `sub` claim), and is what re-login
            // matches on — email can change in Google but `sub` is forever.
            // Nullable because most users won't link Google.
            ensure_column(&self.conn, "users", "google_sub", "TEXT")
                .expect("migration v39: users.google_sub");
            // Partial unique index — one Google account links to at most one
            // CO user. NULL `google_sub` rows are excluded (multiple unlinked
            // users coexist).
            self.conn
                .execute_batch(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_google_sub \
                     ON users(google_sub) WHERE google_sub IS NOT NULL;",
                )
                .expect("migration v39: idx_users_google_sub");
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (39)",
                    [],
                )
                .expect("migration v39: version insert");
        }

        if current_version < 40 {
            // CO-188: universe invitation tokens — single-use invites with
            // role assignment. Raw token only in email; token_hash (sha256)
            // stored here so the DB never sees the raw secret.
            self.conn
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS universe_invitations (
                        token_hash      TEXT PRIMARY KEY,
                        universe_key    TEXT NOT NULL,
                        invited_by      TEXT NOT NULL,
                        invited_email   TEXT,
                        invited_user_id TEXT,
                        role            TEXT NOT NULL DEFAULT 'member',
                        expires_at      TEXT NOT NULL,
                        consumed_at     TEXT,
                        revoked_at      TEXT,
                        created_at      TEXT NOT NULL,
                        FOREIGN KEY (universe_key) REFERENCES universes(key)
                    );
                    CREATE INDEX IF NOT EXISTS idx_invitations_universe
                        ON universe_invitations(universe_key, consumed_at);
                    CREATE INDEX IF NOT EXISTS idx_invitations_recipient
                        ON universe_invitations(invited_email, consumed_at);
                    INSERT OR IGNORE INTO schema_version (version) VALUES (40);
                    ",
                )
                .expect("Failed to run migration v40");
        }

        if current_version < 41 {
            // CO-190: onboarding codes — passwordless sign-in / signup via email.
            self.conn
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS onboarding_codes (
                        id                  TEXT PRIMARY KEY,
                        email_lookup_hash   TEXT NOT NULL,
                        intent              TEXT NOT NULL,
                        code_hash           TEXT NOT NULL,
                        preferred_usuario   TEXT,
                        return_to           TEXT,
                        expires_at          TEXT NOT NULL,
                        consumed_at         TEXT,
                        attempts            INTEGER NOT NULL DEFAULT 0,
                        created_at          TEXT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_onboarding_codes_lookup
                        ON onboarding_codes(email_lookup_hash, consumed_at);
                    INSERT OR IGNORE INTO schema_version (version) VALUES (41);
                    ",
                )
                .expect("Failed to run migration v41");
        }

        if current_version < 42 {
            // CO-205: origin column on users — tracks where each signup came from.
            ensure_column(&self.conn, "users", "origin", "TEXT")
                .expect("migration v42: users.origin");
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_users_origin ON users(origin);
                     INSERT OR IGNORE INTO schema_version (version) VALUES (42);",
                )
                .expect("migration v42: idx_users_origin + schema_version");
        }

        // CO-190 unconditional backfill — ensures the table exists even if
        // v41 was partially applied on an older instance.
        ensure_table(
            &self.conn,
            "onboarding_codes",
            "CREATE TABLE IF NOT EXISTS onboarding_codes (
                id                  TEXT PRIMARY KEY,
                email_lookup_hash   TEXT NOT NULL,
                intent              TEXT NOT NULL,
                code_hash           TEXT NOT NULL,
                preferred_usuario   TEXT,
                return_to           TEXT,
                expires_at          TEXT NOT NULL,
                consumed_at         TEXT,
                attempts            INTEGER NOT NULL DEFAULT 0,
                created_at          TEXT NOT NULL
            );",
        )
        .expect("CO-190 backfill: onboarding_codes table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_onboarding_codes_lookup
                     ON onboarding_codes(email_lookup_hash, consumed_at);",
            )
            .expect("CO-190 backfill: onboarding_codes index");

        // CO-188 unconditional backfill — ensures the table exists even if
        // v40 was partially applied on an older instance.
        ensure_table(
            &self.conn,
            "universe_invitations",
            "CREATE TABLE IF NOT EXISTS universe_invitations (
                token_hash      TEXT PRIMARY KEY,
                universe_key    TEXT NOT NULL,
                invited_by      TEXT NOT NULL,
                invited_email   TEXT,
                invited_user_id TEXT,
                role            TEXT NOT NULL DEFAULT 'member',
                expires_at      TEXT NOT NULL,
                consumed_at     TEXT,
                revoked_at      TEXT,
                created_at      TEXT NOT NULL,
                FOREIGN KEY (universe_key) REFERENCES universes(key)
            );",
        )
        .expect("CO-188 backfill: universe_invitations table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_invitations_universe
                     ON universe_invitations(universe_key, consumed_at);
                 CREATE INDEX IF NOT EXISTS idx_invitations_recipient
                     ON universe_invitations(invited_email, consumed_at);",
            )
            .expect("CO-188 backfill: invitation indexes");

        // CO-165 unconditional backfill: ensure recovery tables exist even if
        // v37 migration was partially applied (same pattern as CO-137).
        ensure_column(&self.conn, "users", "usuario", "TEXT").ok();
        ensure_table(
            &self.conn,
            "user_recovery_channels",
            "CREATE TABLE IF NOT EXISTS user_recovery_channels (
                id                TEXT PRIMARY KEY,
                user_id           TEXT NOT NULL,
                channel_type      TEXT NOT NULL
                                  CHECK (channel_type IN ('email','whatsapp','sms')),
                value_ciphertext  BLOB NOT NULL,
                value_nonce       BLOB NOT NULL,
                value_lookup_hash TEXT NOT NULL,
                verified_at       TEXT,
                created_at        TEXT NOT NULL,
                last_used_at      TEXT,
                lockout_until     TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id)
            );",
        )
        .ok();
        ensure_table(
            &self.conn,
            "recovery_verifications",
            "CREATE TABLE IF NOT EXISTS recovery_verifications (
                id          TEXT PRIMARY KEY,
                channel_id  TEXT NOT NULL,
                user_id     TEXT NOT NULL,
                purpose     TEXT NOT NULL
                            CHECK (purpose IN ('add_channel','reset_password','change_email')),
                code_hash   TEXT NOT NULL,
                expires_at  TEXT NOT NULL,
                consumed_at TEXT,
                attempts    INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL
            );",
        )
        .ok();
        ensure_table(
            &self.conn,
            "password_reset_tokens",
            "CREATE TABLE IF NOT EXISTS password_reset_tokens (
                token_hash  TEXT PRIMARY KEY,
                user_id     TEXT NOT NULL,
                channel_id  TEXT NOT NULL,
                expires_at  TEXT NOT NULL,
                consumed_at TEXT,
                created_at  TEXT NOT NULL
            );",
        )
        .ok();

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

        // CO-183: leads intake — public form submissions + admin queue.
        ensure_table(
            &self.conn,
            "leads",
            "CREATE TABLE IF NOT EXISTS leads (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL,
                nome            TEXT,
                email           TEXT,
                telefone        TEXT,
                mensagem        TEXT NOT NULL,
                servico_titulo  TEXT,
                parceiro_handle TEXT,
                status          TEXT NOT NULL DEFAULT 'new',
                priority        TEXT DEFAULT 'normal',
                assignee_handle TEXT,
                notes           TEXT,
                closed_reason   TEXT,
                promoted_to_al  INTEGER,
                ip_hash         TEXT,
                user_agent      TEXT
            );",
        )
        .expect("CO-183: leads table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_leads_status ON leads(status);
                 CREATE INDEX IF NOT EXISTS idx_leads_created_at ON leads(created_at);
                 CREATE INDEX IF NOT EXISTS idx_leads_assignee ON leads(assignee_handle);",
            )
            .expect("CO-183: leads indexes");

        // CO-193: per-universe chat — rooms + messages tables.
        self.ensure_chat_tables();

        // CO-199: user notifications + preferences tables.
        self.ensure_notification_tables();

        // CO-201: push subscription endpoints table.
        self.ensure_push_subscriptions_table();

        // CO-205 unconditional backfill — ensures users.origin and
        // onboarding_codes.origin exist even if v42 was partially applied.
        ensure_column(&self.conn, "users", "origin", "TEXT")
            .expect("CO-205 backfill: users.origin");
        self.conn
            .execute_batch("CREATE INDEX IF NOT EXISTS idx_users_origin ON users(origin);")
            .expect("CO-205 backfill: idx_users_origin");
        ensure_column(&self.conn, "onboarding_codes", "origin", "TEXT")
            .expect("CO-205 backfill: onboarding_codes.origin");

        if current_version < 43 {
            // CO-206: yggdrasil bridge — links a CO user to their yggdrasil-local identity.
            ensure_column(&self.conn, "users", "yggdrasil_user_id", "TEXT")
                .expect("migration v43: users.yggdrasil_user_id");
            self.conn
                .execute_batch(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_yggdrasil \
                     ON users(yggdrasil_user_id) WHERE yggdrasil_user_id IS NOT NULL;
                     INSERT OR IGNORE INTO schema_version (version) VALUES (43);",
                )
                .expect("migration v43: idx_users_yggdrasil + schema_version");
        }

        // CO-206 unconditional backfill — ensures users.yggdrasil_user_id exists
        // even if v43 was partially applied on an older instance.
        ensure_column(&self.conn, "users", "yggdrasil_user_id", "TEXT")
            .expect("CO-206 backfill: users.yggdrasil_user_id");
        self.conn
            .execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_yggdrasil \
                 ON users(yggdrasil_user_id) WHERE yggdrasil_user_id IS NOT NULL;",
            )
            .expect("CO-206 backfill: idx_users_yggdrasil");

        // CO-179: composite index for public analytics queries filtered by
        // (universe_key, timestamp). Cheap to create if already present.
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_telemetry_universe_time \
                 ON telemetry_events(universe_key, timestamp);",
            )
            .expect("CO-179: idx_telemetry_universe_time");
    }
}
