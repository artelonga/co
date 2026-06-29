use super::super::Storage;
use super::super::schema::ensure_column;

impl Storage {
    pub(super) fn migrate_v001_018(&mut self, current_version: i64) {
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

        // CO-509: versions 3, 4, 5 were historically owned by the (now-removed)
        // embedded tenant migration, which created tenant-only tables and
        // reserved those schema_version numbers. The tenant code is gone, so we
        // no longer create those tables, but we still reserve the version
        // numbers to keep the migration chain contiguous and preserve the exact
        // downstream guard behaviour (the `< 5` block below stays skipped on a
        // fresh DB, just as it did when the tenant migration ran first).
        {
            let v: i64 = self
                .conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if v < 5 {
                self.conn
                    .execute_batch(
                        "INSERT OR IGNORE INTO schema_version (version) VALUES (3);
                         INSERT OR IGNORE INTO schema_version (version) VALUES (4);
                         INSERT OR IGNORE INTO schema_version (version) VALUES (5);",
                    )
                    .expect("Failed to reserve schema versions 3-5 (CO-509)");
            }
        }

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
            // external/bridged users (not stored in the `users` table) can be members.
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
    }
}
