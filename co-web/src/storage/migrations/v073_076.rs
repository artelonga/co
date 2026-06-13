use super::super::Storage;
use super::super::schema::ensure_column;

impl Storage {
    pub(super) fn migrate_v073_076(&mut self, current_version: i64) {
        if current_version < 73 {
            // CO-388: security_findings — stores vulnerability scan results from
            // the security audit pipeline (Project Glasswing step 11).
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS security_findings (
                       id              TEXT PRIMARY KEY,
                       pr_number       INTEGER NOT NULL,
                       severity        TEXT NOT NULL
                         CHECK (severity IN ('critical','high','medium','low','info')),
                       category        TEXT NOT NULL,
                       file_path       TEXT NOT NULL,
                       line_start      INTEGER,
                       line_end        INTEGER,
                       description     TEXT NOT NULL,
                       cwe             TEXT,
                       cve_match       TEXT,
                       suggested_patch TEXT,
                       detected_at     TEXT NOT NULL,
                       resolved_at     TEXT,
                       resolution_kind TEXT
                         CHECK (resolution_kind IS NULL OR
                                resolution_kind IN ('patched','accepted-risk','false-positive','wont-fix')),
                       resolution_pr   INTEGER,
                       -- CO-388 (security hardening): audit trail — which admin
                       -- resolved this finding (flips it to accepted-risk etc.,
                       -- which can open the release gate).
                       resolved_by     TEXT
                     );
                     CREATE INDEX IF NOT EXISTS idx_security_findings_severity
                       ON security_findings (severity, detected_at DESC);
                     CREATE INDEX IF NOT EXISTS idx_security_findings_unresolved
                       ON security_findings (detected_at DESC) WHERE resolved_at IS NULL;
                     -- CO-388 (security hardening): DB-backed per-day scan counter.
                     -- The previous in-memory AtomicU32 was rebuilt on every
                     -- request (build_backend() per call) so the daily cap never
                     -- tripped. One row per UTC day; the count persists across
                     -- requests and process restarts.
                     CREATE TABLE IF NOT EXISTS security_scan_counts (
                       scan_day TEXT PRIMARY KEY,   -- 'YYYY-MM-DD' (UTC)
                       count    INTEGER NOT NULL DEFAULT 0
                     );",
                )
                .expect("CO-388 v73: security_findings table + indexes");
            crate::record_migration!(
                self.conn,
                73,
                "CO-388: security_findings — pre-release vulnerability scan results (Project Glasswing)"
            );
        }

        if current_version < 74 {
            // CO-415: GitHub OAuth linkage. `github_login` stores GitHub's
            // stable login handle (the `login` field from GET /user), and is
            // what re-login matches on first — email can change but GitHub's
            // login is the durable identity. Nullable because most users won't
            // link GitHub. Mirrors the v39 `google_sub` pattern.
            ensure_column(&self.conn, "users", "github_login", "TEXT")
                .expect("migration v74: users.github_login");
            // Partial unique index — one GitHub account links to at most one CO
            // user. NULL `github_login` rows are excluded (multiple unlinked
            // users coexist). NOTE: deliberately its own step, NOT folded into
            // the base UNIVERSE_SCHEMA batch — that batch runs on existing DBs
            // before migrations and would reference a not-yet-added column
            // (CO-354 trap).
            self.conn
                .execute_batch(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_github_login \
                     ON users(github_login) WHERE github_login IS NOT NULL;",
                )
                .expect("migration v74: idx_users_github_login");
            crate::record_migration!(
                self.conn,
                74,
                "CO-415: users.github_login + idx_users_github_login (GitHub OAuth login)"
            );
        }

        if current_version < 75 {
            // CO-426: usage_sessions — fleet-wide token/cost telemetry, one row
            // per co-auto Claude invocation (producer: CO-425's stream-json
            // capture, POSTed to /api/v1/usage/sessions). Deliberately a NEW
            // table, NOT the CO-275 `agent_sessions` table: that one is per-task
            // kanban provenance (task_id, finished_at, exit_code, tool_calls);
            // this is the cross-machine usage ledger (task_key, machine,
            // cache-split tokens, cost_usd, outcome) that feeds the Gestão usage
            // dashboard and the CO-427 downshift policy. Global fleet telemetry
            // → meta DB (not per-universe `universe_pool.rs`).
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS usage_sessions (
                        id                 INTEGER PRIMARY KEY AUTOINCREMENT,
                        task_key           TEXT NOT NULL,
                        universe_key       TEXT NOT NULL,
                        machine            TEXT NOT NULL,
                        model              TEXT NOT NULL,
                        tokens_in          INTEGER NOT NULL DEFAULT 0,
                        tokens_out         INTEGER NOT NULL DEFAULT 0,
                        tokens_cache_read  INTEGER NOT NULL DEFAULT 0,
                        tokens_cache_write INTEGER NOT NULL DEFAULT 0,
                        cost_usd           REAL,
                        started_at         INTEGER NOT NULL,
                        ended_at           INTEGER NOT NULL,
                        outcome            TEXT NOT NULL DEFAULT '',
                        reported_at        INTEGER NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_usage_sessions_started
                        ON usage_sessions(started_at);
                    CREATE INDEX IF NOT EXISTS idx_usage_sessions_universe_started
                        ON usage_sessions(universe_key, started_at);
                    CREATE INDEX IF NOT EXISTS idx_usage_sessions_model_started
                        ON usage_sessions(model, started_at);
                    CREATE INDEX IF NOT EXISTS idx_usage_sessions_machine_started
                        ON usage_sessions(machine, started_at);
                    CREATE INDEX IF NOT EXISTS idx_usage_sessions_task_started
                        ON usage_sessions(task_key, started_at);",
                )
                .expect("migration v75: usage_sessions");
            crate::record_migration!(
                self.conn,
                75,
                "CO-426: usage_sessions (fleet token/cost telemetry for Gestão usage dashboard)"
            );
        }

        if current_version < 76 {
            // CO-437: enrich usage_sessions with metadata captured from the
            // stream-json (CO-425 extended) — `tool_uses` (a name→count JSON
            // map), `pr_url` (the PR co-auto opened) and `turns` (agent turns
            // from the `result` event). Additive, backwards-compatible columns
            // on the existing table; the version guard makes the ALTERs run
            // exactly once. New columns are read via explicit SELECTs (never
            // `.ok()`-swallowed) so a missing column fails loudly.
            self.conn
                .execute_batch(
                    "ALTER TABLE usage_sessions ADD COLUMN tool_uses TEXT;
                     ALTER TABLE usage_sessions ADD COLUMN pr_url TEXT;
                     ALTER TABLE usage_sessions ADD COLUMN turns INTEGER NOT NULL DEFAULT 0;",
                )
                .expect("migration v76: usage_sessions metadata columns");
            crate::record_migration!(
                self.conn,
                76,
                "CO-437: usage_sessions.tool_uses/pr_url/turns (usage metadata enrichment)"
            );
        }
    }
}
