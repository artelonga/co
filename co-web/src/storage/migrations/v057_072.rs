use super::super::Storage;
use super::super::schema::ensure_column;

impl Storage {
    pub(super) fn migrate_v057_072(&mut self, current_version: i64) {
        if current_version < 57 {
            // CO-339: backfill probe/scanner entries — mark existing open rows with
            // empty or short bodies (< 5 chars) as wont-fix so they don't surface
            // in the operator's notification inbox. Idempotent on fresh DBs (0 rows
            // affected). The `owner_response` column (added in v55) holds the reason.
            self.conn
                .execute_batch(
                    "UPDATE feedback
                        SET status = 'wont-fix',
                            owner_response = 'auto-resolved: probe traffic (CO-339)'
                      WHERE status = 'open'
                        AND (message IS NULL OR TRIM(message) = '' OR LENGTH(TRIM(message)) < 5)
                        AND created_at < (strftime('%s','now') - 60);
                     INSERT OR IGNORE INTO schema_version (version) VALUES (57);",
                )
                .expect("migration v57: probe cleanup");
        }

        if current_version < 58 {
            // CO-340: analytics rollups — agregado diário CONSENTIDO, sem PII, por
            // universe. É o que producers (surfaces universe-owned, parceiros, universes
            // co) pusham; o summary central faz a ponte (universe + path histórico).
            // metrics/dims = JSON (DailyRollup, ver openapi do artelonga). PK (universe, day)
            // → upsert idempotente.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS analytics_rollups (
                        universe_key TEXT NOT NULL,
                        day          TEXT NOT NULL,
                        metrics      TEXT NOT NULL,
                        dims         TEXT NOT NULL DEFAULT '{}',
                        updated_at   TEXT NOT NULL,
                        PRIMARY KEY (universe_key, day)
                     );
                     CREATE INDEX IF NOT EXISTS idx_rollups_universe_day
                        ON analytics_rollups(universe_key, day);
                     INSERT OR IGNORE INTO schema_version (version) VALUES (58);",
                )
                .expect("migration v58: analytics_rollups");
        }

        if current_version < 59 {
            // CO-345: graph_views — publishable saved graph views.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS graph_views (
                        slug             TEXT PRIMARY KEY,
                        owner_id         TEXT NOT NULL,
                        name             TEXT NOT NULL,
                        universe_filter  TEXT NOT NULL,
                        type_filter      TEXT,
                        relation_filter  TEXT,
                        depth            INTEGER,
                        root             TEXT,
                        layout_seed      INTEGER,
                        visibility       TEXT NOT NULL DEFAULT 'private',
                        created_at       TEXT NOT NULL,
                        updated_at       TEXT NOT NULL
                     );
                     CREATE INDEX IF NOT EXISTS idx_graph_views_owner
                        ON graph_views(owner_id);
                     INSERT OR IGNORE INTO schema_version (version) VALUES (59);",
                )
                .expect("migration v59: graph_views");
        }

        if current_version < 60 {
            // CO-361: atividades audit log + schema_versoes migration history.
            // `atividades` records every meaningful mutation with a before/after diff
            // (sensitive keys redacted), typed acao/tipo enums, hashed IP, and the
            // app version that generated the event.
            // `schema_versoes` records each migration step with the app version that
            // applied it. Existing versions 1..59 are backfilled from schema_version
            // with descricao='(backfilled)' and versao_app='unknown'.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS atividades (
                       id          INTEGER PRIMARY KEY AUTOINCREMENT,
                       acao        TEXT NOT NULL CHECK (acao IN ('criar','ler','atualizar','excluir','login','logout')),
                       entidade    TEXT NOT NULL,
                       entidade_id TEXT,
                       conteudo    TEXT,
                       tipo        TEXT NOT NULL CHECK (tipo IN ('sucesso','erro','navegacao','sistema')),
                       user_id     TEXT,
                       ip_hash     TEXT,
                       user_agent  TEXT,
                       versao_app  TEXT,
                       criado_em   TEXT NOT NULL DEFAULT (datetime('now'))
                     );
                     CREATE INDEX IF NOT EXISTS idx_atividades_criado ON atividades(criado_em DESC);
                     CREATE INDEX IF NOT EXISTS idx_atividades_user   ON atividades(user_id, criado_em DESC);
                     CREATE INDEX IF NOT EXISTS idx_atividades_acao   ON atividades(acao, entidade, criado_em DESC);

                     CREATE TABLE IF NOT EXISTS schema_versoes (
                       versao      INTEGER PRIMARY KEY,
                       descricao   TEXT NOT NULL,
                       versao_app  TEXT NOT NULL,
                       applied_at  TEXT NOT NULL DEFAULT (datetime('now'))
                     );

                     INSERT OR IGNORE INTO schema_versoes (versao, descricao, versao_app)
                       SELECT version, '(backfilled)', 'unknown' FROM schema_version;

                     INSERT OR IGNORE INTO schema_version (version) VALUES (60);
                     INSERT OR IGNORE INTO schema_versoes (versao, descricao, versao_app)
                       VALUES (60, 'atividades audit log + schema_versoes', 'unknown');",
                )
                .expect("migration v60: atividades + schema_versoes");
        }

        if current_version < 61 {
            // CO-370: lead-user join — leads side.
            // Adds user_id (FK → users.id) and source so every lead knows
            // which acquisition channel created it and which user it belongs to.
            use crate::storage::schema::ensure_column;
            ensure_column(&self.conn, "leads", "user_id", "TEXT")
                .expect("CO-370 v61: leads.user_id");
            ensure_column(&self.conn, "leads", "source", "TEXT DEFAULT 'lead_form'")
                .expect("CO-370 v61: leads.source");
            self.conn
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_leads_email    ON leads(email);
                     CREATE INDEX IF NOT EXISTS idx_leads_user_id  ON leads(user_id);",
                )
                .expect("CO-370 v61: leads indexes");
            crate::record_migration!(
                self.conn,
                61,
                "CO-370: leads.user_id + leads.source + indexes"
            );
        }

        if current_version < 62 {
            // CO-370: lead-user join — users side.
            // Adds lead_id (FK → leads.id), status, and activated_at to users.
            // Backfills both FKs by matching existing records on lowercased email.
            use crate::storage::schema::ensure_column;
            ensure_column(&self.conn, "users", "lead_id", "INTEGER")
                .expect("CO-370 v62: users.lead_id");
            ensure_column(&self.conn, "users", "status", "TEXT DEFAULT 'active'")
                .expect("CO-370 v62: users.status");
            ensure_column(&self.conn, "users", "activated_at", "TEXT")
                .expect("CO-370 v62: users.activated_at");
            self.conn
                .execute_batch("CREATE INDEX IF NOT EXISTS idx_users_lead_id ON users(lead_id);")
                .expect("CO-370 v62: users.lead_id index");
            // Backfill: match existing leads → users by email (first lead wins per user).
            self.conn
                .execute_batch(
                    "UPDATE leads
                       SET user_id = (
                         SELECT id FROM users
                         WHERE lower(users.email) = lower(leads.email)
                         LIMIT 1
                       )
                     WHERE leads.email IS NOT NULL AND leads.user_id IS NULL;

                     UPDATE users
                       SET lead_id = (
                         SELECT id FROM leads
                         WHERE lower(leads.email) = lower(users.email)
                         ORDER BY leads.id ASC
                         LIMIT 1
                       )
                     WHERE users.email IS NOT NULL AND users.lead_id IS NULL;",
                )
                .expect("CO-370 v62: backfill email join");
            crate::record_migration!(
                self.conn,
                62,
                "CO-370: users.lead_id + users.status + users.activated_at + backfill"
            );
        }

        if current_version < 63 {
            // CO-380: universal EDA event_log — append-only replay store.
            // Every event published to the EdaBus is also persisted here for
            // 30-day retention + replay. `AtividadesPersistor` subscriber writes
            // each row; the retention task prunes rows older than 30 days nightly.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS event_log (
                       id           TEXT PRIMARY KEY,
                       event_type   TEXT NOT NULL,
                       universe_key TEXT,
                       user_id      TEXT,
                       payload_json TEXT NOT NULL DEFAULT '{}',
                       visibility   TEXT NOT NULL DEFAULT 'Public',
                       created_at   TEXT NOT NULL
                     );
                     CREATE INDEX IF NOT EXISTS idx_event_log_created
                       ON event_log(created_at DESC);
                     CREATE INDEX IF NOT EXISTS idx_event_log_universe
                       ON event_log(universe_key, created_at DESC);
                     CREATE INDEX IF NOT EXISTS idx_event_log_type
                       ON event_log(event_type, created_at DESC);",
                )
                .expect("CO-380 v63: event_log table + indexes");
            crate::record_migration!(
                self.conn,
                63,
                "CO-380: event_log table + created/universe/type indexes"
            );
        }

        if current_version < 64 {
            // CO-383: reserved — spec-only change (instance-qualified note path),
            // no database schema mutation needed.
            crate::record_migration!(
                self.conn,
                64,
                "CO-383: reserved (spec fix, no schema change)"
            );
        }

        if current_version < 65 {
            // CO-384: bridge_state — tracks per-(source,target) WS bridge state for
            // federated event bus. Stores last ACK'd event ULID for replay on reconnect.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS bridge_state (
                       id                      TEXT PRIMARY KEY,
                       source_deployment       TEXT NOT NULL,
                       target_deployment       TEXT NOT NULL,
                       last_delivered_event_id TEXT,
                       last_connected_at       TEXT,
                       last_disconnected_at    TEXT,
                       state                   TEXT NOT NULL
                         CHECK (state IN ('connected','disconnected','degraded'))
                     );
                     CREATE INDEX IF NOT EXISTS idx_bridge_state_source
                       ON bridge_state(source_deployment);
                     CREATE INDEX IF NOT EXISTS idx_bridge_state_target
                       ON bridge_state(target_deployment);",
                )
                .expect("CO-384 v65: bridge_state table + indexes");
            crate::record_migration!(
                self.conn,
                65,
                "CO-384: bridge_state table for federated WS event bus"
            );
        }

        if current_version < 66 {
            // CO-389: source_marker column added to per-universe entries tables
            // (via universe_pool migration v17). No meta.db schema change needed;
            // this marker records the migration version in schema_versoes.
            crate::record_migration!(
                self.conn,
                66,
                "CO-389: entries.source_marker via universe_pool v17 (yggdrasil-live overlay)"
            );
        }

        if current_version < 67 {
            // CO-385: sync_conflicts — stores detected cross-device conflicts and
            // their resolution state. Indexed for fast unresolved-conflict queries.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS sync_conflicts (
                       id                   TEXT PRIMARY KEY,
                       universe_key         TEXT NOT NULL,
                       path                 TEXT NOT NULL,
                       local_body_hash      TEXT NOT NULL,
                       remote_body_hash     TEXT NOT NULL,
                       common_ancestor_hash TEXT,
                       kind                 TEXT NOT NULL,
                       detected_at          TEXT NOT NULL,
                       resolved_at          TEXT,
                       resolution_action    TEXT,
                       resolved_by          TEXT,
                       FOREIGN KEY (universe_key) REFERENCES universes(key)
                     );
                     CREATE INDEX IF NOT EXISTS idx_sync_conflicts_unresolved
                       ON sync_conflicts(detected_at) WHERE resolved_at IS NULL;
                     CREATE INDEX IF NOT EXISTS idx_sync_conflicts_universe
                       ON sync_conflicts(universe_key, detected_at DESC);",
                )
                .expect("CO-385 v67: sync_conflicts table + indexes");
            crate::record_migration!(
                self.conn,
                67,
                "CO-385: sync_conflicts table for Mac-style cross-device conflict resolution"
            );
        }

        if current_version < 68 {
            // CO-383: source attribution columns for event-bus-backed universes.
            // (Originally reserved in v64 as a no-op; v65–67 were claimed by CO-384/CO-389/CO-385.)
            ensure_column(&self.conn, "universes", "source_kind", "TEXT")
                .expect("CO-383 v68: universes.source_kind");
            ensure_column(&self.conn, "universes", "source_url", "TEXT")
                .expect("CO-383 v68: universes.source_url");
            ensure_column(&self.conn, "universes", "source_last_event_at", "TEXT")
                .expect("CO-383 v68: universes.source_last_event_at");
            // Backfill the yggdrasil universe as an event-bus subscriber.
            if let Err(e) = self.conn.execute(
                "UPDATE universes \
                 SET source_kind = 'event-bus', \
                     source_url  = 'wss://yggdrasil.artelonga.com.br/api/v1/events' \
                 WHERE key = 'yggdrasil'",
                [],
            ) {
                tracing::warn!("CO-383 v68: yggdrasil backfill skipped: {e}");
            }
            crate::record_migration!(
                self.conn,
                68,
                "CO-383: source attribution columns + yggdrasil event-bus binding"
            );
        }

        if current_version < 69 {
            // CO-378: privacy — mark rollup rows that aggregate private-path traffic
            // so the public summary can strip them from top_pages listings.
            ensure_column(
                &self.conn,
                "analytics_rollups",
                "path_private",
                "INTEGER DEFAULT 0",
            )
            .expect("CO-378 v69: analytics_rollups.path_private");
            crate::record_migration!(
                self.conn,
                69,
                "CO-378: analytics_rollups.path_private — private rollup redaction"
            );
        }

        if current_version < 70 {
            // CO-352: per-user spatial canvas state for sala (workspace canvas).
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS workspace_states (
                       id             TEXT PRIMARY KEY,
                       universe_key   TEXT NOT NULL,
                       workspace_slug TEXT NOT NULL,
                       user_id        TEXT NOT NULL,
                       layout_json    TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
                       is_public      INTEGER NOT NULL DEFAULT 0,
                       share_token    TEXT,
                       created_at     TEXT NOT NULL,
                       updated_at     TEXT NOT NULL,
                       UNIQUE (universe_key, workspace_slug, user_id)
                     );
                     CREATE INDEX IF NOT EXISTS idx_workspace_states_user
                       ON workspace_states (user_id);
                     CREATE INDEX IF NOT EXISTS idx_workspace_states_token
                       ON workspace_states (share_token) WHERE share_token IS NOT NULL;",
                )
                .expect("CO-352 v70: workspace_states table + indexes");
            crate::record_migration!(
                self.conn,
                70,
                "CO-352: workspace_states — per-user spatial canvas state"
            );
        }

        if current_version < 71 {
            // CO-367: universal KB sync warehouse.
            // `entry_kb_index`  — history table keyed by (universe_key, entry_path, body_hash).
            // `entry_kb_latest` — latest-version view per (universe_key, entry_path).
            // `entry_kb_fts`    — FTS5 virtual table for full-text search over body_preview.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS entry_kb_index (
                       universe_key     TEXT NOT NULL,
                       entry_path       TEXT NOT NULL,
                       body_hash        TEXT NOT NULL,
                       entry_type       TEXT,
                       frontmatter_json TEXT,
                       body_preview     TEXT,
                       size_bytes       INTEGER,
                       updated_at       TEXT NOT NULL,
                       indexed_at       TEXT NOT NULL,
                       asset_refs       TEXT,
                       PRIMARY KEY (universe_key, entry_path, body_hash)
                     );
                     CREATE INDEX IF NOT EXISTS idx_kb_universe_type
                       ON entry_kb_index(universe_key, entry_type);
                     CREATE INDEX IF NOT EXISTS idx_kb_updated
                       ON entry_kb_index(updated_at);

                     CREATE TABLE IF NOT EXISTS entry_kb_latest (
                       universe_key     TEXT NOT NULL,
                       entry_path       TEXT NOT NULL,
                       body_hash        TEXT NOT NULL,
                       entry_type       TEXT,
                       frontmatter_json TEXT,
                       body_preview     TEXT,
                       size_bytes       INTEGER,
                       updated_at       TEXT NOT NULL,
                       indexed_at       TEXT NOT NULL,
                       asset_refs       TEXT,
                       PRIMARY KEY (universe_key, entry_path)
                     );
                     CREATE INDEX IF NOT EXISTS idx_kb_latest_indexed
                       ON entry_kb_latest(indexed_at DESC);
                     CREATE INDEX IF NOT EXISTS idx_kb_latest_type
                       ON entry_kb_latest(universe_key, entry_type);

                     CREATE VIRTUAL TABLE IF NOT EXISTS entry_kb_fts USING fts5(
                       universe_key UNINDEXED,
                       entry_path   UNINDEXED,
                       body_preview
                     );",
                )
                .expect("CO-367 v71: entry_kb_index + entry_kb_latest + entry_kb_fts");
            crate::record_migration!(
                self.conn,
                71,
                "CO-367: entry_kb_index + entry_kb_latest + entry_kb_fts (universal KB sync)"
            );
        }

        if current_version < 72 {
            // CO-398: delivery pipeline — task_status_log tracks every status
            // transition for lead-time telemetry (time per column, todo→done).
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS task_status_log (
                       id           TEXT PRIMARY KEY,
                       universe_key TEXT NOT NULL,
                       entry_path   TEXT NOT NULL,
                       status_from  TEXT,
                       status_to    TEXT NOT NULL,
                       trigger      TEXT NOT NULL DEFAULT 'manual',
                       triggered_at TEXT NOT NULL
                     );
                     CREATE INDEX IF NOT EXISTS idx_task_status_log_entry
                       ON task_status_log(universe_key, entry_path, triggered_at DESC);
                     CREATE INDEX IF NOT EXISTS idx_task_status_log_status
                       ON task_status_log(universe_key, status_to, triggered_at DESC);",
                )
                .expect("CO-398 v72: task_status_log table + indexes");
            crate::record_migration!(
                self.conn,
                72,
                "CO-398: task_status_log — delivery pipeline lead-time tracking"
            );
        }
    }
}
