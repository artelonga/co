use super::super::Storage;
use super::super::schema::{ensure_column, ensure_table};

impl Storage {
    pub(super) fn migrate_v042_045(&mut self, current_version: i64) {
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

        if current_version < 44 {
            // CO-177: universe-scoped telemetry queries (CO-179/CO-180 consumers).
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_telemetry_universe_time \
                     ON telemetry_events(universe_key, timestamp);
                     INSERT OR IGNORE INTO schema_version (version) VALUES (44);",
                )
                .expect("migration v44: idx_telemetry_universe_time");
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

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 44 {
            // CO-178: geo enrichment — country + city columns on telemetry_events.
            // Nullable: private/internal IPs and rows predating this migration stay NULL.
            ensure_column(&self.conn, "telemetry_events", "country", "TEXT")
                .expect("migration v44: telemetry_events.country");
            ensure_column(&self.conn, "telemetry_events", "city", "TEXT")
                .expect("migration v44: telemetry_events.city");
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_telemetry_country \
                     ON telemetry_events(country);
                     INSERT OR IGNORE INTO schema_version (version) VALUES (44);",
                )
                .expect("migration v44: country index + schema_version");
        }

        // CO-178 unconditional backfill — ensures country/city exist even if v44
        // was partially applied on an older instance.
        ensure_column(&self.conn, "telemetry_events", "country", "TEXT")
            .expect("CO-178 backfill: country");
        ensure_column(&self.conn, "telemetry_events", "city", "TEXT")
            .expect("CO-178 backfill: city");
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_telemetry_country \
                 ON telemetry_events(country);",
            )
            .expect("CO-178 backfill: country index");

        // CO-179: composite index for public analytics queries filtered by
        // (universe_key, timestamp). Cheap to create if already present.
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_telemetry_universe_time \
                 ON telemetry_events(universe_key, timestamp);",
            )
            .expect("CO-179: idx_telemetry_universe_time");

        let current_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 45 {
            // CO-237: hash API tokens at rest.
            // Add token_hash (SHA-256 hex) and token_prefix (first 11 chars for
            // display) columns. Existing plaintext tokens are invalidated — users
            // must re-create their tokens after this migration.
            ensure_column(&self.conn, "api_tokens", "token_hash", "TEXT")
                .expect("migration v45: api_tokens.token_hash");
            ensure_column(&self.conn, "api_tokens", "token_prefix", "TEXT")
                .expect("migration v45: api_tokens.token_prefix");
            self.conn
                .execute_batch(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_api_tokens_hash \
                     ON api_tokens(token_hash) WHERE token_hash IS NOT NULL;
                     DELETE FROM api_tokens;
                     INSERT OR IGNORE INTO schema_version (version) VALUES (45);",
                )
                .expect("migration v45: idx_api_tokens_hash + invalidate + schema_version");
        }

        // CO-237 unconditional backfill — ensures hash columns exist even if v45
        // was partially applied.
        ensure_column(&self.conn, "api_tokens", "token_hash", "TEXT")
            .expect("CO-237 backfill: api_tokens.token_hash");
        ensure_column(&self.conn, "api_tokens", "token_prefix", "TEXT")
            .expect("CO-237 backfill: api_tokens.token_prefix");
        self.conn
            .execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_api_tokens_hash \
                 ON api_tokens(token_hash) WHERE token_hash IS NOT NULL;",
            )
            .expect("CO-237 backfill: idx_api_tokens_hash");
    }
}
