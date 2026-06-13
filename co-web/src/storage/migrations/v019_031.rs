use super::super::Storage;
use super::super::schema::ensure_column;

impl Storage {
    pub(super) fn migrate_v019_031(&mut self, current_version: i64) {
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
    }
}
