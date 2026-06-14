use super::super::Storage;
use super::super::schema::{ensure_column, ensure_table};

impl Storage {
    pub(super) fn migrate_v046_056(&mut self, current_version: i64) {
        if current_version < 46 {
            // CO-241: content-volume metrics on meta-DB entries table.
            // The same three columns are added to per-universe data.db via
            // run_universe_migrations (universe_pool.rs). Both tables share
            // the EntryIndex::upsert path so both need the columns.
            ensure_column(
                &self.conn,
                "entries",
                "body_lines",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v46: entries.body_lines");
            ensure_column(
                &self.conn,
                "entries",
                "body_words",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v46: entries.body_words");
            ensure_column(
                &self.conn,
                "entries",
                "body_chars",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v46: entries.body_chars");
            self.conn
                .execute_batch("INSERT OR IGNORE INTO schema_version (version) VALUES (46);")
                .expect("migration v46: schema_version");
        }

        // CO-241 unconditional backfill — compute body_lines/words/chars for
        // meta-DB entries rows that still have the DEFAULT 0 values.
        ensure_column(
            &self.conn,
            "entries",
            "body_lines",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .expect("CO-241 backfill: entries.body_lines");
        ensure_column(
            &self.conn,
            "entries",
            "body_words",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .expect("CO-241 backfill: entries.body_words");
        ensure_column(
            &self.conn,
            "entries",
            "body_chars",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .expect("CO-241 backfill: entries.body_chars");
        {
            let rows: Vec<(i64, String)> = {
                let mut stmt = self
                    .conn
                    .prepare(
                        "SELECT rowid, body FROM entries \
                         WHERE body_chars = 0 AND body != ''",
                    )
                    .expect("CO-241 backfill prepare");
                stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                    .expect("CO-241 backfill query")
                    .filter_map(|r| r.ok())
                    .collect()
            };
            for (rowid, body) in rows {
                let lines = body.lines().count() as i64;
                let words = body.split_whitespace().count() as i64;
                let chars = body.chars().count() as i64;
                self.conn
                    .execute(
                        "UPDATE entries SET body_lines = ?1, body_words = ?2, body_chars = ?3 \
                         WHERE rowid = ?4",
                        rusqlite::params![lines, words, chars, rowid],
                    )
                    .expect("CO-241 backfill update");
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

        if current_version < 47 {
            // CO-260: changelog_cache — pre-computed per-entry PR-size data.
            // Populated by scripts/release-commit.sh + POST /api/v1/admin/changelog/reindex.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS changelog_cache (
                        version    TEXT NOT NULL,
                        ticket     TEXT NOT NULL,
                        entry_type TEXT NOT NULL DEFAULT 'feat',
                        title      TEXT NOT NULL DEFAULT '',
                        pr_number  INTEGER,
                        pr_size    INTEGER,
                        additions  INTEGER,
                        deletions  INTEGER,
                        commit_sha TEXT,
                        author     TEXT,
                        indexed_at INTEGER NOT NULL DEFAULT 0,
                        PRIMARY KEY (version, ticket)
                    );
                    CREATE INDEX IF NOT EXISTS idx_changelog_cache_version
                        ON changelog_cache(version);
                    CREATE INDEX IF NOT EXISTS idx_changelog_cache_type
                        ON changelog_cache(entry_type);
                    INSERT OR IGNORE INTO schema_version (version) VALUES (47);",
                )
                .expect("migration v47: changelog_cache");
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 48 {
            // CO-267: entry_origin — distinguishes seed-walker writes ('walker')
            // from co-sync push writes ('synced') in the meta-DB entries table.
            // Per-universe data.db gets the same column via universe_pool v14.
            ensure_column(
                &self.conn,
                "entries",
                "entry_origin",
                "TEXT NOT NULL DEFAULT ''",
            )
            .expect("migration v48: entries.entry_origin");
            self.conn
                .execute_batch("INSERT OR IGNORE INTO schema_version (version) VALUES (48);")
                .expect("migration v48: schema_version");
        }
        // CO-267 unconditional backfill — drift-safe guard.
        ensure_column(
            &self.conn,
            "entries",
            "entry_origin",
            "TEXT NOT NULL DEFAULT ''",
        )
        .expect("CO-267 backfill: entries.entry_origin");

        if current_version < 49 {
            // CO-273: deployment_snapshots — one row per deployable unit, updated by worker.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS deployment_snapshots (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        unit TEXT NOT NULL UNIQUE,
                        snapshot_at INTEGER NOT NULL DEFAULT 0,
                        machine_id TEXT NOT NULL DEFAULT '',
                        region TEXT NOT NULL DEFAULT '',
                        vm_size TEXT NOT NULL DEFAULT '',
                        state TEXT NOT NULL DEFAULT '',
                        image TEXT NOT NULL DEFAULT '',
                        version TEXT NOT NULL DEFAULT '',
                        last_deploy_at TEXT NOT NULL DEFAULT '',
                        health_status TEXT NOT NULL DEFAULT 'unknown',
                        error_msg TEXT NOT NULL DEFAULT ''
                    );
                    INSERT OR IGNORE INTO schema_version (version) VALUES (49);",
                )
                .expect("migration v49: deployment_snapshots");
        }

        if current_version < 50 {
            // CO-275: agent_sessions — one row per co-auto invocation, for kanban provenance.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS agent_sessions (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        task_id TEXT NOT NULL,
                        universe_key TEXT NOT NULL,
                        started_at INTEGER NOT NULL,
                        finished_at INTEGER NOT NULL,
                        duration_ms INTEGER NOT NULL,
                        exit_code INTEGER NOT NULL,
                        tokens_in INTEGER,
                        tokens_out INTEGER,
                        tool_calls TEXT,
                        skills_loaded TEXT,
                        context_chars INTEGER,
                        final_commit_sha TEXT,
                        pr_number INTEGER,
                        model TEXT,
                        co_auto_version TEXT,
                        raw_log_blob_sha TEXT
                    );
                    CREATE INDEX IF NOT EXISTS idx_agent_sessions_task_id
                        ON agent_sessions(task_id);
                    CREATE INDEX IF NOT EXISTS idx_agent_sessions_universe_started
                        ON agent_sessions(universe_key, started_at);
                    INSERT OR IGNORE INTO schema_version (version) VALUES (50);",
                )
                .expect("migration v50: agent_sessions");
        }

        if current_version < 51 {
            // CO-330: runtime universe→repo bindings + anon published-only filter.
            // Three additive columns on universes; nullable/default-safe.
            ensure_column(&self.conn, "universes", "local_repo_path", "TEXT")
                .expect("migration v51: universes.local_repo_path");
            ensure_column(&self.conn, "universes", "content_subdirs", "TEXT")
                .expect("migration v51: universes.content_subdirs");
            ensure_column(
                &self.conn,
                "universes",
                "anon_published_only",
                "INTEGER NOT NULL DEFAULT 0",
            )
            .expect("migration v51: universes.anon_published_only");
            // Backfill 8 known universe→repo bindings. All idempotent via
            // `WHERE local_repo_path IS NULL` so re-runs are no-ops.
            self.conn
                .execute_batch(r#"
                    UPDATE universes SET local_repo_path='~/projects/ArteLonga', content_subdirs='["docs","content"]' WHERE key='artelonga' AND local_repo_path IS NULL;
                    UPDATE universes SET local_repo_path='~/projects/quilomboaraucaria', content_subdirs='["relatos","jardim","quadro","eventos","membros","modelos"]' WHERE key='quilomboaraucaria' AND local_repo_path IS NULL;
                    UPDATE universes SET local_repo_path='~/projects/yggdrasil', content_subdirs='["docs","content"]' WHERE key='yggdrasil' AND local_repo_path IS NULL;
                    UPDATE universes SET local_repo_path='~/projects/rfq-gateway', content_subdirs='["docs","content"]' WHERE key='rfq' AND local_repo_path IS NULL;
                    UPDATE universes SET local_repo_path='~/projects/comunicacao', content_subdirs='["docs","content"]' WHERE key='comunicacao' AND local_repo_path IS NULL;
                    UPDATE universes SET local_repo_path='~/projects/mbya', content_subdirs='["docs","content"]' WHERE key='mbya' AND local_repo_path IS NULL;
                    UPDATE universes SET local_repo_path='~/projects/topologia', content_subdirs='["docs","content"]' WHERE key='topologia' AND local_repo_path IS NULL;
                    UPDATE universes SET local_repo_path='~/projects/yuri', content_subdirs='[""]', anon_published_only=1 WHERE key='yuri' AND local_repo_path IS NULL;
                    INSERT OR IGNORE INTO schema_version (version) VALUES (51);
                "#)
                .expect("migration v51: repo backfill + schema_version");
        }
        // Drift-safe guard: ensure columns exist even if v51 was interrupted.
        ensure_column(&self.conn, "universes", "local_repo_path", "TEXT")
            .expect("CO-330 guard: universes.local_repo_path");
        ensure_column(&self.conn, "universes", "content_subdirs", "TEXT")
            .expect("CO-330 guard: universes.content_subdirs");
        ensure_column(
            &self.conn,
            "universes",
            "anon_published_only",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .expect("CO-330 guard: universes.anon_published_only");

        if current_version < 52 {
            // CO-331: git-backed tool registry.
            ensure_table(
                &self.conn,
                "tools",
                "CREATE TABLE IF NOT EXISTS tools (
                    key           TEXT PRIMARY KEY,
                    name          TEXT NOT NULL,
                    description   TEXT,
                    remote_url    TEXT,
                    local_path    TEXT,
                    version_pin   TEXT,
                    entry_command TEXT,
                    installed_at  TEXT,
                    last_updated  TEXT,
                    follow_main   INTEGER NOT NULL DEFAULT 0,
                    lockfile_sha  TEXT
                );",
            )
            .expect("migration v52: tools table");
            self.conn
                .execute_batch("INSERT OR IGNORE INTO schema_version (version) VALUES (52);")
                .expect("migration v52: schema_version");
        }
        // Drift-safe guard: tools table always exists after v52.
        ensure_table(
            &self.conn,
            "tools",
            "CREATE TABLE IF NOT EXISTS tools (
                key           TEXT PRIMARY KEY,
                name          TEXT NOT NULL,
                description   TEXT,
                remote_url    TEXT,
                local_path    TEXT,
                version_pin   TEXT,
                entry_command TEXT,
                installed_at  TEXT,
                last_updated  TEXT,
                follow_main   INTEGER NOT NULL DEFAULT 0,
                lockfile_sha  TEXT
            );",
        )
        .expect("CO-331 guard: tools table");

        if current_version < 53 {
            // CO-333: feedback table — Yggdrasil-compatible + per-entry locus.
            // entry_path NULL = universe-wide (Yggdrasil compat); set = per-entry.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS feedback (
                        id           TEXT    PRIMARY KEY,
                        universe_key TEXT    NOT NULL,
                        entry_path   TEXT,
                        kind         TEXT    NOT NULL CHECK (kind IN ('feedback','duvida','sugestao')),
                        message      TEXT    NOT NULL,
                        name         TEXT,
                        email        TEXT,
                        user_sub     TEXT,
                        anonymous    INTEGER NOT NULL,
                        created_at   INTEGER NOT NULL,
                        status       TEXT    NOT NULL DEFAULT 'open'
                    );
                    CREATE INDEX IF NOT EXISTS idx_feedback_universe_path
                        ON feedback(universe_key, entry_path, created_at);
                    CREATE INDEX IF NOT EXISTS idx_feedback_status
                        ON feedback(status);
                    INSERT OR IGNORE INTO schema_version (version) VALUES (53);",
                )
                .expect("migration v53: feedback table");
        }
        // CO-333 drift-safe guard.
        ensure_table(
            &self.conn,
            "feedback",
            "CREATE TABLE IF NOT EXISTS feedback (
                id           TEXT    PRIMARY KEY,
                universe_key TEXT    NOT NULL,
                entry_path   TEXT,
                kind         TEXT    NOT NULL,
                message      TEXT    NOT NULL,
                name         TEXT,
                email        TEXT,
                user_sub     TEXT,
                anonymous    INTEGER NOT NULL,
                created_at   INTEGER NOT NULL,
                status       TEXT    NOT NULL DEFAULT 'open'
            );",
        )
        .expect("CO-333 guard: feedback table");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_feedback_universe_path \
                     ON feedback(universe_key, entry_path, created_at);
                 CREATE INDEX IF NOT EXISTS idx_feedback_status ON feedback(status);",
            )
            .expect("CO-333 guard: feedback indexes");

        // CO-334: rebased on top of CO-333; bumped to v54 to avoid collision.
        if current_version < 54 {
            // CO-334: cross-repo release notes aggregation.
            // Stores one row per (repo, version) from sister-repo CHANGELOG.md files.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS release_notes (
                        id          INTEGER PRIMARY KEY AUTOINCREMENT,
                        repo        TEXT    NOT NULL,
                        version     TEXT    NOT NULL,
                        date        TEXT    NOT NULL,
                        theme       TEXT,
                        body_md     TEXT    NOT NULL,
                        body_text   TEXT    NOT NULL,
                        ingested_at INTEGER NOT NULL,
                        UNIQUE(repo, version)
                    );
                    CREATE INDEX IF NOT EXISTS idx_release_notes_date
                        ON release_notes(date DESC);
                    CREATE INDEX IF NOT EXISTS idx_release_notes_repo
                        ON release_notes(repo, date DESC);
                    INSERT OR IGNORE INTO schema_version (version) VALUES (54);",
                )
                .expect("migration v54: release_notes");
        }
        // Drift-safe guard: release_notes always exists after v54.
        ensure_table(
            &self.conn,
            "release_notes",
            "CREATE TABLE IF NOT EXISTS release_notes (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                repo        TEXT    NOT NULL,
                version     TEXT    NOT NULL,
                date        TEXT    NOT NULL,
                theme       TEXT,
                body_md     TEXT    NOT NULL,
                body_text   TEXT    NOT NULL,
                ingested_at INTEGER NOT NULL,
                UNIQUE(repo, version)
            );",
        )
        .expect("CO-334 guard: release_notes table");

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 55 {
            // CO-336: traceability columns on feedback — linked PR/commit, owner
            // response, and public visibility flag.
            self.conn
                .execute_batch(
                    "ALTER TABLE feedback ADD COLUMN linked_ref TEXT;
                     ALTER TABLE feedback ADD COLUMN linked_summary TEXT;
                     ALTER TABLE feedback ADD COLUMN owner_response TEXT;
                     ALTER TABLE feedback ADD COLUMN public_visible INTEGER NOT NULL DEFAULT 0;
                     INSERT OR IGNORE INTO schema_version (version) VALUES (55);",
                )
                .expect("migration v55: feedback traceability columns");
        }
        // CO-336 drift-safe guard — ensure columns exist even if v55 was partially
        // applied on an older instance (same pattern as CO-137/CO-333).
        for sql in &[
            "ALTER TABLE feedback ADD COLUMN linked_ref TEXT",
            "ALTER TABLE feedback ADD COLUMN linked_summary TEXT",
            "ALTER TABLE feedback ADD COLUMN owner_response TEXT",
            "ALTER TABLE feedback ADD COLUMN public_visible INTEGER NOT NULL DEFAULT 0",
        ] {
            let _ = self.conn.execute(sql, []);
        }

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 56 {
            // CO-337: remote sister-repo sync — three additive nullable columns on universes.
            for sql in &[
                "ALTER TABLE universes ADD COLUMN remote_url TEXT",
                "ALTER TABLE universes ADD COLUMN remote_ref TEXT",
                "ALTER TABLE universes ADD COLUMN remote_last_sync TEXT",
            ] {
                let _ = self.conn.execute(sql, []);
            }
            self.conn
                .execute_batch("INSERT OR IGNORE INTO schema_version (version) VALUES (56);")
                .expect("migration v56: schema_version");
        }
        // CO-337 drift-safe guard.
        for sql in &[
            "ALTER TABLE universes ADD COLUMN remote_url TEXT",
            "ALTER TABLE universes ADD COLUMN remote_ref TEXT",
            "ALTER TABLE universes ADD COLUMN remote_last_sync TEXT",
        ] {
            let _ = self.conn.execute(sql, []);
        }
    }
}
