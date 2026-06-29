use super::super::Storage;
use super::super::schema::{ensure_column, table_exists};

impl Storage {
    pub(super) fn migrate_v032_041(&mut self, current_version: i64) {
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
            // CO-166 (1.76.0): historically linked tenant accounts to CO accounts.
            // CO-509: the tenant table is no longer created — no-op on a fresh DB,
            // applied only on legacy DBs that still carry it.
            if table_exists(&self.conn, "quilombo_usuarios") {
                ensure_column(&self.conn, "quilombo_usuarios", "linked_co_user_id", "TEXT")
                    .expect("migration v33: quilombo_usuarios.linked_co_user_id column");
            }
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (33)", [])
                .expect("Failed to record migration v33");
        }

        if current_version < 34 {
            // CO-167 (1.79.0): email bridge for legacy tenant users.
            // CO-509: no-op on a fresh DB (tenant table no longer created).
            if table_exists(&self.conn, "quilombo_usuarios") {
                ensure_column(&self.conn, "quilombo_usuarios", "email", "TEXT")
                    .expect("migration v34: quilombo_usuarios.email");
                ensure_column(&self.conn, "quilombo_usuarios", "linked_co_user_id", "TEXT")
                    .expect("migration v34: quilombo_usuarios.linked_co_user_id");
                self.conn
                    .execute_batch(
                        "CREATE UNIQUE INDEX IF NOT EXISTS idx_quilombo_usuarios_email
                             ON quilombo_usuarios(email) WHERE email IS NOT NULL;",
                    )
                    .expect("migration v34: idx_quilombo_usuarios_email");
            }
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (34)", [])
                .expect("Failed to record migration v34");
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
            // CO-509: tenant-table column is no-op on a fresh DB.
            if table_exists(&self.conn, "quilombo_usuarios") {
                ensure_column(&self.conn, "quilombo_usuarios", "telefone", "TEXT")
                    .expect("migration v36: quilombo_usuarios.telefone");
            }
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
    }
}
