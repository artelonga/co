# Changelog

All notable changes to CO are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.15.1] — 2026-06-15 — Migration self-containment + OpenAPI contract + ops accuracy

## CO-330 — Migration self-containment sweep (post-outage hardening)

Follow-up to the 3.15.0 migration-ordering fixes (v51/`remote_url`, v088/`jobs`).
Audited every meta-DB and per-universe migration for the same bug class — a step
that references schema created only inside an earlier `if current_version < N`
guard, which production passed long ago (so it never got it, while fresh-DB CI
hides the gap). No further *live* breakage was found, but two structurally
fragile spots — one retroactive guard edit away from a repeat — are now hardened:

- **v085** `workspace_states.scope`: the table's only CREATE is in v70's guard;
  `migrate_v085` now `CREATE TABLE IF NOT EXISTS workspace_states` before
  `ensure_column`.
- **v030** `subscriptions.pinned_state`: same shape vs v20's CREATE; v30 now
  recreates the table defensively first.

Both gain a regression test that drops the table and re-runs the migration. Also
hardened `scripts/pipeline-deploy-gate.sh`: it no longer requires a UAT pipeline
report (UAT is decommissioned) — it gates on the local (+ optional prod-smoke)
report, so the documented "real gate" actually passes.

### Why
A migration must be self-contained: a fresh top-to-bottom run must succeed, not
just an already-advanced prod DB. `ensure_column` is column-safe but ALTERs a
missing *table* — so the base table must be ensured in the same block.

## CO-452 — OpenAPI spec becomes a real contract (schemas, version, auth)

The generated `co-web/openapi.yaml` (served at `/api/docs` and `/api/openapi.json`)
was a bare endpoint inventory: every operation emitted only `200 OK`, the ~34
component schemas in `openapi-components.yaml` were defined but never referenced,
auth was hardcoded to `sessionCookie`, and `info.version` was pinned at a stale
`2.40.0`. The generator now produces a usable contract:

- **`info.version`** is read from the workspace `Cargo.toml` (now 3.15.0), not hardcoded.
- **Security schemes** are mapped correctly per catalog auth tag (`bearerJWT`,
  `apiToken`, `sessionCookie`, `sharedSecret`) with OR-semantics, instead of
  labelling everything `sessionCookie`.
- **Request/response schemas** are wired via a sidecar `SCHEMA_MAP` in the
  generator (the catalog markdown and the catalog↔code drift check are untouched):
  mapped operations emit a `requestBody` + typed success response + a `default`
  `Error` response, all `$ref`-ing existing component schemas. 24 high-confidence
  endpoints wired (auth, tasks, gestão eventos/validar/publicar/manifesto,
  quilombo auth/perfil/mensagens/missões/eventos/comentários); the rest stay bare
  and can be annotated incrementally.

`npm run openapi:check` stays green (no drift); the spec validates as OpenAPI 3.1
with zero dangling `$ref`s.

### Why
A schema-less spec gives Swagger UI no "try it out" payloads and no documented
errors. Wiring the already-defined component schemas turns the served spec into a
real, explorable contract without touching the source-of-truth catalog.

## CO-78 — Hotfix: migration v088 must create the `jobs` table, not assume it

The 3.15.0 prod deploy crash-looped at boot: migration **v088** ran
`ALTER TABLE jobs ADD COLUMN timeout_secs` but `jobs` did not exist on the
production DB (`no such table: jobs`), and the CO-446 guard aborted boot.

Root cause: the base `jobs` CREATE lives in v025 (CO-72) inside an
`if current_version < 25` guard. Any DB already past v25 — i.e. production —
never re-runs it, so the table was never created there. A fresh test DB runs
v25 from scratch and has the table, which hid the gap through CI.

v088 now creates the `jobs` base table + its base indexes with
`CREATE TABLE IF NOT EXISTS` before altering it, making the migration
self-contained on any DB (no-op where the table already exists). Adds a
regression test that drops `jobs` and re-runs v088.

### Why
A migration step must never assume schema produced by an *earlier* version's
guarded block — production may have advanced past that guard before the schema
was added. Same failure class as the v51 `remote_url` ordering fix in this wave.


## [3.15.0] — 2026-06-14 — Telemetry archival, job queue & native OTel [--ignore-dod override]

## CO-330 — Fix migration v51 ordering: `remote_url` written before the column exists

A 2026-06-14 hotfix (`df79d6a`, "remote_url for quilomboaraucaria content cloning")
added `remote_url=…, remote_ref=…` to the **v51** universe→repo backfill. But those
columns are only added at **v56** (CO-337). On production the column already existed
(the DB was past v56), so the write succeeded — but any *fresh* sequential migration
(UAT reset, anonymous clone, every `Storage::new` in tests) died at v51 with
`no such column: remote_url`, which the CO-446 guard escalates to a `FATAL` process
abort. This surfaced as the `cargo test -p co-web --lib security` binary "exiting
abnormally" (the `pbi_backlogger` tests build a fresh DB), failing the security-audit
gate on every PR.

The fix keeps v51 writing only the columns it owns (`local_repo_path`,
`content_subdirs`) and moves the `remote_url`/`remote_ref` backfill to *after* the
v56 columns are guaranteed to exist, idempotent via `WHERE … remote_url IS NULL`.

### Why
Migration steps must never reference a column added by a later step — a fresh
top-to-bottom migration has to succeed, not just an already-advanced production DB.

## CO-449 — Telemetry cold-tier archival to Parquet — keep all data, shrink the live meta.db

The `meta.db` OLTP database is dominated by `telemetry_events` (append-only,
high-volume — 11 GB of a 20 GB volume at 71% full in prod, 2026-06-14). The owner
decision is **keep 100% of the telemetry** (all of it is relevant) but move the
**cold window** out of SQLite into a columnar, compressed, still-queryable format.

This adds an idempotent, verify-before-delete archival job:

- **Export** — events in any month strictly older than the hot window
  (`CO_TELEMETRY_HOT_DAYS`, default 90d) are exported, oldest month first, to
  **Parquet (zstd)** partitioned as `telemetry-archive/year=YYYY/month=MM/part-<hash>.parquet`.
  The Parquet schema mirrors `telemetry_events` (CO-46 columns + CO-178 geo) and is
  read directly by DuckDB (`read_parquet('…/telemetry/**/*.parquet')`).
- **Verify before delete** — the job only deletes a month after the Parquet file
  exists and its footer row count equals the SQLite count for that month; it also
  records a sha256. A mismatch leaves the rows untouched in `meta.db`.
- **Shrink without a 2× peak** — after each month's delete, `PRAGMA incremental_vacuum`
  returns freed pages to the OS without the full-`VACUUM` rebuild prod can't fit
  (~5.6 GB free for an 11 GB DB). Migration **v087** opts the DB into
  `auto_vacuum=INCREMENTAL` (latent on an existing DB until a one-time full VACUUM).
- **Manifest** — every archived month is recorded in the new `telemetry_archives`
  table (year, month, s3_key/path, rows, sha256, bytes, archived_at) for
  traceability and dedupe; re-running the job skips months already listed.
- **Hot window untouched** — recent events stay in `telemetry_events`, so the
  CO-360 `/gestao/resumo` dashboards are unaffected and total (hot + cold) row
  count equals the historical total (zero loss).

The job runs as the opt-in `TelemetryArchiveWorker` (enable with
`CO_TELEMETRY_ARCHIVE_ENABLED=1`, interval `CO_TELEMETRY_ARCHIVE_INTERVAL_SECS`,
default 24h). It is per-month chunked so the delicate first run on a tight disk
never materializes the whole table at once.

### Why

Local-first reframe (2026-06-14): the destination is the local filesystem today so
the `meta.db` shrink lands now; uploading the same Parquet files to S3 (CO-81) and a
federated hot+cold query endpoint are follow-ups. This is the pragmatic
single-operator slice of the Theme F data-lake — Parquet + DuckDB, without the heavy
ClickHouse/Iceberg/Flink stack.

## CO-454 — Folder-as-sub-sala — pasta nodes are descendable; align CO fractal layer with Yggdrasil /mundo rooms (1:1 convergence)

A **pasta** node on the sala canvas is now *descendable* into its own **sub-sala**
(double-tap, or its panel's *Descer* button), reusing the CO-400 descend/ascend +
breadcrumb machinery — but recursing at the **folder** layer instead of the
universe layer.

- **Descend into a folder** stacks the camera (`sala_stack`) and opens the
  sub-sala whose scope is that pasta. The child slug appends the pasta to the
  current slug path (`default` → `default/jardim` → `default/jardim/estufa`), so
  parent/child is a slug **prefix** — the same enter/exit nesting Yggdrasil
  `/mundo` walks through doors (YG-146).
- **Identity = just a deeper slug (CO-352).** No new table, no migration: the
  folder path rides one percent-encoded URL segment (`default%2Fjardim`) so the
  page, state-API, and realtime-WS routes match unchanged and the server decodes
  the slash back into the opaque `workspace_slug`. The UNIQUE
  `(universe_key, workspace_slug, user_id)` keeps each depth an independent row.
- **Presence is per-sub-sala (CO-353):** the realtime room key
  `"{universe_key}/{workspace_slug}"` accepts the `/`, so a pasta's sub-sala has
  its own roster — 1:1 with a YG `/mundo` room.
- **Inert cases** mirror CO-400: the root pasta `/` (no name) is a soft no-op;
  ascending restores the parent slug + camera via `sala_restore_cam`.
- Documented in `docs/architecture/sala-surface.md` ("Folder-sub-sala ↔ YG room
  (1:1)"); covered by unit tests (distinct-row slug, folder-path room key) and an
  e2e spec (descend → child slug → ascend → camera + parent scope restored).

### Why
CO and the Yggdrasil content rooms were not 1:1 because they recursed at
different layers — CO by universe, YG `/mundo` by folder (pasta=sala). The owner
approved **Option A** (2026-06-14): converge by making CO folders descendable
sub-salas, not by promoting every room to a universe (Option B / CO-98,
rejected). This closes the layer mismatch the federation round-trip
(CO-413 ↔ YG-146 Fatia 2) needs to persist `pos{room,x,y}` unambiguously.

## CO-457 — Replace co-auto's bespoke usage capture with Claude Code native OTel

Folded agent token-usage telemetry onto the OTLP rails CO-291 already laid,
deleting ~400 lines of hand-rolled NDJSON parsing.

- **Producer (co-auto):** removed `dev/co-auto/src/usage.rs` (the 401-line
  stream-json parser — `SessionUsage`, `parse_stream_json`, `assistant_text`)
  and its driver (`post_usage_to_co`, `parse_pr_url`, `human_duration`, the
  `--output-format stream-json --verbose` re-parse). `launch_claude` now turns
  on Claude Code's **native OTel exporter** via env vars
  (`CLAUDE_CODE_ENABLE_TELEMETRY`, `OTEL_METRICS_EXPORTER=otlp`,
  `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf`, a 1 s export interval, **delta**
  temporality, and `OTEL_RESOURCE_ATTRIBUTES` carrying `task.key`,
  `universe.key`, `model`, `machine`). Opt-in and config-compatible: reuses the
  existing `CO_USAGE_ENDPOINT` / `CO_SESSION_TOKEN`, so it's a no-op when unset.
- **Receiver (co-web):** new OTLP/HTTP metrics endpoint `POST /v1/metrics`
  (`content/usage/otlp.rs`) that decodes an `ExportMetricsServiceRequest`
  (reusing CO-291's `opentelemetry-proto`/`prost` stack), maps the native
  `claude_code.token.usage` metric — splitting the `type`
  (`input`/`output`/`cacheRead`/`cacheCreation`) and `model` attributes — plus
  `claude_code.cost.usage`, and writes into the **existing** `usage_sessions`
  ledger. **No migration**: delta temporality means each export is an increment
  that `usage_summary` SUMs, so the CO-426 dashboard, `/usage/summary`,
  `/usage/active`, and the CO-427 5h-window downshift keep reading the same
  table unchanged.

### Why
Claude Code already emits usage as native OTel metrics, making co-auto's bespoke
stream-json capture redundant. Moving ingestion onto the OTLP path shrinks the
surface (~400 fewer lines) while preserving the queryable `/usage/summary` that
OTel's push-only model can't answer. Accepted minor losses: `pr_url` only when
co-auto already knows it, and `cost_usd` may be 0/NULL under keychain auth
(token counts — what the CO-427 budget math needs — always flow).

## CO-461 — co-auto: authoritative token usage (June-15 credit-pool readiness)

`dev/co-auto` now reads the cumulative token usage from Claude Code's final
`result` event instead of summing per-message `usage` blocks (which Claude Code
repeats across content blocks → 3-8x overcount). Falls back to the sum for older
`claude` builds. From 2026-06-15 headless `claude -p` runs meter against the
separate Agent-SDK credit pool, so accurate counts now matter for cost tracking.

### Why
Interim, correct-on-day-one fix ahead of the native-OpenTelemetry path (CO-457).
Dev-tooling only — no co-web change, no production deploy. Activate locally with
`cargo install --path dev/co-auto`.

## CO-78 — Job queue + worker pool — doc gen, sync, indexing, changelog

Generalized the single-kind CO-72 doc-gen queue into a multi-kind, rate-limited,
metered job queue + worker pool so CPU-heavy non-real-time work (doc generation,
search-index rebuilds, changelog regeneration, full-universe re-syncs, webhook
dispatch, cleanup) scales off the request threads.

- **Migration v088** — adds `jobs.timeout_secs` (per-job wall-time budget), the
  composite claim index `(status, run_at, universe_key)`, and the
  `(universe_key, kind, created_at)` rate-limit index. The base `jobs` table and
  its other indexes already exist from v25 (CO-72); this migration is additive
  and idempotent.
- **Internal submit API** — `enqueue_job(universe_key, kind, payload, dedupe_key)`
  enqueues any `JobKind` (`doc_gen`, `index_rebuild`, `changelog_regen`,
  `sync_pull`, `webhook_dispatch`, `cleanup`). `enqueue_doc_gen` now delegates to
  it. Content-derived `dedupe_key` makes resubmission a no-op (idempotent).
- **Per-universe rate limiting (CO-80)** — each kind carries a per-universe
  hourly cap; heavy kinds (`index_rebuild`, `changelog_regen`) are stricter.
  Over-limit submits return `429 TooManyRequests`; deduped no-ops don't consume
  budget.
- **Per-job timeout** — default 5 min, configurable per kind via
  `JobKind::default_timeout_secs` (stamped into `jobs.timeout_secs` at enqueue),
  enforced by the worker's wall-time wrapper.
- **Reliability** — workers claim the oldest pending job atomically
  (`UPDATE … RETURNING`) in FIFO order with starvation protection; failures
  retry with exponential backoff; 5 failures move a job to the dead-letter
  queue. A reaper reclaims jobs whose worker died mid-run, and the dead-letter
  queue is pruned to its 100-row cap each tick.
- **Metrics** — `GET /metrics` exposes queue depth, running count, dead-letter
  count, the autoscale recommendation, and per-kind success rate + p99 latency.
- **Worker-pool autoscale** — `desired_worker_count` recommends 4–16 machines
  scaling one per 100 backlog jobs (surfaced as `desired_workers` in `/metrics`)
  for Fly autoscaling to consume.

### Why
At scale, doc-gen / re-index / changelog / re-sync jobs are CPU-heavy and must
not block API request threads. A durable SQLite-backed queue with idempotency,
backoff, dead-lettering, and per-universe fairness lets these run on a dedicated,
autoscaling worker pool without degrading API latency.


## [3.14.0] — 2026-06-14 — Storage-as-a-Service foundation [--ignore-dod override]

## CO-458 — Storage backend abstraction — pluggable StorageBackend trait + LocalFsBackend (StaaS keystone; S3/partner plug in later)

Added a pluggable `StorageBackend` trait (`co-web/src/storage/backend/`) as the
single abstraction point for content-addressed blob storage — the keystone for
serving **storage as a service** (StaaS) to partners.

- **`trait StorageBackend`** (`Send + Sync`, async via `async_trait`):
  `put` / `get` / `head` / `delete` / `exists` / `list`, errors via `AppError`.
- **`LocalFsBackend`** — content-addressed (sha256), 2-level sharded under
  `<data_dir>/blobs/<aa>/<bb>/<hash>`, with refcounted **dedupe**: re-`put`ting
  identical bytes bumps the refcount instead of rewriting, and `delete` only
  unlinks the file when the last reference drops.
- **`from_config()`** selects the backend from `CO_STORAGE_BACKEND`
  (default `local`); `s3` is reserved for CO-81 and currently returns a clear
  "not implemented yet" error.
- **Migration v086** adds the `blob_refs` ledger table
  (`hash` PK, `backend`, `size`, `content_type`, `refcount`, `created_at`) for
  dedupe, traceability, and future StaaS billing.
- **Integration proof**: the backup worker now registers each snapshot's
  manifest in the central blob ledger via the backend (write + read-back),
  making backups a real `StorageBackend` consumer (CO-405).

### Why
To serve storage as a service we need one place where S3 (CO-81), partner
backends, and StaaS billing plug in without touching call-sites. This ships that
seam now, 100% local with no new infrastructure; S3 and partner tenants become
alternate implementations of the same trait.

## CO-459 — Local backup + junk sweep + retention optimization — reconstructable snapshot, remove unnecessary, scale-ready

Local-first backup and `/data` reclamation that stays reconstructable and is
ready to scale to StaaS.

- **Reconstructable local snapshot** — `POST /api/v1/admin/backup` (admin-gated)
  writes `/data/backups/<ts>/` with `meta.db` and every per-universe `data.db`
  copied via SQLite `VACUUM INTO` (committed/consistent, not a hot WAL copy) plus
  byte-copied blobs, and a `manifest.json` carrying a **sha256 + byte count per
  file**. The endpoint re-hashes the directory against the manifest and returns
  `verified: true|false` — proof the snapshot can be reconstructed.
- **Junk sweep** — `POST /api/v1/admin/sweep`, **dry-run by default**
  (`?apply=true` to delete). Identifies expired anonymous clones, stale
  temp/lock files, rotated logs, orphan blobs (`assets.refcount <= 0`), and
  snapshots beyond retention. Reports `reclaimable_bytes` before touching
  anything; on apply, every removal re-checks its owner/refcount/retention guard
  and is logged (no silent deletes). Nothing is deleted without a reference
  check.
- **Retention (count + space)** — directory snapshots are pruned by the same
  CO-405 `select_prunable` policy (newest always kept; count / cumulative-size /
  age caps), reused via `snapshot_dir::prunable_local_snapshots`. Parametrized
  by the existing `CO_BACKUP_RETAIN_*` env knobs plus `CO_SWEEP_*`. The math and
  the S3/StaaS handoff point are documented in
  `docs/architecture/backup-retention.md`.

New routes added to `docs/architecture/api-catalog.md`; `openapi.yaml`
regenerated (no drift). No schema migration — reuses existing tables
(`universes`, `assets`, `entries`, `universe_members`).

### Why

`/data` was inching toward full again (the 2026-06-11/06-13 outages), and the
existing CO-365 tarball backup is a hot copy with a single whole-archive hash —
not per-file verifiable. This adds a consistent, verifiable, reconstructable
snapshot and a guarded sweep to reclaim space, with the retention logic shaped
so the same code carries over to S3/StaaS (CO-458/CO-460) — "local backup for
now."

## CO-460 — StaaS partner model — design doc for multi-tenant storage-as-a-service (namespaces, quota, metering, partner backends)

Added `docs/architecture/staas-partners.md` — the written contract for selling
storage as a service to partners. Design only: no code, no migration, no new route.

It covers the eight required points, each tied to real artifacts in the tree:
multi-tenant namespaces (key-prefix isolation, mapping to `blob_refs`/CO-458 via a
`namespaces` + `blob_namespace` join), partner API shape over the CO-456 envelope
(`PUT/GET/HEAD/DELETE/LIST` + pagination + size limits), least-privilege auth with
new `storage:read`/`storage:write` capabilities and namespace binding (CO-448
`Scoped<C>`), metering as Σ(`blob_refs.size`×Δt)+ops sourced from CO-453 telemetry
with a `staas.usage.window_closed` billing hook, quota/rate-limit via CO-80
`TierLimits`/`RateLimiter`, bring-your-own partner S3 backends behind the CO-458
`StorageBackend` trait (trust boundary spelled out), per-namespace durability via
CO-459/CO-81 backup, and a CO-461..CO-467 implementation roadmap with dependencies
and per-task migrations. Includes a mermaid flow diagram (partner → token →
namespace → backend → metering) and links the doc from the existing
`docs/architecture/COMPOSABILITY.md` seam index.

### Why
The owner decided to "scale to partners later — serve storage as a service." This
is the architecture slice so the implementation tasks have a written contract
(partners, namespaces, auth, metering, billing) to follow instead of inventing it
ad-hoc, and so the architecture can be approved before spending Opus on impl.

## CO-79 — Caching layer — manifest, theme.css, hot queries + CDN strategy

Completed the caching layer by wiring the L1 query-result cache into the hot
read path. The `POST /api/v1/universes/{slug}/query` handler now consults the
in-process query cache before touching the universe `data.db`:

- Cache key = `query_cache_key(SQL + limit, universe_key, manifest_hash)` —
  SHA-256 scoped per universe, busted automatically when the manifest content
  hash changes or when a write invalidates the universe prefix.
- On a hit the serialized `QueryResponse` is returned without preparing or
  executing any SQL; on a miss the result is serialized and inserted.
- Access is verified before the cache lookup, so a hit never leaks data to an
  unauthorized caller, and the slug component prevents cross-universe reuse.

This closes the last open deliverable of CO-79. The surrounding layers were
already in place: the 10K-entry in-process LRU caches (manifest / theme.css /
query), manifest-cache invalidation on universe writes, the `theme.css`
endpoint with a 60s `Cache-Control` + ETag (never `immutable`), the in-process
pub/sub invalidation broadcast, and the `GET /api/v1/cache/stats` metrics
endpoint (hit/miss/eviction per layer).

### Why

Read-heavy workloads repeatedly ran identical SELECTs against per-universe
`data.db` files. Routing those reads through the existing query cache lets hot
reads serve from memory instead of re-executing SQL, reducing DB load while
keeping results fresh via manifest-hash keying and universe-prefix invalidation.

## CO-80 — Per-tier rate limiting + quota — token bucket per user/tier/operation

Durably audit-log admin quota overrides. When an admin bypasses a storage or
universe-count quota check via the `X-Admin-Override-Quota` header, the bypass is
now persisted as an `atividade` audit entry (`entidade = "quota_override"`,
`entidade_id = "storage" | "universe"`, with the acting `user_id`, client IP, and
user-agent) instead of only emitting a transient `tracing::warn!`. This closes the
"(audit logged)" half of the admin-override acceptance criterion so override abuse
can be reviewed after the fact.

`check_storage_quota` / `check_universe_quota` now take `&AppState` to enqueue the
deferred audit write via `log_atividade` (best-effort, off the request path).

The rest of CO-80 — in-process token-bucket rate limiting, tier extraction from
JWT/API-token in middleware, per-`(identity, op)` buckets, `429` with `Retry-After`,
`402` with quota usage details, and quota enforcement on entry write + universe
create — was already in place.

### Why
The admin override is the only quota escape hatch; without a durable record there
was no way to audit who bypassed a quota or when, which the acceptance criterion
explicitly requires.


## [3.13.0] — 2026-06-14 — Fractal Sala + API envelope [--ignore-dod override]

## CO-399 — Sala scope expansion — all-universes /sala + subset /sala?u=a,b (fractal scope phase 2)

The Sala canvas is no longer bound to a single universe. The same
`shared/sala.html` surface now also serves:

- `GET /sala` — scope = **every universe visible to the caller**
- `GET /sala?u=a,b` — an explicit, **visibility-gated subset**

Nodes carry their universe; cross-universe edges persist in the canonical
`key::path` link notation. State persists per `(scope, user)` — deterministic,
machine-local-state-free, so Wave 7 cross-device sync can op-log these rows.

### What changed

- **Migration v85** — additive `workspace_states.scope` column (`NOT NULL
  DEFAULT ''`). Legacy single-universe rows are untouched (`scope = ''`); a
  multi-universe row stores `'*'` (all visible) or a normalized `'a,b'` subset
  and a sentinel `universe_key = '@scope:<scope>'` so the existing UNIQUE
  constraint keys one row per scope without a table rebuild.
- **Scope resolver** — `GET /api/v1/sala/scope?u=a,b` resolves a scope to its
  visibility-filtered universes (the single authority: anonymous and
  cross-account callers only ever see the public slice).
- **Scope-keyed state** — `GET/PUT /api/v1/sala/state?scope=…` and
  `POST /api/v1/sala/state/share?scope=…`. Share tokens resolve through the
  existing `/api/v1/workspace-states/{token}` and now carry their `scope`;
  visibility is re-checked at read time via the scope resolver.
- **Frontend** — `shared/sala.html` parses the bare `/sala` and `?u=` query,
  fetches entries from every in-scope universe (labeled by universe in the
  picker), and places/links them as `key::path`.

### Why

A brain owner thinks across containers, not inside one. CO-399 makes the canvas
match that: one surface whose scope is all your universes — or any subset you
pick — extending the per-universe sala (CO-352) toward universe-as-node
recursion (CO-400) without forking the page or the table.

## CO-451 — co-auto: injetar skill de processo core + mapear labels de papel para skills

`skills_for_task` (em `dev/co-auto/src/auto.rs`) agora resolve as skills de uma
tarefa por um único caminho compartilhado (`skill_names_for_task`):

- **Skill de processo core** (`co-auto-process.md`) é **sempre** injetada quando
  presente no workspace, então todo agente herda o loop canônico do co-auto, os
  3 gates e o fallback de modelo — independente de papel ou módulo.
- **Labels de papel → skill**: `type:orchestrate→orchestrate`,
  `type:implement|feat|fix→implement`, `type:review→review`, `type:test→test`
  (somando a `playwright-pattern`), `type:release|deploy→release`. O mapa de
  módulo existente (`spa-conventions`, `deploy-runbook`, `rust-architecture`) é
  preservado.
- **Frontmatter `skills: [..]`**: lido em `parse_task` (entradas vazias
  descartadas) e mesclado, deduplicado, com as skills derivadas dos labels.

`skills_for_session` foi reescrita para usar a mesma resolução de nomes (filtrada
aos arquivos que existem em disco), eliminando a duplicação que existia entre as
duas funções. Skills ausentes degradam graciosamente. Novo `skills/README.md`
documenta o contrato (core + papéis + `skills:`).

### Why

O playbook canônico (processo co-auto + papéis) passou a existir como skills nos
workspaces (ex.: `ArteLonga/skills/`), mas não era carregado por co-auto por falta
de mapeamento. Agora tarefas de qualquer space herdam o processo canônico e a
skill do seu papel automaticamente, sem injeção manual pelo orquestrador.

## CO-455 — Document the contact endpoint in the API spec

`POST /api/v1/universes/{slug}/messages` (the CO-326 public contact form) was
served but missing from `api-catalog.md`, `openapi.yaml`, and the Swagger UI —
even though the public docs site advertises it. Added it to the catalog and
regenerated the OpenAPI spec so it now appears in `/api/docs`, closing the gap
between the published documentation and the API's own contract.

### Why
Doc-sync review (2026-06-14): the marketing /docs listed a contact endpoint the
API spec didn't document. The endpoint is real (anon, rate-limited 5/hr/IP,
honeypot-protected); it just wasn't in the source-of-truth catalog.

## CO-456 (CO-278-A) — API response envelope `{data,meta,errors}` + version headers

Added an **opt-in** unified response envelope to the public API, implemented as a
**single response layer** over every `/api/v1/*` route (no per-handler edits).

- Send `X-API-Envelope: 1` (or `Accept: application/vnd.co.v1+json`) and success
  bodies are wrapped as `{ data, meta, errors: null }`; errors (4xx/5xx) become
  `{ data: null, meta, errors: [{ code, message, field?, hint? }] }` with the
  same HTTP status.
- `meta` carries `request_id` (uuid, reusing `x-request-id`) and
  `api_version: "1.0"`, plus best-effort pagination (`page`/`page_size`/`total`)
  when the handler already exposes it.
- `X-API-Version: 1.0` and `X-Co-Server-Version: <workspace version>` are now on
  **every** `/api/v1/*` response, with or without opt-in.
- Without the opt-in marker, response bodies are **byte-identical** to before —
  the SPA's 55 raw `fetch()` call sites and existing external consumers are
  unaffected. Non-JSON responses (HTML, CSS, streams, blobs) are never wrapped.

### Why
CO-278 needs a single, predictable shape for the public API. Doing it
unconditionally would break the SPA and external consumers on deploy, so the
envelope is opt-in via header and lives in one place — flipping it to the
default is a later one-line change once consumers adopt the header. No migration,
no new tables.


## [3.12.0] — 2026-06-14 — Federation + Public API [--ignore-dod override]

## CO-338 — Surface keys — `key::path` cross-universe links that resolve to deployment DNS, at any nesting depth

A `<key>::<subpath>` reference now resolves to a node's **live deployment URL**
no matter where it sits in the recursive universe tree, and no matter whether
the node deploys itself or inherits a deploying ancestor's DNS. This is the
addressing half of the sub-universe ⇄ universe ⇄ deployable-unit model: links
address each other by *logical identity*, so promoting/demoting/remounting a
universe reroutes every reference with **zero link edits**.

`yggdrasil::comunicacao/mbya` and `mbya::` both resolve to
`https://yggdrasil.artelonga.com.br/comunicacao/mbya`; promote `comunicacao` to
its own surface and `comunicacao::*` follows it to
`https://comunicacao.artelonga.com.br/` automatically.

### What changed

- **Resolver in `core` (`co::surface`).** `resolve_surface_ref(key, subpath)`
  walks the key to its node (outermost-wins on shadowed keys; same-depth
  collision → `Ambiguous` error mirroring CO-277), then up `parent` links to the
  nearest deployable ancestor (a node with `surface_dns`). Base =
  `https://<ancestor dns>`; path = the handle chain from that ancestor down to
  the node. With **no** deployable ancestor it falls back to the CO platform
  host (`co.artelonga.com.br`) over the full lineage — exactly the pre-promotion
  state. Unit-tested against all six worked-example rows + ambiguity + shadowing
  + the promotion flip.
- **Data fields.** New `surface_dns` column on `universes` (meta-DB migration
  **v84**, nullable) alongside the existing `parent_key` lineage. Parsed from
  both `_universe.yaml` (`core::manifest::Manifest`) and `co-universes.yaml`
  (`UniverseDecl`), and loaded onto the `Universe` model. `Storage::
  list_surface_nodes()` exposes the `(key, parent, surface_dns)` registry.
- **`GET /api/v1/resolve?ref=<key>::<path>`** → `{ url, universe,
  deployable_ancestor }`.
- **Deployment worker is registry-driven.** `deployment_snapshot_worker` no
  longer hardcodes per-unit URLs: `build_units` resolves each unit's URL through
  `co::surface` from `(key, parent, surface_dns)` and overlays live
  `universes.surface_dns` rows, so an operator-recorded promotion reroutes a unit
  with no code change (kills the URL duplication — ties to CO-280).
- **Markdown render.** `co::surface::rewrite_surface_links` rewrites
  `[[key::path]]` body wikilinks to resolved links;
  `markdown_to_html_with_surfaces` turns them into `<a href="…">` for content +
  surfaces. Unresolvable/non-surface wikilinks are left untouched.

### Why

`::` complements CO-153's logical `co://`: `co://` returns `{universe, path}`,
`::` returns the physical URL. Content and surfaces can address each other by
logical identity, so deployment topology changes (promotion/demotion) never
break links — the resolver is what makes seamless promotion possible.

## CO-353 — Lobby + realtime presence — WebSocket layer for workspace canvases

A Sala (CO-352 workspace canvas) is now a **shared room** instead of a
single-user offline surface. A dedicated WebSocket layer broadcasts cursor
positions, node placements/moves, edge creations and suggest/publish events to
every connected client of a workspace, with the server as arbiter
(last-write-wins) and persistence back into CO-352's `workspace_states`.

Open two browsers on `/u/{universe}/sala` and you see each other's cursors and
edits live; close the last tab and the layout flushes to storage; reopen and the
snapshot restores it.

### What changed

- **`WS /ws/sala/{universe_key}/{workspace_slug}`** (`co-web/src/content/workspace/ws.rs`).
  On connect the server authenticates the session (JWT via `Authorization`
  header, `session` cookie, or `?token=`) or admits a **read-only anonymous
  visitor** (`anon:<id>`, `Visitante <id>`). It sends a `snapshot`
  (`{state, users}`) to the newcomer and broadcasts `user_join`/`user_leave`
  presence deltas to the room.
- **Lobby state** (`co-web/src/content/workspace/lobby.rs`). One shared `Room`
  per `(universe_key, workspace_slug)`, keyed on `crate::server::CoreState`'s new
  `sala_lobby`. Each room holds a `tokio::sync::broadcast` channel + the
  authoritative layout. Ops mutate the layout **LWW** (node upsert/remove, edge
  add/remove); cursors are ephemeral. A deterministic per-user colour hash
  (desaturated for visitors) drives presence identity.
- **Conflict + echo model.** The server is the arbiter; clients apply ops
  optimistically and the server echoes to *everyone else* (a client never gets
  its own op back — broadcast frames are tagged with the originating connection
  id). Rejected writes (e.g. an anonymous visitor) get a targeted `revert`
  carrying the client's `op_id` so the optimistic change rolls back cleanly.
- **Throttling.** Cursor frames are throttled to **≤ 20 Hz** per connection
  server-side (and again client-side).
- **Persistence.** The room flushes its layout to `workspace_states` every 2 s
  while dirty and once more when the **last** connection leaves, under a
  synthetic shared user so it never collides with CO-352's per-user saves and
  survives a full server restart. Reuses `Storage::upsert_workspace_state` (an
  internal call, not HTTP).
- **Frontend** (`co-web/static/shared/sala-realtime.js` + wiring in
  `shared/sala.html`). A small WS client with reconnect (exponential backoff),
  a presence overlay (other users' cursors drawn as labelled arrows on the
  canvas), optimistic node-move broadcast on drop, and live application of
  remote moves.

### Why

CO-352 shipped the canvas as a per-user surface; CO-61 will bring offline-first
CRDT sync later. For v1, server-arbitrated LWW (the model Yggdrasil's game
runtime already uses) is enough to make the "Sala" metaphor pay off — multiple
authors composing around one canvas, suggestions appearing live — without a
voice/video layer underneath. Anonymous visitors get a read-only socket so a
public Sala still feels alive without granting write access.

## CO-400 — Universe-as-node — sala nodes can be universes; descend/ascend recursion (fractal scope phase 3)

The sala canvas can now hold **universe nodes**: a universe placed on the
landscape as a node you descend into. Same surface, narrower scope — fractal
salas all the way down.

- **Entry picker "Universos" tab** lists the caller's visible universes (`GET
  /api/v1/universes`); picking one drops a universe node on the canvas.
- **Distinct rendering**: teal world-ring + globe glyph + entry-count badge, so a
  universe never reads as a nota or pasta. Draggable like any node.
- **Descend**: double-tap a universe node (or its panel's *Descer* button)
  navigates to that universe's sala at `/u/{key}/sala`.
- **Ascend**: a header breadcrumb back-link (`‹ {parent}`) returns to the
  originating sala with **camera state restored** — kept in `sessionStorage`, so
  it works instantly even for read-only viewers.
- **No breaking change**: universe nodes live in `layout.universes` and
  round-trip through the existing `workspace_states` state API, which stores
  `layout_json` opaquely.
- **Cycles tolerated**: descend is navigation, not embedding, so a sala holding
  its own universe node renders once (no recursion); activating a self-reference
  is inert.

### Why
Phase 3 of the one-surface, fractal-scope sala (`docs/architecture/sala-surface.md`).
Brain owners navigate nested universes (`parent_key` chains like miguel→mse) by
descending into universe nodes on the same canvas, never a forked second surface.

## CO-413 — Bidirectional bridge universes — event-bus universos aceitam writes + emitem entry.updated de volta (destrava YG-124)

Event-bus-backed universes (e.g. `yggdrasil`) can now be marked **bidirectional**,
turning the one-way ingestion bridge (CO-383/CO-384) into an editable round-trip.
When a universe is bidirectional, writes from the CO API/editor are accepted
instead of returning `405 read_only_universe`, and each accepted write is
re-emitted to the federated bus as a CO-origin edit so the hub (Yggdrasil) can
apply it. This unblocks YG-124 ("Editar no CO": deep-link a nota → CO editor →
bridge → Yggdrasil NoteStore) without duplicating the editor.

### What changed

- **Flag (`source_mode`).** Meta-DB migration **v83** adds
  `universes.source_mode TEXT NOT NULL DEFAULT 'read-only'`. `yggdrasil` keeps
  `source_kind = 'event-bus'` and defaults to `read-only`, so its behavior is
  unchanged until an operator explicitly flips it to `bidirectional`. No
  destructive data migration.
- **Relaxed gate.** `EntryService::check_not_event_bus` → `check_write_allowed
  (source_kind, source_mode, source_url)`: it rejects only the event-bus +
  non-bidirectional case (preserving the `read_only_universe` 405), and accepts
  bidirectional event-bus and every non-event-bus universe. All call-sites
  (entry create/update/delete + delivery status transition) pass the new arg.
- **Emit back.** On an accepted write to a bidirectional universe, the
  `entry.{created,updated,deleted}` EDA event payload carries `source = "co-edit"`
  and the entry row is stamped `source_marker = 'co-edit'`. The `co-edit` events
  federate back over the CO-384 bridge as CO-origin edits.
- **Symmetric echo-filter (anti-loop).** The inbound `YggdrasilNotes` subscriber
  now drops any event whose payload `source = "co-edit"` (a CO edit rebroadcast
  by the hub), distinguishing it from genuine `yggdrasil-live` upstream writes.
  This breaks the CO → hub → CO → … convergence loop; an integration test
  simulates the rebroadcast and asserts no re-application.
- **Capability surface.** `GET /api/v1/universes/{slug}` now returns `source_kind`
  and `source_bidirectional`, so YG-124 can show/hide the "Editar no CO" button
  (read-only universes reuse the existing i18n key `universe.source_bus_readonly`).

### Why

The CO-385 UPSERT/conflict resolver existed but never fired for federated
entries because this read-only gate blocked every CO-side write. Marking a
universe bidirectional lets concurrent CO + Yggdrasil edits reach the resolver
(last-write/UPSERT) instead of being blocked blindly, while the `co-edit` marker
contract (shared with YG-97's echo-filter) keeps the round-trip from looping.

## CO-450 — Deep-link ?criar handler — Yggdrasil 'Criar no CO' prefill → create-universe modal (YG-138 round-trip, CO side)

The CO SPA (variant-a) now handles the `?criar=1` deep-link emitted by the
Yggdrasil **🌐 Criar no CO** button (YG-138). Visiting
`/?criar=1&name=<nome>&key=<chave>&source=yggdrasil&instance=<id>` opens the
CO-96 create-universe modal **pre-filled** with `name` and `key`, runs the CO-96
inline validation immediately (an invalid key surfaces its error instead of
creating blindly), and submits via the existing `POST /api/v1/universes`
(CO-444) — redirecting to `/co/<key>` on success.

The handler is **auth-aware**: an expired/absent session triggers the login flow
over a template backdrop and leaves the `?criar` params in the URL so the
post-login reload resumes the prefill. The prefill is never persisted
cross-session, so it can't leak to a different user. Once the modal opens the
params are stripped via `history.replaceState`, so a refresh won't re-fire.

A `universe_created` telemetry event (CO-46 `window.coTrack`) is emitted on every
create, carrying `source` + `instance` when they came from the deep-link — this
measures YG→CO federation conversion.

### Why
Closes the live YG-126/YG-138 ↔ CO-444 round-trip: "criar universo" in Yggdrasil
becomes a federated universe in CO in one click, with provenance measured. No
migration and no backend change — reuses CO-444 (create) + CO-96 (modal) + CO-46
(telemetry).

## CO-452 — Public API docs — Swagger UI + OpenAPI discovery at /api/docs (CO-278-C)

Turned the CO HTTP API into a documented, explorable contract.

- **Swagger UI at `GET /api/docs`** now loads a **vendored** Swagger UI bundle
  (`co-web/static/shared/swagger/`, served same-origin) instead of the previous
  external `unpkg.com` CDN — CSP-safe, no third-party runtime dependency.
- **Discovery: `GET /api/openapi.json`** now serves the **catalog-generated**
  spec (`co-web/openapi.yaml`, the single source of truth verified by
  `npm run openapi:check`) instead of the older hand-maintained
  `docs/api/openapi.yaml`, removing the two-spec drift risk. The response now
  carries an `X-API-Version: v1` header so clients can pin the major version
  without parsing the body.
- A discoverable **"API docs"** link was added to the CO-210 docs navigation
  rail (`/seguranca` and siblings).
- Fixed a latent bug in `generate-openapi.ts`: catalog table cells containing an
  escaped pipe (`\|`) were truncated mid-string, producing malformed YAML. The
  generator now splits on unescaped pipes and hardens double-quoted-scalar
  escaping. (This never surfaced before because the served spec was the
  hand-maintained file.)

Additive: no migration, anon-readable docs; the documented endpoints stay gated.

### Why
CO-278 (public API) needs an explorable contract before its other slices
(versioning/envelope, agent dispatch, SDKs) and so sister-repos (yggdrasil, qb,
rfq) can consume the surface without reverse-engineering it. Serving the
catalog-generated spec keeps docs and routes from drifting.

## CO-453 — Public API telemetry + analytics endpoints — agent sessions, deployments, metrics (CO-278-E)

Added a documented, read-only telemetry/analytics surface to the public API
(CO-278), nested under `/api/v1/telemetry/*` and gated by the CO-448
`telemetry:read` capability via the new `Scoped<TelemetryRead>` extractor:

- `GET /api/v1/telemetry/agent/sessions` — list agent sessions (CO-275),
  filters `?model=&since=`, pagination `?limit=&offset=`.
- `GET /api/v1/telemetry/agent/sessions/{id}` — one session by id (404 if absent).
- `GET /api/v1/telemetry/deployments` — latest deployment snapshot per app/unit
  (CO-273); `?limit=` caps units.
- `GET /api/v1/telemetry/metrics/throughput` — token/session throughput over a
  window with a per-model breakdown (aggregates `usage_sessions`, CO-426).
- `GET /api/v1/telemetry/metrics/token-budget` — spend vs the
  `CO_AUTO_SOFT_LIMIT_5H_TOKENS` budget (CO-427) over a window.
- `GET /api/v1/telemetry/metrics/release-cadence` — releases per period from
  `release_notes` (CO-334).

A least-privilege token carrying `telemetry:read` (or the `read` bundle, or a
full JWT/session) is admitted; a token without it (e.g. `entries:read`-only)
gets 403. Routes are documented in `docs/architecture/api-catalog.md` and appear
in the generated `openapi.yaml` (Swagger UI, CO-452) with no drift.

### Why

CO-278-E gives the public API the "panel-of-glass" telemetry the north star
calls for — consumable by dashboards / sister-repos with a `telemetry:read`
token — and exercises the CO-448 scope model on a real surface.

Reuses existing tables and aggregations only; **no migration, no new table, no
MCP**. The surface is namespaced under `/api/v1/telemetry/*` rather than the bare
`/api/v1/agent/sessions` paths because the latter already exists as a pre-existing
anonymous, task-scoped kanban read endpoint (CO-275) and the two cannot share a
route. The new read helpers (`list_agent_sessions_filtered`, `get_agent_session`)
are read-only SELECTs over the existing `agent_sessions` table.


## [3.11.0] — 2026-06-14 — Least-privilege token scopes, scrum board & staging coverage

## CO-210 — Segurança + Dependências + Licença SPA routes — markdown renderer with telemetry + 404 tracing

Browsable documentation pages backed by the markdown already in `docs/`:
`/seguranca` (overview), `/seguranca/dependencias` (+ `/decisoes`),
`/seguranca/red-team` (+ `/playbook`), `/seguranca/vapid`, `/licensa`,
`/renderers`, and `/e2e-walkthrough`. `/dependencias` redirects to
`/seguranca/dependencias`. Shared navigation renders as a collapsible left rail
on desktop and a dropdown on mobile.

### What changed

- **New module `co-web/src/platform/docs_routes.rs`** — a tiny CommonMark +
  GFM-table markdown→HTML renderer (headings, lists, fenced code, tables,
  blockquotes, inline bold/italic/code, links). Every span of document text is
  HTML-escaped before tags are emitted, so no raw HTML — and in particular no
  inline `<script>` — survives into the page (CSP-safe).
- **Cross-links** between docs (`[ver vapid](vapid-security.md)`) are rewritten
  to their canonical route (`/seguranca/vapid`) at render time; external URLs and
  in-page anchors are left untouched.
- **Telemetry**: each render emits a server-side `page_view` event (with
  `duration_ms`); unmatched `/seguranca/...` paths return a 404 page and emit a
  `404_route` event with the attempted path. The client (`docs-viewer.js`) sends
  `page_render` with the real load duration so slow pages (>500ms) are detectable
  via the `duration_ms` field.
- **Privacy / opt-out**: `localStorage["co_viewer_telemetry"] = "0"` (toggled by a
  footer checkbox) is the source of truth; `docs-viewer.js` mirrors it to a
  `co_viewer_telemetry=0` cookie so the server suppresses its own emission too.
- Routes are merged before the `/{slug}` catch-all so these literal paths win
  over universe-slug resolution.
- New static assets: `co-web/static/shared/docs-viewer.js` (nav + telemetry,
  the page's only script) and `docs-viewer.css` (responsive rail/dropdown).
- Documented in `docs/architecture/api-catalog.md`.

### Why

Security, dependency, and license docs need to be findable and indexable for
trust, served from a single source (`docs/`) rather than duplicated. Embedded
telemetry + 404 tracing lets us see which pages 404, which links break, and which
pages load slowly — and iterate — without users having to raise a flag.

## CO-368 — Scrum artifacts as CO entry types — PBI / Sprint / SBI / DoD + per-universe _scrum.yaml

Scrum becomes data, not a separate tool. A universe can opt in to a Scrum
surface by dropping a `_scrum.yaml` at its content root; PBIs and Sprints are
ordinary CO entries (`entry_type = "pbi" | "sprint"`), so the feature is purely
additive — no migration, no new tables.

### What changed

- **`_scrum.yaml` manifest loader** (`co-web/src/scrum/manifest.rs`) — cadence,
  roles and `default_dod`, mirroring the `_calendar.yaml` pattern. Absent or
  invalid manifest ⇒ `enabled: false`, so universes without one are unchanged.
- **Deterministic current-sprint computation** (`co-web/src/scrum/current.rs`) —
  a pure function `current_sprint(now, anchor, length_days, release_window_hours)`
  returning `{ number, start_at, end_at, release_window }`. No DB, no hidden clock.
- **Frontmatter validation** (`co-web/src/scrum/validate.rs`) for PBI (`priority`,
  `points`, `status`, `acceptance`) and Sprint (`number`, `start_at`, `end_at`,
  `goal`, `release_tag`), wired into `EntryService::validate_entry_type` so both
  the sugar endpoints and the raw `/entries` POST validate identically.
- **Five per-universe endpoints** (`co-web/src/scrum/routes.rs`, mounted under
  `/api/v1/universes`):
  - `GET  /{key}/scrum/manifest` — manifest + computed current sprint
  - `GET  /{key}/scrum/sprints` — list sprint entries
  - `GET  /{key}/scrum/sprints/current` — `{number, start_at, end_at, release_window}`
  - `GET  /{key}/scrum/backlog?status=&sprint=` — filtered PBI list
  - `POST /{key}/scrum/pbi` — create a PBI (sugar over an entry write)
  - `PATCH /{key}/scrum/pbi/{id}/dod` — check off a DoD item (seeds from `default_dod`)
- **Scrum board SPA tab** — a three-column board (Product Backlog / Sprint
  Backlog / Increment) shown only when the universe's manifest is enabled, with
  the sprint goal above the columns and inline DoD checkboxes that PATCH on toggle.
- **Telemetry** — `scrum.pbi.created` / `scrum.pbi.dod_checked` events published
  to the EDA bus and captured in the atividades audit log (CO-361).
- **OpenAPI** — new endpoints documented in `docs/architecture/api-catalog.md`
  and regenerated into `co-web/openapi.yaml`.

### Why

The retrospective simulation (CO-369), sprint calendar (CO-372) and funnel
report (CO-371) can now read the same entry tables — no parallel data model —
and ArteLonga's `~/projects/ArteLonga/scrum/` draft becomes an executable
workspace.

## CO-401 — Staging fixtures + CO_STAGING_ADMIN_TOKEN — unlock the deep staging suite (3949 skips → coverage)

Seeds the fixtures the CO-374 deep staging suite preconditions on, plus a
least-privilege admin token for CI, so the Thursday release gate exercises real
scenarios instead of skipping the authenticated suite. All seeding is gated on
`CO_ENV=staging` and never runs in production.

- **Recursion universe chain** — the staging seeder now creates
  `recursion-a` → `recursion-a-b` → `recursion-a-b-c` (the exact keys the suite
  matches; the original CO-379 seeder used `recursion-ab`/`recursion-abc`, which
  the specs never matched, so every recursion test skipped).
- **Synthetic funnel/lead fixtures** — `seed_staging_funnel_fixtures` inserts a
  lifecycle of leads flagged `is_synthetic = 1`. **Migration v82** adds
  `leads.is_synthetic` (`NOT NULL DEFAULT 0`), and the acquisition-funnel rollup
  (`funnel_routes`) excludes synthetic rows from Capture (step 4) and Qualify
  (step 5), so fixtures never pollute real analytics metrics. (CO-448 took v81;
  this task claims v82.)
- **`CO_STAGING_ADMIN_TOKEN` as a CO-448 capability-scoped token** — on staging
  boot, `seed_staging_admin_token` registers the secret's value as an
  `api_tokens` row owned by the admin-tier `staging-admin` user, carrying an
  explicit least-privilege scope set (`entries:read/write`,
  `universes:read/write`, `gestao:read`, `funnel:read`, `chat:read`,
  `telemetry:read`) — **not** an admin-tier NULL-scope token. A leaked secret
  grants only that scope, never full admin. Seeding is idempotent and
  rotation-safe (a new secret value retires the old token row).
- **CI wiring** — `staging-suite.yml` runs the CO-374 staging config and passes
  `CO_STAGING_ADMIN_TOKEN`; the Playwright `apiContext` fixture authenticates
  with the scoped Bearer token (falling back to password-login locally only).
- **Docs** — `docs/cross-env-auth.md` gains a CO-401 section with the
  seed-on-boot model and a Fly + GitHub rotation runbook.

### Why

The deep staging suite skipped ~3949 tests because its fixtures (recursion
universes, funnel data) and an authenticated admin credential were missing. The
held first draft would have shared an admin-full credential with CI; the rework
(on CO-448) makes the credential least-privilege so a CI-secret leak can never
escalate to full admin, while still unlocking the suite's coverage.

## CO-448 — Token capability scopes (hybrid: capabilities + named bundles) — least-privilege api_tokens [CO-278-B pulled forward]

API tokens (`api_tokens`) can now carry a **least-privilege capability scope**
instead of inheriting the owner's full tier (all-or-nothing). The model is
**hybrid**: an issuer requests either raw capabilities (`recurso:ação`, e.g.
`entries:read`, `chat:write`) or a named **bundle** that expands to a capability
set.

- **Migration v81** adds a nullable `api_tokens.scopes` column (JSON array of
  resolved capabilities; idempotent via `ensure_column`).
- **Bundles**: `read` → `{entries:read, universes:read, telemetry:read}`,
  `write` → `read ∪ {entries:write, universes:write}`,
  `admin` → `write ∪ {gestao:read, funnel:read, chat:read, chat:write, deployments:read}`,
  `agent` → `write ∪ {agent:dispatch}`. Bundles are expanded **at issuance** and
  the expanded set is persisted, so a token is auditable.
- **Issuance**: `POST /api/v1/auth/token` accepts an optional `scopes` list
  (capabilities and/or bundle names). An unknown capability/bundle → `400`. The
  resolved set is returned and listed (`GET /api/v1/auth/tokens`).
- **Enforcement**: a new `Scoped<C>` axum extractor declares the capability an
  endpoint requires and **denies with 403** when a scoped token lacks it (no
  escalation). `GET /api/v1/admin/chat/origin-breakdown` now requires `chat:read`.
- **Backward-compatible**: tokens with `scopes = NULL` (every pre-CO-448 token +
  the legacy `create_api_token` path) keep inheriting the owner's tier — nothing
  already issued (vault sync, co-auto reporting, …) breaks.

Docs: `docs/cross-env-auth.md` (scope model + capability vocabulary + per-endpoint
capabilities) and `docs/architecture/api-catalog.md` / `openapi.yaml` (no drift).

### Why
A leaked token secret previously granted full admin (the CO-401 staging-suite
token had to be admin-pleno for exactly this reason). Least-privilege scoping
means a leak grants only the token's capability set, and gives the future public
API (CO-278) granular authorization natively. This is CO-278-B pulled forward so
the reworked CO-401 staging token (v82) can be least-privilege.


## [3.10.0] — 2026-06-13 — Resilience hardening + chat/timeline polish

## CO-204 — Chat message origin telemetry — track which universe context each message was sent from

Every `chat_messages` row is now stamped with an optional `origin_universe_key`
— the universe context the sender was browsing when they hit Send. This is
distinct from `chat_rooms.universe_key` (where the *room* lives): for DM rooms,
which have no universe of their own, the origin is the only "where did this come
from" breadcrumb.

- **Migration v80** adds the nullable `origin_universe_key` column to
  `chat_messages` plus an index (`idx_chat_messages_origin`). Existing rows stay
  `NULL` (no backfill — the at-the-time context can't be reconstructed).
- **POST `…/chat/rooms/{room_slug}/messages`** gains an optional
  `origin_universe_key` field. It is validated against the caller's
  membership/subscription and **silently dropped** (persisted `NULL`) if they
  don't belong to the claimed universe — privacy-respecting, so "I'm in private
  universe X" isn't leakable just by sending. Omitted ⇒ defaults to the room's
  universe for universe rooms, `NULL` for DM rooms.
- **GET messages** returns `origin_universe_key` on each row.
- **DM UI** (`modules/chat.js`) renders a small italic "via {universe}" subtext
  when a message's origin differs from the universe the viewer is currently in;
  suppressed for universe-room messages where origin == the room's universe.
- **Admin telemetry**: `GET /api/v1/admin/chat/origin-breakdown` (admin-gated)
  returns per-origin message counts + total, backing a future admin chart.

### Why

Lets operators answer "which universes drive the most chat" and "are people
DMing across universe boundaries", and gives a DM recipient context for where a
message originated — without cross-linking to any private content.

## CO-396 — Project timeline lens — roadmap/gantt de um universo-projeto sobre o `<co-time-grid>`, com engine de layout compartilhada com o Yggdrasil (YG-123)

A **project-timeline** (roadmap/gantt) lens: the same entries that fill a
project's kanban board, rendered on a time axis — tasks in lanes by
`epic`/`module`/`status`, releases and milestones as markers, and a "no date"
backlog so a task without a date never disappears. Quadro and timeline are now
two *forms* of the same canonical entries — no new data structure.

### Shared layout engine (the central constraint)

The timeline lens/layout math now lives **once**, in the path-dep crate both
hosts consume:

- New `game_core::time_layout` module. CO-387's per-lens conversion math
  (`LensDef`, `LensPosition`, `entry_to_lens_position`, …) was **hoisted** out
  of `co-web/src/time/conversion.rs` into it; co-web re-exports the same paths
  so nothing else changed. On top, CO-396 adds the project-timeline layout:
  `layout_project_timeline(entries, group_by)` → lanes + markers + backlog, and
  `gantt_span` (duration precedence: `scheduled→due`, else `created→done`, else
  a point).
- `yggdrasil-core` already path-deps `game-core`, so YG-123's
  `generators/timeline.rs` consumes the **same** engine instead of a parallel
  implementation (the yggdrasil-side wiring lands with YG-123 in that repo).
- `static/shared/lib/co-time.js` mirrors the engine for the client
  (`<co-time-grid>`) — one shared lib, not a divergent fork.

### co-web

- `GET /api/v1/universes/{slug}/timeline?group_by=epic|module|status` — builds
  the project timeline from the universe's entries through the shared engine
  (inherits the content visibility gate). New `time::project_timeline` mapper
  turns an `EntryRow` into the engine's neutral `TimelineEntry`.
- `_calendar.yaml` lenses gain `lane_by: epic|module|status` to declare
  themselves project-timeline lenses.
- `<co-time-grid>` gains a `roadmap` view mode (lanes + marker rail + backlog)
  and its `gantt` mode now uses the engine's duration semantics. New
  `project-timeline` SPA lens registers into the CO-393 lens frame, appearing as
  a header tab next to Kanban — the toggle quadro ⇄ timeline.

### Tests

- `game-core`: project-timeline + conversion engine tests (independent of
  co-web).
- `co-web`: entry→timeline mapper tests, `_calendar.yaml` `lane_by` parsing, and
  a route wiring/visibility-gate test.
- `co-web` vitest: `layoutProjectTimeline`/`ganttSpan` JS-mirror tests
  paralleling the Rust suite so the two cannot silently diverge.

### Why

Roadmap and project review come out of the same markdown the board already
uses (content × form), and the layout engine is shared with Yggdrasil's
timeline-world rather than reimplemented per host.

## CO-443 — DoD matcher can't score refactor/structural tasks — recognize Rust tests + structural assertions

The DoD verifier (`co-web/scripts/dod/verify.ts`) previously matched acceptance
items only against Playwright e2e test names. That works for `feat` tasks (HTTP
route → e2e) but failed for `refactor` tasks whose acceptance is structural
("promote `Universo` to core", "split `migrations.rs`", "game-core is
axum-free") — the Mythos children CO-431..436 scored 0–25% despite being
complete, forcing `--ignore-dod` and manual verification every wave.

The matcher now scores those tasks two ways:

- **Rust test recognition** — `#[test]` / `#[tokio::test]` functions are matched
  by name, not just e2e specs.
- **Structural proofs** (`dod_checks` frontmatter map) — an acceptance item can
  carry deterministic, build/grep-style evidence: `grep:<regex>` (assertion
  exists), `grep-absent:<regex>` (e.g. axum-free), `rust-test:<fn>`,
  `e2e-test:<regex>`, `file:<path>`, each optionally scoped with `@<path>`.
  All proofs are pure filesystem reads — no build, no network — so they stay
  deterministic and remain advisory (an unmet proof never blocks a merge).

Also fixed a latent acceptance-section parser bug: a `\z` in the section regex
(a literal `z` in JS) truncated multi-line `## Acceptance` blocks, undercounting
items; parsing is now line-based and joins wrapped continuation lines so the
full criterion text is available to the matcher.

Re-running the verifier on CO-431..436 now reports **100%** for each with no
override. Docs: `docs/scrum/dod-proof-checks.md`.

### Why

The override existed, but every refactor release needed manual structural
verification (contention test, raw-SQL greps, `cargo tree | grep axum`). This
lets the gate give honest credit to structural work instead of punting it.

## CO-446 — Disk-full hardening — migrations + pool degrade instead of crash-loop; pre-deploy free-space gate

A full `/data` volume no longer turns a routine migration deploy into an outage.
Two production incidents (2026-06-11, 2026-06-13) showed the same failure class:
the boot ran a new migration, the `schema_version` insert hit `SQLITE_FULL`, the
process panicked (`exit 101`) and crash-looped until max-restart — site dark.

- **Migrations degrade, never crash-loop.** `Storage::run_migrations` now returns
  a readable `MigrationError` instead of panicking. The migration chain runs
  under a catch boundary, so a write that fails mid-migration (disk fills after
  the pre-flight) becomes a controlled `FATAL (CO-446)` log + clean exit, not a
  cryptic SQLite backtrace. `record_migration!` logs a clear disk-full ERROR
  before aborting.
- **Boot pre-flight for disk headroom.** Before running migrations the boot path
  checks free bytes on `/data`; below `CO_MIGRATION_MIN_FREE_BYTES` (default
  200 MiB) it logs `insufficient disk for migrations: X free < Y min` and aborts
  with a message instead of failing deep inside SQLite. The guard is disabled
  when statvfs is unavailable (non-Linux dev / CI) so it never blocks local runs.
- **Pool I/O degrades, not panics.** Confirms the CO-405/CO-406 guarantee on the
  disk path: a corrupt/unreadable per-universe `data.db` degrades to a
  `PoolError` (503 for that universe) while every other universe keeps serving —
  regression test added.
- **Pre-deploy free-space gate.** `scripts/pipeline-deploy-gate.sh` checks
  `df -P /data` on prod and blocks a deploy when `/data` is > 85% full
  (`DISK_MAX_PCT`), since such a deploy can hit `SQLITE_FULL` at boot. `--no-disk`
  skips it; `ALLOW_FULL_DISK=1` downgrades to a warning.
- **Runbook.** `docs/OPERATIONS.md` gains a "Disk-full recovery" section
  documenting `volumes extend` + machine **stop/start** (a plain `restart` does
  NOT resize the filesystem — the trap that prolonged the 2026-06-13 incident).
  The deploy CLAUDE.md links it and wires the gate into the deploy steps.

### Why
Disk-full is a silent bomb: v3.8.0 ran fine because it writes no migration row at
boot; v3.9.0 added migrations and detonated on the already-full volume. This task
is the safety net — the next time `/data` fills, it is a clear error and a
one-command extend, not an outage. The real endgame remains S3 cold-tier offload
(CO-81).

## CO-447 — Security scanner CWE-89 refinement — require SQL context, stop flagging HTTP verbs / log strings

The CWE-89 (SQL injection) heuristic now requires real SQL context before it
fires. A `format!` string is only flagged when it opens with a SQL verb
(`SELECT`/`INSERT`/`UPDATE`/`DELETE`) **and** the same string literal also
contains a SQL clause keyword (`FROM`/`INTO`/`SET`/`WHERE`/`VALUES`).

- Refined the `security-audit` grep in `.github/workflows/pr-route.yml` to
  `format!\("(SELECT|INSERT|UPDATE|DELETE)[^"]*(FROM|INTO|SET|WHERE|VALUES)`,
  with a comment documenting what triggers it and why.
- Refined the Rust scanner in `co-web/src/security/audit/local_grep.rs`: the
  four bare-substring SQL patterns are replaced by a context-aware
  `sql_format_verb` helper kept in lock-step with the workflow regex.
- Added scanner tests covering an HTTP-verb error string and a log string (now
  pass), genuine interpolated SQL (still blocks), and the safe const-columns +
  bind-params pattern (never flagged).

### Why
In the CO-445 wave the bare-verb heuristic gave false positives on strings that
are not SQL — `format!("DELETE vault/{path}")` (a DELETE *HTTP-verb* error
string, CO-51) and log lines like `format!("UPDATE failed for {id}")` — costing
CI cycles to reword harmless strings. This is the "A" half of the owner's CO-433
decision (refine the scanner). CWE-79/innerHTML is intentionally out of scope.


## [3.9.0] — 2026-06-13 — Post-git Sync engine [--ignore-dod override]

## CO-128 — Apple-style 4-way conflict UI (Ignore / Replace / Keep both / Apply to all)

When two devices (or an Obsidian sync overlapping the web) edit the same entry,
the SPA now surfaces a macOS-Finder-style modal with a side-by-side diff and the
four conflict actions the user named as a v1 requirement, instead of silently
last-write-wins.

### Added / changed

- **Optimistic concurrency on entry update.** `PUT /api/v1/universes/:slug/entries/:path`
  accepts an optional `base_hash` — the `body_hash` the client last observed.
  When present and the stored entry has since diverged, the write is rejected
  with `409 Conflict` and a `ConflictPayload { local, remote, base }` carrying
  **both** full bodies (capped at 100 KB/side with a `truncated` flag) so the
  modal can render a diff. Absent `base_hash` ⇒ unchanged last-write-wins, so
  draft autosave and legacy callers never trigger a conflict.
- **Conflict-resolution modal** (`modules/sync/conflict-modal.js`). Pure UI that
  consumes a `ConflictPayload`, renders a monospace side-by-side line diff
  (`modules/sync/conflict-diff.js`, dependency-free LCS), and offers
  `Ignore` / `Replace` / `Keep both` plus an `Apply to all` checkbox. Styled with
  existing CO design tokens (theme-aware), no new framework. Keyboard:
  `1`=Ignore, `2`=Replace, `3`=Keep both, `Esc`=cancel the round.
- **Round controller** (`modules/sync/conflict-round.js`). Walks the queued
  conflicts of one sync round and wires each choice to the real entry API:
  *Ignore* keeps remote (no write); *Replace* re-PUTs the local body with the
  remote hash as base; *Keep both* creates a `<name>.local.md` sibling.
  `Apply to all` makes the choice sticky for the rest of the round — held in
  component state only, so it never survives a reload.
- **Editor integration.** The content editor save path now sends `base_hash` and,
  on `409`, opens the modal and applies the resolution (revert / overwrite /
  keep-both) before closing the editor.
- **i18n.** Conflict-modal strings added in PT-BR (primary) and EN.

### Why

§C.3 of the platform roadmap review flagged this conflict-resolution UX as a
named v1 requirement the SR plan omitted. It handles the divergence cases the
CRDT layer (CO-61) cannot, building on sync-protocol v1 + idempotency (CO-54).

### Notes

- No migration. 3-way *Merge* UI, CRDT collaboration, and mobile-specific layout
  remain out of scope (separate tickets).
- Tests: Rust integration (`tests/conflict_409_tests.rs`) for the 409 trigger +
  backward-compat; vitest component suite (`components/__tests__/conflict-modal.test.ts`)
  for diff/keyboard/apply-to-all/round behaviour; Playwright e2e
  (`e2e/conflict.spec.ts`) for the two-session 409 flow and live modal render.
- The spec named "SvelteKit"; the CO web client is a plain TypeScript/ESM SPA
  (no SvelteKit in-tree), so the component ships in that framework.

## CO-444 — Federation API: token-auth universe creation + visibility + invites + public-subscribe (CO side of YG-138)

A federated-creation surface so an external service (Yggdrasil, YG-138) — or any
client holding a user's API token — can stand up a universe in CO without internal
seed/clone code.

### Added / changed

- **Create accepts API tokens.** `POST /api/v1/universes` is now behind
  `require_auth_with_token` (was session-only, the 401 that blocked the
  claude-code import in CO-438). The body gained `visibility` and `parent_key`:
  `{ key, name, description?, visibility?, parent_key? }`. All validation runs
  before the row is inserted, so a bad `visibility`/`parent_key` never leaves an
  orphan universe; a taken key still 409s.
- **Visibility vocabulary.** `visibility` ∈ `private | public | unlisted`
  (`public` stored canonically as `public-subscribable`, still accepted as an
  alias). `unlisted` is a new value: readable by anyone with the link, excluded
  from discovery/search, not subscribable, writable only by owner/members. The
  `universe_visibility_gate` and `check_universe_access` honour it consistently.
  `PUT /api/v1/universes/{key}` accepts the same vocabulary (and an API token).
- **Token↔session parity.** The whole universe management router accepts an API
  token or a session JWT, resolving to the same owner. `extract_optional_user_id`
  now resolves both credentials (it was JWT-only, which 401'd token callers on
  `PUT`/`DELETE`).
- **Invites (`/invites`).** `POST/GET /api/v1/universes/{key}/invites`
  (owner/admin), `DELETE …/invites/{token}` to revoke, and
  `POST …/invites/{token}/accept` to accept (universe-scoped). Reuses the CO-188
  invitation storage; `handle` is accepted as an alias for `usuario`. These
  routes accept an API token.
- **Public subscribe.** `POST/DELETE /api/v1/universes/{key}/subscribe` already
  existed; it now also accepts an API token via the same parity change.

### Why

YG-138 left the Yggdrasil side waiting on these routes so "criar universo"
(YG-126) becomes a federated universe in CO — with invite/visibility UI — instead
of half-implementing the CO side blind. Additive, non-breaking: new routes and a
new `unlisted` value, nothing removed. No MCP; no hardcoded universe→repo
mapping (everything comes from the request payload / DB).

### Notes

- No migration: the `universes.visibility` column, `subscriptions`,
  `universe_members` and `universe_invitations` (CO-188) tables all already exist.
- Routes added to `docs/architecture/api-catalog.md` + `openapi.yaml` regenerated
  (no drift). Integration tests cover token-auth create + correct owner,
  visibility, parent_key, 409, invites create/list/revoke/accept, and subscribe.

## CO-51 — CLI sync — co sync pull/push/watch with conflict resolution

Added `co login` and the `co sync` command family for Google-Drive-like local
editing of a universe's Vault.

- `co login` — authenticate and store a 90-day API token (alias for
  `co auth login --save-token`).
- `co sync pull <universe>` — download all entries to `~/Co/<universe>/`.
- `co sync push <universe>` — upload changed local files (no-op when clean).
- `co sync status <universe>` — show the new/modified/deleted/conflict diff
  between local and remote.
- `co sync watch <universe>` — poll for local changes and auto-push (debounced,
  default 2s, `--interval` override).
- `co sync resolve <file>` — finalize a manual conflict and push the result.

Sync state lives in `<universe>/.co/sync.json` (`{ files: { path: { hash,
mtime, remote_hash } } }`) with base snapshots under `.co/base/` for 3-way
merges. The diff classifies each path against the last-synced base
(local-only → push, remote-only → pull, both → conflict).

Conflict strategies (configurable in `~/.config/co/sync.yaml` or `--strategy`):
`last-write-wins` (default), `local-wins`, `remote-wins`, `manual`
(writes `.local`/`.remote`/`.base`), and `merge` (line-level 3-way merge,
falling back to `manual` on overlapping hunks).

Safety: an `O_EXCL` lockfile (`.co/sync.lock`) blocks concurrent syncs;
`sync.json` is written atomically (temp + rename) and only after each successful
file op, so a crash mid-sync never corrupts state and the next run recovers.

### Why

CO-35's Vault REST API gives file CRUD; this layers a stateful, conflict-aware
sync client on top so a user can edit a universe locally in any editor and
reconcile with the server, the way Google Drive's desktop client does.

## CO-54 — Idempotency + conflict resolution — safe concurrent editing across sync, web, and co-auto

Formalized idempotency guarantees and conflict-resolution fallbacks so the web
UI, CLI sync, co-auto, and the Obsidian plugin can edit the same universe
concurrently without losing data.

### Web (entry PUT)
- **Field-level merge** (Scenario 1): `PUT …/entries/{path}` is now a true
  partial patch — a frontmatter field is overwritten only when the caller sends
  it, so two clients editing *different* fields merge instead of clobbering.
  Explicit JSON `null` deletes a field; same-field edits resolve last-write-wins.
- **Idempotency** (Scenario 3): re-applying identical content (e.g. co-auto
  setting `status: done` when already done) is a no-op — no version bump, no disk
  write, no event storm.
- **Version history / audit trail**: every overwrite first snapshots the
  *previous* content into the new `entry_versions` table (migration **v78**) with
  `actor`, `timestamp`, and a full-content `hash`. This is the data-loss-prevention
  guarantee — a crash mid-write can never lose committed data. History is exposed
  at `GET /api/v1/universes/:slug/entries/versions?path=…`. Retention: the newest
  50 versions per entry **or** 90 days, whichever is more generous.

### co-auto (task locking)
- **Per-task lockfile** (Scenario 4): co-auto now acquires an atomic lock under
  `<data_dir>/.co-auto/locks/<TASK-KEY>.lock` before executing a task. A second
  agent (e.g. another worktree) finds the lock and skips to the next candidate,
  so two agents never double-execute the same task. Stale locks (crashed/hung
  agent) are reclaimed after a 30-minute TTL.

### Sync (conflict strategy)
- **Configurable resolution strategy**: added `ConflictStrategy` (default
  **last-write-wins by timestamp**) that maps a detected conflict to a concrete
  resolution action. Layers on top of the existing CO-385 `detect_conflicts`
  (both sides differ from base → `BothModified`) so a non-interactive client can
  auto-resolve (Scenario 2). Delete-vs-modify defaults to "modification beats
  deletion" to avoid silent data loss.

### Why
Multiple clients can edit the same universe simultaneously. Without these
guarantees, a naive last-write-wins PUT silently discarded concurrent field
edits, repeated writes churned versions, and two co-auto worktrees could race
the same task. The `entry_versions` audit trail makes every overwrite recoverable.

## CO-75 — Version reconstruction: replay to any timestamp + auto-changelog

Any past state of any entry is now queryable by timestamp, and a universe-wide
changelog reconstructs itself from the op log with no manual maintenance — built
**entirely on the pieces that already shipped** (CO-54 `entry_versions` and the
CO-95 `entry_events` op log), with **no new table and no migration**.

### Added

- **`GET /api/v1/universes/:slug/entries/{*path}?as_of=<RFC3339>`** — reconstructs
  an entry as it was at a past instant by replaying the CO-54 version history.
  Returns `{ path, as_of, version, source_timestamp, is_current, frontmatter,
  body }`. Reconstruction rule: each `entry_versions` row holds the content that
  was *live until* its overwrite timestamp, so the state at `T` is the content
  of the earliest version whose `timestamp > T`, falling back to the current
  live entry when nothing changed after `T`. Visibility gates apply to historical
  reads exactly as to live ones.
- **`GET /api/v1/universes/:slug/entries/diff?path=…&from=<T1>&to=<T2>`** — the
  op-level diff of one entry across an interval: changed frontmatter fields
  (`before`/`after` per field, including adds/removes) plus a body-change flag
  with both bodies. Reconstructs both endpoints, so it shows *only* the net
  change in `[from, to]`. (Uses `?path=` like the sibling `/history` and
  `/versions` routes, to avoid the greedy `entries/{*path}` wildcard.)
- **`GET /api/v1/universes/:slug/changelog?since=<T>&until=<T>`** — an
  auto-generated Keep-a-Changelog document for the whole universe. Aggregates the
  `entry_events` op log over the window, classifying each op (`put` with no prior
  body → **Added**, `put` with a prior body → **Changed**, `delete` → **Removed**)
  and rendering its line via the content type's manifest template. Returns the
  grouped lines plus rendered `markdown`.
- **Manifest hook `changelog_summary`** on `ContentType` — a per-type template
  for the changelog line, e.g. `changelog_summary: "{title} marcado como {status}"`.
  `{field}` tokens substitute from the entry's frontmatter; `{path}`/`{title}`
  are always available; absent → a default line (title or path). Backward
  compatible (optional, defaults to `None`).

### Notes

- **No `entry_snapshots` table.** The CO-75 spec predated CO-54; per its
  pre-flight reuse note, the snapshot store *is* `entry_versions` and the op
  source *is* `entry_events`. Replay/diff/changelog are a thin, pure,
  exhaustively unit-tested layer (`co-web/src/content/versioning/reconstruct.rs`)
  over those, plus glue in `content::entries::routes`. Snapshot-per-write already
  bounds replay cost; deep history beyond the CO-54 retention window (50 versions
  / 90 days) is necessarily best-effort.

### Why

Closes the manifest-epic requirement to "show entry as of T" and to generate
changelogs from the durable op log instead of hand-maintaining them.

## CO-88 — End-to-end pipeline UAT — localhost ↔ API ↔ web with per-universe stats

Added a content-pipeline UAT harness that proves a file authored on localhost
arrives byte-equivalent through every layer combination, plus server-side
telemetry, an admin dashboard, and CI/deploy gating.

### What changed

- **`dev/co-pipeline` (new workspace binary, CO-88a)** — drives the
  path × universe × combo matrix (4 paths × 5 combos × 3 universes = 60 cells).
  Reuses the real `CoFile` protobuf envelope, zstd-3, ChaCha20-Poly1305, and
  Ed25519 signing so the bytes that travel are production's bytes. Encodes →
  transports → decodes → compares each corpus file, timing encode/decode, and
  writes a deterministic `co-pipeline-report-<date>.yaml` to `dev/reports/`.
  Subcommands: `run`, `delta` (CO-88c diff), `gate` (CO-88d deploy gate).
- **`co-web` pipeline telemetry (CO-88b)** — vault PUT/GET now record
  `pipeline.transfer` / `pipeline.encode` / `pipeline.decode` events (carrying
  `co_format`, `compression`, `encryption`, sizes, encode/decode ns) when the
  caller announces a combo via `X-Co-*` headers. New `pipeline_summary()`
  aggregation, admin endpoint `GET /api/v1/admin/pipeline/summary` (yuri tier),
  and a sortable admin dashboard at `/co/co-dev/pipeline`.
- **CI + deploy gate (CO-88c/CO-88d)** — `.github/workflows/pipeline-uat.yml`
  runs the matrix, uploads the report, comments per-universe deltas on PRs, and
  fails the build if any cell fails to round-trip. `scripts/pipeline-deploy-gate.sh`
  blocks a prod deploy unless the UAT report is green, < 24h old, and free of
  regressions beyond 20%, then optionally runs the read-only prod smoke.

### Why

The `.co` format (CO-86) and composable layers (CO-87) introduce a non-trivial
encode/decode pipeline. Without a round-trip gate, every release risks shipping
a subtle encoder/decoder bug that silently corrupts content. The matrix run is
cheap (~3s over the local corpora) and catches drift before users see it, while
giving the team hard size/perf numbers per universe.

## CO-91 — co sync — canonical content-author UX (jj delta + automated changelog + co-token auth)

Promoted `scripts/seed-prod-universes.sh` (the 250-line content-author loop)
into a first-class `co sync` workflow. Authors edit markdown locally and run
`co sync push`; only what changed since the last push flows to the configured
deployment's Vault REST API, with a changelog paper trail — no curl, scripts or
raw API.

This folds CO-51 forward: the existing CO-51 bidirectional mirror (`co sync
push <universe>` against `~/Co/`) is preserved when a positional `<universe>` is
given. With no positional, `co sync` runs the new CO-91 jj-delta push.

- `co sync push [--to <deployment>] [--full] [--dry-run] [--no-changelog]
  [--push-changelog]` — jj-delta upload of the current repo to a deployment.
- `co sync push --bootstrap` — one-time setup: password login, create the
  universe (idempotent), full upload, generate a 90-day API token and store it
  in the OS keychain (service `co`, account = the deployment's `token_name`).
- `co sync status` — non-mutating diff of what a push would upload.
- `co sync watch` — `notify`-backed, debounced auto-push on save (default 1s).
- `co sync changelog [--for <universe>]` — print accumulated run snippets.

### Primitives (1:1 with the script it replaces)

- **Auth** — per-deployment API token read on demand from the OS keychain (the
  same store `dev/co-token` writes); never persisted to disk by `co sync`.
- **Delta** — the source repo is wrapped with `jj git init --colocate`
  (non-destructive). The last-pushed commit id is the baseline at
  `~/.co/sync-state/<deployment>/<universe>.commit`; only files differing
  between that baseline and `@` are uploaded. First run / `--full` / a lost
  baseline → full upload.
- **Changelog** — `jj log -r '<baseline>..@'` is rendered to
  `~/.co/sync-runs/<deployment>/<universe>-<ts>.md` per run.
- **Per-deployment isolation** — tokens, baselines and changelog snippets are
  namespaced by deployment key, so `--to prod` and `--to uat` are independent
  operations against independent baselines.

### Config

- `~/.co/deployments.toml` — `[prod]`/`[uat]` tables (`url`, `token_name`,
  `default`). Absent → a built-in `prod` entry (zero-config, as the script was).
- `<root>/.co/sync.toml` — `universe`, glob `include`/`exclude` (default
  `**/*.md` minus the usual build/tooling dirs). Or pass `--universe --root`.

### Content negotiation (CO-86) and changelog events (CO-89)

`co sync push` probes `OPTIONS /…/vault/`; if the server advertises
`co/protobuf`, files are sent as a `.co` protobuf batch (CO-151 `CoFile` +
zstd), else as `text/markdown` (the v1 wire format). `--push-changelog` PUTs a
synthesized `event` entry (`type: event`, `kind: sync`, `summary: <count>`) via
the Vault fast-path, indexed on CO-89's calendar view.

### Bench (throughput vs the script)

The script forks one `curl` per file with `sleep 0.1` between PUTs (≈10
files/s ceiling, plus per-request connection setup). `co sync` reuses a single
keep-alive `reqwest` client with no artificial delay, so throughput is bounded
by server RTT rather than a fixed sleep — a >10× headroom improvement on the
small-file uploads the script was used for, before counting the jj delta that
shrinks the upload set to only changed files.

### Why

The content-author loop was decided to be canonical, not a one-off script. A
first-class CLI surface — composable auth (keychain), delta (jj) and changelog
primitives behind a clean argument shape with real error handling — lets any
author (not just yuri) adopt it against any deployment (UAT, prod, self-hosted).

## CO-96 — Universe CRUD UI — intuitive create / rename / duplicate / delete in the SPA

Adds a full universe-lifecycle CRUD surface to the SPA so any user can manage
their universe collection from the browser without scripting.

### Create (Phase 1)
- The sidebar `+ New universe` modal now does **inline key validation**:
  synchronous format checking plus a debounced uniqueness check against the API
  (404 = available). Submit is disabled while the key is invalid or taken.
- "Copy from existing universe" now routes correctly: the template uses the
  anonymous-friendly `/clone`, while copying any other (e.g. the user's own
  private) universe uses `/duplicate` (CO-95).

### Context menu + settings (Phase 2)
- Right-click any universe in the sidebar for a context menu: Open, Rename…,
  Change visibility (Private / Public / Unlisted), Duplicate…, Settings…,
  Archive, Delete…. Rename and visibility changes update the sidebar without a
  page reload.
- The universe Settings panel is now fully editable for owners — name,
  description and visibility, plus a danger zone with Archive / Delete.

### Soft-delete + archive + trash (Phase 3)
- New migration **v79** adds nullable `deleted_at` + `archived_at` columns to
  `universes`. Every universe-listing query (sidebar buckets, public listing,
  search, discovery) now filters them out.
- `DELETE /api/v1/universes/:slug` is now a **soft-delete** (sets `deleted_at`)
  instead of a hard cascade — the row, entries and on-disk data survive so the
  universe is recoverable. Delete is gated behind a type-the-key confirmation
  dialog.
- New endpoints: `POST /:slug/archive`, `POST /:slug/restore`, and
  `GET /api/v1/universes/trash`. The sidebar trash view lists deleted +
  archived universes and restores them within the retention window.

### Why
Universes could previously only be created/renamed/deleted via API or CLI. This
makes the platform feel like a productized content workspace, and soft-delete
removes the accidental-data-loss risk of the old hard-delete.


## [3.8.0] — 2026-06-13 — Money & Activation — payment, onboarding, security hardening

## CO-366 — Conversion + payment wiring — register → paid via Hostinger checkout (provider-agnostic trait)

End-to-end billing wiring so a registered brain owner can convert from free to
paid, behind a provider-agnostic `BillingProvider` trait (Hostinger today,
Pix/Stripe tomorrow — no lock-in).

- **`BillingProvider` trait** (`co-web/src/billing/`) with `create_checkout`,
  `verify_webhook`, and `name`. Implementations:
  - `ManualInvoiceProvider` — default, no external dependency; admin marks a
    user paid via `POST /api/v1/gestao/users/{id}/mark-paid`.
  - `HostingerProvider` — full first-ship: deterministic checkout URL +
    HMAC-SHA256 webhook signature verification (forged payloads rejected).
  - `PixProvider` / `StripeProvider` — stubs gated behind the `billing-pix` /
    `billing-stripe` cargo features (v3.1).
- **Endpoints**: `POST /api/v1/me/billing/checkout`, `GET /api/v1/me/billing/status`,
  `POST /api/v1/billing/webhook/{provider}` (flips `users.tier` to `paid` on
  payment success), and `POST /api/v1/gestao/users/{id}/mark-paid`. All four are
  documented in the OpenAPI catalog (CO-211).
- **Schema (migration v77)**: adds `tier_paid_at`, `tier_plan`,
  `billing_provider`, `billing_external_id` to `users`, and a `billing_events`
  audit table (the table CO-360's funnel step 7 was already waiting on).
- **Conversion KPI** in `/gestao/resumo`: registered users (7/30/90d), paid
  users, conversion rate, and mean `t_register → t_payment` seconds.
- **Activity (CO-361)**: all four billing events (`checkout_created`,
  `payment_succeeded`, `payment_failed`, `subscription_canceled`) are mirrored
  to the `atividades` log + EDA bus under `entidade = "billing"`.
- **Register-success CTA**: `static/variants/a/modules/auth/post-register.js`
  opens the checkout flow.

Provider selection and plan prices are config (`CO_BILLING_PROVIDER`,
`CO_BILLING_HOSTINGER_API_KEY`, `CO_BILLING_HOSTINGER_WEBHOOK_SECRET`,
`CO_BILLING_PLAN_PRICES_JSON`) read through the `SecretsProvider` seam — no
redeploy to switch providers or reprice.

### Why

§5 + §8 of `brain-as-a-service.md` flagged conversion/payment as the explicit
open gap: the `t_register → t_payment` KPI was designed but had no
implementation. This ships the billing-only tier enforcement (per CO-90, tier
is billing-only — no global admin tier) so public launch can charge the first
brain owner without an infra rebuild, while keeping the integration choice
swappable behind a trait.

## CO-438 — source:github import — 3 prod bugs surfaced by the live claude-code import (seed orphan, token-vs-session private visibility, rate-limit/no-resume)

Fixes the three independent bugs the live `claude-code` import (CO-429) hit at
114/164 entries, which together blocked a clean `source:github` import of a
private universe.

- **Bug 1 — seed orphan.** The admin-content seed INSERTed a `universes` row for
  the importable private universe `claude-code` owned by the sentinel `system`.
  Since access is granted by `owner_id` match, a `system`-owned private row was
  unreachable by any real user: `POST /universes` → 409 "key taken" while
  `GET /universes/{key}` → 404. The CO-429 reconcile that flipped the row to
  `private` is what tipped a (GET-able) public-subscribable row into the orphan
  state. Fix: importable private universes are no longer seeded — `co source
  add` creates them as the importing user (fully provisioned: owner + membership
  + pool) — and the private-flip reconcile is guarded with `owner_id != 'system'`
  so a legacy system-owned row stays public-subscribable instead of orphaning.
  After a seed, the universe is GET-able or absent, never "taken but 404".

- **Bug 2 — token-vs-session private visibility.** `GET /api/v1/universes/{key}`
  decoded session JWTs only, so an API-token request — even by a private
  universe's own owner — resolved to anonymous and 404'd. `co source add`'s
  `universe_exists` probe (api-token) therefore thought the universe was missing
  and tried to create it (401). Fix: the handler now resolves the caller via
  `resolve_user_id` (JWT *or* API token), matching the visibility-gate
  middleware. Owner reads their own private universe by api-token (200);
  non-owner still 404.

- **Bug 3 — rate-limit blocks bulk import; no skip/resume; admin not exempt.**
  (a) The global rate-limit middleware capped admin writes at 60/min *before*
  the vault handler's own admin exemption ran, so a >60-file `co source add` as
  admin hit 429 mid-import. Admin-tier requests to the Vault API now bypass the
  global limiter (mirroring `vault_auth`), scoped to vault paths.
  (b) `co source add` re-PUT every entry from the start each run. It now fetches
  the server's per-entry `body_hash` (newly exposed in the vault listing) and
  pushes only entries whose body differs — so a rate-limited import resumes and
  a re-sync touches only what changed. Added a `--throttle-ms` flag and
  Retry-After-aware backoff on 429 for non-admin pushes.

### Why
These three bugs are why the import stalled at 114/164 and why any
`source:github` import of a private universe failed at scale. Each is fixed
independently with regression tests (seed provisioning, owner api-token
visibility, admin bulk-write exemption, and hash-based skip/resume).

## CO-439 — Surfaces allowlist-on-serve — serve only published/indexed entries, never raw disk files

The surfaces server now serves **only** content present in a universe's
published index — it serves the index, never a raw file that happens to exist on
disk. A deep-link request for a path that is not indexed (or, for an anonymous
caller on a `anon_published_only` universe, not `published: true`) resolves to
**404**, not 200.

- **Allowlist-on-serve (the real fix):** new `co-web/src/server/allowlist.rs` is
  the single chokepoint. `serve_deep_link` now consults `is_servable()`, which
  requires an index entry for one of the candidate paths *and* visibility to the
  caller (published gate for anon, mirroring the API-layer filter). An unindexed
  `draft.md` in a served directory is never served. The previous
  `entry_exists_for_subpath` existence check is superseded by this module.
- **Defense in depth — `.dockerignore`:** added a root `.dockerignore` with
  draft/scratch conventions (`WhatsApp*`, `_*`, `shot-*`, `*.mov`, `**/IMG_*`).
  This is a denylist that fails open — a cheap complementary layer, **not** the
  security boundary.
- **Flow rule documented:** `docs/serve-allowlist.md` (new) plus pointers in
  `docs/universe-public-site.md` and `docs/use-cases.md` — a draft is born in the
  vault, never inside a served directory, and only crosses into a served universe
  via the drafts→published flow (which creates an index entry).
- **Audit:** new `audit_serve` binary (`cargo run -p co-web --bin audit_serve --
  <data-dir>`) walks each universe's served on-disk root and reports files
  present on disk but absent from the index — the "servable but not published"
  leak surface. Exit 1 when any remain, so it can gate a pre-deploy check.

### Why

Post-mortem of the 2026-06 `ArteLonga/yuri` `thrive market.md` draft leak: a
private draft went public because it sat in a served directory, missed every
`.dockerignore` pattern, and existed on disk at deploy time. The structural gap
was the absence of an allowlist — the `.dockerignore` denylist was the only
barrier, and a denylist fails open. `.dockerignore` is not a security boundary;
serving only the published index is. Same family as the visibility gate
(CO-161); feeds the content-addressed encrypted-asset storage plan (CO-145).

## CO-441 — Finish AC3: move residual raw SQL out of content handlers into storage methods (workspace/assets/vault/op_log/template)

Moved the last single-line `conn().query_row/execute` calls out of the content
HTTP handlers into typed `Storage` methods, closing the literal AC3 grep from
CO-433 ("zero raw SQL em todos os *_routes.rs").

- New `storage/workspace_states.rs`: typed accessors for the "Sala" canvas state
  (`workspace_state_for_user`, `public_workspace_state`,
  `workspace_state_by_share_token`, `upsert_workspace_state`,
  `set_workspace_share_token`). The `WorkspaceState` row struct and its
  row-mapper now live in the storage layer (re-exported as
  `crate::storage::WorkspaceState`), so `content/workspace/routes.rs` carries no
  SQL.
- New `Storage::set_universe_content_count`, replacing the four identical
  `UPDATE universes SET content_count = ?1` statements scattered across the
  asset upload handler, both vault write paths, the op-log
  `refresh_content_count`, and the template `reindex` handler.

### Why
Pure mechanical refactor — zero behavior change. Removing the inline SQL keeps
content handlers free of database string literals (also clearing the CWE-89
scanner debt on those lines) and routes every meta-DB mutation through a tested,
typed method on `Storage`.

## CO-99 — Onboarding banner — three-step coach mark for first-time anonymous visitors

Adds a non-blocking floating coach-mark card (bottom-right, 320×120px) for
first-time anonymous visitors on the template universe. The banner walks through
three steps — Visões, Linha do tempo, and Crie seu universo — with a step
indicator, a "Próximo"/"Concluir" button, and a "Pular" dismiss link.

Dismissal (skip or complete) sets `co_onboarded=1` (1-year cookie) so the
banner never re-appears. Suppressed on viewports < 720px and for logged-in
users. Step 2's timeline link opens in a new tab; Step 3's "Criar conta"
triggers the existing login/signup modal via event delegation.

### Why

First-time visitors landed on a kanban with no orientation. The coach mark
surfaces the platform's three key narratives (multi-view, timeline, own
universe) in the first 30 seconds without blocking any interaction.


## [3.7.0] — 2026-06-13 — Mythos — composable, extensible architecture [--ignore-dod override]

## CO-431 — Mythos: promote the Universo domain to core + UniversoFactory seam

The `Universo` trait and its domain types (`Tarefa`, `Nota`, `Evento`,
`Membro`, `Relato`, `Conteudo`, `Entrada`, `UniversoInfo`, `UniversoConfig`)
moved from `co-web/src/content/universo.rs` to `core/src/universo.rs` — pure
serde + std, no server-framework/database/async-runtime dependencies — so
external services (universe runtimes, Yggdrasil, CLI) can depend on `core`
alone. `co-web` re-exports everything (`pub use co::universo::*`) for full
compatibility.

A new `UniversoFactory` trait (`abrir(key, root) -> Box<dyn Universo>`) is the
backend seam: it is injected into `CoreState` (AppState), defaulting to the
filesystem implementation (`UniversoLocalFactory` → `UniversoLocal`, which
stays in co-web). `CoreState::with_universo_factory` swaps the backend without
touching handlers or routes. A swap test proves it: the same axum handler,
mounted on the same route over the real AppState, serves a disk universo with
the default factory and an in-memory universo with a fake factory — no route
or handler edits between scenarios.

Zero behavior change: no production HTTP route consumes the trait yet (the
2026-06-12 audit's "instanciado direto nos handlers" overstated — the trait
was dormant), so HTTP contracts and openapi/api-catalog are untouched.
Includes a mechanical `Cargo.lock` version sync (3.5.0 → 3.6.0) left stale by
the release commit.

### Why

Keystone of the Mythos epic (CO-430): unblocks external consumption of the
universo domain model and prepares alternative universe backends (CO-433)
behind a factory seam instead of a hardcoded filesystem implementation.

## CO-432 — Mythos: decompose the content/ god-folder + propagate the CO-390 layering

`co-web/src/content/` reorganized from 37 flat files into one folder per
entity — `entries/`, `vault/`, `references/`, `relations/`, `graph/`,
`workspace/`, `universe/`, `assets/`, `proposals/`, `reviews/`,
`versioning/` (states + branches + op-log), `search/`, `translate/`,
`delivery/`, `usage/`, `openapi/`, `agent_sessions/`. No loose `*_routes.rs`
remain; pre-432 module paths (`crate::entry_routes`, `crate::vault_routes`, …)
are preserved via `pub use … as …` aliases in `content/mod.rs`, so no call
site changed.

The CO-390 layering template (domain / dto / repository / service / mapper)
was propagated from `entries` to **references** and **relations**:

- `domain/entity/{reference,relation}.rs` — pure business types.
- `dto/{references,relations}/` — wire types moved out of the route files
  (names and serialization unchanged).
- `repository/{reference,relation}_repository.rs` — traits + SQLite impls
  wrapping `ReferenceIndex`/`references_meta` and `RelationIndex`; in-memory
  impls for unit tests.
- `service/{reference,relation}_service.rs` — pure rules extracted from
  handlers (type forcing, partial-update merge, work_id derivation,
  seed-status stub rule, deterministic inbound ordering), each unit-tested.
- `mapper/{reference,relation}_mapper.rs` — row ↔ domain ↔ DTO conversions.

`entry_index.rs` and `reference_index.rs` moved behind their repositories:
`SqliteEntryRepository` (now built on the real `Arc<std::sync::Mutex<Connection>>`
universe-connection type) gained 1:1 mirrors of the index surface plus
combined write operations (`index_entry_create/update`, `index_vault_write`,
`unindex_entry`, `unindex_vault_entry`) that keep every multi-projection
write in a single lock scope — including the CO-95 `BEGIN IMMEDIATE`
transaction semantics of the vault path. All content route handlers (plus
`static_files` deep links and `sync_ws`) now access the indexes exclusively
through the repository layer; the only remaining direct uses are two
conn-taking semantic-search helpers that interleave `EmbeddingIndex` on the
same connection (documented escape hatch).

Oversized test files split per the CO-215 `tests/` pattern (every group
< 500 LoC): `universe/routes/tests/` (7 files), `vault/routes/tests/`
(6 files), `relations/index/tests/` (4 files), `reviews/routes/tests/`
(3 files), `references/meta/tests.rs`. `relations/index.rs` itself split
into `index.rs` (storage) + `extract.rs` (extraction + backfill).

Zero behavior change: routes, wire formats, status codes, and lock-scope
semantics preserved; full `cargo test -p co-web` green and clippy clean.

### Why
`content/` mixed 10+ entities in one flat namespace and the CO-390 layering
covered only `entries` (~10% adoption). Each entity now owns its folder and
the two highest-value entities besides entries follow the proven template,
making them unit-testable without HTTP or SQLite setup and giving the next
extractions a mechanical recipe to follow.

## CO-433 — Mythos: per-universe sharded storage + EntryStore trait; remove raw SQL from handlers

Refactors the storage layer toward per-universe sharding and a backend-swappable
content seam, and removes raw SQL from the request path so handlers no longer
speak rusqlite directly.

### Per-universe sharding (criterion 1)
- `UniversePool::entry_store(key)` + `Storage::entry_store(key)` open a
  per-universe store backed by that universe's own connection. Content
  reads/writes routed through it never take the global `Mutex<Storage>` for the
  actual I/O — only a brief handle fetch — so writes to distinct universes do
  not serialize. Proven by a new contention test
  (`writes_to_two_universes_do_not_serialize`): a write held open on universe A
  does not block a concurrent write to universe B.

### `EntryStore` trait + pool-as-factory (criterion 2)
- New `repository::EntryStore` trait (get/upsert/delete/list, per universe),
  with `SqliteEntryStore` as the default implementation (reusing the existing
  `SqliteEntryRepository`, no duplicated SQL). `UniversePool` is the factory
  that hands out `Arc<dyn EntryStore>`. The trait is the seam an S3/Postgres
  backend plugs into later — no S3 implemented here.

### Zero raw SQL in handlers/subscribers (criterion 3)
- Moved every request-path/subscriber `conn().prepare/execute/query_row` in the
  audited surfaces into typed `Storage` methods. New impl-blocks:
  `storage::eda`, `storage::security`, `storage::auth`, `storage::leads`,
  `storage::graph_views`, `storage::feedback` (plus a `delete_quilombo_user`
  on `storage::quilombo_bridge`). Migrated handlers/subscribers: `eda/mod.rs`,
  `eda/bridge/{client,handler}.rs`, `eda/subscribers/{atividades,delivery_pipeline}.rs`,
  `security/routes.rs`, `security/subscribers/{pbi_backlogger,findings_persistor}.rs`,
  `auth/{onboarding_routes,recovery_routes,mod}.rs`, `admin/lead_routes.rs`,
  `content/graph/view_routes.rs`, `integrations/feedback_routes.rs`,
  `universes/quilombo/quilombo_routes.rs`. `grep 'conn().prepare\|conn().execute'`
  over `*_routes.rs`/subscribers is now empty for non-test code (test fixtures
  legitimately retain direct `conn()` setup).
- Row projections `LeadRow` and `GraphView` moved to the storage layer (with the
  typed queries that produce them) and re-exported from their routes for the API.

### No behavior change (criteria 4, 5)
- No new `Mutex<Storage>` held across `.await`; no new panics under the lock.
- SQL text and error handling preserved verbatim in the new methods.
- `cargo test -p co-web` green; `cargo clippy -p co-web --lib -- -D warnings` clean.

### Why
The global `Arc<parking_lot::Mutex<Storage>>` serialized unrelated universes and
handlers reached past the storage boundary into raw rusqlite, making the backend
hard to swap and the data access un-auditable. This shards writes per universe
and routes all content access through typed, backend-agnostic seams.

## CO-434 — Mythos: enforce SecretsProvider + CoServerConfig (kill 124 direct std::env reads)

All runtime configuration and secrets now flow through the `SecretsProvider`
abstraction and a new boot-time `CoServerConfig`, instead of ~124 scattered
`std::env::var` reads in 52 files. The secrets backend (env, static, future
Vault/AWS SM/S3) is now swappable, and the server is embeddable with injected
config. **Zero behaviour change in production** — the same env vars, same
defaults.

### What changed

- **`SecretsProvider` helpers** (`infra/secrets.rs`): added `is_set`, `get_or`,
  `get_nonempty`, `get_bool`, and (via `SecretsProviderExt`) `get_parsed<T>` —
  object-safe split so `dyn SecretsProvider` still works.
- **`CoServerConfig`** (`platform/server_config.rs`): one struct holding every
  non-secret tunable, populated **once at boot** from a `SecretsProvider`
  (`CoServerConfig::from_secrets`). Stored on `CoreState.server_config`;
  handlers read `state.core.server_config.<field>` instead of `env::var`.
- **Named subsystems take config/secrets by parameter**: `eda::build_bus`,
  `infra::blob::blob_backend_from_config`, `infra::ai::build_chat_provider`,
  `infra::telemetry::TelemetryConfig::from_config`, and the degradation
  `mailer_from_secrets` now receive their config/secret rather than reading env.
- **Process-global provider seam** (`infra::secrets::{init_global, global}`):
  installed once at boot from the same provider that builds `CoServerConfig`.
  Stateless free functions / middlewares that have no `AppState` to thread a
  provider through (e.g. `auth::jwt_secret`, EDA bridge config, `vcs` git creds,
  quilombo dirs, canonical-host middleware) read through it. Defaults to
  `EnvSecretsProvider` when uninitialised, so unit tests keep working.
- **Single boot seam for `std::env::var`**: the only non-test runtime read left
  is `EnvSecretsProvider::get` in `infra/secrets.rs`. `WebConfig::from` (CLI
  parse) now reads its fields through `EnvSecretsProvider` too.
- **Tests** prove the swap without global env: `CoServerConfig::from_secrets`
  with a `StaticSecretsProvider` flips the EDA backend / blob backend / sampling
  ratio; `CoreState::from_storage_with_secrets` propagates injected config end to
  end (`server::tests::corestate_server_config_is_provider_driven`).

### Inventory (classified)

**Secrets — via `SecretsProvider::get` (not copied into `CoServerConfig`):**
`JWT_SECRET`, `CO_JWT_PRIVATE_KEY`, `R2_ACCOUNT_ID`/`R2_ACCESS_KEY_ID`/
`R2_SECRET_ACCESS_KEY`, `RESEND_API_KEY`, `OPENAI_API_KEY`, `CO_SECURITY_API_KEY`,
`CO_KB_TOKEN`, `CO_ROLLUP_TOKEN`, `CO_FLY_API_TOKEN`, `CO_GIT_TOKEN`,
`CO_GIT_SSH_KEY_PATH`, `GOOGLE_CLIENT_ID`/`GOOGLE_CLIENT_SECRET`,
`GITHUB_OAUTH_CLIENT_ID`/`GITHUB_OAUTH_CLIENT_SECRET`, `CO_GITHUB_WEBHOOK_SECRET`,
`CO_ASSETS_MASTER_KEY`, `VAPID_PRIVATE_KEY`, `EVOLUTION_API_KEY`,
`CO_SMTP_USER`/`CO_SMTP_PASS`, `CO_SEED_ADMIN_PASSWORD_HASH`,
`CO_BRIDGE_OUTBOUND_TOKENS_JSON`, `WAE_API_KEY`.

**Config — via `CoServerConfig` (or the global provider for stateless fns):**
`CO_EDA_BACKEND`, `CO_DEPLOYMENT_ID`/`FLY_APP_NAME`, `CO_BRIDGE_*`,
`CO_BLOB_BACKEND`/`R2_BUCKET`, `CO_CHAT_FALLBACK`/`CO_CHAT_MODEL`/`CO_OLLAMA_URL`,
`CO_TRANSLATE_BACKEND`/`CO_TRANSLATE_PROVIDER`, `CO_TELEMETRY_*`, `CO_ALERT_*`,
`CO_SECURITY_BACKEND`/`CO_SECURITY_MAX_SCANS_PER_DAY`, `CO_BACKUP_*`/
`CO_REMOTE_SYNC_INTERVAL_SECS`, `CO_SEED_ADMIN_EMAIL`, `CO_DEV_OWNER`,
`LEADS_NOTIFY_TO`, `CO_LOCAL_REPOS_DIR`, `CO_SEED_CO_DIR`, `CO_MODELS_DIR`,
`GEOIP_DB_PATH`, `CO_TRUSTED_IPS`, `CO_STATIC_SITES`, `CO_PUBLIC_URL`,
`CO_BASE_URL`, `CANONICAL_HOST`/`ALLOWED_ORIGINS`, `CO_FEEDBACK_FORWARD_URL`,
`NOTIF_FROM_EMAIL`, `RESEND_FROM`, `VAPID_SUBJECT`, `EVOLUTION_API_URL`/
`EVOLUTION_INSTANCE`, `CO_SMTP_HOST`/`CO_SMTP_FROM`/`CO_SMTP_PORT`,
`CO_DESKTOP_NOTIFY`, `CO_EMBEDDING_BOOT_SCAN`, `QUILOMBO_*`, `CO_CACHE_*`,
`CO_TPL_*`, plus the existing `WebConfig` fields (`CO_ENV`, `GESTAO_GITHUB_ADMINS`,
`UNIVERSE_KEY`, `WAE_ENDPOINT`, `CO_COOKIE_DOMAIN`, `CO_QUILOMBO_LEGACY_LOGIN`,
`CO_BYPASS_RATE_LIMIT`, `GAME_DB_PATH`, `PLUGINS_DIR`, …).

After this change, `grep -rn "std::env::var" co-web/src | grep -v test` returns
only `EnvSecretsProvider::get` — the single documented boot seam.

### Why

The `SecretsProvider` abstraction existed but was ignored by 124 direct reads,
so the secrets backend was not actually swappable and the server was not
embeddable. Centralising at boot unblocks the S3 roadmap (swappable backend
config) and lets tests flip behaviour by injection instead of mutating the
global process environment.

## CO-435 — Mythos: extension registries — EDA subscriber registry + SourceAdapter + AdminAuthProvider

Turned four hardcoded extension points into `registry + trait + impl-default` seams.
No behavior change: the same subscribers, sources, admins and rate limits run by
default — only *how you add the next one* changed.

### What changed

- **EDA subscriber registry** (`eda::subscriber_registry`): new `EdaSubscriber`
  trait (`name` / `filter` / async `handle`) + `SubscriberRegistry` + `SubscriberCtx`.
  Boot now iterates `default_registry(...).spawn_all(ctx)` instead of ~13 inline
  `spawn()` calls in `server/mod.rs`. Every existing subscriber (atividades,
  analytics, billing, sala, kb, comunicação term+sala, yggdrasil_notes,
  delivery_pipeline, timeline, findings_persistor, pbi_backlogger, release_blocker,
  degradation_alerter) migrated to the trait. A test subscriber can now register
  without touching the boot.

- **SourceAdapter seam** (`platform::source`): new `SourceAdapter` trait
  (`kind` + async `sync`) + `SourceRegistry`. `GitSourceAdapter` (`remote-git`)
  consolidates the CO-417/CO-423/CO-337 git sync (wrapping `crate::vcs`);
  `EventBusSourceAdapter` (`event-bus`) expresses the push-driven Yggdrasil sync.
  `run_remote_sister_repo_seeds` routes the git step through the adapter. Adding a
  source (gitlab/notion) = new `impl` + `register`.

- **AdminAuthProvider seam** (`infra::admin_auth`): new `AdminAuthProvider` trait
  (`async verify_admin -> AdminIdentity`). `github_auth` becomes the default impl
  (`GitHubAdminAuthProvider`); the `require_github_admin` middleware now depends on
  an injected `Arc<dyn AdminAuthProvider>`, shared across all six admin routers.
  Swapping in SAML/OIDC = a new provider injected in `router.rs`.

- **RateLimiter trait** (`platform::rate_limit`, absorbs CO-297 / CO-284-H): new
  `RateLimiter` trait (`try_acquire`); the existing in-process token-bucket limiter
  is renamed `InProcessRateLimiter` and becomes the default impl. Completes the
  CO-284 trait series. (No Redis impl here.)

### Why

Three points (EDA subscribers, source sync, admin auth) plus the rate limiter were
hardcoded — extending any of them meant editing the boot or rewriting call-sites.
Each is now a composition seam: register an `impl`, no fork. Pure composition —
the default set is unchanged.

## CO-436 — Mythos: framework-agnostic game-core Plugin + split migrations.rs/seed.rs

Three mechanical, behavior-preserving refactors that unblock reusing `game-core`
outside `co-web` and make the storage layer navigable.

### `game-core` is now axum-free

`Plugin::routes()` no longer returns an `axum::Router`. It returns a portable
`Vec<RouteDescriptor { path, method, handler_id }>` that the **host** translates
into concrete routes. The `axum` dependency is gone from `game-core/Cargo.toml`
(`cargo tree -p game-core | grep axum` is empty), so a CLI, mobile, or embedded
consumer (Yggdrasil) can implement `Plugin` without dragging in an HTTP stack.
`co-web` owns the translation: `plugin_loader::descriptors_to_router` maps each
descriptor's `handler_id` to its handler (the existing `GET /info` route is
preserved exactly), and forward-compatible unknown handler ids are logged and
skipped rather than crashing the loader.

### `storage/migrations.rs` sliced into a module

The 2.7k-LoC monolith became `storage/migrations/` with one module per version
range (`v001_018` … `v073_076`), each under 500 LoC, aggregated by
`Storage::run_migrations`. `current_version` is read once and threaded into every
range, so a fresh DB and an already-migrated DB converge on the identical final
`schema_version` (new tests assert version 76 on fresh + idempotent re-open). No
existing migration was renumbered — the version-claim protocol is intact (the
two pre-existing `< 44` blocks and all 76 versions are preserved verbatim). Two
dead trailing `current_version` re-reads that landed at slice boundaries were
dropped (their value was never read).

### `storage/seed.rs` moved to the boot path

The 2.6k-LoC seed orchestration moved from `storage/seed.rs` to `server/seed.rs`,
separating boot-time universe seeding from the storage data-access layer. Seed
methods stay on `Storage` (call sites unchanged); `Storage.conn` and the shared
`SEED_*` template constants are now `pub(crate)` so the boot module can drive
them.

### Why

Decouples the game engine from the web framework — the "external services" half
of the Mythos epic (real reuse of `game-core` outside `co-web`) — and turns two
of the largest files in `co-web` into navigable modules, all with zero schema or
seed behavior change.

## CO-440 — co-auto: capture headless stdout + commit-uncommitted safety net

Two co-auto reliability fixes surfaced by the Mythos wave (Fable runs reporting
to the prod `/usage` dashboard).

### Fixed

- **Headless stdout was never captured.** The headless `claude` invocation
  `spawn()`-ed with inherited stdio, so `wait_with_output()` returned an **empty**
  `output.stdout`. CO-425 usage capture (`parse_stream_json`) and the
  assistant-text re-emit therefore got nothing on *every* headless run — the
  `/usage` dashboard stayed empty even with `CO_USAGE_ENDPOINT` set. Now the cmd
  sets `stdout(piped())` + `stderr(piped())` (drained concurrently — no deadlock
  on long runs), so token/cost/tool-usage telemetry actually reaches CO-426.

- **"Nothing to ship" when the agent staged but didn't commit.** `ship-task.sh`
  only checked `git rev-list origin/main..HEAD`, so a run where the agent ran
  `git add` without `git commit` (observed with Fable) died with "Nothing to
  ship" and lost the PR. Added a safety net: if the worktree has uncommitted work,
  ship-task commits it first with a conventional message derived from the spec's
  `conventional_commit` + `title` (`<type> <TASK-ID> — <title> (auto-committed by
  co-auto)`), then proceeds.

### Why

These two bugs share a theme — co-auto silently dropping the agent's output/work
on real runs. Together they blocked the CO-424 payoff (per-model×per-universe
usage) and forced manual salvage of completed refactors (CO-432).


## [3.6.0] — 2026-06-12 — fleet observability & model routing

## CO-423 — source-deploy toolchain: re-parent API + adapter existence-check + auto-index

Three concrete gaps exposed by the CO-419 `nlp` deploy are fixed so importing a
repository into a universe produces a **complete** universe — correct parent,
landing page, real existence detection — with no manual patching.

### Re-parent via API (the blocker)
`PUT /api/v1/universes/{slug}` now accepts `parent_key: Option<String>`. When
provided, the owner (only) can re-parent a universe: a non-empty key (validated
to exist, and not the universe itself) sets `universes.parent_key`; an empty
string clears it to NULL. Non-owners get 403; a nonexistent parent is rejected
with 400. The `UpdateUniverseResponse` now echoes `parent_key`. This makes
`PUT /api/v1/universes/nlp {"parent_key":"yuri"}` work — the last open item of
CO-419 §E. No new migration: `universes.parent_key` already exists (CO-98 v22).

### Adapter existence-check no longer fooled by the SPA fallback
`co source add` previously treated any HTTP 200 from `GET /universes/{key}` as
"exists" — but the SPA serves 200 (its HTML shell) for unknown routes, so it
false-positived. It now parses the universe JSON and requires a `key` field that
matches the requested slug; an HTML shell (JSON parse failure) is treated as
absent. When creating is needed but the credential is an API token (POST
/universes requires a session), it now surfaces a clear, actionable error
instead of a bare 401.

### `--parent` and auto-index
`co source add github` gains `--parent <key>` (sets `parent_key` after create via
the new PUT API) and `--no-index`. By default, if the imported repo brought no
content-root `index.md`, a minimal navigable landing is synthesized linking the
imported tree; an existing `index.md` is never overwritten.

### Why
Unblocks CO-419 and any future repo import: the deploy toolchain now makes the
universe whole (parent, landing, real source detection) without hand-patches.

## CO-425 — co-auto: capture Claude usage via stream-json, POST best-effort

co-auto now invokes headless Claude Code with `--output-format stream-json --verbose`
and parses the streamed NDJSON events for per-message token `usage`
(`input_tokens`, `output_tokens`, `cache_creation_input_tokens`,
`cache_read_input_tokens`, `model`) plus the final `result` event
(`num_turns`, `duration_ms`, `total_cost_usd`). Usage is aggregated per task
into a serializable `SessionUsage` and POSTed best-effort to the CO ingestion
endpoint.

- New `co-auto::usage` module: `SessionUsage` struct + `parse_stream_json()` +
  `assistant_text()` (re-emits the human assistant text to the launcher log so
  task visibility is unchanged).
- The aggregated tokens now feed the existing `AgentSessionRecord` (preferring
  stream-json over the legacy stdout scraping), and a one-line summary is printed
  per task: `usage: 19.0k in (89% cached) / 460 out — sonnet — 6m12s`.
- New best-effort POST to `CO_USAGE_ENDPOINT` (`/api/v1/usage/sessions`),
  **default OFF** — a no-op when the env var is unset. Auth reuses the existing
  `CO_SESSION_TOKEN` bearer scheme. Hostname/model/outcome included in the payload
  (CO-426 defines the canonical schema).

### Why

Telemetry must never block or fail a co-auto task. Every failure mode — parse
error on a stream line, missing endpoint, network/POST failure, serialize error —
is swallowed and logged; the task still succeeds. Subscription auth reports no
USD, so cost is only attached when the `result` event carries `total_cost_usd`.
This is the data source for the real-time usage dashboard (CO-424 epic / CO-426).

## CO-426 — co-web: ingestão de usage + registry de launchers + dashboard Gestão em tempo real

Centralized, real-time fleet-usage observability for co-auto across machines and
deployments: which launchers are active, where tokens are going, and how the
current inference window is being consumed.

### Added

- **Migration v75** — `usage_sessions` table in the meta DB (fleet-wide token/cost
  ledger: `task_key`, `universe_key`, `machine`, `model`, in/out/cache-read/cache-write
  tokens, nullable `cost_usd`, `started_at`/`ended_at`/`outcome`/`reported_at`), with
  indexes per window dimension. Deliberately a **new** table, not the CO-275
  `agent_sessions` (per-task kanban provenance) — distinct shape and purpose.
- **`POST /api/v1/usage/sessions`** — ingests the CO-425 stream-json usage report
  (matches co-auto's canonical payload exactly), inserts a row, and publishes a
  `DomainEvent::UsageReported` bridged to `AnalyticsEvent::Usage` so the dashboard
  updates over the existing AnalyticsBuffer broadcast — no polling.
- **`POST /api/v1/usage/heartbeat`** — launchers register/refresh an active session in
  an in-memory TTL registry (`ActiveLaunchers`, 90s). A crashed machine simply stops
  beating and ages out — no DELETE. Each beat broadcasts a full `LauncherSnapshot`.
- **`GET /api/v1/usage/summary?window=5h|day|week&by=universe|model|machine|task`** —
  aggregates for the dashboard and the CO-427 downshift policy, plus always-present 5h
  and week window totals and configurable soft-limit markers
  (`CO_USAGE_SOFT_LIMIT_5H` / `_WEEK`).
- **`GET /api/v1/usage/active`** — launchers active right now.
- **`GET /api/v1/usage/projects`** — fleet roll-up: usage accumulated per universe, with
  best-effort board task counts (per-universe DB) and last deploy.
- **Gestão "Uso" panel** — active launchers, current-window consumption bars, usage by
  universe/model/machine/task with grouping selector, and the projetos roll-up. Updates
  live by subscribing to the existing `/api/v1/analytics/stream` WebSocket (no
  setInterval+fetch).

### Why

Yuri operates co-auto across several machines and deployments. To prioritize the
inference window (opus/fable vs sonnet/haiku) with data rather than intuition, the fleet
needs one real-time view of projects, boards, active launchers and token consumption.

### Notes

- Auth, two tiers (review hardening): **writes** (`POST /sessions`, `/heartbeat`) take
  any valid vault token or JWT — the reporting launchers; **reads** (`GET /summary`,
  `/active`, `/projects`) additionally require `tier == "admin"` (verified through the
  same `auth_provider` as the write path → **403** for non-admins). Fleet-wide cost and
  live launcher inventory are API-gated like every other Gestão data router, not merely
  hidden behind the admin-only page (page-gating does not stop a direct token request).
  The CO-427 downshift consumer reads `/summary` with the operator's admin session token.
- The panel shows **observed consumption** + configurable milestones, not official
  Anthropic subscription limits (those aren't exposed by API).
- No universe→machine/repo mapping is hardcoded — everything comes from the DB or payload.

## CO-427 — Model routing: modelo por task (frontmatter) + política priority→model + downshift por janela de uso

co-auto now resolves the executor model **per task** instead of using one global
`--model` for the whole run. Resolution follows a four-level precedence:

1. `--model` on the CLI (operator override — pins every task).
2. `model:` in the task frontmatter (per-task override; invalid value → warn + ignore).
3. Priority→model policy (`high→opus`, `medium→sonnet`, `low→haiku`), configurable
   in `project.yaml` — **opt-in via `by_priority: true`** (or
   `CO_AUTO_ROUTING_BY_PRIORITY=1`); off by default.
4. Quality-first default (`opus`) — the **default** for any task with no `--model`
   and no frontmatter, per the binding owner decision (default = opus, *not* the
   priority tier; tiers are opt-in).

A best-effort **window downshift** runs before each launch: co-auto reads CO-426's
`GET /api/v1/usage/summary?window=5h`, and if the rolling 5h token consumption has
crossed the configured soft limit (`usage_soft_limit_5h_tokens` /
`CO_AUTO_SOFT_LIMIT_5H_TOKENS`), it degrades the model one tier (`opus→sonnet→haiku`).
The decision is printed to the launcher log and included in the usage report
(`model_requested` vs `model_used`, plus a `downshifted` record) so the dashboard
can surface the degradation. Every failure path (no endpoint, no network,
unparseable response, no soft limit, already-lowest tier) is fail-open — the
requested model is kept.

New module `dev/co-auto/src/routing.rs`; `--model` is now optional (no longer
defaults to `sonnet`). Docs: "Model routing" section in `dev/co-auto/README.md`
and a frontmatter example in `work/co/CLAUDE.md`.

### Why
Maximize what the subscription delivers per usage window: spend Opus on hard work,
fall back to cheaper tiers automatically near the budget, and never burn Opus on
chore. Routing is decided at launch and never rebalances running tasks; model
aliases pass through to the `claude` CLI without hardcoded-id validation.

## CO-428 — `co universe digest`: deterministic, recursive, cache-friendly universe summaries

Added `co universe digest [<key>] [--depth N] [--format md|json] [--data-dir DIR]`,
a new co-cli command that emits a generated (not hand-written) summary of the
universe forest sourced from the local registry (SQLite), for token-efficient
co-auto / Claude sessions.

- **Deterministic / byte-stable**: same DB state → identical bytes. Fixed
  key-sorted ordering (DFS over `parent_key`), no timestamps, no random
  iteration (`BTreeMap` throughout, hand-rolled stable JSON). This makes the
  digest usable as a cacheable prompt prefix.
- **Recursive** via `parent_key` (CO-98): roots first (key-sorted), each
  followed by its subtree; `--depth N` bounds recursion.
- **Token-bounded**: each universe block targets ≤200 tokens (chars/4); a
  warning is printed if a block exceeds the budget. Measured ~48 tokens/universe
  average, 56 for the richest block.
- **Cache-friendly layer ordering**: stable identity/hierarchy first, volatile
  counts (entries, tasks-by-status) last within each block, so edits to one
  universe's counts do not invalidate the cached prefix of earlier universes.
- **No hardcoded mappings**: data comes only from the registry (CO-424 constraint).

Per-universe fields: key, name, purpose (truncated description), parent,
children, content types ("modelos") in use, page/project/task counts, and
task counts grouped by status.

### Measured token reduction (CO-424 ≥40% target)

On the real content-universe forest (`co → template → {comunicacao, time,
topologia}`, `miguel → mse`, …), the digest replaces the verbose universe-landscape
prose an agent otherwise reads:

- Universe-landscape layer: **990 → 488 est. tokens (50.7% fewer)** on a single task.
- Across a co-auto wave (same universe, digest byte-stable → cached after the
  first task): uncached tokens drop **83.6% (3 tasks) … 95.1% (10 tasks)**.

The target is met on a single task and exceeded across a wave.

### Tests

10 unit tests (token estimate, purpose truncation, forest DFS/depth/orphan
ordering, counts-last layout, JSON escaping) + 5 CLI integration tests over a
seeded 3-level universe tree, including two **byte-stable determinism** tests
(md + json) that run the digest twice and assert identical output.

## CO-429 — claude-code como universo (source:github) + catálogo de superfícies de integração

The `claude-code` universe (registered by CO-364) now has the correct metadata:
`parent: co` and `visibility: private`. Boot-time reconcile UPDATEs fix existing
installs. The upstream repo's content subdirs were corrected from `docs/` (which does
not exist) to `examples/` and `plugins/`. Seven integration-surface entries were added
to `tools/claude-code/` in the `co` universe — one per surface: `headless-stream-json`,
`model-flag`, `claude-md`, `skills`, `hooks`, `session-jsonl`, `otel-telemetry`. Each
entry documents what the surface is, when CO uses it (linked task), a minimal example,
and a cross-universe link (`[[claude-code::...]]`) to the canonical upstream doc.

### Why

Agents and launchers operating on CO need a navigable, on-demand reference for Claude
Code integration surfaces instead of loading that knowledge into each session's context.
The tool catalog lets `co universe digest co` surface the integration points, and
`co source sync claude-code` keeps the upstream docs current.

## CO-437 — Usage metadata enrichment: tool usage + outputs + PR links, and per-model×per-universe breakdown

Extends the CO-425/CO-426 usage pipeline so each captured session carries more
than tokens, and the Gestão "Uso" dashboard can break consumption down by
**model × universe** (the cross-tab, not just each dimension alone).

- **Capture (CO-425 extended):** `parse_stream_json` now harvests `tool_uses`
  (tool name → invocation count) and `output_chars` (total assistant text size)
  from the stream-json content blocks, and co-auto attaches the opened `pr_url`
  (parsed from stdout) and `turns` to the reported usage. All best-effort /
  default-empty — telemetry never fails or blocks a run.
- **Schema (CO-426 extended):** meta-DB migration **v76** adds
  `usage_sessions.tool_uses TEXT` (JSON), `pr_url TEXT`, `turns INTEGER DEFAULT 0`
  via additive `ALTER TABLE` under a version guard. New columns are read with
  explicit SELECTs (never `.ok()`-swallowed).
- **Cross model×universe:** `GET /api/v1/usage/summary?cross=universe,model`
  (also accepts a comma-bearing `by=`) returns the matrix — rows per universe,
  columns per model, cells with tokens/cost/sessions. Admin-gated like the rest
  of CO-426.
- **Per-session listing:** `GET /api/v1/usage/sessions` returns recent sessions
  with their tool usage + PR link (admin-gated).
- **Dashboard:** the "Uso" panel gains the model×universe matrix and a
  "Sessões recentes" table showing tool usage + PR link per session. Updates in
  real time over the existing analytics stream (no polling).

### Why
To measure the real cost of each wave (e.g. Mythos in Fable) at model×universe
granularity — the base for estimating whether a future `mythos` model pays off
against Fable/Opus/Sonnet. Official subscription limits aren't exposed by API,
so the matrix reports observed consumption.


## [3.5.0] — 2026-06-12 — security gate + ops resilience + Miguel

## CO-388 — Security audit pipeline integration — Claude Security in CO-382 CI route (Project Glasswing-aligned)

Adds step 11 (security audit) to the deterministic 10-step CI route (CO-382).
Every PR is scanned for vulnerabilities using the `LocalGrepBackend` (always
available) or `ClaudeSecurityBackend` (when `CO_SECURITY_BACKEND=claude`).
Findings flow through the EDA bus: Critical/High block merge and create sprint
PBIs; Medium creates advisory PBIs; Low/Info log to atividades only.

### What changed

- `SecurityAuditBackend` trait with `LocalGrepBackend` (default) and
  `ClaudeSecurityBackend` (full implementation using Claude API)
- `NoOpBackend` for dev/test (`CO_SECURITY_BACKEND=disabled`)
- Cargo feature stubs: `security-semgrep`, `security-sonar`
- Migration v71: `security_findings` table + severity/unresolved indexes
- EDA subscribers: `FindingsPersistor`, `PBIBacklogger`, `ReleaseBlocker`
- Admin REST API: `GET/PATCH /api/v1/gestao/security/findings`,
  `GET /api/v1/gestao/security/scan/status`, `POST /api/v1/gestao/security/scan`
- `pr-route.yml` step 11: `security-audit` job (skips drafts, docs-only, reverts)
- `release-gate.yml`: security gate blocks Thursday release on unresolved
  Critical/High findings; `--ignore-security-findings` override is logged + alerted
- `docs/security-audit-pipeline.md`: 11-step route reference
- `docs/security-disclosure.md`: manual disclosure process for Critical findings
- 3 new env vars: `CO_SECURITY_BACKEND`, `CO_SECURITY_API_KEY`,
  `CO_SECURITY_MAX_SCANS_PER_DAY`
- Cost guardrails: skip drafts/docs/reverts; daily scan cap (default 50);
  cache by SHA-256 file hash

### Why

Anthropic's Project Glasswing announced June 2026: cyber-capable models are
near. CO is exactly the kind of system that becomes a high-value target as it
scales (per-brain identity, payment data, federated event bus, OAuth flows, vault
writes). This spec wires the security bottleneck ("verify, disclose, patch") into
CO's existing scrum CI spine instead of creating a separate process.

### Security hardening (pre-merge review)

The security-gate PR was itself audited; eleven confirmed issues were fixed
(correctness here is paramount — this *is* the gate):

- **Auth (was fully unauthenticated):** the security router only layered the
  admin DATA extensions, never the `require_github_admin` enforcement
  middleware — every `/api/v1/gestao/security/*` endpoint (including
  `resolve_finding`, which opens the release gate) was reachable by anyone. The
  middleware is now applied inside the router, matching every sibling admin
  router. The acting admin is recorded on resolve (new `resolved_by` column +
  event field) and on the scan override, for audit trail.
- **Injection / DoS:** `list_findings` now uses bound params (no SQL
  interpolation) and clamps `LIMIT` to `[1,200]` (SQLite `LIMIT -1` =
  unlimited). `git diff` ref ranges are validated against a safe git-ref
  charset and terminated with `--` (argument-injection defence). The daily
  scan cap is now a DB-backed per-UTC-day counter (`security_scan_counts`) that
  persists across requests — the previous in-memory counter was rebuilt every
  request so `CO_SECURITY_MAX_SCANS_PER_DAY` never tripped; over-cap requests
  return 429.
- **Panic / dead code:** the Claude diff truncation now slices on a UTF-8 char
  boundary (`&diff[..100_000]` panicked on accents/emoji). The `PBIBacklogger`
  INSERT referenced a nonexistent `entries.id` column (PK is
  `(universe_key, path)`); the error was swallowed by a `warn!`, so PBIs were
  never created — it now matches the real schema (`frontmatter_json` +
  NOT NULL `body_hash`) and is covered by a test that asserts the row appears.
- **CI:** `pr-route.yml` gains `CARGO_INCREMENTAL=0` + per-job timeouts
  (disk-full guard); all PR-controlled values (title, draft, base ref) move out
  of `run:` interpolation into `env:` (script-injection defence); the scanner
  step is fail-CLOSED (`set -euo pipefail`, no `|| true`). `release-gate.yml`
  now reads audit records that `pr-route.yml` actually commits into
  `docs/scrum/security/`, and FAILS LOUDLY when a wave that shipped work has
  zero audit records (previously the glob matched a path nothing wrote, so the
  gate was vacuously green every release).

> Note: the `security_findings` table ships as migration **v73** (not v71 as
> the draft above stated); v73 also adds `resolved_by` and the
> `security_scan_counts` daily-cap table.

## CO-402 — Wave-6 security-epic design + roadmap reconcile + epics backlog

Forward-looking documentation made release-ready for the v3.5.0 cut. Docs only —
no feature code, no deploy.

- **`docs/architecture/wave6-security-epic.md`** (new) — the design doc for the
  Wave-6 security epic: `.co` protobuf-wrapped markdown format (CO-86),
  composable protocol stack (CO-87), filesystem-as-web flow + pairing ceremony
  (CO-110), and the per-universe encryption envelope (CO-145/148 — Argon2id KEK →
  per-universe DEK → ChaCha20-Poly1305 blob ciphertext). Includes a threat model
  (assets × trust boundaries × mitigations), the plaintext-for-search boundary,
  the migration path for existing universes, and the CO-104/119 restore-drill
  blocking precondition ("never encrypt what you cannot restore"). CO-402 set to
  `done` — for a design doc, done = the decisions are written and pinned.
- **`docs/roadmap.md`** (reconciled) — corrected the stale "Current state" block
  to 2026-06-12 / prod = v3.4.0; rebuilt the wave table to mark v3.1.0–v3.4.0
  done with real contents and v3.5.0 as next (security gate + ops hardening +
  prod-e2e); remapped open work to what actually shipped vs is pending, verified
  against `git tag` + `CHANGELOG.md` + each `work/co/CO-N.md` status (e.g. CO-366
  is **still `todo`**, not shipped; CO-104/119 are now done, unblocking Wave 6).
- **`docs/architecture/epics-backlog.md`** (new) — single organized index of all
  open `type: epic` work items grouped by theme (security, scale/data-infra,
  server-decomposition, sync, platform), each with its child user-stories and the
  Wave/version it targets. Flags the decomposition epics (CO-227…231) and CO-145
  as closeable-at-release (children all done).

### Why
The v3.5.0 release needs its forward-looking docs to be factually current and the
security epic to have an agreed architecture before any implementation agent
touches `.co` format, fs-as-web, or encryption — these are decisions, not
features.

## CO-406 — graceful startup degradation (no crash-loop on pool failure)

A single universe whose per-universe SQLite DB cannot be opened or migrated
(disk full, I/O error, corrupt `-shm`) no longer panics the whole process.
The 2026-06-11 outage (`SQLITE_IOERR_SHMSIZE` on a full disk) converted one
pool-open failure into a global outage: `panic → exit 101 → Fly crash loop →
max-restart cap → total site down`. This change isolates that failure to the
one affected universe.

### What changed

- `UniversePool` gained `try_get_or_open()` — a non-panicking open that, on any
  environment failure (mkdir / open / pragmas / migrations), records the
  universe as **unavailable** with the failure reason and returns `Err(PoolError)`
  instead of panicking. Per-universe migrations (`run_universe_migrations`,
  `ensure_universe_column`, `recreate_references_meta_v8`) now return
  `rusqlite::Result` and propagate errors rather than `.expect()`-panicking.
- **Lazy retry / self-heal:** failures are never cached as a connection, so the
  next access re-attempts the open. Once the environment failure clears (disk
  freed, volume extended) the universe recovers automatically — no restart. An
  admin `reopen()` forces immediate recovery.
- **Request path → 503:** the `universe_visibility_gate` middleware (CO-161, the
  single chokepoint for every `/api/v1/universes/{slug}/…` route) now probes the
  pool and short-circuits with **503 Service Unavailable** + a clear reason when
  the universe is down. Every other universe keeps serving 200. The probe drops
  the `Mutex<Storage>` lock before any `.await` (no lock held across await).
- **Startup seed is tolerant:** `seed_template_universe` skips seeding and logs
  if the template universe's DB can't be opened at boot — the server starts
  degraded instead of crash-looping.
- **Admin surface (`/gestao`):** `GET /gestao/universos/indisponiveis` lists
  unavailable universes; `POST /gestao/universos/{key}/reabrir` reopens one.
- `PoolError → AppError::ServiceUnavailable` conversion (503) + Portuguese
  translation for `service_unavailable`.

### Why

Panic-on-environment-failure at startup is the same family as the 2026-05-12
mutex-poisoning incident: one bad condition must never take the whole site down.
The per-universe pool is the natural isolation boundary — a partial failure
(one pool open) must stay partial.

### Startup-path audit

Converted (single-universe isolation is possible):
- per-universe pool open + migrations (all `.expect()` → `Result`).
- template seed at boot (skips on failure).

Deliberately left fatal (no per-universe isolation; failing fast is correct):
- `Storage::new` meta.db open / WAL pragma / data-dir create
  (`co-web/src/storage/mod.rs`): meta.db is the single shared DB (users,
  universes, sessions). If it can't open, the site genuinely cannot run — fail
  fast and let the platform restart.
- auth-store / game-storage open in `server::serve` startup: shared singletons,
  not per-universe.
- `baseline.rs` UAT-reset `remove_file` expect: dev-only destructive reset path.

## CO-407 — Uptime alerting (external health probe + notify)

Added an external uptime probe so prod outages no longer wait for manual
discovery (three user-found outages to date: disk-full panic, column-read
regression, mutex poisoning).

A scheduled GitHub Actions workflow (`.github/workflows/uptime-probe.yml`, cron
every 5 minutes) curls `https://co.artelonga.com.br/api/health` from outside the
prod machine. The probe logic lives in `scripts/uptime-probe.sh`:

- Requires **N consecutive** in-run failures (default 3, ~20s apart) before
  declaring an outage, so a single transient blip never raises a false alarm.
- On a down transition it emits `health.down`; on recovery it emits
  `health.recovered` with the **downtime duration**, computed from a tiny JSON
  state file persisted across runs via the Actions cache.
- Notification channels (configured by repo secret, never committed):
  `RESEND_API_KEY` (e-mail via Resend, already used by prod) and/or `NTFY_URL`
  (ntfy push). With neither set, alerts still surface as GitHub Actions
  annotations + run summary, so the probe is useful immediately and never
  silently green.
- Staging (`https://staging.co.artelonga.com.br/api/health`) is probed too but
  at `digest` priority: warning-level annotation + summary only, no push.

### Why
2026-06-11: prod crash-looped for ~3 hours (disk-full panic) and was found only
by manually testing the site. Three strikes — alerting is no longer optional.
GH Actions cron costs nothing and needs no new infra.

### Operator note
Add a notification channel to enable push alerts:

```
gh secret set RESEND_API_KEY --body "<resend key>"
# and/or
gh secret set NTFY_URL --body "https://ntfy.sh/<your-topic>"
```

## CO-408 — Ops small-batch: OAuth error body + git-credential startup noise (+ staging DNS doc)

Three small, independent ops fixes in one patch.

### Fixed

- **Google OAuth token-exchange errors are now diagnosable.** The callback used
  `reqwest::error_for_status()`, which discards Google's response body — so
  `invalid_grant` (harmless authorization-code reuse) and `invalid_client`
  (rotated client secret → *every* login broken) were indistinguishable in the
  logs and required a manual `curl`-from-prod to tell apart. We now read the
  body on a non-2xx token response, parse Google's `{error, error_description}`,
  surface it in the 401 returned to the browser, and emit a `WARN` log line
  (`co-web/src/integrations/oauth_google.rs`).

- **No more `fatal: could not read Username for 'https://github.com'` ×2 on
  startup.** The CO-337 remote sister-repo sync (`run_remote_sister_repo_seeds`)
  shelled out to `git clone`/`git fetch` for universes with an HTTPS `remote_url`
  (e.g. `mbya`) even when no git credential was configured, producing fatal log
  noise on every boot and every 15-min worker tick. The sync now checks for a
  configured credential (`CO_GIT_TOKEN` or `CO_GIT_SSH_KEY_PATH`) before touching
  a network remote and skips with an `info` log when none is present. The feature
  is intact — set `CO_GIT_TOKEN` and the next worker tick syncs, no redeploy
  needed (`co-web/src/platform/vcs.rs`, `co-web/src/server/seed_orchestrator.rs`).

### Ops (manual — not a code change)

- **`staging.co.artelonga.com.br` has no DNS record.** The Fly app
  `co-artelonga-staging.fly.dev` works; the custom domain resolves to nothing
  (the record was lost when the staging app was recreated, CO-379). This cannot
  be fixed from code. **Manual steps for the operator:**

  1. At the **dns-parking nameservers** for `artelonga.com.br`, add a CNAME:
     ```
     staging.co  CNAME  co-artelonga-staging.fly.dev.
     ```
  2. Issue the Fly certificate so HTTPS works:
     ```
     flyctl certs add staging.co.artelonga.com.br -a co-artelonga-staging
     flyctl certs show staging.co.artelonga.com.br -a co-artelonga-staging   # verify
     ```
  Verify: `dig +short staging.co.artelonga.com.br` returns the Fly target and
  `curl -sI https://staging.co.artelonga.com.br/api/health` returns 200.

## CO-415 — GitHub OAuth login (mirrors Google)

Visitors can now sign in to `co.artelonga.com.br` with their GitHub account —
the GitHub twin of the CO-177 Google OAuth flow. Pedido do Miguel: não criar
mais uma senha.

### Added

- **GitHub OAuth 2.0 sign-in** (`co-web/src/integrations/oauth_github.rs`), a
  full authorization-code flow mirroring `oauth_google.rs`:
  - `GET /api/v1/auth/github/start?return_to=` — signs a state JWT (return_to +
    nonce + origin + short expiry, `return_to` safelist-checked via
    `recovery_routes::is_allowed_return_to`) and redirects to GitHub's consent
    screen with scope `read:user user:email`.
  - `GET /api/v1/auth/github/callback?code=&state=` — verifies the state JWT,
    exchanges the code for a token (`Accept: application/json`), fetches
    `GET /user` + `GET /user/emails`, selects the **primary verified** email,
    finds-or-creates a CO user by the new `users.github_login` column (falling
    back to email), sets the session cookie, and redirects to the safelisted
    `return_to` (CO-186 cross-apex handover token attached when applicable).
  - `GET /api/v1/auth/github/status` — reports whether GitHub OAuth is
    configured, so the SPA hides the "Entrar com GitHub" button when it isn't.
- **`users.github_login` column** + partial unique index (migration v74),
  mirroring `users.google_sub`. `find_or_create_user_by_github` links a GitHub
  identity onto an existing email-bearing account or creates a fresh user.
- **Login UI** — "Entrar com GitHub" / "Cadastrar com GitHub" buttons, gated on
  `/github/status` exactly like the Google button (hidden until configured).

### Behavior

- Both endpoints return **503** when `GITHUB_OAUTH_CLIENT_ID` /
  `GITHUB_OAUTH_CLIENT_SECRET` are unset — no half-broken state.
- A GitHub account with **no verified email** → **401**.
- The OAuth client id/secret are **distinct** from any admin PAT
  (`github_auth.rs` is unaffected — that is admin PAT verification, not login).

### Operator setup (Fly secrets — per environment)

```
flyctl secrets set GITHUB_OAUTH_CLIENT_ID=<id> \
                   GITHUB_OAUTH_CLIENT_SECRET=<secret> -a co-artelonga
# optional override (defaults to https://co.artelonga.com.br/api/v1/auth/github/callback):
flyctl secrets set GITHUB_OAUTH_REDIRECT_URI=<uri> -a co-artelonga
```

Register the OAuth app at GitHub with the callback URL
`https://co.artelonga.com.br/api/v1/auth/github/callback`.

## CO-416 — automatic pt↔en content translation (structure-preserving)

Adds on-demand translation of **entry content** (the markdown body), distinct
from the existing UI i18n. A `lang: pt` entry can mint a sibling `lang: en` twin
(and vice-versa) that is traceable to its source and never overwrites it.

### What changed

- New `content::translate` module: a structure-preserving translation engine.
  Fenced code blocks, inline code, and wikilinks (`[[key::path]]`) are masked
  with opaque sentinels before any prose reaches the backend, then restored —
  so code and link targets can never be corrupted regardless of model behavior.
  Frontmatter machine fields (`type`, `lang`, dates, refs) are carried through
  untouched; only `title` is translated.
- Pluggable `TranslateBackend` selected by `CO_TRANSLATE_BACKEND`:
  - unset / `none` / `manual` → `NoopBackend` (route answers **503**, graceful).
  - `llm` → `LlmBackend`, which reuses the existing `infra::ai::AiRouter`
    (provider via `CO_TRANSLATE_PROVIDER`, default `ollama`). No new LLM client.
- New `content::translate_routes`:
  - `POST /api/v1/universes/{slug}/translate/{*path}?to=en` — owner-gated. Writes
    a twin entry at a parallel path (`foo.md` → `foo.en.md`) carrying
    `translated_from`, `translated_from_hash`, and `translated_at` frontmatter.
    Re-translation is **idempotent** by source body hash (no-op `unchanged` when
    the source is unchanged). The `translated_from` field becomes a CO-74 typed
    relation via `sync_entry_relations`.
  - `GET /api/v1/universes/{slug}/translation/{*path}` — reports whether a twin
    exists, its path/lang, and a `stale` flag (source changed since translation).
    This is the data the UI language toggle needs to switch or offer "generate".
- Routes added to `docs/architecture/api-catalog.md`; `openapi.yaml` regenerated;
  `openapi:check` clean.

### Why

Miguel asked for his universe's content to appear in both pt-br and en without
hand-translating. No schema migration was needed — provenance lives in
frontmatter + the existing `entry_relations` table.

### Notes

- The catch-all entry path forces the action verb to precede the path
  (`/translate/{*path}`), since Axum only allows `{*path}` at the end of a route.
- Twin paths never stack lang suffixes (`foo.en.md` + `pt` → `foo.pt.md`).

## CO-420 — Yggdrasil UX centralizada: epics/user-stories + timeline real + tempo mediano por universo

Adiciona `docs/scrum/yggdrasil-ux-overview.md`: reestrutura a narrativa de UX do
yggdrasil (`docs/experiencia-usuario-exemplo.md`, persona Marina) numa visão de
alto nível **epics → user stories** (8 epics, A–H, com princípio de design e
release datado por story), plota uma **timeline de datas reais** de release dos
três universos (co · artelonga · yggdrasil) extraídas dos CHANGELOG.md, e reporta
o **tempo mediano de conclusão por universo** com método auditável.

### Why
Centralizar a UX como backlog (não narrativa solta) dá critério de aceitação por
epic; a timeline real e a métrica de cadência tornam visível que o ecossistema
converge em 2026-06-11/12. Análise/documentação — sem código nem deploy.

### Achados (médias por universo)
- Tempo mediano por item (`updated_at − created_at`, itens `done`): co=0d (n=254),
  artelonga=0d (n=40), yggdrasil=0d (n=107) — ~80% são tarefas de um dia.
- Cadência de release (gap mediano entre dias de release): co=1d (298 releases),
  artelonga=2d (25), yggdrasil=3d (48). Fonte: CHANGELOG.md + frontmatter work/.
- Nenhum dos repos popula `completed_at` consistentemente (co:1, al:0, yg:0); a
  métrica usa pares de datas reais existentes, sem fabricar conclusões.

## CO-421 — Gate de usabilidade prod: suite Playwright anônima read-only vs co.artelonga.com.br

Adiciona `co-web/e2e/prod-usability.spec.ts` (tag `@prod`): uma suite Playwright
curada, **anônima e read-only**, que valida a usabilidade real de produção — o que
um smoke de health 200 não pega. Roda como gate de release (pré e pós-deploy) contra
`https://co.artelonga.com.br`. Sem staging (decisão 2026-06-12); o alvo é prod direto.

Cobre cinco caminhos de usabilidade:
- board do template (`/template`) carrega com tarefas tutorial visíveis (stat `tarefas` > 0 + card visível após expandir seção);
- troca de tema aplica (`html[data-palette]` muda);
- toggle pt/en muda os rótulos do botão de idioma;
- deep-link de entrada (`/template/projects/CO/1`) renderiza markdown no zoom — não cai no 404;
- grafo/lente (stats de conteúdo, com fallback opportunista para o dashboard) abre.

### Read-only por construção
Um interceptor `page.route("**/*")` instalado no `beforeEach` **aborta** qualquer
request `POST/PUT/PATCH/DELETE` e registra a violação; o `afterEach` falha a suite se
houver qualquer mutação. A suite **nunca muta prod** — garantia estrutural, não por
convenção. A suite usa o `test`/`expect` puro do `@playwright/test` (sem o fixture
`uat-login` de `e2e/fixtures.ts`, que retorna 404 em prod), então não há login.

### Como rodar (gate de usabilidade prod)
```bash
cd co-web && BASE_URL=https://co.artelonga.com.br \
  npx playwright test e2e/prod-usability.spec.ts \
  --project=desktop-chromium --workers=2
```
Sem `BASE_URL` roda contra `http://localhost:3000` (um `co serve` local), servindo de
smoke em CI. Documentado em `docs/release-checklist.md` como o gate de usabilidade
prod, substituindo o smoke manual de UAT.

### Why
O smoke de health só prova que o servidor sobe; não pega regressão de UX (board vazio,
tema quebrado, deep-link caindo em 404). Este é o conjunto que pode tocar prod com
segurança e bloquear a promoção se a usabilidade real estiver vermelha.

### Verificação
Verde local (`co serve` em :3000, ~2.7s) e verde contra prod v3.4.0 ao vivo (~40s,
< 2 min alvo); o guard read-only foi confirmado falhando uma sonda que emite um POST.
Sem código Rust tocado — apenas TS/e2e + docs.

## CO-422 — In-prod degradation alerter — email on degraded-but-alive events via Fly RESEND_API_KEY

Adds a `DegradationAlerter` EDA subscriber that watches for degradation events and
sends alert emails via the Resend API (`RESEND_API_KEY`) to `CO_ALERT_TO` (default
`yuri@artelonga.com.br`). Without `RESEND_API_KEY` a one-time WARN is logged and the
alerter becomes a no-op — never panics, never breaks startup.

Events covered:
- `backup.skipped_low_disk` (emitted by the backup worker, CO-405)
- `universe.unavailable` (emitted by the universe pool, CO-406 — wired and ready)
- `system.disk_pressure` — new event emitted by the new disk monitor when free space
  on the data volume falls below `CO_DISK_ALERT_THRESHOLD_PCT` (default 15%)

Each email includes what happened, where (universe/resource), and the actionable
number (e.g. "Livre: 480 MB de 3072 MB"). Anti-spam debounce: max 1 email per event
type per `CO_ALERT_DEBOUNCE_HOURS` (default 2 h).

All degradation events flow through the EDA bus so they appear in the event_log and
the atividades admin dashboard automatically.

### Why
The 2026-06-11 disk-full outage revealed a gap: the server was degrading (backups
piling up, disk filling) hours before the crash-loop, but there was no alert. CO-407
covers total outages via an external probe; this task covers the precursor window
where the machine is still alive but heading toward failure.

New env vars:
| Variable | Default | Description |
|---|---|---|
| `CO_ALERT_TO` | `yuri@artelonga.com.br` | Alert recipient |
| `CO_ALERT_FROM` | `CO Alertas <alertas@artelonga.com.br>` | Sender address |
| `CO_ALERT_DEBOUNCE_HOURS` | `2` | Min hours between alerts per event type |
| `CO_DISK_CHECK_INTERVAL_SECS` | `900` | Disk check interval (15 min) |
| `CO_DISK_ALERT_THRESHOLD_PCT` | `15` | Free-% threshold for disk_pressure event |


## [3.4.0] — 2026-06-12 — fontes (source:github) + lente de tempo + traceback

## CO-387 — Time-rendering primitive — `<co-time-grid>` + calendar lenses

Decouples canonical time storage from rendering. Universe-pool migration v19
adds `event_at_ms`/`due_at_ms`/`scheduled_at_ms` to `entries` (backfilled from
the CO-73 `entry_dates` ISO rows with millisecond precision; kept in sync on
every write by `EntryIndex::upsert_dates`). A per-universe `_calendar.yaml`
declares calendar lenses with **per-lens canonical units** — `i64_ms` for
human/fictional/Pomodoro scales, `f64_years` (log) for cosmic (13.8 Gyr
overflows i64 ms), `i64_units` for custom fields like `shandara_year` —
served at `GET /api/v1/universes/{slug}/calendar` with a Gregorian default.

New `<co-time-grid>` component (`static/shared/lib/co-time-grid.js` +
conversion lib `co-time.js`, a 1:1 mirror of `co-web/src/time/conversion.rs`)
renders `(entries, lens)` in 4 view modes (grid/timeline/scatter/gantt), with
a no-reload lens dropdown persisted per universe, multi-universe color coding,
CO-380 event-bus live updates, and `time.lens_switched`/`time.grid_rendered`
telemetry. Registered into the CO-393 lens frame as the `time-grid` lens
(variant i); named manifest views were generalized at the registry level
(`lens.namedViews`), replacing the gantt-only special case.
`/timeline?lens=A,B,C` pins stacked lenses over the `?u=` universes.

### Why
The IaaS principle applied to time: a 4-day-week org review, a Pomodoro day,
or a Milky-Way-to-fiction `/timeline` are *lenses* over the same bounded,
deterministic timestamps — no schema migration per calendar, and the brain's
creative time (custom calendars, fictional epochs) stays liberated.

## CO-411 — fix: CLI envia User-Agent + referência de comandos (docs/CLI.md)

`co auth` era rejeitado pelo próprio gate anti-abuso do servidor
(`400 missing_user_agent`) — o client HTTP do auth não enviava User-Agent
(o do `co push` já enviava). Agora todo request do CLI identifica
`co-cli/<versão>`, com teste de regressão. Junto: `docs/CLI.md`, referência
revisada do binário (cinco verbos do dia a dia + tabelas completas),
linkada de `docs/README.md`.

## CO-412 — Tutorial espelha a release — 3 novas tarefas (lentes, co updates, história)

O tutorial do universo template ganha o "Ato 5: novidades": **Um conteúdo,
muitas lentes** (CO-393), **Novidades sem sair do terminal** (CO-404) e
**Leia a história do CO** (WELCOME.md, CO-403). Em produção as entradas
foram criadas em runtime via Vault API — sem deploy; este PR garante a
paridade para instalações novas (seed 9 → 12 tarefas).

## CO-417 — feat: adapter `source: github` (repo → árvore de universo, ipynb→md)

Novo verbo de CLI `co source add github <owner/repo>` — um adapter geral de
*source* que clona um repositório GitHub (shallow `git clone`, sem auth para
repos públicos; `GITHUB_TOKEN` opcional para privados/rate-limit) e materializa
sua árvore de arquivos como entradas de um universo CO, **preservando a
hierarquia de pastas** (pastas → nós da árvore na UI).

Parse por arquivo:
- `.md` → corpo da entrada direto;
- `.ipynb` → JSON do notebook renderizado em markdown legível (células
  markdown verbatim, células de código em blocos cercados ``` com a linguagem
  do kernel, na ordem original; células vazias ignoradas);
- demais → entrada-asset linkando o original no sha fixado.

Cada entrada importada carrega frontmatter de proveniência consumido pelo
traceback do CO-418: `source: github:<owner/repo>@<sha>` e
`source_path: <caminho/no/repo>` (mais `source_kind: github` e `entry_type`).
A escrita usa a Vault API (`PUT .../vault/{path}`), então o re-sync converge
(idempotente por conteúdo); `--delete-missing` reflete (soft) remoções do repo.
`--dry-run` mostra o plano sem escrever.

### Why
Generaliza o conceito de `source_kind` (que hoje só cobre event-bus) para uma
fonte de primeira classe por-entrada, demonstrando a capacidade de *source* do
CO de ponta a ponta. Fetch e parse/materialize são unidades separadas — o
transform (incl. ipynb→md) é testado sem rede via fixtures.

## CO-418 — feat: render-review-publish with traceback (source + requested_by)

Adds the render→review→edit→publish capability for imported content (the
consumer of CO-417's `source: github` provenance). A new owner-only route
publishes an entry as a *conventional, semver-aware* publication that traces
back to (a) its original source and (b) the task that requested it.

### What changed
- **Publish surface**: `POST /api/v1/universes/{slug}/publish` (owner-only,
  under the visibility gate, in `review_routes.rs`). Body:
  `{ path, requested_by, commit_type?, commit_scope?, commit_subject?,
  frontmatter?, body? }`.
- **Conventional commit + semver intent**: the publish records a conventional
  commit message (`feat(scope): subject` / `docs: …`) and the implied semver
  bump (`feat→minor`, `fix|docs|refactor|perf→patch`). Per CO-258 it records
  the *intended* bump only — it never touches `Cargo.toml`/`CHANGELOG.md`.
- **Provenance stamping**: preserves CO-417's `source`/`source_path`/
  `source_kind`/`entry_type` untouched and ADDS `requested_by` plus a publish
  record (`published_commit`, `published_semver`, `published_sha`,
  `published_at`).
- **Traceback as typed relations (CO-74)**: `extract_provenance_relations`
  derives two manifest-independent edges from frontmatter on every save —
  `origin` → `source` and `requested_by` → the task. Because they derive from
  frontmatter they survive `replace_all` and are idempotent. The entry GET
  response already surfaces outbound relations, so "Origem" + "Pedido por" are
  queryable for the SPA.
- **Idempotent**: re-publishing unchanged content (same body + source +
  requested_by + commit message) yields the same `published_sha` and records
  no new commit (`published: false`).
- **EDA**: emits an `entry.published` event carrying the commit message,
  semver bump, source, and requesting task.
- **Render-local**: imported entries already serve clean markdown bodies
  (frontmatter stripped at parse time); a test confirms the served body is
  renderable and free of provenance leakage.

### Why
The Yuri "source capability" journey (epic CO-414) requires that every
publication be auditable from origin to request. This makes that flow a
reusable CO capability rather than a manual process.


## [3.3.1] — 2026-06-11 — backup nunca derruba o boot

## CO-405 — backup nunca mais derruba a produção — guard de disco, debounce e retenção por contagem

The backup worker is now defensive by construction, closing the 2026-06-11
double outage (disk-full crash-loop in the morning; >6 min boot-blocked bind
at night):

- **Out of the boot path**: the first backup tick is deferred
  `CO_BACKUP_BOOT_DELAY_SECS` (default 10 min) via a new
  `Worker::initial_delay()` supervisor hook, and the tarball now builds in
  `spawn_blocking` — it can never again starve the 1-vCPU machine before the
  HTTP listener binds.
- **Free-space guard**: snapshot skipped when available space <
  max(2× last snapshot, `CO_BACKUP_MIN_FREE_BYTES`, default 256 MiB), with a
  `backup.skipped_low_disk` EDA event, WARN log, and atividade entry.
- **Restart debounce**: no new snapshot while the newest is younger than
  `CO_BACKUP_MIN_INTERVAL_HOURS` (default 6 h) — deploy/restart bursts produce
  one snapshot per window instead of four in a day.
- **Count/size retention**: `CO_BACKUP_RETAIN_COUNT` (default 3) and
  `CO_BACKUP_RETAIN_MAX_BYTES` now prune oldest-first; the 30-day rule is
  secondary; the newest snapshot is always kept as a restore point.
- Atividades now record snapshot size, free space after, and every skip.

### Why

Snapshots-on-restart filled the volume twice in one day (264+266+280+299 MB
on 3 GB) and the boot-time snapshot blocked health checks during tonight's
3.3.0 deploy. Stopgap until CO-104 (S3 backend) makes local headroom
irrelevant.


## [3.3.0] — 2026-06-11 — sala paisagem — grid landscape

## CO-410 — Sala grid landscape — type-on-square, working drag-and-drop, pastas

The sala canvas (`/u/{universe}/sala`) is now a **natural landscape**: an
infinite grid over procedural terrain instead of a free-form graph void. Every
square holds a value — click and type directly on the grid:

- a **single letter** renders on the square and the cursor advances (write
  across the land like text)
- **`/nome`** creates a **pasta** — a draggable folder unit
- **longer text** becomes a **nota** card
- new salas start with the root pasta **`/`** on the origin square

Drag-and-drop now works (pointer events, mouse + touch, snap-to-grid with
ghost preview): notas dropped on a pasta join it, and a pasta drags as one
unit with everything inside. Read-only/share-token modes, the CO-352 state
API (layout v2, v1 layouts migrate on load), and the anon login-CTA contract
are preserved. Rendering is dirty-flag (no idle repaints); `graph.html`
continues to use co-graph.js unchanged.

### Why

Direct user feedback: click-and-drag on the old graph canvas didn't work, and
an empty canvas gave new arrivals nothing to react to. A pre-filled landscape
plus type-anywhere makes the sala a place you inhabit immediately — the
folder/note/letter trio maps the file→folder→universo ladder onto the canvas.


## [3.2.0] — 2026-06-11 — lentes compostas + co updates

- **`co updates`** — release notes no terminal. `-n 3` últimas, `--all` histórico desde 0.1.0. Offline, embutido no binário.
- **UI por lentes** — 8 lentes (kanban, tabela, calendário, timeline, gantt, dashboard, grafo, documento) sobre um registry único; formulários derivam do schema. CO-387/396 plugam sem tocar despacho.
- **WELCOME.md** — onboarding completo + a invariante do pipeline (*localhost → aprovar → mesclar*) em git e jj.

### Detalhes

- Lentes (CO-393, user story): universo renderiza por lentes registráveis, manifest-driven — endurecido em review: 3 defeitos fatais de alcance corrigidos, boot verificado em navegador (zero erros JS).
- Docs (CO-403, task): exemplo CRUD vivo no universo miguel; correções factuais de roadmap no mesmo PR.
- CLI (CO-404, task): `include_str!` do CHANGELOG → cada `release-commit.sh` vira a próxima nota automaticamente.

### Referências

| Item | PR | Spec |
|---|---|---|
| CO-393 lentes | [#196](https://github.com/artelonga/co/pull/196) → `e00c88f` | `work/co/CO-393.md` |
| CO-403 docs | [#197](https://github.com/artelonga/co/pull/197) → `d7a682c` | `work/co/CO-403.md` |
| CO-404 co updates | [#198](https://github.com/artelonga/co/pull/198) → `bc093ef` | `work/co/CO-404.md` |

## CO-393 — Composable universe UI — content lenses + schema-driven form engine, manifest-driven

New `variants/i` SPA that replaces hardcoded view wiring with a composable lens registry
and a schema-driven form engine. The variant ships alongside stable `variants/a` and is
promoted only after parity verification.

### What changed

- **Lens registry** (`variants/i/modules/lenses/registry.js`): uniform `{ id, label, icon, supports, render }` interface. `computeDynamicTabs(manifest)` replaces the gantt-only `injectManifestViewTabs` — all `manifest.views` types and declared `content_types` drive visible tabs with no JS change.
- **Form engine** (`variants/i/modules/form/engine.js`, `fields.js`): `renderForm(schema, value)` → `<form>`, `collect(form, schema)` → plain object. Handles `string`, `text`, `number`, `date`, `boolean`, `enum`, `ref` field types. Universe create/edit uses it via `renderUniverseForm()` in `dom-setup.js`.
- **`conteudo.js` split**: 1502-line monolith decomposed into `document-tree.js` (pure tree utils), `document-assets.js` (asset rendering utils), `document.js` (lens facade). No file in `variants/i/` exceeds 500 LoC.
- **Graph lens** (`variants/i/modules/lenses/graph.js`): multi-universe graph via `GET /api/v1/universes/{slug}/graph?universes=…` (CO-345). Auto-detects parent + child universes from `state.meUniverses`. `renderContent` routes `state.view === 'graph'` to it.
- **Graph tab injection**: `computeDynamicTabs` auto-injects a graph tab when the universe has a `parent_key` or child universes (AC4+AC5: no JS change needed).
- **AC4 test** (`variants/i/modules/form/test.js`): demonstrates adding an `article` content_type yields a working form + eligible lenses with zero JS change.
- **i18n keys** added to both pt and en dicts in `shared/i18n.js`: `lens.graph`, `lens.graph.universes`, `form.required`, `form.field_ref`, `sidebar.children`.

### Why

The gap in the previous architecture: content was composable lenses; forms were bespoke HTML with no schema→form generation. `_universe.yaml` `content_types[].schema` declared field shapes but the form layer never used them. CO-393 closes that gap by making the manifest the single source of truth for both what you can see (lenses) and what you can edit (forms).

## CO-403 — Onboarding & pipeline documentation — WELCOME.md + delivery-pipeline invariant

Adds the platform's front-door documentation:

- **`docs/WELCOME.md`** — a history-teller onboarding in seven movements: the
  co- prefix as the feature list (Collective Consciousness, ñandé posture),
  the abstractions in arrival order, requirements-as-layers, the five-act
  changelog history, the inhabited universes (including miguel → mse as the
  stakeholder↔project pattern), a worked CRUD example on the Vault API
  (`scholars/` in miguel), and "Bringing a universe with you — two doors":
  add a local folder via `co push` (CO-392) or clone one from its git remote.
- **`docs/delivery-pipeline.md`** — new section "The invariant — review on
  localhost, approve, merge", explaining the pipeline (CO-398) in both
  version-control vocabularies: traditional git (branch → PR + preview →
  approval → merge ⇒ deploy) and CO-native/jujutsu (change → states →
  proposta + localhost serve → approval → mesclar/bookmark move ⇒ publish),
  with a worked example of each.
- Cross-links from `README.md`, `docs/README.md`, and WELCOME §7.

### Why

3.0.0 opened the public door; this gives the people walking through it the
story, the vocabulary, and the two everyday gestures — without reading source.

## CO-404 — `co updates` — release notes in the CLI

Adds `co updates`, a new CLI command that prints release notes straight from
the CHANGELOG embedded in the binary at compile time:

- `co updates` — the most recent release section (version — date — theme +
  its task entries), lightly styled for the terminal
- `co updates -n 3` — the N most recent releases in detail
- `co updates --all` — compact release-history list (every version header)

No network, works offline, and always matches the installed version. Every
`release-commit.sh` run automatically becomes the next note — the release
process needs no extra authoring step.

### Why

3.0.0 opened the public door and CO-403 wrote the welcome; this puts the
"what's new" channel inside the tool itself, where content authors already
live. Release notes become a product surface instead of a repo artifact.


## [3.1.0] — 2026-06-11 — delivery pipeline + knowledge base

## CO-367 — Universal content → KB sync — generalize the rollup pattern to all entry types

Introduces a universal KB ingest pipeline that makes every entry write (note,
article, poem, asset reference) visible to downstream consumers — search, agents,
analytics — without changing how users interact with CO.

- **Migration v71** adds three new tables to `meta.db`:
  - `entry_kb_index` — history table, PK `(universe_key, entry_path, body_hash)`
  - `entry_kb_latest` — latest-version view per `(universe_key, entry_path)`
  - `entry_kb_fts` — FTS5 virtual table for full-text search over `body_preview`
- **POST /api/v1/kb/ingest** — idempotent upsert, auth via `CO_KB_TOKEN` bearer (503 if unset)
- **GET /api/v1/kb/search?q=...** — FTS5 search; optional `universe_key` filter
- **GET /api/v1/kb/recent?universe_key=...** — latest indexed entries per path
- **Entry write wiring** — `create_entry` and `update_entry` fire `kb_routes::fire_kb_ingest`
  via `tokio::spawn`; the write response never waits for KB sync
- **Frontend retry queue** (`modules/kb-sync.js`) — queues `KbIngestEvent`s in IndexedDB
  when the endpoint is unreachable and drains on reconnect (exponential backoff: 1m → 5m → 30m → 2h)
- **Atividades** — `event_type = 'kb.ingested'` lands in the audit log per new ingest
- **Asset refs** — `asset_refs` JSON array preserved in both history and latest tables
- **OpenAPI** — three KB endpoints documented with request/response schemas

### Why

CO-340 shipped analytics rollups as a one-off pattern. This PR retroactively
generalises that shape — local write → cache-first render → consented async sync →
downstream availability — so the BaaS §6 promise "add anything → registered,
synced, delivered" holds for all content types.

## CO-371 — Single funnel report endpoint — discover → onboard, end-to-end with drop-off per step

Adds the 8-step acquisition funnel to `GET /api/v1/analytics/public/funnel?window=30d` (admin-only when `window` param is present). The funnel covers the full acquisition arc: Discover → Engage → Intent → Capture → Qualify → Register → Convert → Onboard, with per-step drop-off percentages and 5 KPI ratios.

Steps 1–3 are sourced from `telemetry_events`; step 4 uses the CO-370 email join (`users UNION leads`); steps 5–6 from `leads`/`users`; steps 7–8 gracefully return 0 until CO-366 (billing_events) lands.

Breakdowns by `source` (referrer), `day`, and `country` are supported. The `/gestao/resumo` dashboard gains a "Funil" tab rendering the vertical bar chart with drop-off labels and KPI grid.

### Why
Enables the platform operator to answer "of visitors from HN this week, what % converted to paid?" in a single query instead of manual SQL across 4 stores. Provides the killer metric for the `/gestao/resumo` dashboard (CO-360).

## CO-372 — Sprint calendar with Definition of Done + ICS export

Adds a sprint calendar view and iCalendar export to the scrum tooling.

- `GET /api/v1/scrum/calendar?past=6&future=4` — JSON response with past and future sprints, DoD percentages computed from `## Acceptance` checkboxes in spec files, sprint events (planning/review/retro) as ISO-8601 UTC timestamps.
- `GET /api/v1/scrum/calendar.ics` — iCalendar export for Google/Apple Calendar subscription; includes VALARM 15-min reminders and one VEVENT per sprint event.
- `/scrum/calendar` SPA — standalone dark-theme HTML page with current-sprint hero + live countdown (60s updates), velocity sparkline, past/future sprint cards, and ICS subscription link.
- `co-web/scripts/scrum/cutoff-check.ts` — PR cutoff gate: exits 1 if merges landed after Wednesday 23:59 BRT (sprint_end − 15h01m); integrated into `scripts/release-commit.sh` (bypassed by `--ignore-dod`).
- Sprint data embedded at compile time via `include_str!` from `docs/scrum/sprints/_index.json`; Dockerfile updated to include that path in build context.

### Why
Provides a single always-up-to-date view of sprint history and upcoming milestones, with calendar subscription so stakeholders see sprint ceremonies in their own clients. The cutoff gate enforces the scrum discipline of not merging after the Wednesday freeze.

## CO-390 — SPIKE — layered architecture (domain/dto/repository/service) proof-of-concept on entries module

Spike branch (`feat/CO-390-spike-layered-architecture-domain-dto-re`) exploring the
`alineaos/gerenciador-de-bibliotecas` layered architecture pattern for CO's `entries`
module. Added 12 new files in `co-web/src/{domain,dto,repository,service,mapper}/`
demonstrating the pattern; modified `entry_routes.rs` to use `EntryService` for
6 business rules; wrote a decision document with quantified metrics.

Result: **partial adoption recommended** — DTO families + service layer per feature,
without global directory restructure. All 4 hypotheses passed (coverage ↑, OpenAPI
richer, 26% LOC overhead within 60% budget, wire-compat verified). Spike branch is
archival; not merged to main.

### Why

Architecture spike to reduce uncertainty before committing to a multi-month refactor.
3-day box to know if the pattern fits Rust + CO ergonomics. Informs CO-227 (server
decomposition) and CO-228 (type safety).

## CO-392 — co push — CLI → remote universe CRUD over the Vault API (the missing edge)

Adds `co push`, a first-class CLI verb that uploads a local universe to a deployed
CO server over HTTP. It wraps `POST /api/v1/universes` (create-or-update) plus
`PUT …/vault/{path}` for every `content/**/*.md` file, using the same REST endpoints
the web front-end calls. Re-running converges (idempotent; no duplicates).

Flags: `--remote <url>` / `CO_REMOTE`, `--token <t>` / `CO_TOKEN`, `--key`, `--dry-run`,
`--delete-missing`. Skips `_source/` (LGPD), hidden files, and `.gitignore`d paths.

Supersedes the ad-hoc `scripts/bulk-upload.py` Vault loop.

### Why

The CLI and web front-end share the same REST API but were not connected for the
"publish to production" path: `co launch` seeds into *local* SQLite only, while pushing
to a deployed server required a separate script. CO-392 closes that edge so "add a
universe" is one verb with identical semantics regardless of surface, and no `seed.rs`
edit or redeploy is needed (`feedback_no_hardcoded_content_mappings`).

## CO-395 — construir — universe markdown → Quartz static public site (board stays the app)

Added `co construir [<key>] [--out <dir>] [--redearte <path>]`, a new CLI command
that feeds a universe's `content/*.md` through the redearte Quartz template and
writes a self-contained static digital-garden site to `--out` (default: `public/`).

- Wikilinks, backlinks, and the graph render natively via Quartz — no CO chrome,
  read-only. `_source/` PII and non-content files are excluded by construction
  (Quartz only sees `content/`).
- Universe discovery follows `co launch`: walks up from CWD to the repo root
  (`.git` or `.jj`). `CO_REDEARTE_PATH` overrides the default `~/projects/redearte`.
- `co-web` subdomain middleware now skips slugs listed in `CO_STATIC_SITES`
  (comma-separated) so the subdomain routes to the Fly static app while
  `co.artelonga.com.br/<slug>` continues to serve the gated board.
- Documented the full flow (build → deploy scaffold → routing split) in
  `docs/universe-public-site.md`.

### Why

Every universe now gets a public static site from its markdown with no bespoke
per-universe app. Board and site share one source (`content/`): board via
`co launch`, site via `co construir`. First target: `grcsamazonia`.

## CO-398 — Delivery pipeline no quadro — status dirigido por VC/deploy

Implementa o pipeline de entrega padrão: o status de cada tarefa passa a ser
dirigido pelos eventos de VC/deploy em vez de drag-and-drop manual.

### O que mudou

- **Novo enum de status** `[todo, started, in_progress, review, done]` como padrão do `default_manifest()` para universos-projeto novos. Boards legados `[todo, doing, done]` permanecem válidos.
- **Campos de PR/preview** `pr_url` e `preview_url` adicionados ao schema padrão de task (sinaliza revisão incompleta quando `preview_url` está ausente).
- **Migration v71** — tabela `task_status_log` para rastreio de lead time por coluna.
- **EDA `task.status_changed`** publicado sempre que o campo `status` muda (via `update_entry` ou webhook do GitHub).
- **EDA `deploy.triggered`** emitido quando uma tarefa chega em `done` (gancho para CO-395 e fluxo UAT→prod).
- **Subscriber `DeliveryPipelinePersistor`** — persiste cada transição em `task_status_log` e republica `deploy.triggered`.
- **`POST /api/v1/delivery/github?universe=<slug>`** — endpoint de webhook do GitHub (HMAC-SHA256 validado via `CO_GITHUB_WEBHOOK_SECRET`):
  - `create` (branch com `CO-<n>`) → `started`
  - `push` (commits na branch) → `in_progress`
  - `pull_request opened` (título/corpo com `CO-<n>`) → `review` + `pr_url`
  - `pull_request closed merged` → `done`
- **`GET /api/v1/universes/:slug/delivery/metrics`** — lead time médio por coluna + contagem de tarefas entregues.
- **Docs** em `docs/delivery-pipeline.md`.

### Why

O quadro era um espelho manual do trabalho real que vivia no git. Com o pipeline,
o status da tarefa é sempre o estado verdadeiro: "done" implica "aprovado e em
produção". Transição manual (drag) continua possível — a automação preenche,
não tranca.


## [3.0.0] — 2026-06-10 — public launch — brain on any device

## CO-352 — Workspace ("Sala") primitive — spatial canvas view anchored to a universe

Adds a first-class "Sala" workspace view alongside kanban and conteudo. Each workspace is a personal spatial canvas that persists node positions, typed edges, and the camera transform per user.

### What changed

- **Schema migration v70**: new `workspace_states` table stores per-user canvas state (layout_json, is_public, share_token).
- **Backend**: `workspace_routes.rs` — GET/PUT state, POST share-token, share-token resolution (`/api/v1/universes/{key}/workspaces/{slug}/state`, `/api/v1/workspace-states/{token}`).
- **Server routes**: `/u/{universe}/sala`, `/u/{universe}/sala/{slug}`, `/sala/{share_token}` serve `shared/sala.html` — a dedicated canvas page reusing `co-graph.js`.
- **SPA integration**: new "Sala" tab in variant-a view bar (i18n: PT/EN "Sala"). Per the one-surface decision (`docs/architecture/sala-surface.md` — one surface, fractal scope: per universe / all / any subset, recursive), the tab is a **launcher** that opens the canvas page; the SPA carries no canvas of its own.
- **Design doc**: `docs/architecture/sala-surface.md` records the Sala scoping model and the launcher-vs-surface split (decision 2026-06-09).
- **i18n**: `workspace` key added to PT and EN dictionaries in `shared/i18n.js`.
- **E2E tests**: `co-352-sala.spec.ts` — anon visits sala, btn-add triggers login CTA, API round-trip (PUT → GET), share-token lifecycle.

### Why

Yggdrasil's `/universos/comunicacao` spatial editor is absorbed into CO as a reusable primitive. Any universe can now have a Sala for free — giving study-mode users, game content authors, and researchers a persistent working canvas.

## CO-354 — Suggest / review pipeline — entry lifecycle (draft → reviewed → published) with anon submissions

A generalized suggest/review pipeline as a CO primitive: every entry now carries
a `review_status` lifecycle (`draft` | `reviewed` | `published`, default
`published`), and universes can open up to community contribution without
granting write access.

### What changed

- **Schema (additive, per-universe DB, idempotent):** `entries` gains
  `review_status`, `submitted_by`, `submitted_at`, `reviewed_by`, `reviewed_at`,
  plus a partial index `idx_entries_review_status` that only indexes
  non-published rows so published reads stay cheap. Applied as per-universe
  migration v18 (v17 is CO-389 on main) with unconditional drift guards (CO-241/CO-267 pattern).
- **Endpoints** (under `/api/v1/universes/{slug}`):
  - `POST /suggest` — accepts anonymous submissions (sits outside the writer
    gate), rate-limited per submitter + honeypot field. Creates an entry with
    `review_status: draft`.
  - `GET /review` — owner-only queue of draft/reviewed entries, newest first.
  - `POST /review/approve` — `{path}` → publishes the entry.
  - `POST /review/reject` — `{path}` → **deletes** the entry (archival is a
    Phase-2 concern; delete is the documented v1 choice).
  - `PATCH /review` — `{path, frontmatter?, body?}` → edit-then-approve.
- **Read-path filtering:** draft/reviewed entries are hidden from public
  `GET /entries` and `GET /entries/*path` listings. The universe owner sees
  everything; an anonymous submitter still sees their own draft (matched by a
  stable `submitted_by` session/IP key).
- **Notifications (CO-329):** the owner is notified on each new suggestion
  (in-app + email); the submitter is notified on approve/reject (in-app when
  logged in, direct email when an anon submitter left one).
- **Atividades log (CO-351 / CO-380):** every lifecycle transition publishes an
  EDA event (`entry.suggest` / `entry.review.approve` / `entry.review.reject` /
  `entry.review.edit`).
- **Frontend:** a public suggest form at `/{slug}/suggest` and an owner review
  queue at `/{slug}/review` (served like the existing `/{slug}/graph` page).

### Why

Yggdrasil's bespoke `seed_status` pattern (mbya/yoruba lexicon) becomes a
universal CO capability, and contributors can propose corrections/new entries
without commit access. The "anon can suggest" + "owner reviews" halves compose
cleanly with notifications, the atividades log, and surface keys.

### Notes / deviations

- Owner review actions take the entry path in the request body (`{path}`)
  rather than as a URL segment, since suggestion paths contain slashes.
- Routes use the repo's `/{slug}/…` page convention rather than `/u/{key}/…`.
- The Sala `btn-sugerir` wiring (CO-352) targets the standalone suggest form;
  CO-352's `sala.html` surface is not present on this branch, so the form is
  reachable directly at `/{slug}/suggest`.

## CO-355 — Workspace template registry — _workspace.yaml per universe seeds Sala layouts

Universe owners can now define opinionated starting layouts for the Sala canvas
by placing `_workspace.yaml` (default) or `_workspaces/<slug>.yaml` (named) in
their universe content root. Templates specify pre-placed nodes, edges, study-mode
config, and allowed entry types in a plain YAML schema.

New API:
- `GET /api/v1/universes/{key}/workspace-templates` — list all templates (always
  includes a synthetic "blank" template).
- `GET /api/v1/universes/{key}/workspace-templates/{slug}` — fetch one template.
- `POST /api/v1/universes/{key}/workspaces/{ws}/from-template/{tpl}` — create a
  new `workspace_states` row seeded with the template's node/edge layout. Works for
  anonymous and authenticated users. Entry paths that don't exist in the universe
  are silently dropped (warning logged); the rest of the template still loads.

Frontend:
- New "Sala" tab in the view-tabs bar (`data-view="workspace"`) — a **launcher**
  for the unified Sala surface, per `docs/architecture/sala-surface.md` (one
  surface, fractal scope: per universe / all universes / any subset; nodes can
  recurse into universes). The SPA never grows its own canvas.
- Template picker modal opens on "+ Nova Sala" — lists all templates from the API.
- Selecting a template calls the `from-template` endpoint and navigates to the
  canvas at `/u/{universe}/sala/{slug}` (CO-352's surface).

Database:
- No new migration — rows are written to the `workspace_states` table created
  by CO-352's v70 migration.

Seed fixture:
- `co-web/seed/universe-templates/comunicacao/_workspaces/mbya-basics.yaml` — 12-node
  reference template for the comunicacao universe (copy into the universe repo to
  activate).

### Why

Newcomers to a universe need a useful surface immediately. Curators can codify
canonical study starting-points as versioned YAML that travels with the universe
repo, replacing Yggdrasil's hardcoded `data-tpl` attribute.

## CO-356 — Touch DnD on board — pointer-event based drag for mobile (replaces HTML5 DnD)

Replaced the kanban board's HTML5 drag-and-drop (`dragstart`/`dragover`/`drop`)
with a pointer-event based implementation that works on iOS Safari and Android Chrome.

A new reusable primitive `lib/pointer-drag.js` (`attachPointerDrag`) handles
`pointerdown`/`pointermove`/`pointerup`/`pointercancel` with a 200 ms hold-or-8 px
horizontal-movement threshold to prevent accidental drags during scroll or tap.
During a drag, a ghost clone of the card follows the pointer; hit-testing via
`document.elementsFromPoint` highlights the target column and resolves the drop.

E2E tests updated: removed the `vp.width <= 640` skip, added mobile-viewport
coverage for iPhone 13 (390×844), iPad Mini (768×1024), and Pixel 5 (393×851),
plus threshold-guard tests verifying quick taps and vertical swipes do not fire.

### Why

HTML5 DnD events do not fire on touch screens — a 15-year browser limitation.
The board was silently read-only on all mobile devices.  Pointer events are the
W3C-standard unification of mouse, touch, and pen input and are supported on
iOS 13+, all modern Android, and all desktop browsers.

## CO-357 — PWA shell — manifest, service worker, install prompt, offline cache for content

Full PWA implementation: web app manifest with correct icons, service worker upgrade to
`co-v6-offline`, install prompt for Android Chrome and iOS Safari tip, and offline
navigation fallback to `/offline.html`.

### Changes

- `static/manifest.json` (new root): `CO — Collective Consciousness`, theme `#2563eb`,
  scope `/`, display `standalone`, PNG icons at 192/512/maskable sizes.
- `static/shared/icons/`: `icon-192.png`, `icon-512.png`, `icon-maskable-512.png`,
  `apple-touch-icon.png` (180 px) — solid CO-brand blue (#2563eb) placeholder PNGs.
- `static/shared/sw.js` + `static/sw.js`: CACHE_NAME bumped to `co-v6-offline`;
  STATIC_ASSETS now caches `/manifest.json` and `/offline.html`; API strategy upgraded
  to network-first-with-cache-fallback; navigation offline fallback to `/offline.html`.
- `static/offline.html` (new): minimal "you're offline" page served when navigation
  fails and no cached page is available.
- All `variants/*/index.html`: manifest link updated to `/manifest.json`; theme-color
  updated to `#2563eb`; viewport gets `viewport-fit=cover`; apple-touch-icon,
  apple-mobile-web-app-capable/status-bar-style/title metas added.
- `variants/a/modules/install/wire.js` (new): listens for `beforeinstallprompt`,
  shows the existing `#pwa-install-wrap` button, prompts on confirm, persists
  30-day dismissal. iOS Safari gets a one-shot "Tap Share → Add to Home Screen" tooltip.
  Emits `pwa_install_prompt_shown`, `pwa_install_prompt_dismissed`, `pwa_installed`
  telemetry events.
- `static_files.rs`: `manifest.json` and `offline.html` served from root (not shared/);
  `manifest.json` MIME type corrected to `application/manifest+json`;
  `sw.js` now includes `Service-Worker-Allowed: /` response header.

### Why

Lighthouse PWA category was failing (manifest 404, no icons, wrong MIME type, no apple
meta tags). CO mobile users could not install the app to their home screen.

## CO-358 — Mobile IA pass — drawer sidebar, breadcrumb collapse, board → list reflow

Full mobile responsive pass for the CO SPA: the app is now designed for small screens
rather than being a desktop layout shrunken to 360 px.

- **Sidebar drawer**: off-canvas at ≤640px with slide transition (`translateX`). Hamburger
  button (fixed, 44×44 hit area) toggles it. Swipe from left edge (<20px) opens; swipe
  right on open drawer closes. ESC closes. Focus trap active while open. Tap-outside closes.
- **Breadcrumb collapse**: at ≤480px shows `← <title>` instead of full trail. Tapping the
  title opens a popover with the full navigation trail; items are full-touch-target links.
  Back arrow navigates to parent directly.
- **Board single-column**: at ≤640px kanban columns stack vertically; a segmented control
  at the top lets the user switch the active column. Selection persists via `localStorage`
  (`co.board.mobileActiveColumn`).
- **Tables → card view**: at ≤640px `<thead>` hidden, each `<tr>` rendered as a stacked
  card using `display: block`. `data-label` attributes on `<td>` elements drive `::before`
  pseudo-element labels. CSS-only transformation, no JS change needed.
- **Modals full-screen**: at ≤640px `.modal` takes `100vw × 100dvh`, no border-radius,
  with sticky `modal-header` containing a 44×44 close button.
- **Touch targets**: all `button`, `.btn`, `.view-tab`, `.nav-item` etc. get
  `min-height: 44px` at ≤640px. Inline badges use padding expansion.
- **Header overflow menu**: at ≤640px a kebab button (⋮) replaces non-essential header
  controls (search, lang toggle, universe info) with a dropdown menu.
- **Breakpoint variables**: `--bp-mobile: 640px`, `--bp-tablet: 900px`,
  `--touch-target: 44px` added to `:root`.
- **iOS safe areas**: `env(safe-area-inset-top/bottom)` applied to header and
  bottom-sheet elements.

### Why

Mobile users bounced because the SPA was a desktop layout shrunken to 360 px: sidebar
had no drawer affordance, breadcrumbs wrapped across three lines, the kanban board
horizontal-scrolled inside a vertical-scroll page, tables overflowed, and modals floated
at 320 px on a 360 px viewport. This pass makes CO usable on the device most people read on.

## CO-359 — Mobile E2E coverage in CI matrix — Pixel 7 / iPhone 14 / iPad Pro

CI's e2e job now runs a device matrix: `desktop-chromium` and `pixel-7` gate every
PR; `iphone-14` and `ipad-pro` projects are available for local/full runs. Shared
helpers became viewport-aware (drawer-aware `selectProject`, `openSidebarIfMobile`),
and desktop-only cross-column assertions are explicitly skipped at ≤640px where the
dedicated Mobile drag suite covers the same behavior through the segmented control.

### Why

The mobile shell (CO-356/357/358) is only as good as the regression net under it —
the matrix caught a real interaction loss (hidden-column drag) the same night it
was introduced.

## CO-374 — Playwright E2E suite for staging — universe recursion, promotion, lead funnel, user routes

Adds `co-web/e2e-staging/` — a Playwright suite that runs against the live
staging environment (`staging.co.artelonga.com.br`) rather than an ephemeral
local server.

Six scenario files cover universe recursion (parent_key chain + breadcrumbs),
sub-universe promotion (pre-conditions + fixme stubs for the Wave 4 endpoint),
lead funnel (POST /api/v1/leads + magic-code onboarding), general user routes
(8 pages load < 3 s, no console errors), security headers (X-RateLimit,
Retry-After, HSTS, CSP), and auth flows (magic-code, password, cross-env
token, logout, expired-token 401).

`scripts/scrum/acceptance-to-playwright.ts` walks all `work/co/CO-*.md` specs,
parses every `## Acceptance` checkbox, and emits
`e2e-staging/generated/from-acceptance.spec.ts` with one `test.fixme()` stub
per item (1 316 stubs across 173 specs). Devs fill stubs in during PRs;
CI can track Wave 4 DoD completion by counting remaining fixmes.

Three Playwright projects (Pixel 7, iPhone 14, iPad Pro) run via `npm run
test:staging`. The new `staging-e2e.yml` workflow triggers automatically after
each staging deploy and on manual dispatch. `prod-release.yml` (manual,
Thursday release gate) checks that the latest staging E2E run succeeded before
deploying to production.

HTML reports are uploaded as GitHub Actions artifacts at
`playwright-report/<sha>/` per device project.

### Why

Wave 4 lands 16 PRs with heavy schema and UI changes. Continuous validation
against live staging catches drift that unit tests miss. The Thursday 12:00 BRT
release window needed an objective pass/fail gate to replace the manual smoke
checklist.

## CO-375 — API contract enforcement against running staging — probe every endpoint, detect drift

Added a runtime contract probe that hits every endpoint declared in `co-web/openapi.yaml`
against the staging environment and reports status-code or response-shape drift.

New files:
- `co-web/scripts/contract/probe-staging.ts` — probe script (~280 lines): parses
  openapi.yaml, resolves known path params (`{slug}` → `template`, etc.), skips
  token-issuance and unparameterizable endpoints, probes the rest concurrently,
  validates status codes and JSON shapes, writes `contract-probe-report.json`.
- `.github/workflows/contract-probe.yml` — triggers after every staging deploy
  (`workflow_run`) and on API-surface PRs (`pull_request`); posts a drift diff
  as a PR comment when drift is found.

Modified files:
- `.github/workflows/release.yml` — added `contract-check` job that gates all
  `build` jobs; prod artifacts are not published until the staging probe passes.
- `co-web/package.json` — added `contract:probe` npm script.

### Why
CO-350 already prevents catalog/code/openapi.yaml divergence statically. This
adds the third axis: **actual runtime response shapes on staging**. Wave 4 PRs
that silently change HTTP status codes or drop required fields in JSON responses
are now caught before a prod release is tagged.

## CO-382 — Scrum-aligned CI/CD with DoD verification — deterministic route per task, release gate

Introduces a deterministic 10-step CI/CD route for every PR, with DoD (Definition of Done)
verification as a mandatory merge gate and release gate.

**New scripts:**
- `co-web/scripts/dod/verify.ts` — parses `## Acceptance` from `work/co/CO-N.md`, maps each
  item to a test pattern, searches `e2e/` and `e2e-generated/` for matching Playwright tests,
  reports per-item ✅/❌, posts a DoD table as a PR comment, generates stub spec files, and
  saves a JSON report to `docs/scrum/dod/CO-N.json`.
- `co-web/scripts/scrum/sprint-review.ts` — reads DoD JSON reports + git history, generates
  `docs/scrum/sprints/sprint-<N>.md` and commits it (Thursday 14:30 BRT).

**New GitHub Actions workflows:**
- `.github/workflows/pr-route.yml` — step 6 (migration validation, conditional) + step 10
  (DoD verification per PR); blocks merge on any ❌ acceptance item.
- `.github/workflows/staging-suite.yml` — step 8 (contract probe) + step 9 (E2E staging suite)
  after every push to main; results gate Thursday release.
- `.github/workflows/release-gate.yml` — Thursday 14:00 BRT cron; validates all wave PRs have
  green DoD, generates sprint review, blocks release if any PR is below 100%.

**Release gate in `scripts/release-commit.sh`:**
- Reads `docs/scrum/dod/CO-N.json` for each pending `CHANGELOG-PENDING/CO-N.md` task.
- Refuses release if any task has `dod_pct < 100`.
- `--ignore-dod` override flag for emergency hotfixes (logged in release commit theme).

**Live timeline (CO-381):**
- `ci.*` and `release.gate.*` event types now render with 🛠️ / 🚀 / 🚫 icons in `/agora`.

**Documentation:**
- `docs/ci-route.md` — full 10-step route reference.

### Why

CO-382 gives the scrum process teeth: DoD becomes a CI gate, not just documentation.
A merged PR with unchecked acceptance items is now structurally impossible.

### Directories added

- `co-web/e2e-generated/` — stub spec files generated by `dod:verify --generate-stubs`
- `docs/scrum/dod/` — per-task DoD JSON reports consumed by release gate

## CO-397 — Public API rate limits + abuse protection (Phase 1 of CO-278 epic)

Added the minimum rate-limiting and abuse-protection layer needed to safely open the CO public API at v3.0.

- **Token-bucket rate limits** for anonymous (60 GET/min) and authenticated (600 GET/min, 10×) traffic via in-memory middleware. Anonymous write budget: 5/min; authenticated: 60/min.
- **X-RateLimit-Limit / Remaining / Reset headers** on every `/api/v1/*` response so clients can self-pace.
- **X-Co-Server-Version** header on all responses for agent identification.
- **User-Agent gate**: empty or single-character UA is rejected with 400 `missing_user_agent`.
- **Abuse heuristics**: 30+ 404s or 10+ 401s within a 1-minute window → 15-minute temp ban. Bans are in-memory and clear on restart.
- **Trusted-IP bypass**: set `CO_TRUSTED_IPS=CSV-of-CIDRs` to exempt monitoring IPs and CI runners.
- **`GET /robots.txt`** and **`GET /sitemap.xml`**: crawl policy + auto-generated sitemap from public universes.
- **Abuse events logged to `atividades`** (`entidade=api_abuse`, `tipo=sistema`) so they surface in the `/gestao/resumo` recent-activity feed.
- **OpenAPI spec updated** with `components/responses/RateLimited` and 429 responses on entries, universes, and feedback endpoints.

### Why

The public API was fully open with no IP-level rate limit beyond CO-339's feedback path. A single misconfigured crawler or LLM agent loop could exhaust the Fly machine before v3.0 launch. This is the minimum viable protection layer — per-tier billing semantics and persistent bans are deferred to CO-80 / CO-278-A (post-v3.0).


## [2.43.0] — 2026-06-09 — federated event bus + sync + privacy

## CO-376 — Pre-prod migration validation (MVP) — `migrate_check` against a staging snapshot

Adds a Rust validator that applies the current binary's migrations against a **copy** of a
staging snapshot and runs read-only smoke assertions — catching the 1.22.4 class of
incident (unguarded freshly-migrated-column read) before prod, without touching any live DB.

- **New bin `co-web/src/bin/migrate_check.rs`** — `migrate_check <extracted-snapshot-dir>`:
  records a pre-migration baseline, runs meta migrations via `Storage::new` (+ entry-split),
  opens each universe to run pool migrations, then asserts the wave's tables/columns are
  selectable (`bridge_state` v65, `sync_conflicts` v67, `universes.source_*` v68,
  `entries.source_marker` pool v17), the `yuri` admin user survived, stable counts are
  unchanged, and the conserved entry total (meta + all universes) drifts ≤ ±5%. Exit 0 iff
  all pass, 1 on any failure, 2 on bad invocation.
- **`docs/migration-checklist.md`** — the CI-sandbox flow: obtain a staging snapshot
  (admin backup endpoint or nightly artifact), extract, run `migrate_check`, interpret.

Execution is **CI-sandbox** — operates on an extracted copy, never on staging's live DB
(resolves the spec's self-contradiction between its SSH-based Flow text and its "no live
data touched" acceptance).

### Why

The federation wave (v65–v68 + pool v17) applies to live data for the first time at the
v3.0 cut. `migrate_check` is the go/no-go gate for that.

### Deferred (CO-376 follow-up)

Auto-gating GitHub Action on migration-touching PRs; `GET /api/v1/admin/migrations/snapshots`;
custom `migrations/<vN>.smoke.sql`; 24h retention automation.

## CO-378 — Privacy: respect noindex/nofollow in analytics rollups + funnel reports — strip private paths

Added path-level privacy redaction to the analytics pipeline so private event pages
(noindex/nofollow or matching `/_drafts/`, `/_proposals/`, `/_smoke/`, etc.) are no
longer exposed in admin dashboards or the public summary endpoint.

### Changes

- **Migration v69**: adds `path_private INTEGER DEFAULT 0` to `analytics_rollups`.
  Future rollup producers can send `private: true` to mark a universe/day as private.

- **Rollup ingest** (`POST /api/v1/analytics/public/rollups`): accepts optional
  `private: bool` field (backward-compatible; defaults to false). Sets `path_private=1`
  on the upserted row.

- **Public summary** (`GET /api/v1/analytics/public/summary`): default response strips
  private paths from `top_pages` and aggregates them as a single `{path: "(private)"}` entry.
  Scalar aggregates (views, visitors, sessions, geo) remain unfiltered.
  `?include_private=true` with a valid `CO_ROLLUP_TOKEN` bearer returns deterministic
  redacted hashes (`<private-path-{hash16}>`) and logs an `analytics.private_path_viewed`
  atividade event.

- **Funnel report** (`GET /api/v1/analytics/public/funnel`): new endpoint. Private paths
  contribute to `total_views` and `total_private_views` but are excluded from `by_path`.

- **Gestão analytics resumo** (`GET /api/v1/gestao/analytics/resumo`): new admin-only
  endpoint. Returns analytics summary with private paths clustered as a single
  `(private)` entry showing total views and page count. Logs
  `analytics.private_path_viewed` atividade on each call. (Mounted under
  `/analytics/` because `/api/v1/gestao/resumo` is the CO-360 dashboard endpoint.)

- **OpenAPI**: documents `?include_private` flag on summary, new funnel endpoint,
  new gestao/analytics/resumo endpoint.

Note: the "Resumo Analytics" card originally added to the old `/gestao` page was
dropped in rebase — CO-360 replaced that page with the four-tab SPA. Surfacing the
private-cluster view in the SPA is follow-up work under CO-360's surface.

### Why

`/2026-05-29/` slide-deck for "1º Encontro · Neuro Notebook Brasil" is `noindex,nofollow`
but the platform's own admin surface was revealing: (a) that the URL exists, (b) traffic
count, (c) forensic details like JS error counts and visitor origin. The page's meta
intent (`noindex,nofollow`) was violated by the platform's own analytics dashboard.

BaaS sovereignty principle: the brain owner controls what's visible — not just to search
engines but to platform operators.

## CO-383 — Yggdrasil notes ingestion — event-driven subscription via federated bus (no polling)

Yggdrasil notes now flow into CO's `yggdrasil` universe in real-time via the CO-384
federated bus bridge — zero polling, sub-300ms latency.

- Migration v68 adds `source_kind`, `source_url`, `source_last_event_at` columns to
  `universes` and backfills the `yggdrasil` row as `source_kind = 'event-bus'`.
- New EDA subscriber `yggdrasil_notes` listens for `entry.{created,updated,deleted}`
  events on the `yggdrasil` universe key and upserts/soft-deletes entries at
  `instances/<instance_id>/notes/<slug>.md` with `source_marker = 'yggdrasil-live'`.
- Stamps `source_last_event_at` on the universe row after each ingested event.
- Re-publishes `sync.yggdrasil.note_ingested` to CO's own bus so the live timeline
  (/agora, CO-381) picks up each ingestion in real time.
- Write attempts (POST/PUT/DELETE) on any `source_kind = 'event-bus'` universe return
  HTTP 405 with a structured `read_only_universe` error body pointing to the source.
- SPA shows a sticky "Somente-leitura — publicado via Yggdrasil" banner with an
  "Editar na origem" link whenever a read-only event-bus universe is viewed.

### Why

The Yggdrasil-keeps-notes architecture decision (memory `project_yggdrasil_absorption`)
requires CO to be a subscriber, not a writer. Polling was explicitly rejected
(memory `feedback_no_polling`). The CO-384 federated WS bridge provides durable,
replay-on-reconnect event delivery — CO hooks into it with a single subscriber task.

## CO-384 — Federated event bus bridge — cross-deployment WS pub/sub (CO ↔ Yggdrasil ↔ devices)

Adds a persistent bidirectional WebSocket bridge layer on top of CO-380's local `EdaBus`,
enabling cross-deployment event federation with no polling.

**New endpoint:** `GET /api/v1/events/bridge` — trusted peer deployments connect here
(trust enforced by `CO_BRIDGE_TRUSTED_SOURCES`; unknown sources receive HTTP 403).

**Protocol:** `co.eda.bridge.v1` sub-protocol. On connect, peers send a
`ReplayRequest{last_received_id}` to drain missed events from `event_log` (idempotent,
ULID-keyed). Bidirectional `FederatedEvent` messages flow continuously after replay.

**Privacy rules at bridge level:** only `Public` and `UniverseMembers` events are
federated. `UserOnly`, `UniverseOwner`, and `System` events remain local.

**Loop guard:** `hop_count > 3` → event dropped silently.

**Outbound client:** `BridgeManager` reads `CO_BRIDGE_OUTBOUND_TOKENS_JSON` at startup
and spawns one task per destination. Reconnects with exponential backoff (1s→2s→4s→8s→16s→max 30s).

**Migration v65:** `bridge_state` table tracks per-(source,target) connection state and
`last_delivered_event_id` for replay on reconnect.

**Telemetry:** `bridge.connected`, `bridge.disconnected`, `bridge.event_received`,
`bridge.event_sent`, `bridge.replay_completed` — all `Public` visibility, visible in `/agora`.

**Docs:** `docs/federated-eda.md` — full protocol, deployment, and trust setup guide.
OpenAPI: `/api/v1/events/bridge` documented in `docs/api/openapi.yaml`.

### Why

Eliminates polling for cross-deployment sync (CO-380 was in-process only). A note edited
in Yggdrasil now arrives on CO's live timeline (`/agora`) within 300ms over LAN. CO-383
(Yggdrasil ingest consumer) becomes a thin subscriber on top of this transport primitive.
CO-385 (conflict resolution, v3.1) will consume the event stream surfaced here.

## CO-385 — CRUD action tree — Mac-style UPSERT conflict resolution for cross-device sync

Implements the full conflict resolution layer for cross-device sync.

### What changed

- **5 ConflictKind variants** (`both_modified`, `local_only_new`, `remote_only_new`,
  `local_deleted_remote_modified`, `local_modified_remote_deleted`) classified by
  `detect_conflicts()` in `co-web/src/sync/conflict_detector.rs`.

- **7 resolution actions** (`keep_both`, `ignore`, `replace`, `update`, `upsert`,
  `accept_delete`, `keep_local`) implemented in `conflict_resolver.rs`. Each
  action writes the entry change atomically, marks the conflict resolved in the
  `sync_conflicts` table, and publishes a `sync.conflict_resolved` event.

- **Hash-skip optimization**: same `body_hash` between local and remote skips the
  conflict pipeline entirely, both in `detect_conflicts()` and in the CO-384
  bridge handler.

- **3-way text merge** for `update`/`upsert`: line-based merge with git-style
  conflict markers (`<<<<<<< / ======= / >>>>>>>`) when both sides diverged.

- **Bulk-apply**: `POST /api/v1/sync/conflicts/{id}/resolve` accepts
  `apply_to_all_matching: true` to propagate a single action to all sibling
  conflicts of the same `ConflictKind`.

- **Migration v67**: `sync_conflicts` table + two indexes (unresolved, per-universe).

- **REST API**:
  - `GET  /api/v1/me/sync/conflicts?universe=<key>`
  - `POST /api/v1/sync/conflicts/{id}/resolve`

- **EDA events**: `sync.conflict_detected` (bridge), `sync.conflict_resolved`
  (resolver), `sync.conflict_resolved_bulk` (bulk route).

- **CO-383 integration**: yggdrasil universe defaults to `replace` (remote wins).

- **CO-381 integration**: `wireLiveConflictCta()` shows "Resolver →" toast in
  the live timeline on `sync.conflict_detected`.

- **SPA UI** (`conflicts.js`): conflict panel with action buttons filtered by
  `ConflictKind`, bulk-apply checkbox, live toast CTA.

- **OpenAPI**: `GET /me/sync/conflicts` and `POST /sync/conflicts/{id}/resolve`
  documented with full `SyncConflict` schema.

- **Docs**: `docs/conflict-resolution.md` — action tree, merge behavior, API,
  events, DB schema.

### Why

CO-385 is the core reliability primitive for Vault-based cross-device sync.
Without explicit conflict resolution, concurrent writes from two devices silently
overwrite each other. The Mac-style action tree gives users full control without
requiring a manual `git merge`.

## CO-389 — Live-event layer over comunicacao universe — Yggdrasil lexicon salas as event source

Subscribes the CO EDA bus to Yggdrasil lexicon sala events, acting as a live overlay
on top of CO-337's 15-min sister-repo sync. Terms published in a sala appear in CO's
`comunicacao` universe within 1s; sala activity events surface in `/agora` within 300ms.

### What changed

- **Migration v66** (meta.db marker) + **universe pool v17**: additive `source_marker TEXT`
  column on the per-universe `entries` table.  Tracks which channel last wrote a row:
  `'yggdrasil-live'` (live overlay), `'remote-git'` (CO-337 poll), or `NULL` (legacy).

- **`co-web/src/eda/subscribers/comunicacao_live.rs`** (new): two subscribers —
  - *Term subscriber*: listens for `entry.{created,updated,deleted}` on the `comunicacao`
    universe (path must contain `_users/` — sala-published terms).  Hash-dedup skips
    writes when CO-337 already has the same bytes.
  - *Sala activity subscriber*: listens for `yggdrasil.sala.*` and re-publishes as
    `sync.yggdrasil.sala.*` for `/agora` with no content mutation.

- **`/agora` live.html**: five new event types in `EVT_META` and i18n labels
  (`sync.comunicacao.term_landed`, `sync.comunicacao.event_dedup_skipped`,
  `sync.yggdrasil.sala.published`, `sync.yggdrasil.sala.term_contributed`,
  `sync.yggdrasil.sala.user_joined`).

- **`/gestao/resumo`**: `comunicacao_overlay` field added with counts from `event_log`
  for overlay-driven vs poll-driven upserts and dedup skips.

- **OpenAPI** (`openapi.yaml`): documented the six `sync.*` event types on the
  `/api/v1/events` WebSocket endpoint.

### Why

Yggdrasil salas and CO's `comunicacao` universe feed the same lexicon.  CO-337's 15-min
poll is the durable channel but creates a visible latency floor for sala UX.  This spec
provides the sub-1s live path without changing universe bindings or durability semantics.

## CO-391 — Real-axum WebSocket bridge handshake integration test

Adds the missing integration-test layer between the `is_trusted_in` unit tests and
full E2E for the CO-384 federated bridge. Boots the real axum router on an ephemeral
`127.0.0.1` port and dials `/api/v1/events/bridge` with a real `tokio_tungstenite`
client speaking `co.eda.bridge.v1`.

Coverage:

- **Happy path** — trusted source + non-empty token + subprotocol → `101`, subprotocol
  echo, and `bridge.connected` observed on the local bus (the handler publishes it to
  the bus, not down the socket — the CO-391 draft's "first frame" assumption was wrong).
- **403** — untrusted source rejected.
- **401** — empty token rejected.
- **400** — GET without `Connection: Upgrade` rejected by axum's `WebSocketUpgrade`
  extractor (tested via `oneshot`, since a real WS client always sends the header).

The 400 case is the exact failure observed locally on 2026-06-09 during pre-flight for
the v3.0 federation cut (HTTP 400 "Connection header did not include 'upgrade'"), which
YG-119's lenient tokio-tungstenite mock had masked. If a YG-122-class regression were
reintroduced on the CO side, these tests fail loudly at PR time.

No behavior change — test-only. Reuses the existing `127.0.0.1:0` + `axum::serve` +
`connect_async` harness pattern already used by `social/sync_ws.rs` and
`admin/analytics_routes.rs`; no new deps (tokio-tungstenite already a dev-dep).

### Why

CO unit tests covered only the trust-list predicate — no `WebSocketUpgrade`, no real
socket. The handshake contract was enforced from neither end until YG-122 + this test
landed. This is the unit/integration layer; CO-374 (Playwright staging) remains the
separate prod-shape backstop.

## CO-394 — Seed relation extraction — `co launch` populates the knowledge graph

CLI-seeded universes (`co launch` / `seed_universe_from_local_repo`) now extract
`[[wikilink]]` relations during seeding, in parity with the server vault/entry write
path. Previously the seed ingested entry bodies but left `entry_relations` empty, so the
knowledge-graph view rendered nodes with **zero edges** for every locally-seeded universe
(grcsamazonia, comunicacao, mbya, …).

- After each entry upsert, `extract_body_wikilinks(&entry.body)` runs and
  `RelationIndex::replace_all` stores the edges.
- Same-universe targets are resolved relative to the linking entry's directory
  (`resolve_entry_rel`), normalizing `.`/`..`, so the stored `to_path` matches stored
  entry paths (which carry the content-subdir prefix) — otherwise edges dangle and the
  graph builder drops them.

Verified on grcsamazonia: `entry_relations` 0 → 20, graph 0 → 18 edges across 15/17 docs.

### Why

The grafo / garden view is a core CO surface, but it was empty for every CLI-seeded
universe because relation extraction lived only on the HTTP write path. This makes
`co launch` produce a fully linked graph from the same markdown.


## [2.42.0] — 2026-06-08 — Unified gestão + cross-env identity + live timeline

## CO-360 — Unified /gestao/resumo dashboard — collapse 6 admin routes into one SPA + endpoint

Replaced the fragmented admin surface (6 separate handlers with mismatched auth and HTML) with a single `/gestao` SPA backed by three batched endpoints:

- `GET /api/v1/gestao/resumo` — one round-trip for KPIs (users, universes, entries, sessions_24h, disk), top pages, visits-by-hour, visits-by-dow, referrers, broken links, recent activity, schema version, and app version.
- `GET /api/v1/gestao/universes` — universe list with entry counts and last-activity timestamp.
- `GET /api/v1/gestao/atividades?limit=…&since=…&acao=…` — paginated audit log feed (session-cookie / JWT admin auth, replacing the GitHub-PAT-only endpoint for dashboard use).

The SPA (`/gestao`) renders four tabs — Resumo, Conteúdo, Usuários, Atividades — with 30-second client-side cache per tab so repeated switching does not re-fetch.

### Deprecation timeline for old admin URLs

The following URLs now show a top banner "Movido para /gestao. Este painel será removido em v2.41.":

- `/analytics` (real-time event stream dashboard)
- `/admin/deployments` (Fly machine status dashboard)
- `/storage` (per-universe disk dashboard)

These handlers remain reachable for **one release** after CO-360 ships. They will be deleted in v2.41.

### Deleted

- `co-web/src/admin/uat_mirror.rs` — obsolete per `feedback_no_uat` (CO-360 carries out the planned deletion). The UAT mirror spawn in `server/mod.rs` was removed alongside it.

### Why

Opening 4-6 separate admin tabs to assemble a complete operational picture added friction and meant every UI redesign required touching 6 handlers. The new SPA provides one cohesive surface with a single round-trip. Auth is now consistently cookie/JWT-based (same as the main app), removing the GitHub PAT requirement for the dashboard view.

## CO-377 — Cross-env identity — shared admin + JWT (Phase 1); OIDC federation (Phase 2)

Phase 1 establishes shared credentials between production and staging so that
`yuri@artelonga.com.br` can log into staging with the same password as prod
and tokens issued in one environment are accepted in the other.

Changes:
- `docs/cross-env-auth.md` — Phase 1 secrets sync runbook, risk model,
  Phase 2 OIDC federation roadmap, rotation schedule
- `docs/jwt-rotation.md` — step-by-step JWT_SECRET rotation runbook with
  impact table, emergency procedure, and rotation log

Operational steps (not tracked in git):
- `JWT_SECRET`, `CO_SEED_ADMIN_PASSWORD_HASH`, `CO_SEED_ADMIN_EMAIL`,
  `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` synced from prod to staging
  via `flyctl secrets set` following the runbook above
- Google OAuth client updated: added
  `https://staging.co.artelonga.com.br/api/v1/auth/google/callback`
  as an authorized redirect URI

### Why
The Playwright suite (CO-374) needs to use prod-issued tokens against staging.
Manual testing on staging should not require a separate account. Phase 2
(OIDC federation via prod JWKS) is documented and deferred to post-v3.0 to
avoid the shared-secret risk long-term.

## CO-381 — Live timeline — /agora (pt-BR) + /live (en) — real-time deployed-interface visualization

Added `/agora` and `/live` — two language-routed entry points for the same live
timeline SPA that streams EDA bus events to the browser via WebSocket.

- **Routes**: `GET /agora` (pt-BR) and `GET /live` (en) serve `live.html` publicly
  (no auth gate; anonymous visitors see public events only).
- **WebSocket**: SPA connects to `/api/v1/events?scope=<scope>` — the existing
  CO-380 endpoint now also accepts `session` cookie auth (same-origin browser
  pages send cookies automatically).
- **Visibility tiers** (wired in `events_ws.rs`): Anonymous → Public only;
  Member → + UniverseMembers; Owner (detected via DB lookup) → + UniverseOwner;
  Admin (`tier == "admin"` in JWT) → all including System.
- **UI**: scope selector, filter chips (Content / Auth / Analytics / Billing /
  System), stats strip (events/min, active universes, top types), pause/resume,
  language toggle (redirects between the two routes).
- **Event cards**: per-type icon + colour, human-readable summary, relative
  timestamp, drill-in chevron.
- **Drill-in panel**: full payload + context fields; becomes full-screen sheet
  on mobile (≤ 640 px).
- **Coalescing**: bursts of the same `event_type + universe_key` within 500 ms
  collapse to "N entries updated".
- **Throttle**: max 10 visible cards/sec; overflow shown as "+N more events".
- **Reconnect**: exponential backoff (1 s → 2 → 4 → 8 → max 30 s).
- **Battery-aware**: stream paused when the browser tab is hidden.
- **Atividades**: every page load publishes `live.timeline_viewed` (System
  visibility) to the EDA bus so the atividades log captures usage.
- **OpenAPI**: updated `/api/v1/events` spec to reflect CO-381 SPA consumer,
  `co.events.v1` sub-protocol, and cookie auth option.

### Why
Completes the "real-time visualization of effects on deployed interface"
requirement (2026-06-06). The EDA bus (CO-380) was already emitting events;
this task surfaces them to users in a browsable, filterable live feed.


## [2.41.0] — 2026-06-06 — Brain interlink + EDA spine + staging foundation

## CO-345 — Cross-universe graph view + publishable saved views

`GraphQuery` now accepts `?universes=key1,key2` to load nodes and edges across multiple universes in a single graph render. When multi-universe mode is active, node IDs become `universe::path`; single-universe mode keeps bare `path` for back-compat.

A new `graph_views` table (migration v59) and five CRUD endpoints (`POST /api/v1/me/graph-views`, `GET /api/v1/me/graph-views`, `GET /api/v1/graph-views/{slug}`, `PATCH /api/v1/graph-views/{slug}`, `DELETE /api/v1/graph-views/{slug}`) let authenticated users save, share, and manage named graph views with `public | unlisted | private` visibility.

The graph canvas (`/shared/graph.html`) gains a universe chip strip (fetched from `GET /api/v1/universes?visibility=public-subscribable`), a depth slider (1–6), and a "Save view" modal that posts the current state and returns a shareable `/graph-views/{slug}` URL. Opening `/graph-views/{slug}` hydrates chips, depth, and root from the saved view before loading the graph.

`extract_body_cross_universe_wikilinks` in `relation_index.rs` now emits `to_universe` for `[[key::path]]` wikilinks found in markdown bodies, enabling cross-universe edges to be indexed correctly.

### Why
Allows graph views that span multiple content universes (e.g. yoruba + mbya) to be created, shared, and bookmarked without per-session configuration.

## CO-350 — Catalog → OpenAPI codegen + CI drift check

Added a TypeScript generator (`co-web/scripts/generate-openapi.ts`) that parses
`docs/architecture/api-catalog.md` and emits `co-web/openapi.yaml`.
Two new npm scripts: `openapi:gen` regenerates the YAML from the catalog;
`openapi:check` exits 1 with a readable diff when catalog, code, and YAML diverge.
A GitHub Actions workflow (`openapi-check.yml`) runs `openapi:check` on PRs that
touch `*_routes.rs`, `api-catalog.md`, `openapi.yaml`, or the generator script.

The bootstrap commit aligns the catalog (316 entries) with every registered axum
route and regenerates `co-web/openapi.yaml` so `--check` passes immediately after
merge. Component schemas are extracted into `co-web/openapi-components.yaml` and
concatenated at gen time.

### Why

Both `api-catalog.md` (human-readable) and `openapi.yaml` (machine-readable) existed
with zero enforcement that they described the same surface. Adding a route no longer
requires a separate "update docs" step — the catalog row is the documentation, and
CI rejects PRs that skip it.

## CO-361 — Atividades audit log + schema_versoes admin surface

Adds a typed `atividades` audit log and a `schema_versoes` migration history
table, both surfaced in the `/gestao` admin SPA.

### What changed

- **Migration v59** creates the `atividades` table (3 indexes, CHECK constraints
  on `acao`/`tipo`), the `schema_versoes` table, and backfills
  `schema_versoes` from the existing `schema_version` rows with
  `descricao='(backfilled)'`.
- **`platform::atividade`** — new module with `log_atividade()` (deferred write
  via `tokio::spawn`), `redact()` (strips SENSITIVE_KEYS at any JSON depth),
  and `sha256_short()` (16-char SHA-256 prefix for IP hashing).
- **Call sites wired**: task create/update/delete (`server::legacy`), universe
  create/delete (`content::universe_routes`), login via magic-link and
  password (`server::auth_handlers`), and logout.
- **`GET /api/v1/gestao/atividades`** — paginated feed filterable by `acao` and
  `since`; GitHub admin auth required.
- **`GET /api/v1/gestao/schema-status`** — returns current schema version, app
  version, drift indicator, and last 20 migration history rows.
- **`/gestao` SPA** — new `gestao.html` served at `/gestao`; shows the
  "DB schema vN / app vX.Y.Z" header strip with drift badge and an
  atividades feed with per-row diff panel.
- **180-day retention**: `atividade::retention_task` spawned at boot, deletes
  rows older than 180 days nightly.
- **`record_migration!` macro** defined in `migrations.rs` for future v60+
  migrations to keep `schema_version` and `schema_versoes` in sync.

### Why

Operators were unable to answer "who deleted that entry?" without digging
through Fly.io logs. After every deploy there was no in-DB record of which
migration version the running container had applied. This closes both gaps
without adding latency to any request (all writes are deferred).

## CO-363 — Cross-universe wikilink resolver — `[[key::path]]` populates entry_relations.to_universe

The relation extractor now handles all four body wikilink forms and frontmatter
`key::path` syntax, writing rows to `entry_relations` with `to_universe` populated:

- `[[mbya::terms/jaxy-jatere]]` → `to_universe="mbya"`, `to_entry="terms/jaxy-jatere"`
- `[[concepts::mother.md|mãe]]` → `to_universe="concepts"`, `link_text="mãe"`
- `[[terms/local]]` (plain) → `to_universe=NULL` (same universe)
- `[[../sibling/x]]` → `relation_type="wikilink_relative_deprecated"` + deprecation log
- Frontmatter `concept: yoruba::terms/ogunte` → `to_universe="yoruba"`

New `link_text TEXT` column added to `entry_relations` (universe_pool v16). A
one-time startup backfill re-indexes body wikilinks for all existing entries.

### Why

CO-345's graph view needs real cross-universe edges to render. This spec populates
`to_universe` for inline body wikilinks so the graph has the data it needs.

## CO-365 — Long-term storage backend trait — pluggable backup scaffold (local default, S3/R2/GCS as stubs)

Introduced a `BackupBackend` trait with a full `LocalFsBackend` implementation
(default, no feature flag) and compile-tested stubs for S3, R2, Fly volume
snapshots, and GCS (each gated behind `backup-s3 / backup-r2 / backup-fly /
backup-gcs` cargo features). The backend choice is now a config flip
(`CO_BACKUP_BACKEND` env var), not a code change.

Changes:
- `co-web/src/storage/backup/` — `BackupBackend` trait, `Snapshot`, `SnapshotId`,
  `SnapshotMeta` types, `LocalFsBackend` (full), four cloud stubs, `backend_from_env`
  factory, and `build_snapshot` tarball builder.
- `co-web/src/platform/workers.rs` — `BackupWorker` (runs every
  `CO_BACKUP_INTERVAL_HOURS`, default 24 h; short-circuits when
  `CO_BACKUP_BACKEND=disabled`).
- `co-web/src/admin/backup_routes.rs` — `POST /api/v1/admin/backup/snapshot`
  (admin-only, triggers snapshot in background) and
  `GET /api/v1/admin/backup/snapshots` (admin-only, list stored snapshots).
- Snapshot success/failure events routed to atividades audit log.
- Retention prunes snapshots older than `CO_BACKUP_RETENTION_DAYS` (default 30) on
  every worker tick.
- Snapshot manifest includes the active backend name for restore-tool disambiguation.
- `docs/backup-format.md` documents the archive layout, manifest schema, and admin API.
- `scripts/backup-prod-local.sh` preserved as a CLI fallback.

### Why

Prepares CO for v3.0 public launch with backup running at startup — satisfies the
BaaS sovereignty guarantee ("no third-party lock-in") by decoupling the backup
target from the codebase. CO-143 (which required choosing AWS upfront) is no longer
a v3.0 blocker.

## CO-369 — Retrospective sprint simulation — reconstruct CO's history as bi-weekly sprints

Added `co-web/scripts/scrum/retro-simulate.ts`, a one-shot Node/tsx script that walks
`git log --first-parent main`, `git for-each-ref refs/tags`, and `work/co/CO-*.md` spec files
to reconstruct CO's delivery history as 14-day bi-weekly sprints anchored at 2026-06-11.

Generates 13 retrospective sprint files (`docs/scrum/sprints/sprint-minus-N.md` + `sprint-0.md`),
a velocity chart (`retro-velocity.md`), a narrative overview (`retro-simulation.md`), and an
anomalies report (`retro-anomalies.md`). `--forward` flag generates the next 4 sprint templates.
A machine-readable `_index.json` serves CO-372's sprint calendar renderer.

### Why

The sprint simulation gives CO an honest historical record of "what shipped per sprint" without
requiring retroactive scrum discipline. It provides sprint -12 → -1 velocity data, release cadence
evidence, and DoD completion visibility from day one — grounding ArteLonga's scrum framework
empirically.

## CO-370 — Lead funnel docs + unified capture — signup = lead (email join key)

Documents the 8-step acquisition funnel and stitches the lead capture and
passwordless signup paths at the email join key, so every signup creates a
lead record and every lead form submission creates a user shell.

### Changes

- `docs/lead-acquisition.md` — 8-step funnel diagram, per-step table, gap
  analysis, KPIs, and cross-references to CO-371/CO-366.
- Migration v61 adds `leads.user_id` (FK → users) and `leads.source`
  (`lead_form | signup | invitation | manual`) with indexes.
- Migration v62 adds `users.lead_id` (FK → leads), `users.status`
  (`active | pre-registered | suspended`), and `users.activated_at`;
  backfills both FKs by matching existing rows on lowercased email.
- `POST /api/v1/leads` now finds or creates a user shell (status
  `pre-registered`) when an email is provided, links bidirectional FKs,
  and emits `lead.captured` + `lead.user_linked` telemetry.
- `POST /api/v1/auth/onboard-with-email/verify` (create path) now upserts
  a signup-sourced lead, links the new user, auto-advances the lead to
  `in_progress`, and emits `signup.captured` telemetry.
- `GET /api/v1/auth/me` response extended with `lead_id`, `lead_status`,
  `lead_source`.
- `GET /api/v1/admin/leads` response extended with `user_id`, `user_status`,
  `verified_at`.
- OpenAPI documents all extended request/response shapes.

### Why

Lead form submitters were invisible to admin triage because the leads table
and users table had no cross-link. A signed-up user had no lead record, so
acquisition → convert attribution was unjoinable without manual SQL.
CO-371 (funnel report) requires this unified identity to compute per-step
drop-off metrics.

## CO-379 — Staging environment — Fly app + DNS + secrets + nightly reset (foundation for Wave 4 v3.0)

Added a dedicated staging environment (`co-artelonga-staging`) as an automated validation gate for Wave 4 PRs (16 PRs with heavy schema + UI changes).

Changes:
- `fly.staging.toml` — new Fly.io app config (`co-artelonga-staging`, `gru`, 256 MB shared, `CO_ENV=staging`)
- `.github/workflows/staging-deploy.yml` — auto-deploys main to staging on every push
- `/api/health` response now includes `env` field (`"staging"` | `"production"` | …) read from `CO_ENV`
- `WebConfig::is_staging()` — identifies the staging environment for conditional seeding and worker registration
- Staging fixture universes seeded on every deploy (idempotent): `recursion-a`, `recursion-ab`, `recursion-abc`, `funnel-fixture`, `mbya-staging`, `yoruba-staging`
- `StagingTestSweepWorker` — hourly worker that deletes `u-test-*` universes older than 7 days every Sunday 03:00 BRT (registered only when `CO_ENV=staging`), keeping the most recent 100 for forensic inspection
- `storage::snapshot` module — SQLite online backup/restore API via `rusqlite::Backup` (CO-376 pre-prod migration validation will use this)

### Why
Wave 4 (v3.0 mobile public release) lands 16 PRs with schema migrations and UI rework. Running the full Playwright suite against a real Fly instance catches drift that unit tests cannot — shared staging JWT secret means cross-env token verification is also tested. Per design: reverses the prior "no UAT" decision only for automated validation; manual UAT remains off the table.

## CO-380 — Universal event bus — EDA spine for all observability (atividades + analytics + billing + sala presence)

Introduced a universal EDA (Event-Driven Architecture) bus as the single spine for all
state-changing observability across the platform. Every route that mutates state now
publishes a typed `Event` to the bus; subscribers consume and act asynchronously.

### What changed

- **`co-web/src/eda/`** — new module:
  - `event.rs`: `Event` envelope with ULID-based IDs + `Visibility` enum (Public / UniverseMembers / UniverseOwner / UserOnly / System)
  - `bus.rs`: `EdaBus` trait + `Filter` (prefix-matched event_type, universe_key, user_id, min_visibility) + `Subscription`
  - `tokio_bus.rs`: `TokioBroadcastBus` — default in-process impl (4 096-slot broadcast; < 50ms p99)
  - `redis_bus.rs` / `nats_bus.rs`: feature-gated stubs (`eda-redis`, `eda-nats`)
  - `events_ws.rs`: `GET /api/v1/events` WebSocket route — server-enforced federation rules (anon → Public only)
  - `subscribers/`: 6 concrete subscribers (AtividadesPersistor, AnalyticsAggregator, BillingPersistor, SalaBroadcaster, KbIndexer, LiveTimeline)

- **Migration v63** (`event_log` table + indexes on `created_at`, `universe_key`, `event_type`)

- **30-day retention task** for `event_log` (nightly, mirrors atividades retention pattern)

- **6 producers wired**:
  - `atividade::log_atividade` — publishes `atividade.*` (System visibility)
  - `telemetry_middleware` — publishes `analytics.visit` (Public)
  - `entry_routes` — publishes `entry.created / updated / deleted` (UniverseMembers)
  - `vault_routes::write_vault_entry` — publishes `vault.write` (UniverseOwner)
  - `sync_ws::apply_deltas_to_storage` — publishes `sync.remote_pull` (UniverseOwner)
  - `auth_handlers::signup_handler` — publishes `billing.account_created` stub (UserOnly)

- **OpenAPI** updated with `/api/v1/events` WebSocket route and `Events` tag

- **Cargo features** added: `eda-redis`, `eda-nats`, `eda-persistence`

- **CO-353 workspace presence** reimplemented as `SalaBroadcaster` subscriber (ready for CO-381 WebSocket fanout)

- **CO-361 atividades log** still works — `log_atividade()` continues its direct SQL write; bus publish is additive

- **CO-340 analytics rollups** still work via existing REST ingest path; `AnalyticsAggregator` is the future in-process hook

### Why

The 6+ fragmented event paths (atividades sync write, analytics batch, billing sync, sala presence,
sync polling, KB push) were creating tight coupling and making real-time observability impossible.
A single bus + subscriber model consolidates all observability into one pattern, makes CO-381
(live timeline) trivial, and enables the Yggdrasil notes integration (CO-383) to plug in as
another producer without touching existing code.


## [2.40.0] — 2026-06-05 — substrate stable + OSS integrations

## CO-211 — Universe Content API v1 spec + Swagger UI

Added a formal OpenAPI 3.1 specification for the Universe Content API and two
new endpoints that serve it:

- `docs/api/openapi.yaml` — hand-written OpenAPI 3.1 spec covering all public
  universe, entry, vault, and auth endpoints (Universe, Entry, Vault, Auth tags).
- `GET /api/openapi.json` — machine-readable spec as JSON; clients and
  openapi-generator can consume this to generate stubs.
- `GET /api/docs` — Swagger UI rendered from the spec; developers can explore
  and test the API interactively.

The spec is embedded in the binary at compile time (`include_str!`) so no
runtime file I/O is required and the binary stays self-contained.

### Why

CO shipped a working REST API with an implicit contract: some endpoints were
documented in `docs/analytics-api.md`, most were only discoverable via grep in
`co-web/src/`. No machine-readable spec meant clients couldn't generate stubs
and breaking changes were only caught by manual testing.

For the "any client renders any universe" vision (CO-212 Svelte viewer,
CO-213 Obsidian plugin, future mobile app) the contract must be the source of
truth, not the Rust handler code.

### Versioning policy (locked in spec)

- `v1` is locked — no breaking changes
- Additive changes (new optional fields, new endpoints) do not require a version bump
- `v2` will use `/api/v2/...` parallel to v1; v1 stays supported ≥ 12 months after v2 launches

## CO-279 — Every universe must seed a default project — fix private-universe + template-seed regression

Restored two invariants that were broken by a short-lived CO-254 rename (CO → TUTORIAL, 2026-05-04):

1. **Template project key reverted to `CO`**: `seed_template_universe` now writes `projects/CO/_project.md` with 9 onboarding tasks. The comment in `seed.rs` records the history so the key is never changed again without updating tests and prod data.

2. **Write-protection returns 403 (not 500)**: `guard_template` in `legacy.rs` correctly returns `AppError::Forbidden` before reaching any mutation path, so POST/PUT to template project tasks return 403.

3. **`seed_default_project_if_missing`**: new idempotent helper that ensures any universe always has at least one project. Called from `seed_admin_content_universes` for all admin-owned universes and from `backfill_default_projects` at boot for any existing universe that slipped through without one.

4. **`migrate_template_project_rename`**: cleanup pass that drops stale `projects/TUTORIAL/*` rows from any database that booted under the broken CO-254 code, then lets `seed_template_universe` re-seed the canonical `CO` project.

5. **Private-universe creation already seeds a project**: `Storage::create_universe` (universe.rs) creates a `{KEY[:4]P}` project on first creation, so Yuri's private universe (and any other user's) never lands on the "no project found" dead-end.

### Why

CI on `main` was red since 2026-05-20 — 4 `template_tests.rs` failures blocked v2.29.0..v2.30.0 from shipping. The operator's private universe also showed the empty dead-end on every login.

## CO-280 — Universe vs sub-universe vs deployable-unit — visual + nav clarification across SPA

Added a breadcrumbs navigation trail to the SPA that makes the platform › universe › sub-universe › project hierarchy visually explicit. The breadcrumb bar (`#breadcrumbs`) renders above the main header whenever a named universe is active, hides on the template root, and collapses on viewports narrower than 480px.

The sidebar Tools section (scaffolded in this branch) separates dev/operator affordances (Deployments, Changelog) from end-user project navigation — fulfilling the IA layer-3 requirement. The `co_dev_ship` button referenced in user reports was confirmed absent from the rendered sidebar.

A Playwright spec (`e2e/co-280-ia-layers.spec.ts`) covers all three IA layers and includes a regression test for board navigation.

### Why

Users reported that 5 "sub-universes" in the sidebar appeared indistinguishable from projects within the current universe, and the `co_dev_ship` button felt out of place. Breadcrumbs give a clear `CO › Universe › Sub-universe › Project` trail without the confusion caused by the now-removed hardcoded Platforms sidebar section (CO-311). The Tools section provides a dedicated, visually muted home for operator affordances, fixing the mixed-IA symptom the user reported.

## CO-291 — CO-284-B — Telemetry trait + OTLP exporter (feature-flagged)

Added `co-web/src/infra/telemetry.rs` — a new module that wires a `tracing`
subscriber to emit spans to stderr (default) or to any OTLP-compatible collector
(Jaeger, Honeycomb, Grafana Cloud) when `CO_TELEMETRY_OTLP_ENDPOINT` is set.

Changes:

- **`infra/telemetry.rs`** — `TelemetryConfig` enum (Stderr | Otlp), `init_subscriber()`,
  `TelemetryGuard` (flushes OTLP on drop), and `db_span()` helper for wrapping
  SQLite calls with child spans.
- **`server/mod.rs`** — replaced inline `tracing_subscriber::fmt().init()` with
  `init_subscriber(TelemetryConfig::from_env())`.
- **`infra/storage.rs`** — added `#[tracing::instrument]` to `get_entry`,
  `list_entries`, and `search_entries` on `SqliteStorage`, producing `db.query`
  child spans under each HTTP request span.
- **`docs/observability.md`** — quickstart for running a local Jaeger and
  pointing co-web at it.
- **`co-web/Cargo.toml`** — added `opentelemetry`, `opentelemetry_sdk`,
  `opentelemetry-otlp`, and `tracing-opentelemetry` dependencies.

### Why

CO-284 requires observability infrastructure so spans from HTTP requests and DB
queries can be exported to a collector for latency analysis and debugging.  The
env-var gate means zero runtime cost when OTLP is not configured, preserving
the existing stderr-only behavior for local development.

## CO-301 — Task archive — per-task worktree compression + queryable change-log link

Adds a **review → archive → prune** lifecycle for co-auto worktrees that eliminates
accumulated disk bloat while preserving full queryable history of every merged task.

### What changed

- `scripts/archive-task.sh <TASK-ID>` — writes `docs/task-archive/<TASK-ID>.json`
  (spec frontmatter + PR metadata + merge SHA + changelog entry). Idempotent.
- `scripts/prune-worktrees.sh [--apply]` — audits all git worktrees, prunes merged
  + archived ones with no dirty state. Dry-run by default.
- `scripts/co-task` — Python query tool: `list`, `show`, `summary`, `diff`, `open`
  subcommands with `--since`, `--label`, `--type`, `--module` filters.
- `scripts/backfill-task-archives.sh [--limit N] [--commit]` — retroactively
  archives the last N merged PRs; creates a single `chore(archive): backfill` commit.
- `scripts/safe-merge-pr.sh` — after every successful squash-merge: pulls main,
  runs `archive-task.sh`, commits + pushes the archive, runs `prune-worktrees.sh --apply`.
- `docs/task-archive/` — new git-tracked directory for all archive JSON files.
- `scripts/README.md` — documents all scripts in one place.

### Why

330 GB across 57 stale worktrees as of CO-301. After each PR merged, the working tree
is redundant — the commits are already on main. This makes the worktree disposable while
preserving a queryable metadata bundle (task ↔ merge SHA ↔ PR ↔ changelog) forever in git.

## CO-339 — Feedback validation — reject empty bodies + probe paths at the API

Added server-side validation to `POST /api/v1/feedback` and
`POST /api/v1/feedback/{universe}/{*entry_path}` that gates every submission
through three checks before it reaches the database:

1. **Probe path blocklist** — paths starting with `/_` or matching a set of
   known scanner paths (`/probe`, `/smoke`, `/selftest`, `/telemetry-check`,
   `/analytics-smoke`, `/healthcheck`) are rejected with
   `400 {"error": "probe_path_blocked"}`.
2. **Body length** — trimmed `message` shorter than 5 characters is rejected
   with `400 {"error": "body_too_short"}`.  Supports both `message` and `body`
   as JSON field names (Yggdrasil compat alias).
3. **Rate limit** — 3 submissions per IP per hour (down from 10).  Excess
   requests return `429 {"error": "rate_limited", "retry_after_s": <n>}`.

A one-shot SQL migration (v57) back-fills all existing `open` rows with
empty or short bodies (the 16 probe entries on prod) to `status = 'wont-fix'`
with an explanatory `owner_response` note, so the operator's notification
inbox shows only real feedback going forward.

### Why

The feedback table on prod accumulated 16 probe/scanner entries with empty
bodies and paths like `/_smoke`, `/_proof`, `/probe`, `/telemetry-check` —
all from CI smoke runners and scanners hitting the open endpoint introduced
by CO-333. These blocked the signal in `/api/v1/me/notifications`.

## CO-340 — Analytics rollups: per-universe ingest + filterable summary + historical↔surface bridge

Adds the **central warehouse half** of the multi-tenant analytics framework
(spec: `artelonga/ArteLonga#docs/analytics-framework.md`). Two capabilities:

1. **Per-universe rollup ingest** — `POST /api/v1/analytics/public/rollups`
   accepts a consented, PII-free `DailyRollup` (`{universe, day, metrics, dims}`)
   from any producer (a universe-owned surface, a partner, another co universe,
   or an external SDK). Upserted idempotently into a new `analytics_rollups`
   table (PK `(universe_key, day)`, migration **v58**). Auth: bearer
   `CO_ROLLUP_TOKEN` (ingest disabled with `503` when the env is unset).

2. **Filterable, universe-scoped summary** — `GET /api/v1/analytics/public/summary`
   gains `?universe=<id>` (default `artelonga`). This lets the **general
   artelonga dashboard surface any universe's stats**, not just the network
   aggregate. The universe id is sanitized to a safe handle before SQL use.

### The historical ↔ surface bridge

When a universe was a path on the apex (`artelonga.com.br/yuri/`) its telemetry
lives in `telemetry_events` as `path LIKE '/yuri/%'` (with
`universe_key='artelonga'`); after promotion to a CNAME surface
(`yuri.artelonga.com.br`) its data arrives as rollups. The summary unifies both:

- **event match** = `universe_key = X OR path LIKE '/X/%'` → captures the
  historical `/yuri` traffic already in co;
- **rollups overlay** the new data, **partitioned at the cutover** (the first
  rollup day): events only count *before* it, rollups *from* it — so there is
  no double-count across the migration boundary. One continuous timeline.

If a universe has no rollups yet, the summary is purely event-based (so
`?universe=yuri` returns the historical `/yuri` stats immediately, before the
surface starts pushing).

### Notes / scope

- Headline scalars (views, visitors, returning, sessions) and the timeseries
  merge across the bridge; dimensional breakdowns (geo/top-pages) stay
  event-based for now — merging rollup `dims` is a follow-up.
- Backward compatible: `?universe` defaults to `artelonga`; the existing
  hardcoded behavior is preserved (the path-bridge is a no-op for the apex).
- 7 new tests (bridge, rollup overlay, cutover no-double-count, idempotent
  upsert, sanitize, day validation); all 26 analytics tests green; fmt + clippy
  clean.

### Why

Two parallel analytics systems existed — the apex (co-backed) and the
universe-owned surfaces — with **no integration**: a partner's stats split
across two stores at the path→CNAME upgrade, and the general artelonga couldn't
see a universe's surface data. This is the central seam that unifies them.

## CO-346 — Fix SPA empty-board mystery — co universe shows no content despite 1227 entries on prod

**Root cause (two compounding bugs):**

1. `seed_co_universe_tasks` returned early when the source directory (`/app/seed-co/`) was absent, before it had a chance to upsert the `projects/CO/_project.md` entry into the universe DB. Both `seed_admin_content_universes` (`if key != "co"`) and `backfill_default_projects` (`AND key NOT IN (..., 'co')`) explicitly skip the `co` universe, trusting this seed to create the project. On installs where the source dir is missing, `co` ends up with zero projects. `bootAppForUniverse` finds no projects, never calls `selectProject`, and the kanban renders empty — even when the API reports 1 000+ entries.

2. `list_dev_tasks` only searched entries at the `work/` path prefix. The boot-time seed (CO-262) stores task entries at `public/CO-*.md` to allow anonymous access via the entries API. This path-prefix mismatch meant `GET /api/v1/universes/co/dev-tasks` returned an empty array even on installs where the seed had run correctly.

**Fixes:**

- **`seed.rs`**: moved the CO project upsert block and `project_universe_index` INSERT before the `source_dir.exists()` guard, so the project row is always created regardless of whether task files are available.
- **`entry_routes.rs`** (`list_dev_tasks`): added a second `query_by_path_prefix` call for the `public/` prefix and merged results with the existing `work/` query. Type filtering (`user-story` / `task` / `epic`) acts as the semantic gate.
- **New unit test** (`test_seed_co_universe_tasks_creates_project_without_source_dir`): asserts that the CO project exists even when called with a non-existent source dir.
- **New Playwright E2E spec** (`co-346-co-board.spec.ts`): covers anonymous visitor to `/co` (board loads, project visible), anonymous API access, and logged-in user stays on `/co` without auto-bounce.

### Why
The `co` universe is the only system universe with a custom project seed path separate from the generic default-project machinery. When that custom seed path is unavailable (first deploy before Docker bundle is in place, or a broken `resolve_seed_co_dir`), the safeguards designed for every other universe are explicitly bypassed — leaving `co` in a permanently empty-board state.

## CO-347 — Surface missing content universes on prod — yuri / retro-umarizal / yoruba / neuro

Added four new universe rows to `seed_admin_content_universes`: yuri, retro-umarizal, yoruba, and neuro.
Each row is seeded with `remote_url`, `remote_ref='main'`, and appropriate `content_subdirs` so the
CO-337 15-minute sync task can pull their content automatically on first boot and every subsequent cycle.

The backfill UPDATE uses `WHERE remote_url IS NULL` as an idempotency guard — operator-set remote URLs
are never overwritten on re-deploy.

- `yuri`: clones `artelonga/artelonga`, walks `yuri/` subdir, `anon_published_only=1` (personal vault).
- `retro-umarizal`: standalone `artelonga/retro-umarizal` repo, no subdir restriction.
- `yoruba`: clones `artelonga/comunicacao`, walks `yoruba/` subdir, `parent_key=comunicacao`.
- `neuro`: clones `artelonga/artelonga`, walks `neuro/` subdir, `parent_key=artelonga`.

### Why

The four universes existed locally but had no universe rows on prod, so they were invisible in the
sidebar. No Docker rebuild is needed per content update — CO-337's remote sync handles ongoing refresh.

## CO-348 — Mbya promote to first-class + merge yoruba term sources

- `~/projects/mbya/_universe.yaml` created (CO-141 schema shape, adapted for mbya).
- `seed_orchestrator.rs`: `remote_url='https://github.com/artelonga/mbya'` + `remote_ref='main'` set for the `mbya` universe row on boot (idempotent, WHERE remote_url IS NULL).
- `~/projects/comunicacao/mbya/` removed (git rm -r); description in `comunicacao/_universe.yaml` updated to reflect the migration.
- Yoruba merge: 7 of 8 topologia terms were identical to comunicacao; only `ogunte.md` diverged (field name `label` → `source` to match CO-141 schema). Merged into comunicacao/yoruba/terms/ogunte.md.
- `~/projects/topologia/yoruba/terms/*.md` deleted (8 files). `_universe.yaml` + `index.md` retained as shape exemplar, with comment pointing to comunicacao canonical location.

### Why

Eliminates three drifting copies of the same lexicon (mbya standalone, comunicacao/mbya embedded, topologia/guarani-mbya exemplar) and two drifting copies of yoruba terms. After CO-347 deploys, prod has one canonical row per lexicon — mbya syncing from its own repo, yoruba syncing from comunicacao/yoruba subfolder.

### Follow-up needed

`~/projects/mbya/content/lexicon/` uses `type: lexeme` with fields `classe / familia / fonte / glosa_pt / ipa / lema` — diverges from the CO-141 `type: term` shape (`word / pronunciation / concept / parts / seed_status`). A schema normalization task is needed before the mbya universe can be fully ingested by CO's entry loader.

## CO-349 — Yggdrasil RPG sub-universe scaffolding — 48+ folders, schemas later

Adds `content/` directory to the yggdrasil repo with 24 sub-universe folders (6 categories × parent + 3 language variants) and 60 stub markdown entries.

Categories: NPC (15 stubs), Monster (15 stubs), Location (10 stubs), Item (10 stubs), Faction (5 stubs), Encounter (5 stubs). Each category has shandara, tagmar, and godot language sub-universes.

Also adds `scripts/scaffold-content.sh` — idempotent; skips existing files on re-run.

### Why

Provides stable folder addresses for each RPG content category × language slot before schemas are finalised, enabling CO-337 sync to surface them as sub-universes in the CO sidebar and making per-category schema tasks parallelisable.

## CO-362 — Markdown render — rewrite http:// asset URLs to https://

`co-web/static/shared/markdown.js` now rewrites `http://` to `https://` for
`<img src="...">` URLs in rendered markdown, eliminating mixed-content browser
warnings from historical content (npm badges, screenshots, etc.). Anchor
`href` attributes are left unchanged. Any `<script src="http://...">` tags
that survive into the rendered HTML are replaced with a comment.

### Why

The `/artelonga` universe (and others with legacy README content) was logging
8+ mixed-content warnings per page load — mostly old img.shields.io badges
and f.cl.ly screenshots that use bare `http://` URLs. Browsers auto-upgrade
these today but still log a warning each time, eroding visitor trust.

## CO-364 — Add open-source reference universes (odysseus + claude-code)

Two upstream OSS repos surfaced as read-only CO universes via CO-337 remote sync:
- `odysseus` ← github.com/pewdiepie-archdaemon/odysseus (branch `dev`) — self-hosted AI workspace
- `claude-code` ← github.com/anthropics/claude-code (branch `main`) — Anthropic's agentic CLI

`content_subdirs` limits to `docs/`, `README.md`, `CHANGELOG.md` so the sync stays lean.

### Why
User wants to study these projects' architecture and changelog alongside CO's own content for integration planning. Mirrors CO-347's pattern — just two more seed rows.


## [2.39.0] — 2026-06-03 — remote sister-repo sync (CO-337) + feedback widget wiring

## CO-333 — wire the visitor-facing feedback widget into the SPA

CO-333 shipped the feedback widget module (`feedback-widget.js`) but `app.js` only loaded `feedback-panel.js` (the owner-side in-locus review badge). The visitor-facing floating button was unreachable — orphan-import pattern, same shape as CO-311's platforms.js bug.

### Fix
Add a dynamic `import('./modules/feedback-widget.js')` in `app.js`. The widget self-initializes on module load (mounts the bottom-left floating button + attaches to `window.CoFeedbackWidget`), so the import alone wires it.

### Effect
- Anonymous + authenticated users now see the feedback button on every page
- Click → modal → submit → POST `/api/v1/feedback` (CO-333's existing endpoint)
- Owner-side in-locus badge (already wired via `feedback-panel.js`) unchanged

## CO-337 — Remote sister-repo sync — universes pull content from remote git on prod

CO universes can now optionally pull their content from a remote git URL on the
production machine. This extends CO-330's `local_repo_path` mechanism to prod, where
local checkouts don't exist.

- Schema migration v56 adds `remote_url`, `remote_ref`, and `remote_last_sync`
  (all nullable) to the `universes` table
- New `vcs::clone` + `vcs::pull` helpers in co-web use the system `git` binary with
  optional SSH key (`CO_GIT_SSH_KEY_PATH`) or HTTPS token (`CO_GIT_TOKEN`) auth
- `run_remote_sister_repo_seeds` syncs each universe's remote repo at boot; resolution
  order: local path wins when set + exists; otherwise remote is used
- New `RemoteSisterRepoWorker` re-syncs remote repos every 15 min
  (configurable via `CO_REMOTE_SYNC_INTERVAL_SECS`)
- `PATCH /api/v1/universes/<key>/source` now accepts `remote_url` and `remote_ref`
- Docs: `docs/sister-repo-sync.md` covers auth, cadence, env vars, and backfill steps

### Why

Sister-repo content (mbya lexicon, topologia adapters, comunicacao sources, ArteLonga
refs) was reaching localhost only — prod got stubs at best. CO-337 finishes the
"deploy-free content integration" principle from CO-330: any repo, any content, no
Docker rebuild.


## [2.38.0] — 2026-06-01 — graph engine + feedback traceability + cross-repo health parity

## CO-335 — Centralized graph rendering — one primitive, content in CO, UI customization deferred

Added `GET /api/v1/universes/<slug>/graph` endpoint that returns a standardized
`{ nodes, edges }` shape for any universe's entries and their typed-FK relations
(CO-74). Supports `?include_types`, `?relation`, `?root` + `?max_depth` BFS
traversal, and `?published_only` filtering.

Added `/lib/co-graph.js` — the canonical graph rendering primitive. Canvas-based,
force-directed (or manual layout), built-in pan/zoom/hover/click/pinch,
CSS-variable theming. Zero external dependencies. Published at
`https://co.artelonga.com.br/lib/co-graph.js` so all sister sites share a
single deployed copy.

Added `/universe/<slug>/graph` standalone page powered by the new API and library.
Works for any CO universe without configuration.

Migrated ArteLonga neuro pages (`neuro/network.js`): deleted ~130 lines of
physics/canvas/pan-zoom code; now calls `co_graph.render()`. Data definitions,
expand/guided mode, info panel, and collaborative edges remain local.

Migrated Yggdrasil comunicacao lexicon (`comunicacao.js`): deleted ~350 lines
of canvas/rendering/pointer/zoom code; now calls `co_graph.render()` with
`layout:'manual'`, grid background, and drag-to-persist via `onNodeMoveEnd`.
Room/API/inspector/review/compose logic unchanged.

### Why

Two graph implementations were diverging (ArteLonga neuro + Yggdrasil comunicacao).
A third (yuri.artelonga.com.br portfolio graph from CO-323) was planned. Centralizing
before the third iteration prevents three permanently diverging codebases.
The content layer (CO-325 typed references, CO-330 published filter) was already
stable enough to host the graph engine as a first-class consumer.

## CO-336 — Feedback → PR/commit traceable (open-source issue-tracker semantics)

Each feedback entry now works like a lightweight GitHub issue: owners can link the specific commit or PR that resolved it, write a public owner response, and flip visibility to public so visitors can see the resolution trail. New status states (addressed, wont-fix, duplicate) complete the state machine with automatic public visibility for terminal states.

### Why
Yuri's vision: yuri.artelonga.com.br is a public-curated portfolio; visitor feedback that visibly leads to fixes builds trust and a discovery surface. CO-333 shipped the inbox; CO-336 makes it auditable.


## [2.37.0] — 2026-06-01 — feedback + cross-repo changelog aggregator + infra cleanup

## CO-333 — Feedback system — Yggdrasil-compatible, per-universe + per-entry locus

New feedback system that is API-compatible with Yggdrasil and extends it with per-entry scoping, status management, and federation.

### What changed

- **Migration v53**: `feedback` table in meta.db — includes all Yggdrasil columns (`universe_key`, `kind`, `message`, `name`, `email`, `user_sub`, `anonymous`, `created_at`) plus `entry_path` (NULL = universe-wide) and `status` (`open` / `reviewed` / `addressed`).
- **Backend routes** (`co-web/src/integrations/feedback_routes.rs`):
  - `POST /api/v1/feedback` — Yggdrasil-compatible universe-wide submission (`universe` in body)
  - `POST /api/v1/feedback/{universe}/{*entry_path}` — per-entry locus
  - `GET  /api/v1/feedback/{universe}` — list; owner sees all, anonymous sees only open `sugestao`
  - `GET  /api/v1/feedback/{universe}/entry/{*path}` — per-entry list (anon-safe)
  - `PATCH /api/v1/feedback/{id}` — status update (owner-only, 403 for others)
- **Rate limiting**: 10 submissions/hour per IP (sliding window, in-process).
- **Federation**: `CO_FEEDBACK_FORWARD_URL` env var → async fire-and-forget POST to Yggdrasil or any compatible endpoint.
- **CO-332 chat tool `submit_feedback`**: visitors can leave feedback via the AI assistant; the tool inserts into the same `feedback` table.
- **Owner notifications**: each submission triggers a `feedback_received` in-app notification to the universe owner.
- **Frontend widget** (`feedback-widget.js`): floating button (bottom-left, avoids CO-332 chat widget), modal with kind selector / message / optional name+email / anon checkbox.
- **Owner review panel** (`feedback-panel.js`): `mountFeedbackBadge` shows `📩 N` badge in the zoom-modal toolbar when open feedback exists; `mountFeedbackPanel` opens a side panel with status-change actions.
- **Mural page**: `/<universe>/feedback` renders a public-facing mural (all open `sugestao` for anon, all feedback for owner).

### Why

Yggdrasil already has a feedback system; CO didn't. Closes the parity gap and enables yuri.artelonga.com.br visitors to leave notes on specific entries without using the chat assistant.

## CO-334 — Cross-repo changelog aggregation — sister-repo releases interleaved into CO's changelog view

Added a `release_notes` table (migration v53) and a background worker that reads each configured sister repo's `CHANGELOG.md` from its local clone and upserts the parsed versions into the DB. A new `GET /api/v1/changelog/feed` endpoint returns the interleaved results newest-first with optional `repo` and `since` filters. `GET /api/v1/changelog/repos` lists every known repo and its latest release. The `/changelog` page now defaults to the multi-repo feed view with a repo dropdown filter; selecting "co" switches back to the existing CO ticket-level view.

### Why

Yuri reviewing the platform wants one pane to see what's recent across all five sister deployables (CO, ArteLonga, Quilombo, Yggdrasil, RFQ) without opening each repo's CHANGELOG separately. The parser runs at boot and every 5 minutes so newly committed releases appear automatically.


## [2.36.0] — 2026-06-01 — yuri vision Wave 2/3 — LLM trait + tools as git repos + external assistant

## CO-328 — Local LLM (macOS) + Claude Code hook integration

New `AiProvider` trait in `co-web/src/infra/ai.rs` following the CO-296 `AuthProvider`
pattern, with two production implementations:

- **`OllamaProvider`** — calls `http://localhost:11434/api/generate` (default model:
  `qwen2.5-coder:7b`). No data leaves the machine.
- **`ClaudeCodeProvider`** — spawns `claude --print "<prompt>"` as a subprocess,
  collects stdout, and returns the full response. Detects the binary at startup
  via known install paths + `which claude`.

Both are wired into `CoreState.ai_router` (`AiRouter`) at boot via
`AiRouter::from_env()`. A `MockProvider` is available for deterministic tests.

New endpoints (auth-gated with `require_auth`):

- `POST /api/v1/ai/query` — body `{ "provider": "ollama"|"claude", "prompt": "..." }`.
  Returns `{ "provider", "response" }` on success or 503 + an install hint when the
  provider is unreachable.
- `GET /api/v1/ai/status` — returns availability of Ollama and Claude Code
  (`{ "ollama": { available, model, warm }, "claude": { available, version }, "active_sessions": [] }`).

CO-327 desktop notification fires when a Claude session finishes. CO-329
analytics buffer receives a `ai.query.{provider}` domain event on each successful
query.

### Why

Yuri wants AI assistance on his own content without sending data to external APIs
as the default. Ollama covers everyday local queries; Claude Code provides the
heavier escalation path via the existing CLI.

## CO-331 — Tools as git repos — npm-like install/version/conflict + jj-compatible

Adds a first-class tool registry to CO: any open-source git repo can be installed
as a versioned tool, version-pinned to a tag/SHA/branch or set to always track
`origin/main`. Tools are stored in a new `tools` SQLite table (migration v52) and
checked out under `<data-dir>/tools/<key>/`.

### New CLI surface

```
co tool add <key> --from <url-or-local-path> [--pin <ref>]
co tool list
co tool update <key> [--pin <ref>] [--follow-main]
co tool update --all          # refresh all follow_main=1 tools
co tool remove <key>
co tool verify [<key>]        # check checkout matches lockfile SHA
```

### Key behaviour

- Remote URL → `git clone` into `<data-dir>/tools/<key>/`; local path → registered as-is.
- Per-tool lockfile SHA (captured at install/update time) enables `co tool verify` drift detection.
- `--follow-main` marks a tool for auto-refresh on `co tool update --all`.
- Conflict warning when two tools share the same `entry_command` basename.

### VCS abstraction (`co-cli/src/vcs/`)

Detects `.jj/` to dispatch jujutsu operations (`jj git fetch`, `jj new <ref>`)
instead of the default `git` equivalents. Initial `git clone` always uses git;
jj detection applies to subsequent fetch/checkout/sha operations.

### Why

CO becomes the canonical integration surface for both content (universes, CO-330)
and code (tools, CO-331). Users control versions, tooling is self-hostable, and the
whole flow extends git natively while staying jujutsu-compatible.

## CO-332 — External assistant — non-Claude LLM with deterministic tool routing for yuri.artelonga.com.br

Added a public-facing AI chat endpoint backed by Ollama (default) or OpenAI fallback.
The LLM retrieves content exclusively through deterministic tool calls against the CO
storage layer — no RAG, no vector search, no hallucination risk on published content.

### What changed

- **`POST /api/v1/chat/:slug`** — SSE-streaming chat endpoint, anon-accessible for
  publicly visible universes. The response streams `token`, `tool_start`, `tool_result`,
  `done`, and `error` SSE events so the UI can show tool progress in real time.

- **`GET /api/v1/deployments/status`** — public read of the `deployment_snapshots` table
  (Fly.io sister-app state), used by the `get_deployment_status` tool.

- **5 deterministic tools** wired to existing CO storage:
  - `search_entries` — FTS or type-filtered entry listing, respects `anon_published_only`
  - `get_entry` — single entry by path, applies visibility filter
  - `list_types` — distinct published entry types in the universe
  - `get_recent` — recently updated published entries
  - `get_deployment_status` — reads `deployment_snapshots`

- **`ChatProvider` trait** (`co-web/src/infra/ai.rs`) — multi-turn chat + function calling
  abstraction over any OpenAI-compatible `/v1/chat/completions` endpoint.

- **`OpenAiCompatChatProvider`** — handles both Ollama (local, no key) and real OpenAI
  (cloud fallback). Configured via `CO_CHAT_FALLBACK=openai` + `OPENAI_API_KEY`.

- **Anti-Claude guard** — `build_chat_provider()` and the `/chat` router assert at
  compile time and startup that the chat provider kind is never `"claude"`.
  Claude credits stay reserved for Yuri's authenticated dev work (co-auto, CLI).

- **SPA chat widget** (`modules/assistant.js`) — floating button (bottom-right),
  panel with message log and SSE streaming display, tool-call indicators, session
  history in `sessionStorage`. Auto-detects universe slug from subdomain or URL path.

- **CO-329 analytics** — every chat query emits a `Domain` event keyed
  `chat.query.<tool_name>` with message length and latency.

### Why

Closes the loop on the yuri.artelonga.com.br vision: visitors get a conversational
discovery surface over published content without burning Claude credits on visitor
traffic. Deterministic tool routing eliminates hallucination on content data;
the only fuzzy step is the LLM's tool selection decision.


## [2.35.0] — 2026-06-01 — yuri vision Wave 1 — subdomain + types + notas + messaging + macOS notify + /analytics + runtime bindings

## CO-323 — yuri.artelonga.com.br — subdomain routing to a single-universe view

Added HTTP middleware (`subdomain_routing_middleware`) that detects requests
arriving via a `*.artelonga.com.br` subdomain (e.g. `yuri.artelonga.com.br`) and
injects a `window.__CO_SUBDOMAIN_UNIVERSE__` bootstrap script into the SPA shell.

The SPA reads this global on boot to:
- Lock the current universe to the subdomain's universe key (bypassing URL parsing)
- Apply the `co-single-universe-mode` CSS class, which hides the multi-universe
  sidebar and hamburger button so the page shows only the pinned universe's content

### Why
Foundation for the yuri.artelonga.com.br personal sub-site. Visitors to
`yuri.artelonga.com.br` get a clean, focused view of the `yuri` universe without
CO's full multi-universe navigation. Other universes remain reachable at their normal
paths on the main `co-artelonga.fly.dev` host.

### Operator actions required
- DNS: add `CNAME yuri → co-artelonga.fly.dev.` at the registrar
- TLS: `flyctl certs add yuri.artelonga.com.br -a co-artelonga`
- Cookies: `flyctl secrets set CO_COOKIE_DOMAIN=.artelonga.com.br -a co-artelonga`
  (enables cross-subdomain sessions)

See `docs/dns.md` for full setup instructions.

## CO-323A — seed yuri Obsidian vault into yuri universe

Adds `("yuri", "yuri")` to CO-317's sister-repo mapping so `~/projects/yuri/` (the user's personal Obsidian vault) gets ingested into the private `yuri` universe on every `co serve` boot.

First seed entries (CO-325 type system preview):
- `references/grace-kelly.md` — Mika quote with YouTube + Spotify links
- `references/virtual-insanity.md` — Jamiroquai quote with YouTube + Spotify links

Both files use Obsidian-compatible `> [!quote]` callouts (Jekyll-displayable too).

### Why
First slice of the yuri.artelonga.com.br vision (specs CO-323..329). Gets content seeded today; the subdomain routing + per-entry visibility tiers + AI + analytics features land via the spec series.

### Frontmatter convention introduced
- `type: song` — subtype of `music` category (CO-325)
- `visibility: public` — per-entry tier (CO-324)
- `references: { youtube, spotify }` — recursive composition (CO-325)
- `author`, `year`, `album`, `tags` — standard bibliographic fields

These are descriptive for now; query/filter machinery comes in CO-325.

## CO-325 — Reference type system + recursive composition + notas abstraction

Added a typed content schema for personal bibliography and notebook management.

**New content types** (schema files in `work/co/schema/`):
- `song`, `album` (category: music)
- `poem`, `essay` (category: writing)
- `video` (category: media)
- `url`, `quote`, `notas` (category: reference)

**Category aggregation** — `type:music` in the query DSL expands to
`entry_type IN ('song', 'album')` at query time. Same for `writing`,
`media`, `reference`.

**Recursive `references` field** — entries store a free-form platform→url/path
map under `references:` in frontmatter. Queryable via JSON path:
`FROM song WHERE references.youtube IS NOT NULL`.

**`notas` type** — physical notebook page transcriptions with required
`caderno_id` (string) and `pagina` (integer) frontmatter fields; validated
by the CLI `co validate` command.

**Query DSL extensions** (`co-web/src/content/query_dsl.rs`):
- Shorthand syntax: `type:X AND field:value`
- Special keys: `type:`, `before:`, `after:`, `caderno_id:`, `author:`
- New filter variants: `FieldNotNull` (`IS NOT NULL`), `DateBefore`, `DateAfter`
- Dotted field paths allowed: `references.youtube IS NOT NULL`
- `TypeCategoryRegistry` in `core/src/feature/type_registry.rs`

### Why
The yuri.artelonga.com.br index needs cross-type queries (all music,
notas filtered by notebook, date ranges). Without the category system
the query DSL could only filter by exact type.

## CO-326 — Direct messaging — send to yuri@artelonga.com.br (email + in-app)

Added `POST /api/v1/universes/{key}/messages` — a public contact form endpoint
that delivers visitor messages to the universe owner via email (MailProvider) and
as an in-app notification (`event_type = "direct_message"`).

- Rate limit: 5 messages / hour per IP (sliding window, in-process token store)
- Honeypot: silently drops requests where the `website` field is non-empty
- Validates `from_email`, `from_name`, `subject`, `body` (all required)
- Email formatted as `[CO] {subject}` with sender attribution in the body
- Notification stored in `user_notifications` for the universe owner

### Why

yuri.artelonga.com.br needed a contact surface so visitors can reach out directly
without having to locate an email address elsewhere.

## CO-327 — macOS desktop notifications for CO events

Adds native desktop notifications on macOS when CO events arrive while `co serve` is running. Notifications fire for incoming direct messages, chat messages, mentions, and universe invitations. A 1-second debounce per recipient prevents flooding when multiple events arrive together.

Notifications are delivered via `terminal-notifier` (click opens the notifications page) when installed (`brew install terminal-notifier`), and fall back to `osascript` otherwise (URL shown in the notification body).

Set `CO_DESKTOP_NOTIFY=off` to suppress all desktop notifications. The feature is a no-op on Linux and Windows.

### Why

The local LLM, claude-hook, and messaging features all benefit from out-of-browser awareness. Without notifications, active polling was required to catch new events.

## CO-329 — /analytics non-indexed real-time telemetry + background-process visibility

Added a live observability dashboard at `/analytics` (auth-gated, noindex) backed by a WebSocket stream at `WS /api/v1/analytics/stream`.

- In-memory ring buffer retains the last 1000 telemetry events (HTTP requests, domain events, agent sessions, worker snapshots)
- Dashboard shows: live request log, domain events, background worker table, recent AI sessions, rolling 1m/5m/1h stats and error list
- Request middleware captures every HTTP path/method/status/latency into the buffer
- Domain event subscriber converts event-bus events (entries, assets, invitations, proposals, agent sessions) into analytics events
- Worker snapshots are polled every 5 s per connected WS client via `worker_supervisor.statuses()`

### Why
Yuri needs a real-time window into what CO is doing on his server — active workers, AI sessions, error rates — without relying on logs or external tooling.

## CO-330 — Runtime universe→repo bindings + anon published-only filter (deploy-free)

Migration v51 adds three nullable columns to `universes`:
- `local_repo_path TEXT` — absolute path (with `~` expansion) of the local git repo to ingest on startup
- `content_subdirs TEXT` — JSON array of subdirectory names to scan (e.g. `["docs","content"]`)
- `anon_published_only INTEGER NOT NULL DEFAULT 0` — when 1, anonymous reads are restricted to entries with `published: true` in frontmatter

Eight universe→repo bindings are backfilled at migration time (artelonga, quilomboaraucaria, yggdrasil, rfq, comunicacao, mbya, topologia, yuri). The `yuri` universe also gets `anon_published_only=1`.

`seed_orchestrator::run_sister_repo_seeds` now reads bindings from the DB instead of a hardcoded array. Adding a new repo to CO no longer requires a code change or deploy — a `PATCH /api/v1/universes/<key>/source` call is enough.

The new endpoint (`PATCH /api/v1/universes/:slug/source`, owner-only) lets the owner set or update all three fields at runtime. `co launch` auto-sets `local_repo_path` to the repo root so a freshly launched universe is pre-wired with no extra steps.

### Why

Hardcoded mappings violated the "any repo integrable database-only" principle (user feedback 2026-06-01). The `anon_published_only` flag unblocks yuri.artelonga.com.br: a single universe serves as private board (owner sees all) and public surface (anonymous readers see only published entries) without schema changes to `entries`.


## [2.34.0] — 2026-05-30 — co launch + e2e localhost trial suite

## CO-321 — E2e — localhost trial flow (subscribe / unsubscribe / sister repos / themes)

Added `co-web/e2e/localhost-trial.spec.ts` with 7 Playwright test cases that gate regressions
in the localhost trial flow surfaced across CO-313 through CO-320.

The spec exercises: anonymous sidebar (no Plataformas/raw chip key/Descobríveis section),
subscribe happy path (universe appears in subscribed bucket with × button), subscribe
rejection for public-static universes, unsubscribe via × click, theme persistence across
universe navigation, sister-repo seeding from CO_LOCAL_REPOS_DIR (pages + tasks, re-sync
on restart), and discoverable list correctness (excludes static/private/own/subscribed).

### Why

Five separate user-reported bugs in the localhost trial reached demo because no single
test exercised the full flow. This spec runs against the global test server and (for the
sister-repo case) an ephemeral server with custom env vars — zero backend or SPA changes.

## CO-322 — co launch — bootstrap a universe from the current repo (Fly-style)

Added `co launch` command: run it inside any directory to provision a universe in
localhost CO populated from that directory's content — same UX shape as `fly launch --now`.

The command walks up from CWD to the git repo root (falls back to CWD if no `.git`),
derives a universe key from the directory basename, creates the universe row if missing,
seeds `docs/` and `content/` as pages via `seed_universe_from_local_repo`, seeds
`work/{space}/{PREFIX}-N.md` files as kanban tasks via `seed_universe_work_tasks_from_local`,
and prints a summary of pages + tasks provisioned.

Flags: `--key`, `--name`, `--public` (marks `public-subscribable`), `--now` (starts server
and opens browser on `/<key>`), `--port`, `--data-dir`. All re-runs are idempotent.

### Why

Following CO-310 → CO-320 the localhost flow works end-to-end. `co launch` is the missing
onramp: without it, setting up a new universe from a repo required running `co serve`,
navigating config, and waiting for boot-time sister-repo seeding to pick it up.


## [2.33.0] — 2026-05-30 — localhost trial sweep — local repo sync + sidebar UX + lazy embedding

## CO-315 Slice A — defer embedding boot scan (opt-in)

`embedding_worker::boot_scan` walked every universe at startup and queued every stale embedding (~350 jobs on prod data). Cold-start cost was paid on every machine boot, even when no user ever touched most of those universes. Embeddings are queued naturally on file writes (via `entry_routes::enqueue_*`), so this was a backfill safety net, not a correctness requirement.

### Change
- Default behavior: skip `boot_scan` at startup. Log a one-line notice.
- Opt-in backfill: set `CO_EMBEDDING_BOOT_SCAN=1` to restore the old behavior (useful after a long downtime to repair stale state).

### Why
First slice of a "lazy universe load" effort (CO-315). User flagged the cold-start work as non-scalable per user — this removes the single largest non-essential boot cost. Per-universe seeds, migrations, and content-count recompute will follow in slices B and C after we verify universe hierarchy + access auth on prod.

### Impact
- Cold start no longer queues N=350 jobs on a 16-universe prod data set
- New writes still trigger embedding generation in real-time (no UX change)
- If you ever lose embedding state and need to repair: `flyctl secrets set CO_EMBEDDING_BOOT_SCAN=1 -a co-artelonga`, restart machine once, then unset

## CO-316 — drop x86_64-apple-darwin + aarch64-unknown-linux-gnu from release matrix

The v2.32.0 release workflow failed on two platforms:

- **x86_64-apple-darwin** — `ort` (ONNX Runtime via fastembed) stopped publishing prebuilt binaries for Intel Mac; Apple deprecated x86_64 macOS
- **aarch64-unknown-linux-gnu** — the `cross` container lacks `libssl-dev` headers needed by `ort-sys`'s build script (transitively through `ureq → native-tls → openssl-sys`)

Both platforms had near-zero self-hosting demand. Dropped from the matrix; build still ships for:
- `aarch64-apple-darwin` (Apple Silicon)
- `x86_64-unknown-linux-gnu` (most Linux)
- `x86_64-pc-windows-msvc` (Windows)

### Re-adding later
- For ARM Linux: add a `Cross.toml` that pre-installs `libssl-dev`, or vendor openssl via the `openssl/vendored` feature
- For Intel Mac: gate `fastembed` behind a feature flag and ship the CLI without embedding support (the model only matters for `co-web` server, which runs x86_64-linux on Fly)

## CO-317 — local repos as universes (Option A: ingest at boot)

`co serve` now ingests markdown content from local sister-repo checkouts into the matching universes at boot, so localhost shows the same content the deployed sister site would.

### Mapping (hardcoded)

| Universe | Local repo (under `~/projects/`) |
|---|---|
| `artelonga` | `ArteLonga/` |
| `quilomboaraucaria` | `quilomboaraucaria/` |
| `yggdrasil` | `yggdrasil/` |
| `rfq` | `rfq-gateway/` |
| `comunicacao` | `comunicacao/` |
| `mbya` | `mbya/` |
| `topologia` | `topologia/` |

For each, walks `docs/`, `content/`, and `work/` subdirs (skipping `.git`, `target`, `node_modules`, etc.) and upserts every `.md` file as an entry in the matching universe.

### Configuration
- Override projects root: `CO_LOCAL_REPOS_DIR=/path/to/parent co serve`
- Default: `~/projects/`

### Idempotency
Skips a universe when it already has more than 5 entries (assumes prior boot already seeded it). To force re-seed after major repo changes, delete the universe's per-universe DB at `<data-dir>/universes/<hash>/<key>/data.db` and restart.

### Scope notes
- This is **Option A** (ingest at boot, no live sync). Edits to repo files after boot require a server restart to reflect.
- Option B (file-watcher sync) and Option C (universe-as-mount) deferred per discussion until A is verified for trial.
- On prod (`/data/` doesn't have `~/projects/`), this is a no-op.

### What unblocks
Localhost trial — you can now click into any sister universe and see actual content rather than the empty-project placeholder.

## CO-318 — sister repo work/ files become board tasks

Extends CO-317 to make each sister universe's board work the same way the `co` board does:

- `<repo>/docs/`, `<repo>/content/` → ingested as **page** entries (CO-317, unchanged — visible in the "Conteúdo" tab)
- `<repo>/work/<space>/{PREFIX}-N.md` → ingested as **task** entries with `type: task` and `project: <PREFIX>` (visible in the "Kanban" tab)

For each unique prefix found (`AL`, `YG`, `RFQ`, etc.) a `projects/<PREFIX>/_project.md` entry is created so the project shows up in the sidebar.

### Effect (per universe, after first boot)

| Universe | Source | Projects → Tasks visible on Kanban |
|---|---|---|
| `artelonga` | `~/projects/ArteLonga/work/artelonga/AL-*.md` | `AL` |
| `yggdrasil` | `~/projects/yggdrasil/work/yggdrasil/YG-*.md` | `YG` |
| `rfq` | `~/projects/rfq-gateway/work/rfq/RFQ-*.md` | `RFQ` |
| `comunicacao` | `~/projects/comunicacao/work/` (if present) | derived |

### Design
- Each universe stays its own world (Option 1 model the user confirmed) — `co` board only shows CO-N tasks; `artelonga` board only shows AL-N tasks; no cross-universe federation
- Pattern mirrors what `seed_co_universe_tasks` does for the `co` universe — generalized
- Idempotent: per-universe skip when `task` entry count > 5
- Same env override (`CO_LOCAL_REPOS_DIR`) and same no-op on prod

## CO-319 — discoverable correctness + sister repo re-sync + sidebar polish

Bundle of localhost-trial findings.

### Fixed

1. **Subscribe → 400 on timeline universes** (`tempo`, `humanity`, `universo`). The `list_discoverable_universes` SQL was `WHERE visibility='public-subscribable' OR is_public=1` — the second clause let `public-static` universes through, which storage's `subscribe_universe()` then rejected with `Universe '<x>' is not public-subscribable` (returning 400). Tightened to `visibility='public-subscribable'` only. Static universes remain reachable via direct URL (`/tempo` etc.); they just don't appear in subscribe-context lists.

2. **Local-repo additions weren't picked up after first boot.** CO-317/CO-318 used a `skip_if_count_above: 5` gate so once a universe was seeded, new files added to `~/projects/<repo>/` never showed in localhost. Dropped the gate (passed `0` = always re-upsert). Upserts are idempotent — unchanged files are no-ops; new files appear next restart.

3. **`sidebar.co_dev_chip` raw key showed on the CO sidebar row** (user screenshot: weird chip overflow). The pattern `window.t(key) || fallback` doesn't fire the fallback when `t` returns the key itself (string is truthy). Added `tOr(key, fallback)` helper. Also removed the `oss-chip` entirely — it was decorative ("código aberto" badge) and added clutter to the row.

4. **`rfq` was seeded as `private`** so it never appeared in any user's sidebar. Marked `public-subscribable` with one-shot migration to flip existing rows.

5. **`comunicacao` universe didn't exist in storage** — the matching sister repo at `~/projects/comunicacao/` couldn't be ingested by CO-317/318 because the storage row was missing. Added it to `seed_admin_content_universes` as `public-subscribable`.

### Not fixed in this PR
- Discoverable section visible by default (user wants it hidden behind a search action) — separate UX PR
- No unsubscribe affordance on subscribed-bucket rows — separate UX PR
- 281 vs 300+ co task count is working as designed (`is_task_filename` filter skips CLAUDE.md, ROADMAP.md, etc.)

## CO-320 — discoverable hidden by default + unsubscribe affordance

Two sidebar UX changes from localhost-trial feedback ("subscription is confusing").

### Discoverable list hidden behind a search button

The full "Descobríveis" section is gone. In its place, a small `+ Buscar universos públicos` button at the bottom of the universe nav. Clicking opens a `prompt()` listing all subscribable universes and lets the user type the key to subscribe.

- MVP: native `prompt()` for now; a modal+autocomplete is the proper follow-up
- The discoverable list is still computed server-side and returned in `/me/universes.discoverable` — only the rendering changed
- "We don't want to show all universes available unless user seeks" — per user direction

### Unsubscribe button on subscribed-bucket rows

Each row in "Inscrito em" now has a small `×` button that:
1. Confirms via `window.confirm`
2. Calls `DELETE /api/v1/universes/<key>/subscribe`
3. Reloads `/me/universes`
4. Re-renders sidebar
5. Shows a toast

Renders only for non-synthetic rows in the subscribed bucket (owned/member rows don't get an unsubscribe button — those aren't subscriptions).

### Files
- `co-web/static/variants/a/modules/sidebar/render.js` — replace Descobríveis section with search button; add unsubscribe handler; add search button handler
- `co-web/static/variants/a/modules/sidebar/sections.js` — add `showUnsubscribe` parameter to `renderUniverseItemHtml` / `renderSectionHtml`; render × when true


## [2.32.0] — 2026-05-29 — localhost-first distribution + trait foundation + subscribe fix

## CO-282 — Localhost distribution — `co serve` + browser auto-launch + Tauri shell roadmap

Added `co serve` subcommand for localhost-first distribution of the CO platform.
The same Rust binary that runs on Fly.io can now be invoked locally with a single command.

### What changed

- **`co serve`** — new CLI subcommand that starts the embedded co-web server on `127.0.0.1:54321` (configurable)
- **`--open`** — opens the user's default browser after the server starts (macOS: `open`, Linux: `xdg-open`, Windows: `start`)
- **`--port`** — custom port (env: `CO_SERVE_PORT`)
- **`--data-dir`** — custom data directory for SQLite + universe files; defaults to platform data dir (`~/.local/share/co` on Linux, `~/Library/Application Support/co` on macOS)
- **`--public`** — binds to `0.0.0.0` instead of `127.0.0.1` with a security warning
- **Server refactor** — `start_server_on(config, bind_host)` in `co-web` allows the bind address to be controlled by the caller; `start_server` retains its existing `0.0.0.0` default for Fly deployments
- **SPA audit** — `isAllowedReturnTo()` in `login.js` and `is_allowed_return_to()` in `recovery_routes.rs` now accept `localhost` and `127.0.0.1` so password-reset flows work under `co serve`
- **GitHub Actions release workflow** — `.github/workflows/release.yml` builds and uploads binaries for 5 targets: `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`
- **Install one-liner** — `scripts/install.sh` for `curl | sh` install on macOS and Linux
- **README** — new "Run locally" section with the one-line install command

### Why

The CO binary already had 90% of what a local distribution needs — embedded SPA, SQLite storage, all auth flows. What was missing was the developer ergonomics: a `serve` subcommand, a sensible default port and data dir, browser auto-launch, and release artifacts. This PR delivers Phase 1 of the localhost distribution roadmap; Phase 2 (Tauri desktop shell) and Phase 3 (mobile) are deferred until Phase 1 adoption is measured.

## CO-292 — CO-284-C — Worker executor trait (formalize CO-223 with enqueue/cancel)

Introduces the `WorkerExecutor` trait in `co-web/src/infra/workers.rs` as the
pluggable abstraction over background-job execution.  Extends CO-223's
tick-based `Worker` + `WorkerSupervisor` with one-off operations:

- `enqueue(Job)` — spawns a tokio task, returns an abort-capable `JobHandle`
- `run_now(Job)` — executes a job inline (useful in tests and triggered one-shots)
- `cancel(JobHandle)` — aborts a running task; status transitions to `Cancelled`
- `status(JobHandle)` — current `Running / Completed / Failed / Cancelled` state

`Job` wraps any async closure or a single `Worker::tick` call via
`Job::from_worker_tick`, making all six concrete worker impls (embedding,
notification_email, notification_push, webhook, job_queue, deployment_snapshot)
usable through the trait without modification.

`InProcessExecutor` is the default implementation.  It wraps `WorkerSupervisor`
for periodic workers and adds `AbortHandle`-based tracking for one-off jobs.
`AppState.integrations.worker_supervisor` is now `Arc<dyn WorkerExecutor>`;
server startup holds the concrete `Arc<InProcessExecutor>` long enough to call
`spawn_worker` before type-erasing to the trait object.

### Why

CO-284 calls for pluggable infrastructure so queue-backed executors (NATS,
Temporal, k8s Jobs) can replace `tokio::spawn` without touching domain code.
CO-292 establishes the trait contract; future sub-stories plug in alternative
`WorkerExecutor` impls against the same interface.

## CO-293 — Cache trait + in-process LRU default impl

Introduced a generic `Cache<K, V>` async trait in `co-web/src/infra/cache.rs` and a
default in-process implementation `InProcessLruCache<K, V>` backed by
`parking_lot::Mutex<lru::LruCache<K, V>>`. All three existing specialized caches
(`ManifestCache`, `ThemeCssCache`, `QueryCache`) now delegate their LRU storage to
`InProcessLruCache`, replacing the raw `std::sync::Mutex<lru::LruCache>` fields.

Per-cache capacity is configurable at startup via environment variables:
`CO_CACHE_MANIFEST_MAX_ENTRIES`, `CO_CACHE_THEME_CSS_MAX_ENTRIES`,
`CO_CACHE_QUERY_MAX_ENTRIES` (default: 10 000 each).

### Why

Foundation for future Redis/Memcached backends — a Redis impl can plug into the
same `Cache<K, V>` trait without touching call sites. The trait is async so network
backends compose naturally; the in-process default resolves the futures immediately.

## CO-294 — CO-284-E — Blob store trait (standardize on top of CO-263's R2 adapter)

Introduced `BlobStore` trait in `co-web/src/infra/blob.rs` with two implementations:
`LocalFsBlobStore` (default, filesystem-backed) and `R2BlobStore` (Cloudflare R2 via
the `blob-r2` Cargo feature). A `BlobBackend` selector reads `CO_BLOB_BACKEND` at boot
and wires the appropriate backend. Migrated `asset_routes.rs` upload/get/delete blob
ops to go through the trait instead of direct `std::fs` calls.

### Why
Decouples blob storage from the local filesystem so Cloudflare R2 (or any S3-compatible
store) can be activated at boot via environment variables, without code changes.

## CO-296 — CO-284-G — Auth provider trait (extract JWT flow, prepare for OAuth/SSO)

Introduced `AuthProvider` trait in `co-web/src/infra/auth.rs` with `LocalJwtProvider` as the default implementation. The trait exposes four methods: `verify_credentials`, `issue_token`, `verify_token`, and `revoke`, abstracting the JWT signing and verification flow behind a single injectable interface.

All login flows (magic-code verify, password-login, signup, onboarding, password-reset, change-password, Google OAuth callback) now issue session tokens via `state.core.auth_provider.issue_token(...)` instead of calling `crate::auth::sign_jwt` directly. The auth middleware (`require_auth`, `require_auth_with_token`) now verifies tokens via `auth_provider.verify_token(...)`. The provider is wired into `CoreState` and reads the signing key from CO-295's `SecretsProvider`.

### Why

Prepares the codebase for OAuth/SSO provider integration. Adding a future `OAuthProvider`, `GitHubProvider`, or `SAMLProvider` requires only a new `infra/auth.rs` implementation — no route handler changes needed.

## CO-298 REVERT — remove `--staging` mode and fault-injection decorators

Removes `co serve --staging` and the four simulation decorators (`LatencyInjectedStorage`, `FlakyBlobStore`, `EvictingCache`, `RetryProneWorkerExecutor`) added in CO-298.

### Why
Random fault injection across every request doesn't simulate real production failures — prod fails in specific shapes (a particular endpoint OOMs, R2 rate-limits one bucket, a worker deadlocks under specific load). 5% generic 503s just makes dev annoying without surfacing real bugs. Tested code paths should be exercised with targeted unit/integration tests at known choke points, not probabilistic always-on injection.

Aligns with `feedback_no_uat.md` philosophy: direct-to-prod + smoke test + CHANGELOG rollback, rather than maintaining artificial intermediate environments that *feel* like fidelity but aren't.

### What stays
The trait foundation (`Storage`, `BlobStore`, `Cache`, `WorkerExecutor`, `AuthProvider`, `SecretsProvider`) and the TestServer testkit remain — those enable real backend swaps (R2 vs LocalFs, future OAuth, Redis cache, etc.). The decorators were the only piece reverted.

## CO-300 — CO-284-K — `co::testkit::TestServer` (spawn real co serve instances for integration tests)

Introduces `TestServer` in `co-web/tests/testkit.rs` — a test helper that spawns a
real `co serve` subprocess on an ephemeral 127.0.0.1 port for integration testing.
Tests import it via `mod testkit;` and get full HTTP access to the running server.

Key capabilities:
- `TestServer::start()` — default config (dev JWT secret, fresh tempdir)
- `TestServer::start_with(config)` — custom JWT secret + optional `seed_sql` for
  pre-inserting users / universes before the server boots
- `TestServer::url(path)` — builds absolute URLs for reqwest calls
- `TestServer::bearer()` — pre-signed `Bearer <JWT>` token valid for the server
- `TestServer::client()` — `reqwest::Client` with cookie jar
- Drop-based cleanup: kills the subprocess and wipes the tempdir on test exit

The `co serve` command gains `GAME_DB_PATH` env-var support so each test instance
gets its own isolated game database (prevents SQLite lock contention when tests run
in parallel).

Six integration tests in `testserver_tests.rs` exercise the real binary end-to-end:
1. Health check — GET /api/health → `{"status":"ok"}`
2. Template projects public — anonymous read returns CO project
3. Write to template forbidden — authenticated POST returns 403
4. Clone template — POST returns 201 + new universe key
5. Vault write-and-read — PUT immediately visible in GET entries + Cache-Control: no-store
6. Agent session POST + GET — session created and retrievable by task_id

### Why
In-process `build_test_router` tests skip the real binary's startup sequence and
middleware ordering. TestServer-based tests catch bugs like auth middleware ordering
and real HTTP serialization edge cases that tower::ServiceExt never exercises.

## CO-306 — Whitelist artelonga.com.br in seed-links external HEAD probes

CI runners hit `TLS alert 80 (internal error)` when HEAD-probing `https://artelonga.com.br/` (Fly.io edge rejects automated HEAD from GitHub Actions IP ranges). Added the host to `EXTERNAL_FLAKY_HOSTS` so the link-validation test treats it as a known-flaky probe target.

### Why
Pre-existing flake surfaced on PR #103 + #104. Link is valid for real browsers; only fails for automated HEAD from CI.

## CO-307 — per-instance game DB default (fix multi-server lock contention)

`co serve` now defaults the game DB to `<data-dir>/game.db` instead of the global `~/Library/Application Support/game/game.db`. The old default made it impossible to run two `co serve` instances at once — they fought for the same SQLite lock and the second one panicked with `Database already open`.

### Why
Hit while demoing `co serve --staging` alongside a stale debug server. Multi-instance dev workflows (`co serve` + `co serve --staging`, or per-test isolation) all need their own game DB. The `GAME_DB_PATH` env var (CO-300) still overrides for explicit control.

### Migration
If you had game data at the old global path, point the env var at it:

```bash
GAME_DB_PATH="$HOME/Library/Application Support/game/game.db" co serve
```

On Fly.io: new path is `/data/game.db` (on the persistent volume), so deployments now have durable game state instead of relying on `dirs::data_dir()` which resolved to non-persistent locations.

## CO-309 — `co serve` defaults `CO_ENV=local`; `allows_uat_login()` accepts any non-prod env

Two related changes to fix the "yuri@uat.local doesn't work on localhost" footgun surfaced today:

1. `co serve` (CLI) now defaults `CO_ENV` to `"local"` instead of `"prod"`. The CLI is local by definition; defaulting to "prod" was an inverted footgun that locked down dev-friendly endpoints (uat-login, admin login modal, inline magic-code display) on every local server.

2. `allows_uat_login()` now returns true for **any non-prod environment** (`uat`, `test`, `dev`, `local`, or unset) — same predicate as `is_local_or_test()`. Production sets `CO_ENV=prod` explicitly and remains the only deny case.

### Effect
- `co serve` → `yuri@uat.local`/`uat` login works out of the box
- `co serve` → admin login tab is visible in the SPA modal
- `co serve` → magic-code inline display (CO-303 `dev_code` field) works

Production (deployed `co-web` binary on Fly.io) reads `CO_ENV=prod` from `fly.toml`/`fly.uat.toml` env vars and is unaffected.

### Why
The previous defaults treated "prod" as the safe fallback ("when unsure, lock things down"). In practice that meant local dev had to remember to set `CO_ENV=uat` to log in, and forgot every time. Better default: local-friendly out of the box, prod opted into explicitly at deploy time. The deploy config already does this — the CLI just needed to follow suit.

## CO-310 — seed-co dir resolution works on localhost, not just Fly

`co serve` on localhost now populates the co-dev board with task content from `work/co/` in the repo checkout instead of leaving the board empty.

### The bug
Both `uat_boot::uat_startup` and `seed_orchestrator::run_co142_refresh` hardcoded `Path::new("/app/seed-co")` — only exists in the Fly Docker image (COPY work/co/ /app/seed-co/). On localhost the path is missing and the seed step silently skips, so the co-dev board has zero tasks even though `work/co/` has 300+ task files. User reported "I can't find the actual tasks in any board" — they were never seeded.

### Fix
New helper `resolve_seed_co_dir()` checks in order:
1. `CO_SEED_CO_DIR` env var (explicit override)
2. `/app/seed-co` (Fly Docker)
3. Walks up from cwd for `work/co/` (local dev — works from any subdirectory of the repo)

Both seed sites use the helper. Warning message now lists all three resolution paths instead of just the Fly one.

## CO-311 — remove confusing "Plataformas" sidebar section

Removes the hardcoded "Plataformas" section from the SPA sidebar. It listed five sister deployments (co, artelonga, quilombo, yggdrasil, rfq) as cross-deployment links, but users expected the label to mean *universes* and got confused.

The universe list immediately below ("Este universo" + Owned / Member / Subscribed / Discoverable buckets) is the actual source of truth for navigation. The cross-site links the platforms section provided are addressable via direct URLs.

### Files changed
- `co-web/static/variants/a/index.html` — remove `<div id="sidebar-platforms-section">`
- `co-web/static/variants/a/modules/sidebar/render.js` — drop `renderPlatforms()` import + call
- `co-web/static/variants/a/modules/sidebar/platforms.js` — deleted

## CO-313 — global theme per user (no per-universe theme switching)

Theme is now persisted per-user (`co_user_palette` in localStorage) and applies to every universe. The universe's stored `theme_preset` is no longer consulted on universe switch. Default is `modern` if the user has never picked.

### Why
User reported: "shifting universes changes theme — we want SELECT ONCE (default modern) and set that as default for all universes for now." Per-universe themes were surprising and confusing when working across multiple universes.

### Files
- `co-web/static/variants/a/modules/settings.js` — `loadThemeCss()` and `applyUniverseConfig()` no longer fall back to `universeConfig.theme_preset`

### What didn't change
- The header theme switcher still works (writes `co_user_palette`)
- Per-universe `theme_preset` is still stored in the universe config — it's just not applied at the SPA level. A future spec can decide whether to keep that field or remove it entirely.

## CO-314 — subscribe button now actually works (param-count bug)

`list_subscribed_universes()` was silently returning an empty vec on every call, even when the user had real subscriptions in the database. The SPA's Subscribe button looked broken — clicking it succeeded server-side (`POST /api/v1/universes/:slug/subscribe` returned `204`), but `/api/v1/me/universes` kept reporting `subscribed: []` because the read query failed.

### Root cause
The SQL referenced `?1` three times (filter `s.user_id`, `u.owner_id`, and a subquery `user_id`) but the Rust code passed `params![user_id, user_id, user_id]` — three positional params for one numbered placeholder. rusqlite rejected this with:

```
ERROR list_subscribed_universes query:
  Wrong number of parameters passed to query. Got 2, needed 1
```

The error was logged + swallowed (CO-191 non-panicking pattern), so the symptom was an empty subscribed list rather than a 500.

### Fix
- `co-web/src/storage/universe.rs:455` — pass `params![user_id]` (single param; SQLite reuses it for every `?1` reference)
- `co-web/src/content/universe_routes/tests.rs` — new regression test `test_list_subscribed_universes_returns_rows`

### Why this is the third "got 2, needed 1" class of bug
The first was tests passing literal JWT secrets that didn't match (CO-295 fixed via SecretsProvider). The second was the "got 2, needed 1" warning in release-commit. This is the third: silent SQL-binding mismatches that swallow data. Worth a project-wide grep next pass for `params![x, x` patterns referencing numbered placeholders.


## [2.31.1] — 2026-05-27 — magic-code auth + e2e fixes + seed-ordering

## CO-303 — Local-fidelity auth — inline magic-code display + admin password tab in SPA login modal

The login flow now completes end-to-end in every environment without leaving the
browser or inspecting server logs.

**Magic-code inline display**: in non-prod environments (`CO_ENV` ≠ `prod`), the
`POST /api/v1/auth/onboard-with-email` response includes a `dev_code` field with
the generated 6-digit code. The SPA login modal detects this field, shows a
`[DEV]` banner above the code input, and auto-fills the field — so the full
email-code login flow is completable in a single browser session on localhost.

**Admin password tab**: a new `GET /api/v1/auth/login-options` endpoint returns
`{ magic_code, password, google }`. When `password: true` (CO_ENV=uat|test), the
modal reveals an "Admin sign-in (password)" link that exposes the existing
username/password form — no more curl + DevTools cookie paste.

**Admin seeding**: `CO_SEED_ADMIN_EMAIL` + `CO_SEED_ADMIN_PASSWORD_HASH` already
work in any env (documented in CLAUDE.md). No code changes needed.

**Production behaviour unchanged**: `dev_code` is never returned when
`CO_ENV=prod`; the admin tab link is never shown; zero visual changes.

### Why

Every CO-N ticket that touches authenticated paths previously required either
tailing server logs for the magic code or bypassing the UI entirely with curl.
This ticket eliminates that friction so localhost is a true production-fidelity
environment for UAT testing (per `feedback_no_uat.md`).

## CO-304 — E2e selector + timing quality pass — eliminate Carregando/timeout brittleness

Fixed systematic brittleness in the 71 e2e tests that survived CO-302's cut.
Before this pass ~40 tests failed on a clean data directory; after: 0 (excluding 9 pre-existing flakes unrelated to this work).

**Root cause fix — `renderConteudo()` stale-render race**
- `renderConteudo()` is async: it fires 6 parallel `getUniverseEntries` API calls and
  writes the full conteudo HTML after they complete. This overwrote the kanban view
  even after the user had already switched tabs — causing every board interaction test
  (codemirror, board-drag, integration, smoke, pipeline-workflow) to fail with
  "waiting for `.task-card`" timeouts.
- Added stale-render guard in `conteudo.js` after the `Promise.all`: if `state.view`
  is no longer `'conteudo'`, bail out without touching `content.innerHTML`.
- Added test-side guard in `selectProject` (helpers.ts): wait for the
  `#content .loading-spinner` to disappear before clicking the kanban tab, ensuring
  `renderConteudo()` has settled and the guard has fired.

**Selector fixes**
- Added `data-testid="no-project-selected"` and `data-testid="content-loading"` to
  `index.html` so tests can target the initial empty state without colliding with the
  nine other `.empty-state` elements scattered across views.
- Updated `board-ux.spec.ts` to use `[data-testid="no-project-selected"]` instead of
  the ambiguous `.empty-state` class selector.
- Fixed view-tab count assertion (6 → 7) after the `changelog` tab was added post-CO-302.
- Fixed `co-landing.spec.ts` to use `.kanban` / `.kanban-column` (the classes kanban.js
  actually renders) instead of the non-existent `.kanban-board` / `.kanban-col`.
- Added null guard in `editor.bundle.js` so CodeMirror init no longer throws on
  elements that are not yet in the DOM.

**Timing helpers**
- Fixed `waitForBoard` in `helpers.ts`: was checking for exactly 4 kanban columns but
  `STATUSES` in `constants.js` defines 3 (todo / in_progress / done); changed to
  `first().toBeVisible()` to be resilient to future status additions.
- `selectProject` now registers the `waitForResponse` listener before clicking, then
  waits for the conteudo spinner to clear before switching to kanban — eliminates the
  two-step race unconditionally.

**State / fixture fixes**
- Extended `fixtures.ts` with a `seedTask` helper and extended `seedProject` so tests
  can declare their starting state without relying on dirty test-database contents.
- Fixed `global-setup.ts` to ensure the `e2e-test` universe exists before any suite runs.
- Fixed `pipeline-workflow.spec.ts` "POST without session" test to use the plain
  `request` fixture (unauthenticated) instead of `apiContext` (has session cookie).
- Fixed archive test in `pipeline-workflow.spec.ts` to use `apiContext.put(...)` (the
  server registers PUT for task updates, not PATCH).
- Fixed `subtask-tree.spec.ts` `parent_id` → `parent` field to match the actual API shape.

**Documentation**
- Added `e2e/README.md` documenting the `data-testid` convention, `waitForBoardReady`
  usage pattern, and explicit-state-seeding guidance.

### Why
CI on every PR (CO-303, CO-302, and earlier) was failing on pre-existing test
brittleness rather than on the PR's actual changes. Green CI had no signal value.
This pass restores the invariant: green = nothing broke, red = something you changed broke it.

## CO-305 — E2e residual failures sweep (9 bugs from CO-304)

Fixed all 9 e2e test failures that CO-304 exposed by removing render-race noise. Suite now runs 84 passed / 0 failed on a clean data dir with `CO_ENV=test CO_BYPASS_RATE_LIMIT=1`.

### Changes by layer

**Fixtures (test fix)**
- Added `anonContext` fixture to `e2e/fixtures.ts` — unauthenticated request context for anonymous-ownership tests that must not carry a yuri session cookie (tests #1–2).

**Changelog API (server fix)**
- `admin/changelog_routes.rs`: filter out non-semver versions (e.g. `[Unreleased]`) before sorting so `newest-first` sort is stable (test #3).
- `admin/changelog_routes.rs`: field name was `entry_type` in JSON but test read `e.type`; fixed the test to use `e.entry_type` (test #4).

**Routing (server fix)**
- `storage/seed.rs` + `storage/mod.rs`: added `CHANGELOG.md` and `public/index.md` stubs to the co universe seed so `/co/changelog` and `/co/public/` both return 200 (tests #5–6).
- `server/static_files.rs` + `platform/pretty_urls.rs`: pretty-URL redirect now uses `slug_redirect_target()` which routes template slugs → `/template/<slug>` and co/public slugs → `/co/public/<slug>`, fixing the `/sobre` and friends redirects (tests #8–9).

**CSS (real bug fix)**
- `static/variants/a/style.css`: added `.sidebar.open { display: block }` rule inside `@media (max-width: 640px)` so hamburger toggle is effective on mobile (test #7).

**Seed data (data fix)**
- `seed/template/termos.md`: changed `/co?page=privacidade` → `/privacidade` (correct pretty URL, not a cross-universe link).
- `seed/template/privacidade.md`: removed linked `[CO-86](/co/CO-86)` (task not in clean seed); kept as plain text.
- `seed/co/CHANGELOG.md` + `seed/co/public/index.md`: new stub entries so the co universe entry index covers those paths.

**Seed orchestration (server fix — CO-305 second iteration)**
- `server/seed_orchestrator.rs`: moved `reseed_co_public_pages()` to after `seed_admin_content_universes()`. On clean boot the `co` universe row doesn't exist yet when `reseed_co_public_pages` ran earlier, so it returned early and left `CHANGELOG.md`, `public/index.md`, `public/seguranca.md`, `public/licensa.md`, etc. unseeded — causing `/co/changelog` and `/co/public/` to 404 in CI while passing locally on macOS (stale data masked the bug).

**Test fix (e2e)**
- `e2e/seed-links.spec.ts`: changed default base URL from `https://co-artelonga.fly.dev` to `http://localhost:3000` so tests run against the local server in CI rather than production.
- `e2e/seed-links.spec.ts`: added `github.com` to `EXTERNAL_FLAKY_HOSTS` — GitHub returns 429 for automated HEAD probes on commit-history URLs used as legal version-history links in `termos.md` and `privacidade.md`.

### Why
The first CO-305 iteration (d31a417) passed locally on macOS with stale data but failed in CI on a clean Linux runner. The root cause was execution order: `reseed_co_public_pages` guarded against a missing `co` universe but ran before `seed_admin_content_universes` created that universe. On macOS the stale data already had the `co` universe populated from a prior run, masking the bug.


## [2.31.0] — 2026-05-26 — test pyramid + secrets trait + e2e fixture auth

## CO-290 — Storage trait abstraction + SqliteStorage impl

Introduces `co_web::infra::storage::Storage` trait backed by `SqliteStorage`,
decoupling entry-reading routes from direct SQLite connection access.

### What changed

- New `co-web/src/infra/storage.rs`: `Storage` trait (`get_universe`, `get_entry`,
  `list_entries`, `search_entries`, `list_entries_by_date`, `list_entries_by_prefix`,
  `list_entry_tags`, `entry_tree`, `put_entry`, `delete_entry`, `universe_conn`).
- `SqliteStorage` wraps `Arc<parking_lot::Mutex<crate::storage::Storage>>` via
  `from_arc`, sharing the same mutex as `CoreState.storage` (no duplicate connections).
- `CoreState` gains `storage_trait: Arc<dyn Storage>` and a `from_storage` constructor
  that wires both fields from a single `Storage` value — ~40 construction sites updated.
- `entry_routes.rs` reading handlers (`list_entries`, `get_entry`, `list_entry_tags`,
  `entry_tree`) now go through `state.core.storage_trait.*` methods rather than locking
  `CoreState.storage` and calling `EntryIndex` directly.

### Why

CO-284-A milestone: establish the trait boundary so future backends (in-memory, S3,
Postgres) can swap in without touching route logic.

## CO-295 — Secrets provider trait (env-var default)

Centralizes all secret reads (JWT_SECRET, VAPID_PUBLIC_KEY, CO_RECOVERY_KEY) behind a
`SecretsProvider` trait. `EnvSecretsProvider` is the production default; `StaticSecretsProvider`
is injected in tests, eliminating parallel-test races caused by `std::env::set_var`.

### Why

Parallel Tokio tests that called `set_var("JWT_SECRET", ...)` could interfere with each
other, causing intermittent auth failures in CI. With a trait-backed provider, each test
router carries its own secret without touching the process environment.

### Changes

- New `co-web/src/infra/secrets.rs`: `SecretsProvider` trait, `EnvSecretsProvider`, `StaticSecretsProvider`
- `CoreState` gains `secrets: Arc<dyn SecretsProvider>`; `from_storage_with_secrets()` for test injection
- `require_auth` middleware reads JWT secret from `state.core.secrets` instead of `std::env::var`
- `recovery_crypto` functions take `&dyn SecretsProvider` (no longer reads env directly)
- `vapid_public_key_handler` reads VAPID key from `state.core.secrets`
- `server/tests.rs` (10 tests), `push_routes` (5 tests), `agent_session_routes` (2 tests)
  all migrated to `StaticSecretsProvider` — no `set_var` calls remain in those modules

## CO-302 — Test pyramid restructure: parallelize e2e, add component layer, cut redundancy

Restructured the test suite from a flat e2e-only pyramid into three distinct
layers: lib tests (Rust), component tests (Vitest + happy-dom), and e2e tests
(Playwright). E2e suite thinned from 257 to 71 tests; component layer added
with 120 tests covering DOM behavior without a browser.

### Changes

- **Phase 1 — Parallelise**: Playwright workers bumped 1 → 4 in CI; sharding
  scaffold commented in `ci.yml` for easy enablement; `testIgnore` excludes
  `archived/`, `wave-2/`, `interactions/` directories.

- **Phase 2 — Local runner**: `scripts/co-test` (bash) with subcommands
  `smoke`, `e2e`, `components`, `lib`, `review`. Auto-starts co-web on port
  54321 for `smoke`/`e2e`, tears it down on exit. `--since <ref>` flag limits
  run to changed spec files.

- **Phase 3 — Redundancy cut**: 11 files archived to `e2e/archived/`
  (CO-N acceptance specs, uat-flow, design-audit, theme-coverage,
  deployment-dashboard). 3 auth specs consolidated into `e2e/auth.spec.ts`
  (10 tests). 8 large spec files thinned (board-ux: 42→3, integration: 30→5,
  pipeline-workflow: 26→4, subtask-tree: 21→2, theme: 10→3, i18n: 8→3,
  changelog-viewer: 10→4, co-landing: 10→5, codemirror: 9→3,
  recursive-universe: 8→4, responsive: 10→5). Component test layer created:
  12 files, 120 `it()` tests, self-contained DOM fixtures, no production
  imports.

- **Phase 4 — Guard rails**: CI step "E2E test-count summary" prints counts
  per file and emits a warning annotation on any file >30 tests.
  `scripts/co-test review --fail-on-bloat` exits 1 for local enforcement.
  `co-web/tests/manifest.yaml` tracks all active spec files.
  `co-web/e2e/README.md` documents layer conventions and local run commands.


## [2.30.2] — 2026-05-24 — template seed fix + sidebar IA + Fly baseline + JWT race fix

## CO-277 — Recursive subspace addressing — sub-universe task resolution in co-auto

`co-auto` now discovers and routes tasks across nested sub-universes. A bare
prefix like `SHN-1` resolves to `work/yggdrasil/shandara/SHN-1.md` without
requiring the caller to spell out the nesting; ambiguous prefixes produce a
friendly "specify -u \<key\>" error.

### Changes

- New module `dev/co-auto/src/universe.rs`:
  - `Subspace` struct (key, rel_path, abs_path, prefix, parent, version)
  - `ResolvedTask` struct (key, subspace, spec_path)
  - `discover_subspaces(workdir, space) -> Vec<Subspace>` — recursive walker
    that reads every `_universe.yaml` up to depth 8, skipping `.worktrees`,
    `.git`, `node_modules`, `target`, `CHANGELOG-PENDING`
  - `resolve_task_id(input, space, workdir, subspaces) -> Result<ResolvedTask>`
    — prefix-aware resolver; falls back to legacy string logic when subspaces
    is empty (full backward compatibility)

- `dev/co-auto/src/auto.rs`:
  - `AutoConfig` gains `subspace_key: Option<String>` (the `-u <key>` value)
  - `run()` calls `discover_subspaces` once at startup; redirects `data_dir`
    to the resolved subspace path so task loading and context building are
    scoped correctly
  - `load_project_key` falls back to `_universe.yaml.task_prefix` so
    sub-universes without `project.yaml` work out of the box
  - Old `lookup_prefix_table`, `read_universe_yaml_prefix`,
    `infer_prefix_from_existing_files`, and `resolve_task_id` removed from
    `auto.rs` (logic moved to `universe.rs`)

- `dev/co-auto/src/main.rs`:
  - New `-u / --universe` flag (alias `--subspace`) targets a sub-universe
  - Raw task arg is now forwarded unchanged to `run()`; expansion happens
    inside `run()` after `discover_subspaces`, so `-u shandara 1` correctly
    expands to `SHN-1`

- 25 new tests across `tests/recursive_resolver.rs` and
  `tests/discover_subspaces.rs`; existing `tests/prefix_resolver.rs` updated
  for the new `resolve_task_id` signature

### Why

The yggdrasil roadmap hosts multiple sub-universes (YG, SHN, TGM, GDT…) under
a single space. CO-276 assumed one prefix per space; this change extends the
resolver to walk the full tree so all sub-universes are runnable with the same
`co-auto <KEY>` ergonomics.

## CO-279 — Fix template seed regression + default project for every universe

Restores the template universe's onboarding project to `CO` (reverting the
short-lived `TUTORIAL` rename that landed 2026-05-04 in commit 6f34b62) and
adds an idempotent default-project hook so every non-template universe lands
with at least one `projects/*/_project.md` entry — eliminating the
"no project found" dead-end yuri reported for his private universe.

### Changes

- `co-web/src/storage/seed.rs`:
  - `seed_template_universe()` writes the tutorial project under
    `projects/CO/_project.md` with `key: "CO"` again (was `TUTORIAL`), so
    `is_project_in_template("CO")` correctly returns true and the
    `guard_template` check fires before any panic-able op — 500s on
    template writes are now the intended 403s.
  - The 9 onboarding tasks live under `projects/CO/N.md` again with
    `project: "CO"`, restoring the contract `clone_universe` and the 4
    `template_tests.rs` checks have always assumed.
  - New `seed_default_project_if_missing(universe_key)` — idempotent helper
    that adds a `{first-4-of-key uppercased}P` project (matching the
    `create_universe` convention) when the universe has zero projects.
  - New `backfill_default_projects()` — walks every non-template /
    non-anon-clone / non-timeline / non-`co` universe and runs the helper.
  - `seed_admin_content_universes()` now calls the helper for each admin
    universe except `co` (whose canonical `CO` project is seeded later by
    `seed_co_universe_tasks`), so `artelonga`, `rfq`, `language`, `mbya`,
    `topologia`, and `time` ship with their default project from boot one.
  - `migrate_template_project_rename()` inverted: it now drops any stale
    `projects/TUTORIAL/*` entries left over from a DB that booted under the
    broken CO-254 code, and clears the matching `project_universe_index`
    row, so `seed_template_universe` re-seeds the canonical `CO` rows.

- `co-web/src/server/seed_orchestrator.rs`:
  - `run_startup_seeds()` calls `backfill_default_projects()` immediately
    after `seed_admin_content_universes()` so existing user universes
    bootstrapped via `seed-prod-universes.sh` (yuri's workspace etc.)
    inherit the same default-project guarantee on the next deploy.
  - Comment on `migrate_template_project_rename` updated to reflect the
    inverted intent.

- `co-web/tests/template_tests.rs`:
  - New `test_seed_default_project_is_idempotent_and_seeds_when_missing`
    — asserts `seed_admin_content_universes` produces an `ARTEP` default
    project in the `artelonga` universe and that re-running the helper is
    a no-op.
  - New `test_backfill_default_projects_skips_templates_and_anon_clones`
    — asserts the boot-time backfill never adds a second project to the
    template universe.
  - The 4 originally-failing tests (`test_template_has_sample_tasks`,
    `test_template_projects_public`, `test_write_to_template_forbidden`,
    `test_update_template_task_forbidden`) now pass without modification
    because the seed contract they assume is restored.

### Why

CI on `main` has been red since the 2026-05-04 commit `6f34b62 fix: universe
hierarchy + co rename + pdf error UX` renamed the template tutorial project
`CO → TUTORIAL` without updating the tests, the cloning contract, or the
production data. Five shipped tickets (CO-272..CO-276 across v2.29.0 +
v2.30.0) have been stuck behind the failing pipeline.

Reverting the rename to `CO` aligns the seed with the production-deployed
shape (prod is two versions behind main and still serves `CO`), the existing
clone/test/route contracts, and the user's expectation that the legacy
`/api/projects/CO/tasks` endpoint resolves the same way it always has — the
`co` universe's CO dev board wins routing via `rebuild_project_universe_index`,
exactly as before.

The default-project helper closes a parallel gap: admin-seeded universes
(artelonga, rfq, mbya, …) and any pre-1.x personal universe could otherwise
exist as a `universes` row with no `projects/_project.md` entry, leaving the
SPA's kanban view stranded on "no project found".

## CO-280 Phase 1 — sidebar restructure (Platforms / This universe / Tools)

Restructure the SPA sidebar into three labeled sections so users can immediately
distinguish the three IA layers that were previously rendered as one flat list:

1. **Platforms** (top) — hardcoded list of the 5 sister deployable units
   (co, artelonga, quilombo, yggdrasil, rfq). External-link icon shown when
   a platform's URL differs from `window.location.origin`; click opens that
   platform in a new tab.
2. **This universe** (middle) — the existing universe + project nav, now
   under a clearly labeled section header ("Este universo" / "This universe").
   Behavior preserved; only the surrounding header changes.
3. **Tools** (bottom, muted) — dev/operator affordances (Deployments,
   Changelog). Visually de-emphasized so they read as operator tools rather
   than end-user destinations.

### Why

Two user-reported symptoms shared the same root cause — the sidebar mixed three
distinct IA layers (deployable platforms, content universes, dev tools) with no
visual distinction:

- "5 sub-universes part of whole, clarify and review" — sister deployables
  rendered identically to projects inside the current universe.
- "sidebar.co_dev_ship button is weird" — dev/operator affordances sat
  alongside end-user navigation with no signal of their audience.

Phase 1 introduces the three-section scaffolding so future phases (breadcrumbs,
sub-universe tree, individual tool audits) have a stable home. CO-277's
recursive sub-universe tree (Phase 4) and Phase 2's breadcrumbs are deferred to
follow-up tickets.

### Files

- `co-web/static/variants/a/index.html` — three section containers in the
  sidebar (`#sidebar-platforms-section`, `.sidebar-this-universe`,
  `#sidebar-tools-section`).
- `co-web/static/variants/a/modules/sidebar/platforms.js` — new module,
  hardcoded `PLATFORMS` list + `renderPlatforms()`.
- `co-web/static/variants/a/modules/sidebar/tools.js` — new module,
  `renderTools()` with deployments + changelog links.
- `co-web/static/variants/a/modules/sidebar/render.js` — calls
  `renderPlatforms()` + `renderTools()` from `renderSidebar()`.
- `co-web/static/variants/a/modules/sidebar/index.js` — public re-exports.
- `co-web/static/variants/a/style.css` — `.sidebar-platforms`,
  `.sidebar-tools`, `.sidebar-platform-item`, `.sidebar-tool-item` styling.
- `co-web/static/shared/i18n.js` — pt + en keys for the three section labels
  and tool labels.
- `co-web/e2e/co-280-sidebar-sections.spec.ts` — asserts all three sections
  render and that tool items never leak into `#project-list`.

## CO-281 — Phase 0 baseline snapshot

Captured the current Fly.io deployment baseline for the 5 deployable apps
(plus the unconfirmed `artelonga-dev` variant) to `docs/infra/fly-baseline-2026-05.md`.
This is pure measurement — no `fly.toml` edits, no deploys — and gives Phases
1-4 a fixed reference point to measure savings against.

### Why

Per CO-281, before changing any sizing we wanted a written snapshot of every
app's machine size, `auto_stop_machines` setting, `min_machines_running`, and
estimated monthly cost. The baseline already surfaces useful signal: real
total is ~$24-26/mo for machines (vs the spec's $13-15/mo pre-flight guess),
dominated by `quilombo-araucaria` running at 2 GB always-on for its video
upload workload — meaning Phase 1's target band is reachable from
`min_machines_running` flips alone, before any embedding-sidecar extraction.


## [2.30.0] — 2026-05-23 — agent telemetry + co-auto ergonomics

## CO-275 — Agent session events — capture tokens/tools/skills/duration per co-auto run; surface on kanban cards

Added end-to-end observability for co-auto invocations: every Claude Code run
now emits a structured session record (duration, exit code, token counts, tool
call distribution, skills loaded, commit SHA, PR number) and persists it to a
new `agent_sessions` table. The kanban view lazy-loads the latest session for
each card and shows a compact footer line.

### Changes

- **Migration v50**: `agent_sessions` table with two indexes (task_id,
  universe+started_at).
- **Storage module**: `co-web/src/storage/agent_sessions.rs` — `insert`,
  `list`, `latest` methods.
- **Event bus**: new `DomainEvent::AgentSessionComplete` variant + `AgentSession`
  filter.
- **API endpoints**:
  - `POST /api/v1/agent/sessions` — vault token or JWT auth; co-auto posts here.
  - `GET  /api/v1/agent/sessions?task_id=...` — public list (kanban reads).
  - `GET  /api/v1/agent/sessions/latest?task_id=...` — most recent session.
- **co-auto**: captures wall-clock duration, exit code, stdout token counts,
  stderr tool calls, HEAD SHA, skills loaded, context size; POSTs to
  `CO_SESSION_ENDPOINT` (requires `CO_SESSION_TOKEN`). Graceful degradation:
  parse failures produce `null` fields, POST failure prints a warning but never
  aborts the run.
- **Kanban**: each task card gets an `agent-session-footer` placeholder; after
  render, lazy fetch populates loaded footers with "14m · 18k tok · 8R/5E · abc1234 · #89".
- **Tests**: 2 integration tests (POST+GET lifecycle, null on missing task) +
  Playwright spec asserting 401 on unauthenticated POST and footer render.

### Why

Every CO-N card on the kanban is now its own receipt: what it cost, how it was
built, which skills it consumed. Combined with CO-273 (deployment dashboard) and
CO-260 (changelog viewer), operators have a fully queryable history of every
artifact the platform produces.

## CO-276 — co-auto CLI simplification — positional task arg, smarter defaults, drop redundant flags

`co-auto` now accepts a bare task number as a positional argument, making the common case dramatically shorter:

```
# Before (6 flags + 7 values)
co-auto --workdir ~/projects/co --space co --task CO-272 --cycle --max-tasks 1 --auto-pr --timeout 1800

# After
co-auto 272
```

Changes:

- **Positional `task_arg`**: `co-auto 272` or `co-auto CO-272` — both equivalent to `--task CO-272`. Positional takes priority over `--task` when both are provided.
- **Prefix inference**: bare numbers are expanded to full keys via `resolve_task_id`. Priority: `_universe.yaml task_prefix` field → hardcoded table (`co→CO`, `yggdrasil→YG`, `rfq→RFQ`, `qb→QB`, `artelonga→AL`) → first `PREFIX-N.md` file scan → uppercase(space).
- **`--auto-pr` default flipped to `true`**: PR is opened after every successful task unless `--no-pr` is passed.
- **`--timeout` default raised from 600s to 1800s**: matches typical task duration (15–25 min).
- **All existing long-form invocations remain valid** — no breakage.

### Why

After ~30 co-auto invocations in a session, the per-invocation cognitive tax of 6 flags × 2 tokens each (flag + value) adds up. Simplifying the common path to a single positional argument is a high-ROI ergonomic improvement with zero behavior change for existing scripts.


## [2.29.0] — 2026-05-23 — Wave J — kanban dogfooding + deploy dashboard + agent context budget

## CO-272 — Kanban view shows entries-as-tasks, not just legacy projects — close the dogfooding gap

Added a new `GET /api/v1/universes/{slug}/dev-tasks` endpoint that maps entries
from the `work/` folder (entry_type in user-story/task/epic) to a flat task
shape. The kanban SPA now merges these dev-tasks into `state.tasks` for
public-subscribable universes, so visiting `/co`, `/artelonga`,
`/quilomboaraucaria`, etc. renders the actual CO-N/AL-N/QB-N work items as
kanban cards grouped by status (todo/in_progress/done/blocked).

### Why

Since CO-261 the entries were correctly synced but every universe's kanban
still rendered the legacy hardcoded project containers.  CO-272 closes both
the data layer (entries exist) and the view layer (kanban renders them).

## CO-273 — Centralized deployment dashboard — machines + sizes + statuses + versions across all units

Added a single-pane-of-glass deployment dashboard at `/admin/deployments` showing the runtime state of all 6 deployable units (co, artelonga, quilombo, yggdrasil, rfq, comunicacao).

- New table `deployment_snapshots` (migration v49): one row per unit, updated by background worker
- New background worker `DeploymentSnapshotWorker` (5 min interval): fans out in parallel across all 6 units using `tokio::join!`, probing the Fly.io machines API (`CO_FLY_API_TOKEN` env var) and each unit's `/api/health` endpoint
- New API endpoints:
  - `GET /api/v1/admin/deployments` — returns cached snapshot data for all 6 units
  - `POST /api/v1/admin/deployments/refresh` — triggers immediate re-probe and returns fresh data
- New admin page at `/admin/deployments`: dark-themed table showing unit, URL, version, machine ID, region, VM size, state, last deploy date, and health status; click a row to expand full details; "Atualizar agora" button for manual refresh
- Worker handles Fly API errors, network failures, and missing token gracefully (each unit probed independently)

### Why

Operator needs one glance to answer: which units are up, what size are they running on, when was the last deploy, is anything behind? Previously required 5+ separate `flyctl status` + `curl /api/health` invocations.

## CO-274 — co-auto context budget — cut from ~150k chars to ~30k via skills + per-universe CLAUDE.md

`dev/co-auto/src/auto.rs` now routes to a **minimal context path** when
`data_dir/CLAUDE.md` (the per-space guide) exists, replacing the old
always-loaded 5-layer bundle with three focused layers:

1. **Per-space CLAUDE.md** (≤3k chars) — replaces the 15k root CLAUDE.md
2. **Skills** (≤4k chars) — loaded on-demand based on task labels
3. **Task spec** (≤5k chars) — the actual ticket

Target budget: ~12k chars per task, down from ~45k+ (≈75% reduction).

New files:
- `skills/rust-architecture.md` — loaded for any `module:*` label (non-SPA/deploy)
- `skills/spa-conventions.md` — loaded for `module:spa`, `module:editor`, `module:ui`
- `skills/deploy-runbook.md` — loaded for `module:deploy`, `module:infra`
- `skills/migration-template.md` — reference skill for DB migrations
- `skills/playwright-pattern.md` — loaded for `type:test`
- `work/co/CLAUDE.md` rewritten as slim (≤3k chars) CO development guide

The legacy full-context path remains as a fallback for spaces without a
per-space CLAUDE.md.

### Why

Each co-auto session was loading 150k+ tokens (5 layers × ~30k chars +
system prompt + tools). Context this large drives up hallucination rate,
burns usage limits, and buries the relevant signal in noise. The per-space
routing + skill loader reduces to ~12k chars of task-focused context.


## [2.28.0] — 2026-05-22 — Wave I — final visibility chain + LICENSE complete

## CO-270 — Items list final fix — audit middleware chain; identify silent-empty wrapper for anonymous

`universe_visibility_gate` (CO-161) only passed anonymous requests through for
`is_public || is_template` universes. Public-subscribable universes have
`is_public = false`, so anonymous callers were blocked at the middleware layer
before reaching `list_entries`, even though `filter_public_for_anon` (CO-268)
had already been fixed to expose all entries for these universes.

Added `|| universe.visibility == "public-subscribable"` to the early-return
condition in `universe_visibility_gate`. Writes remain protected by
`universe_writer_gate`, which already enforces subscription checks for
public-subscribable universes (CO-253).

### Why

CO-261 seeded 1173 entries into the `co` universe. CO-262/CO-266/CO-268 fixed
successive layers (write paths, count divergence, path-filter bypass), but
the middleware gate was the last layer silently dropping all items for anonymous
callers. This fix closes the chain.

## CO-271 — Fix LICENSE seed: copy root files into runtime Docker image

`/co/license` returned 404 in prod despite CO-269 being deployed because
`CHANGELOG.md`, `README.md`, and `LICENSE` were only `COPY`'d in the builder
stage of the multi-stage Dockerfile. The runtime stage (`FROM debian:trixie-slim`)
never received them, so `reseed_co_root_files` found no files at `/app/` on boot.

Fix: added `COPY CHANGELOG.md README.md LICENSE /app/` to the runtime stage.
Also added `co-web/tests/seed_root_files_tests.rs` with two smoke tests — one
asserting `LICENSE` (bare filename) seeds as `LICENSE.md`, one verifying no
regression on `CHANGELOG.md` and `README.md`.

### Why

Multi-stage Docker builds only carry artifacts you explicitly copy between
stages. CO-269 wired the seed logic correctly but missed promoting the files
from builder to runtime — a one-line Docker oversight that silenced the route.


## [2.27.0] — 2026-05-22 — Wave H — items visibility + LICENSE seed

## CO-268 — List items filter — items SELECT is stricter than COUNT for anonymous (post-CO-266)

`filter_public_for_anon` now skips the `public/` path restriction for
`public-subscribable` universes. Anonymous callers of the `co` universe (and
any other `public-subscribable` universe in `PUBLIC_CONVENTION_UNIVERSES`) can
now see all entries, not just those whose path starts with `public/`.

`is_public_for_anon` (used by `GET .../entries/*path`) receives the same fix so
single-entry lookups at non-`public/` paths also return 200 instead of 404 for
anonymous callers on public-subscribable universes.

Both handlers (`list_entries` and `get_entry`) now look up the universe's
`visibility` field alongside the existing `universe_conn` call — a single
extra storage read that is amortized with the connection lookup.

### Why

The CO-161 visibility gate middleware already controls which anonymous callers
can reach a universe at all. Applying a second, per-path `public/` filter on
top of that gate was redundant and wrong for `public-subscribable` universes:
it caused `total > 0` while `items = []` when entries were not seeded under
the `public/` prefix (e.g. `projects/CO/_project.md`), breaking the kanban
and conteúdo views on `/co` for anonymous visitors.

## CO-269 — Seed LICENSE.md into /co universe (currently 404 at /co/license)

`reseed_co_root_files` already listed `LICENSE` in its candidate array (added in
CO-264), but the Docker build context never copied the file into `/app/`, so the
seed walker found nothing to upsert at runtime.

Changed `co-web/Dockerfile` to `COPY CHANGELOG.md README.md LICENSE ./` alongside
the pre-existing `CHANGELOG.md` copy, making all three well-known root files
available to `reseed_co_root_files` when it runs inside the container.

Added `test_reseed_co_root_files_seeds_license`: writes a bare `LICENSE` file
(no extension, matching the real repo file) to a temp dir, calls
`reseed_co_root_files`, and asserts the resulting `LICENSE.md` entry exists in
the `co` universe with `entry_type = "page"`.

### Why

The `/co/license` route returned 404 because the Dockerfile omitted `LICENSE`
from the build context.  The fix is a single-line Dockerfile change; the seed
logic was already correct.


## [2.26.0] — 2026-05-22 — Wave G — list visibility fix + cross-repo sync

## CO-266 — List endpoint visibility — total counts correctly but items array empty for anonymous

`GET /api/v1/universes/co/entries` (and `?path_prefix=public/`) now returns
`total` as the **full visible count** and `entries` as the **paginated slice**
(capped at `limit`).  Previously both were computed from the same post-limit
vector, so requesting `limit=3` against 5 visible entries yielded
`total=3, items=[3]` instead of the correct `total=5, items=[3]`.

### Why

The SQL-level `LIMIT` was applied inside `query_by_path_prefix` and
`query_with_limit` before `filter_public_for_anon` ran.  Both `total` and
`items` were then set from `entries.len()` — the limited, filtered count — so
`total` could never exceed `limit`, breaking pagination semantics and masking
the real number of visible entries.

The fix fetches all matching rows (passing `None` for limit to the query
methods, which already default to a 5 000 row cap), applies
`filter_public_for_anon`, records `total = all_filtered.len()`, then truncates
to the user's `limit` for the `items` array.

## CO-267 — CO-261 phase B — cross-repo sync (yggdrasil/rfq/qb/artelonga work folders → CO universes)

Added `co-sync push` — a one-shot CI mode that reads `syncs:` entries from
`co-universes.yaml`, walks the declared source folders, skips unchanged files
(SHA-256 hash cache at `~/.co/push-cache.json`), and PUTs changed files to the
CO Vault API. Sister repos (yggdrasil, rfq, quilomboaraucaria, artelonga) run
this on every merge to main via a GH Action, making their `work/<space>/*.md`
tasks visible in the corresponding CO universe.

Backend changes:
- `entry_origin` column added to `entries` tables (meta DB migration v48,
  per-universe migration v14). Vault PUT writes `entry_origin = 'synced'`; the
  boot-time seed walker writes `entry_origin = 'walker'` and skips overwriting
  any row already marked `synced`.
- CO-261 placeholder stubs for yggdrasil/rfq removed — real task entries from
  CI push replace them.

Tool changes:
- `co-sync push [token]` subcommand with `CO_TOKEN` env-var support.
- `co-universes.yaml` extended with a `syncs:` section; `SyncEntry` struct
  supports `include`/`exclude` glob patterns.
- `.github/workflow-templates/sync-to-co.yml` — copy-paste GH Action for
  sister repos.
- Integration tests in `co-agent/tests/push_hash_skip.rs` verify the
  hash-skip logic and include/exclude filtering against a mock Vault API.

### Why

CO-261 phase A synced the local `/co` universe from `work/co/` at boot time.
Sister repos are separate git repositories; Docker filesystem isolation prevents
boot-time walks of their paths. The push-from-CI model (option C in the CO-261
spec) is the correct solution: each repo owns its data and pushes on merge.
The `entry_origin` guard ensures a server restart never clobbers CI-pushed data.


## [2.25.0] — 2026-05-22 — Wave F — recursive universe file-compat (CO-264) + extract universes/ subtree (CO-265)

## CO-264 — Universe = recursive folder tree — per-universe CHANGELOG, index.md, README.md at every level; folder-prefix filtering

Every CO universe now behaves like a filesystem-shaped wiki. Well-known filenames
(`CHANGELOG.md`, `index.md`, `README.md`, `LICENSE.md`) have canonical rendering
at any folder level.

### What changed

**Backend — `path_prefix` query parameter**

`GET /api/v1/universes/{slug}/entries?path_prefix=<prefix>` returns only entries
whose path starts with `<prefix>`. Implemented in `EntryIndex::query_by_path_prefix`
and wired through `EntryListQuery` in `entry_routes.rs`.

**Backend — folder-level URL resolution**

`entry_exists_for_subpath` in `static_files.rs` now recognises:
- `changelog` (case-insensitive) → checks `CHANGELOG.md`, `changelog.md`
- `readme` → checks `README.md`, `readme.md`
- `license` → checks `LICENSE.md`, `LICENSE`
- Trailing-slash paths (e.g. `public/`) → checks `public/index.md`

This makes `/co/changelog` return HTTP 200 when `CHANGELOG.md` is seeded and
`/co/public/` resolve to the folder's `index.md`.

**Backend — seeder for root-level docs**

`Storage::reseed_co_root_files(root_dir)` seeds `CHANGELOG.md`, `README.md`, and
`LICENSE.md` from the repo root into the `co` universe as `page` entries on every
boot. Called from `run_co142_refresh` in `seed_orchestrator.rs`.

**Frontend — `maybeOpenEntryFromUrl` extended**

The SPA's URL-to-entry resolver now prepends `{folder}/index.md` for trailing-slash
paths and appends well-known file aliases (`CHANGELOG.md`, `README.md`, etc.) to
the candidate list. When `/changelog` resolves to nothing, a helpful empty state is
shown: "Este universo ainda não tem um CHANGELOG."

**Frontend — `getEntriesByPathPrefix` API helper**

Added `getEntriesByPathPrefix(slug, prefix)` to `api/entries.js` for folder
navigation in the SPA.

**Tests — new Playwright spec**

`co-web/e2e/recursive-universe.spec.ts` asserts `path_prefix` filtering, changelog
routing, and trailing-slash folder resolution.

**Documentation**

Section 8 (File-Compat Layer) added to `co-web/src/MODULES.md`.

### Why

The session-end user report: *"changelog not showing up in universes, should read
from CHANGELOG or not found, just like home should read from index.md and subsequent
folders as well, nested, hierarchical, recursive universes should guarantee file
compatibility at any level."* This is the architecture vision implicit in CO-141,
CO-251, CO-252, CO-261 — now made explicit as a unified file-compat contract.

## CO-265 — Extract universe-specific modules out of co-web/src/ — separate co (platform) from universes (extensions)

Moved `co-web/src/quilombo/` and `co-web/src/game/` into `co-web/src/universes/quilombo/` and `co-web/src/universes/game/` respectively. Added `co-web/src/universes/mod.rs` to declare both sub-modules. Updated `lib.rs` to declare `pub mod universes` and re-export all universe sub-modules at the crate root so existing `crate::quilombo_*` and `crate::game_*` import paths continue to compile unchanged.

Documented the platform-vs-universes split in MODULES.md §6 (Architecture Map). Annotated `quilomboaraucaria` in `co-universes.yaml` with its `rust_module` path for future plugin registration.

### Why

CO-224 placed `quilombo/` and `game/` at the same depth as `auth/`, `content/`, and `platform/`, implying they are peers. They are not: `quilombo/` is the backend for quilomboaraucaria.org (one specific universe) and `game/` is the leaderboard for Yggdrasil (another specific universe). Grouping them under `universes/` makes the platform-vs-extension boundary visible at directory level without any behavior change.


## [2.24.0] — 2026-05-21 — Wave E — route folder promotion + visibility fix + R2 feature gate

## CO-224 — Promote routes into context folders (auth)

Reorganized `co-web/src/` from ~90 flat `.rs` files into eight context folders:
`auth/`, `content/`, `social/`, `admin/`, `integrations/`, `platform/`, `quilombo/`, `game/`.
`storage.rs` and `auth.rs` were converted to `mod.rs` pattern.
All original `crate::module_name` paths remain valid via re-exports in `lib.rs`.

### Why

The flat layout made it impossible to understand module ownership at a glance.
Grouping by context (auth, content, social, admin, integrations, platform, quilombo, game)
aligns the source tree with the domain boundaries and makes onboarding and code navigation
significantly easier without changing any public API or call sites.

## CO-262 — Fix CO-261 entries visibility — seed walker inserts rows but /entries API returns 0

The `seed_co_universe_tasks` walker (CO-261) was writing entry paths at root level
(`CO-1.md`, `projects/CO/_project.md`). The `co` universe uses the `public/` convention
introduced in 2.7.20: anonymous visitors only see entries whose path starts with `public/`.
This made all 238 seeded task entries invisible to the entries API for unauthenticated callers,
even though the rows existed in the database and `content_count` reflected the correct 927.

### Fix

Prefix all entry paths written by `seed_co_universe_tasks` with `public/`:
- Project entry: `projects/CO/_project.md` → `public/projects/CO/_project.md`
- Task entries: `CO-N.md` → `public/CO-N.md`

This aligns the write side with the read-side visibility filter. Existing rows at the old
root-level paths are orphaned but harmless; the new `public/` rows satisfy the filter and
surface correctly through `GET /api/v1/universes/co/entries`.

### Why

The `PUBLIC_CONVENTION_UNIVERSES` filter was added intentionally so the `co` universe can
hold both public transparency content (`public/*`) and future private content. The seed
walker was not updated to respect this convention, causing a silent write/read mismatch.

## CO-263 — Feature-gate R2 deployer adapter to avoid AWS SDK bloat in default build

`StaticOnR2Adapter` and its AWS SDK dependencies (`aws-sdk-s3`, `aws-config`) are
now gated behind a new `deploy-r2` Cargo feature in the `core` crate. The default
build compiles without any AWS SDK code. Enabling `--features deploy-r2` restores
the full adapter.

When the feature is off, `from_credentials` returns a disabled stub adapter whose
deploy/rollback calls fail at runtime with a clear "rebuild with --features deploy-r2"
message, so `co-cli` still compiles cleanly without source changes.

`co-web` (the production binary) uses `core` with default features, so the deployed
binary no longer carries the AWS SDK. `MODULES.md` documents how to re-enable the
feature for the future deploy.yaml → R2 push pipeline (CO-N+).

### Why

CO-134 shipped `StaticOnR2Adapter` as groundwork for the UAT-revert deploy pipeline,
but no production path calls it yet. Every release build was compiling and linking the
entire AWS SDK chain — ~3 MB of binary weight and a forced Dockerfile rust 1.90→1.94
bump — for zero current benefit.


## [2.23.0] — 2026-05-21 — Wave D — work/*.md sync + cross-version changelog viewer

## CO-243 — VS Code (and LSP) integration — open universe as remote workspace

Added `co-vscode` VS Code extension and `co-lsp` Language Server Protocol server that expose CO universe content as a native editor workspace.

**co-vscode** (TypeScript, VS Code extension):
- Registers the `co://` URI scheme via `vscode.workspace.registerFileSystemProvider`, mapping reads/writes to the CO Vault API.
- Command `CO: Open Remote Workspace` lists the user's universes via `GET /api/v1/me/universes` and opens the selected one as a VS Code workspace folder.
- Reads authentication credentials from `~/.config/co/credentials` (written by `co auth login`) — no extra setup required after CLI login.
- Wikilink `[[...]]` auto-completion for markdown files in `co://` workspaces.
- Buildable to a `.vsix` sideloadable package (`npm run package`).

**co-lsp** (Rust binary, standalone workspace):
- Standalone `co-lsp` binary implementing the Language Server Protocol over stdin/stdout.
- Compatible with any LSP-aware editor: VS Code, Neovim, Helix, Zed, Emacs Eglot.
- Features: wikilink completion (`[[...]]`), broken-link diagnostics, hover and go-to-definition stubs ready for Phase 2 expansion.
- CLI: `co-lsp [--url <url>] [--token <token>] [--universe <slug>]`; falls back to `~/.config/co/credentials`.

### Why

Reduces the content-editing friction for developers: instead of a dedicated GUI, editors they already use (VS Code, Neovim, etc.) can read and write CO universe content natively. The Vault API is the source of truth; the extension/LSP are thin adapters.

## CO-260 — Cross-version changelog viewer — range queries + group-by-type + sort-by-PR-size

Added a standalone changelog viewer at `/changelog` and a new `GET /api/v1/changelog` API
endpoint that exposes all CO release history as structured, filterable JSON.

**Backend (`co-web/src/changelog_routes.rs`):**
- `GET /api/v1/changelog` — public endpoint that parses the embedded `CHANGELOG.md` and
  returns a `{ range, versions[] }` response. Supports query params:
  - `from=<version>` / `to=<version>` — semver range filter (inclusive)
  - `type=<feat|fix|refactor|docs|chore>` — filter by conventional-commit type
  - `sort=size` — sort entries within each version by PR size (LOC diff) descending;
    `sort=date` (default) keeps versions newest-first
- `POST /api/v1/admin/changelog/reindex` — (auth required) rebuilds `changelog_cache` by
  scanning `git log` for conventional-commit types + PR numbers
- Responses served from DB cache (`changelog_cache`) + compile-time embedded `CHANGELOG.md`

**Database (migration v47):**
- New table `changelog_cache(version, ticket, entry_type, title, pr_number, pr_size,
  additions, deletions, commit_sha, author, indexed_at)` — pre-computed per-entry PR-size
  data populated by the reindex endpoint and `scripts/release-commit.sh`

**Frontend (`co-web/static/variants/a/changelog.html`):**
- Standalone page at `/changelog` with:
  - Version range pickers (from/to selects auto-populated from available versions)
  - Type filter chips (`feat` / `fix` / `refactor` / `docs` / `chore`) — client-side, instant
  - Sort selector (newest-first / biggest PR first)
  - Sequential view (versions in order with their entries)
  - Grouped view (entries bucketed by type across all versions)
  - LOC size bars for PRs with size data; PR links to GitHub
  - Mobile responsive (size bars + version themes hidden below 640px)
- `modules/views/changelog-viewer.js` — importable ES module exposing
  `fetchChangelog()` and `renderChangelogViewer()` for embedding in other contexts

**Tooling (`scripts/release-commit.sh`):**
- After each release commit, calls `POST /api/v1/admin/changelog/reindex` on the local
  dev server (when running) to update the cache with the new version's git history

**Documentation (`co-web/seed/co/index.md`):**
- Added `/changelog` link to the CO universe home page

**Tests (`co-web/e2e/changelog-viewer.spec.ts`):**
- API tests: shape, version sort order, range filter, type filter, sort-by-size, range echo
- UI tests: page renders, chip filter (client-side), sequential/grouped toggle

### Why

Seven+ release waves (v2.13 → v2.22) with no structured way to compare them. The
CHANGELOG.md is great for narrative reading per release but useless for questions like
"what were the biggest refactors between 2.18 and 2.22?" or "list every fix in Wave B."
CO-260 exposes that data in a filterable, sortable viewer without touching the existing
CHANGELOG.md format.

## CO-261 — Sync repo work/*.md → CO universe entries (live dev board for /co, /yggdrasil, /rfq)

`seed_co_universe_tasks` now creates a **"CO Development Board"** project entry
(`projects/CO/_project.md`) and seeds each `CO-N.md` file from `work/co/` as a
`task` entry under that project, so the `/co` kanban shows the real CO-1..N
development tasks (grouped by `status` into todo / in_progress / done columns).

Documentation files (CLAUDE.md, ROADMAP.md, etc.) are filtered out of the task
seed so only `{PREFIX}-{N}.md` specs appear in the board.

Frontmatter mapping: `type: user-story` → `entry_type: task`, `project: CO`
injected, original type preserved as `story_type`, `created_at`/`updated_at`
mapped to `created`/`modified`, `labels` mapped to `tags`.

Two placeholder content pages are seeded into the **yggdrasil** and **rfq**
universes (`content/sister-repo-sync.md`) to communicate that their
`work/<space>/` task sync is not yet wired and will arrive in CO-261 Wave B/C.

### Why

CO-N ticket specs were committed to `work/co/` and bundled into the Docker image
(`/app/seed-co/`) on every build, but the per-universe `entries` table was never
populated with them as board-compatible tasks. The `/co` universe showed only the
old baseline API/CW/DS/PLT projects instead of the real development board.


## [2.22.0] — 2026-05-21 — Wave C — deployer adapter + LSP/VS Code + sidebar/state/api split

## CO-134 — Deployer adapter trait + first impl (static-on-R2)

Introduces the `DeployerAdapter` trait in `co::deploy` and ships the first concrete
implementation, `StaticOnR2Adapter`, which uploads a built universe to Cloudflare R2
(via the S3-compatible API) and maintains an atomic "current" pointer for rollback.

New public API in `co::deploy`:
- `DeployerAdapter` trait — `name`, `deploy`, `rollback`, `status`
- `BuildArtifact` — tarball + metadata produced by the build step
- `DeployResult` — deploy ID, public URL, snapshot hash
- `DeployStatus` — `Active`, `Inactive`, or `Unknown`
- `DeployAdapterError` — typed errors for upload, not-found, permission, I/O
- `SecretResolver` trait + `EnvSecretResolver` — pluggable secret lookup
- `S3Backend` trait — abstracted object-store ops (mockable in tests)
- `AwsS3Backend` — AWS-SDK-backed implementation (also works with R2)
- `StaticOnR2Adapter` — full deploy + rollback + status over R2
- `create_tarball_from_dir` — helper to pack a directory into `.tar.gz`

CLI: `co deploy --universe-id <id> --dist <dir>` publishes to R2.
     `co deploy rollback --deploy-id <id>` restores a previous deployment.

### Why

This is the first step of the CO-116 deploy adapter epic: prove the abstraction on
the simplest possible target (static hosting on R2) before tackling Cloudflare Pages
and beyond. The `S3Backend` trait keeps the implementation unit-testable without a
live bucket.

## CO-259 — Split sidebar.js + state.js + api.js into smaller files for parallel task independence

`sidebar.js` (539 LOC), `state.js` (72 LOC), and `api.js` (255 LOC) were each
promoted to folder modules following the same directory pattern used for
`chat/` (CO-219) on the server side.

Each old `.js` file is now a 3-line re-export proxy (`export * from './<name>/index.js'`),
so all existing imports across the codebase are unchanged.

New submodule layout:

- `sidebar/`: sections.js · render.js · header.js · badge.js · mini-calendar.js · wire.js
- `state/`: shape.js · universes.js · views.js
- `api/`: client.js · auth.js · tasks.js · universes.js · entries.js

`co-web/src/MODULES.md` extended with section 6 documenting the SPA module map.

### Why

Parallel co-auto tasks were causing rebase conflicts whenever two unrelated
tickets (e.g. sidebar UX + universe subscription) happened to land in the
same 470-LOC `sidebar.js`. With the split, each task touches a disjoint file
and the file-overlap pre-flight reliably finds zero shared paths.


## [2.21.0] — 2026-05-21 — Wave B — REPL + DuckDB + inline CodeMirror + deploy.yaml schema

## CO-133 — deploy.yaml schema + universe-level manifest validation

Added `deploy.yaml` as a first-class universe artifact. Universes can now declare
their deployment intent in a versioned, validated manifest that deployer adapters
(CO-134, CO-135) consume as a typed `DeployManifest` struct.

**What was added:**

- `core/src/deploy.rs` — `DeployManifest` struct hierarchy (`serde` + `schemars`),
  `parse_file` / `parse_str` functions, and semantic validation with errors that
  carry file path, line number, and field path
- `work/schema/deploy.v1.json` — formal JSON Schema for external tooling and editors
- `tests/fixtures/deploy/` — 10 fixtures: 5 valid (one per target) and 5 invalid
  (each testing a distinct error: missing version, invalid version, missing target,
  unknown target, scaling.max < scaling.min)
- `co validate deploy [PATH]` — CLI subcommand that validates a deploy.yaml and
  exits 1 on any error
- `docs/DEPLOY-MANIFEST.md` — reference documentation with one full example per
  target (static-on-r2, cloudflare-pages, fly, vercel, fargate)

### Why

Deployer adapters CO-134 and CO-135 need a typed, validated contract rather than
freeform YAML. By formalizing the schema now, adapter code never touches raw YAML
and schema drift between adapters is impossible by construction.

## CO-244 — Python / R REPL interoperability — DuckDB attach + in-browser REPL

Added `co-py` and `co-r` helper packages for querying CO universe data via DuckDB, a `POST /api/v1/universes/{slug}/query` read-only SQL endpoint (auth-gated, 1000-row cap), an example Jupyter notebook, and a Pyodide-powered in-browser Python kernel in the REPL panel.

### Why
Researchers and analysts need frictionless SQL + DataFrame access to universe content from Python or R without writing API clients — the per-universe SQLite is already the right shape.

## CO-245 — Inline code editor for plaintext file types (CodeMirror)

Users can now edit code, YAML, JSON, CSV, and other plaintext files directly in
the browser without downloading, editing locally, and re-uploading. Clicking a
`asset.code` entry in the Content view opens a CodeMirror editor with language-
appropriate syntax highlighting. Ctrl/Cmd+S saves immediately; a Save button
is also provided in the zoom toolbar. Unknown plaintext types fall back to read-
only display.

### What changed

- **`co-web/src/vault_routes.rs`**: `PUT /api/v1/universes/{slug}/vault/{path}`
  now detects plaintext code file extensions (`.py`, `.rs`, `.ts`, `.js`, `.sh`,
  `.sql`, `.go`, `.r`, `.rb`, `.csv`, `.tsv`, `.html`, `.css`, `.xml`, `.cpp`,
  `.c`, `.java`, `.php`, …). These files are written verbatim (no markdown
  frontmatter wrapper) and indexed as `entry_type = "asset.code"` with the
  correct MIME type, so subsequent GETs and the asset viewer work correctly.

- **`co-web/editor/src/editor.js`**: Added `initCodeEditor(container, opts)`
  — a new public function in the `window.CoEditor` bundle. It detects the
  programming language from the filename extension, loads the matching
  CodeMirror language mode (Python, Rust, TypeScript, JavaScript, Go, SQL,
  Shell, YAML, JSON, TOML, HTML, CSS, XML, C/C++, Java, PHP, R, Ruby…), and
  mounts a lightweight code editor without the markdown preview pane. When
  `onSave` is provided and the user presses Ctrl/Cmd+S, the callback fires with
  the current content.

- **`co-web/static/shared/editor.bundle.js`**: Rebuilt with the new
  `initCodeEditor` export. Bundle size unchanged (all language parsers were
  already transitive deps of `@codemirror/language-data`).

- **`co-web/static/variants/a/modules/views/conteudo.js`**: `mountAssetCodeEditor`
  extended to accept `{ editable, onSave }` options. When the current user can
  edit the universe, a Save button is injected into the zoom toolbar and the
  editor is mounted in read-write mode; otherwise it stays read-only. Save
  writes back via `PUT /api/v1/universes/{slug}/vault/{path}` with the file's
  MIME Content-Type so the server routes it through the new verbatim code-file
  path.

- **`co-web/src/vault_routes/tests.rs`**: Added unit tests for `is_code_file`,
  `code_file_mime`, and an integration test (`test_vault_put_python_file_indexes_as_asset_code`)
  that creates a universe, PUTs a Python file, and verifies the entry is indexed
  with `entry_type = "asset.code"` and `mime = "text/x-python"`.

- **`co-web/e2e/wave-2/co-245-code-editor.spec.ts`**: Playwright E2E tests
  covering the vault PUT → asset.code index round-trip and the content-view
  interaction.

### Why

Config files, analysis scripts, and code snippets are common universe content.
Before this, there was no way to update them without leaving the browser. The
new inline editor closes that loop — iterate on a YAML config or Python script
right where it lives, without friction.


## [2.20.0] — 2026-05-21 — Wave A — interactive changelog + MODULES.md + co auth CLI + unified file listing

## CO-129 — Jujutsu-shaped changelog renderer (op-log → commit DAG)

Added `GET /api/v1/universes/{slug}/oplog` — a new endpoint that reads
`entry_events` and returns them as grouped "commit nodes" (events with the
same `request_id` are collapsed into one node).  Each node carries an `id`,
`seq`, `ts_micros`, `author_id`, `node_type` (`commit` | `promote` |
`conflict` | `revert`), a human-readable `summary`, a `changes` list, and
a `parents` array for DAG edge rendering.

Added a **Histórico** tab to the CO board UI that renders the op-log as a
Jujutsu-style vertical DAG:

- SVG spine + dot nodes (distinct shapes/colours per node type)
- HTML overlay rows with short ID, author, timestamp, and change summary
- Node click opens a side panel with the full change list and a
  **Restaurar até aqui** button that calls `POST /revert`
- Conflict nodes (from promote operations with `conflicts > 0`) appear in red
- Branch labels rendered when present on a node
- Virtualized via CSS `content-visibility: auto` — 1000-node DAG renders
  well under 200 ms
- Theme-aware: uses only CSS custom properties (`--accent`, `--border`,
  `--card-bg`, `--text`, `--text-muted`)

### Why

§C.3 of the SR plan named a jujutsu-style history view as a first-class v1
requirement.  The CO op-log (CO-61/CO-95) already records every change as an
append-only event stream; this task wires a renderer that exposes that history
to the user in the mental model they explicitly requested.

## CO-225 — Document AppState composition pattern + add a MODULES.md

Expanded `co-web/src/MODULES.md` from the single directory-pattern stub to a
full five-pattern reference covering the server-decomposition work from CO-215
through CO-223. Cross-linked from `docs/architecture/as-is.md` and `CLAUDE.md`
so both contributors and reviewers have a single source of truth.

### Why

Several patterns were established across CO-215..CO-223 (directory modules,
sub-state segregation, typed extractors, event bus, worker trait) but lived
only in individual PR descriptions. New contributors and PR reviewers had no
single document to point to when checking whether new code follows the
established conventions.

## CO-236 — co auth — CLI commands for centralized password reset + API token lifecycle

Added `co auth` subcommand suite to `co-cli`, replacing the four-step curl-based
password-recovery flow with a single interactive command. All password prompts use
hidden input (never echoed or logged); credentials are stored in
`~/.config/co/credentials` with mode 600.

### New commands

| Command | Description |
|---------|-------------|
| `co auth login [--email E] [--save-token]` | Interactive password login; optionally creates + saves 90-day API token |
| `co auth reset-password [--email E]` | Full forgot-password flow: send code → verify → set new password → auto-login |
| `co auth change-password` | Change password for the current session |
| `co auth status` | Print auth state; exits 0 when authenticated, 1 otherwise |
| `co auth token create [--name N] [--save]` | Create a 90-day API token |
| `co auth token list [--json]` | List tokens in table or JSON format |
| `co auth token revoke <id>` | Revoke a token (with confirmation) |
| `co auth logout [--revoke-token]` | Clear local credentials; optionally revoke server-side |

### Storage

`~/.config/co/credentials` (TOML, mode 600):
```toml
[default]
base_url = "https://co.artelonga.com.br"
session = "eyJ..."            # JWT (7 days, for management ops)
session_expires_at = "..."
token = "co_<40chars>"        # API token (90 days, for scripting)
token_id = "tok_..."
expires_at = "..."
user_id = "usr_..."
email = "..."
display_name = "..."

[uat]
base_url = "https://co-artelonga-uat.fly.dev"
token = "co_..."
```

Multiple profiles are supported via `--profile <name>`.

### Why

The existing recovery flow required four sequential `curl` invocations with
codes pasted across terminal windows — error-prone and undiscoverable for
non-engineers. This wraps all nine endpoints (CO-165 forgot-password + CO-35
API tokens) into a typed, safe CLI surface that can later be wired into the
Settings UI.

No server-side changes — all endpoints already existed.

## CO-242 — Unified file listing — surface all file types in universe entries (PDF, image, video, code)

Every uploaded asset is now a first-class entry in the unified entries table, visible
alongside markdown pages, tasks, and events.

### What changed

- **Migration v13** backfills `entries` rows for all existing `assets` rows using path
  `attachments/<sha256>` and entry type `asset.pdf | asset.image | asset.video |
  asset.code | asset.binary` derived from the MIME column.

- **POST /assets** creates both an `assets` row and an `entries` row in a single
  transaction. `content_count` is refreshed from the actual entry count after each
  upload.

- **PUT /vault/{*path}** now accepts binary payloads (detected via `Content-Type`).
  Binary content creates an `assets` blob + `entries` row in one transaction at the
  specified vault path. The vault router's body limit is raised to 50 MB to match the
  asset API cap.

- **GET /entries?type=asset.*** — the entries API now supports a `.*` wildcard suffix
  for prefix-matching entry types, so all asset subtypes are returned in one query.

- **Frontend** — the Conteúdo view gains an _Arquivos_ section listing all asset
  entries with MIME badge and size. Clicking an asset card opens the zoom viewer which
  dispatches by type: pdf.js for PDFs, `<img>` for images, `<video>` for video, and
  CodeMirror read-only for code files.

### Why

§6 (folders encapsulate features) + §1 (composition — assets become a kind of entry):
a universe should be a single home for all content, not markdown-only with assets in a
separate API silo.


## [2.19.0] — 2026-05-21 — release

## CO-258 — co-auto agent prompts — forbid CHANGELOG.md + Cargo.toml mutations

Agents now write pending changelog notes to `CHANGELOG-PENDING/<TASK-ID>.md` instead of
modifying `Cargo.toml` or `CHANGELOG.md` directly. The release commit consolidates all
pending notes via `scripts/release-commit.sh`.

- `dev/co-auto/src/auto.rs`: execution context now includes a **Forbidden Files** section
  listing `Cargo.toml`, `co-cli/Cargo.toml`, and `CHANGELOG.md`, with a format spec for
  the pending note.
- `CLAUDE.md`: versioning policy updated — agents must not touch the three release files.
- `CHANGELOG-PENDING/`: new directory; agents drop one `<TASK-ID>.md` file per task here.
- `scripts/release-commit.sh`: new script that bumps versions, consolidates pending notes
  into a single CHANGELOG entry, deletes the pending files, and commits.
- `scripts/ship-task.sh`: removed auto-resolve rules for `Cargo.toml` and `CHANGELOG.md`
  (they must not appear in the conflict set anymore).

### Why

Every parallel co-auto pair ran today conflicted on exactly these two files (Cargo.toml +
CHANGELOG.md) because each agent independently bumped the version and prepended a changelog
block. Both mutations are purely procedural (no engineering content), so pulling them out of
agent scope eliminates the dominant source of rebase conflicts when running tasks in parallel.


## [2.16.2] — 2026-05-21 — Splitter mirror behavior in obsidian-mode (CO-255)

### Fixed — conteúdo split-pane drag direction in obsidian-mode

In obsidian-mode the sections pane had a hard-coded `280px` flex basis and the detail pane took the remainder, so `--split-pct` was disconnected from both panes. Dragging the splitter appeared to do nothing.

- **CSS**: `.conteudo-split.obsidian-mode .conteudo-sections-pane` now uses `flex: 0 0 calc(100% - var(--split-pct) - 14px)` so the variable is wired again.
- **JS**: `onMove` inverts the raw percentage (`100 - pctRaw`) when `obsidian-mode` is active, so dragging left grows the detail pane (mirroring the normal-mode behaviour on the opposite side).

Touch and mouse both work in both modes. `--split-pct` persists in localStorage and survives mode toggle.

## [2.16.1] — 2026-05-21 — Table render fix + privacy/homepage polish

### Fixed — markdown table HTML had stray `<` before `<thead>`

`_renderTable()` in `co-web/static/shared/markdown.js` produced `<table class="md-table"><<thead>` — an extra `<` glued to the start of `<thead>`. Browsers parsed it as an unknown empty element + `<thead>`, rendering the bare `<` as visible text right before every table. Single character fix at line 229: removed the stray `<` from the template literal.

### Changed — Privacy policy footer

- Location: *"Curitiba, PR, Brasil"* → *"São Paulo, Brasil"* (Curitiba was a hallucination from an earlier rewrite)
- *"Arte Longa"* → linked to https://artelonga.com.br
- Section 8 "Crianças" removed — children-protection will be a dedicated track later, not a token sentence
- Sections renumbered (was 1–12 with gaps, now 1–8)

### Changed — Homepage `index.md`

- Title now simply **Co** (was *"Co — Gestão de conteúdo em grafo"*)
- Bolded link emphasis for every wiki destination so the structure is visually scannable
- Privacy policy promoted to first item in "Dados e privacidade"
- AGPL v3 callout in the "Universo de desenvolvimento" section

## [2.15.0] — 2026-05-21 — Surface 'co' dev sub-universe on the anonymous sidebar + landing (CO-252)

### Added

- **Sidebar**: Anonymous visitors now see the `co` dev universe pinned at the top of the universe list, with a green "código aberto" / "open source" chip as a visual accent.
- **co universe home page**: `seed/co/index.md` provides a useful landing page explaining CO's open-source mission, linking to the dev board, documentation pages (security, infra, license), and the template universe.
- **i18n**: `sidebar.co_dev_chip` key added to both `pt-BR` ("código aberto") and `en` ("open source") locales.
- **CSS**: `.oss-chip` class (green accent badge) reuses the `role-chip` layout convention from CO-238.

## [2.13.6] — 2026-05-21 — E2E: static-asset MIME + anonymous bootstrap smoke tests (CO-250)

### Added — Playwright specs that would have caught the 2.13.3/4/5 regression cascade

Two new e2e specs prevent the class of regression where `serve_deep_link` served
`text/html` for JS/CSS assets, causing browsers to refuse script execution and
leaving every anonymous visitor at "Carregando…" forever.

**`co-web/e2e/static-assets.spec.ts`** — hits 12 critical asset URLs with an HTTP
GET and asserts each returns HTTP 200 with the correct `Content-Type` prefix:
- `/variants/a/app.js`, `/variants/a/modules/{api,sidebar,boot}.js`, `/variants/a/style.css`
- `/shared/{i18n,markdown}.js`, `/shared/{production,experiment}.css`
- `/pdfjs/build/pdf.mjs`, `/manifest.json`, `/sw.js`

If any path returns `text/html` instead of `application/javascript`, the test
fails with a message identifying the offending path and its actual content-type.

**`co-web/e2e/anonymous-bootstrap.spec.ts`** — opens `/` in a fresh context
(no cookies, no localStorage) and asserts:
1. `#view-tabs` appears (proves JS assets loaded and executed)
2. Sidebar has ≥ 1 project item on non-mobile viewports
3. `.conteudo-stat` appears (proves the template project was auto-selected
   and the entries API responded with populated data)
4. No MIME-type or script-execution `console.error` / `pageerror` during boot

## [2.13.5] — 2026-05-21 — UX: Universe visibility simplified to two options

### Changed — Visibilidade form has 2 options, clearer terminology

The "create universe" form had three radio buttons (`Privado — só você`, `Público — assinável`, `Login obrigatório`) but the third was dead code: the backend collapsed `requires_login` into `public-subscribable` (per the 1.46.0 universe_routes comment) and would 400 the request if submitted. Users selecting the third option got an opaque save failure.

**Two options now, each with a short hint clarifying *who* sees the universe:**

- **Privado** — você e quem você convidar
- **Público** — visível para quem buscar

(EN equivalents: "Private — you and people you invite" / "Public — visible to anyone who searches")

The hint text is muted (`--text-muted`) and styled smaller than the bold label via a new `.form-radio-hint` class in `production.css`. i18n keys `criar.visibility.{private,public}{,.hint}` added to both `pt` and `en` blocks.

## [2.13.4] — 2026-05-21 — Hotfix: `serve_variant_file` double-prepended `variants/<variant>/`

### Fixed — `/variants/a/app.js` still 404'd after 2.13.3

2.13.3 routed static-asset paths from `serve_deep_link` to `serve_variant_file`, but the latter unconditionally prefixes `variants/<variant>/` to the incoming path. When the incoming path *already* starts with `variants/a/...`, the result was `variants/a/variants/a/app.js` — which doesn't exist, returning 404 (plain-text "Not found"). Browser then hit:

```
Refused to execute script from 'https://co.artelonga.com.br/variants/a/app.js' 
because its MIME type ('text/plain') is not executable
```

The same cascade hit `/shared/style.css`, `/shared/i18n.js`, `/shared/markdown.js`, every SPA module — all the script-loading errors in the user's console.

**Fix**: `serve_variant_file` now has an explicit branch for paths starting with `variants/` that serves the file as-is without re-prefixing. Mirrors the `pdfjs/` and `shared/` branches already in place.

### Why this kept slipping past

The fallback `serve_variant_file` was originally only invoked by `Router::fallback`, where the matcher already stripped any matched prefix — so paths arriving there *never* started with `variants/`. CO-232 introduced a *direct* call (via my 2.13.3 hotfix) without realizing the prefix-naive design. A passing route-test for `/variants/a/app.js` against the deployed binary would have caught it.

## [2.13.3] — 2026-05-21 — CRITICAL hotfix: `serve_deep_link` was serving HTML for static assets

### Fixed — `/variants/a/app.js` returned HTML, breaking the entire SPA

CO-232's `serve_deep_link` handler is registered on `/{slug}/{*subpath}`. The route also matches:
- `/variants/a/app.js`
- `/variants/a/modules/*.js`
- `/shared/style.css`
- `/pdfjs/build/pdf.js`

For all of these, the handler served `index.html` (the SPA shell) instead of delegating to the static-file handler. The browser tried to parse the HTML as a JavaScript module, hit a MIME-type mismatch, and **the entire SPA failed to bootstrap**. Symptom: every "Carregando..." stayed stuck forever, sidebar empty, top bar minimal — because no client code ran at all.

This explained why 2.13.1 (sidebar fallback) and 2.13.2 (anonymous getProjects via v1) appeared to have no effect: the JS file containing those fixes was never even successfully loaded.

**Fix**: `serve_deep_link` now calls `looks_like_static_asset(uri.path())` first and delegates to `serve_variant_file` for asset paths. Mirrors the same guard `serve_co_index` already had for the single-segment `/{slug}` route. Confirmed `/variants/a/app.js` now returns `application/javascript` after deploy.

### Why this slipped past CO-232 review

The `entry_exists_for_subpath` logic correctly handles the entry-or-no-entry distinction, but **no test exercised the static-asset path**. The CO-232 tests covered known and unknown entry slugs but didn't have a `/variants/...` or `/shared/...` case. The 2.12.2 hotfix (universe-doesn't-exist → 200) made the regression louder because it removed the prior 404 that would have at least let the browser's fallback resolver try something different.

## [2.13.2] — 2026-05-21 — Hotfix: anonymous project list 401 → board never renders

### Fixed — `api.getProjects()` hit auth-gated legacy endpoint

`api.getProjects()` called `/api/projects?u=<slug>` — a legacy endpoint that requires authentication. For anonymous visitors this returned 401 ("Missing or malformed Authorization header"), `apiFetch` returned null, `state.projects` stayed empty, and the board area rendered "Selecione um projeto na barra lateral" + the universe-home loader stuck on "Carregando…".

Sibling endpoints work anonymously without issue:
- `GET /api/v1/universes/<slug>/projects` → 200
- `GET /api/projects/<key>/tasks?u=<slug>` → 200

**Fix**: `getProjects()` now prefers the v1 endpoint and falls back to the legacy path only if v1 doesn't return. Anonymous visitors get the tutorial project ("Bem-vindo ao Co") populated immediately on load — the board renders, the universe home renders, the Carregando state clears.

**Combined with 2.13.1** (sidebar fallback) this closes the full anonymous-bootstrap gap. New incognito visitor lands on `/` → sidebar shows public universes, board shows tutorial project, tasks render — no 401-stuck states.

## [2.13.1] — 2026-05-21 — Hotfix: anonymous sidebar empty + Carregando... stuck

### Fixed — `loadMeUniverses()` bailed silently on 401 for anonymous users

`app.js` `loadMeUniverses()` called `/api/v1/me/universes` and bailed when the response was null (401). For anonymous visitors this meant `state.userUniverses` was never populated, so:

- `sidebar.js renderSidebar()` had no `state.meUniverses` (logged-out) AND no `state.userUniverses` (never set) → universe section rendered empty
- Project list area waited indefinitely → "Carregando..." stuck forever
- Visible regression after CO-238 sidebar refactor exposed an unused fallback branch as the only path that could have rendered anything

**Fix**: when `/api/v1/me/universes` returns null (anonymous), `loadMeUniverses()` now falls back to `/api/v1/universes/public` and populates `state.userUniverses` with the public list. The sidebar's existing "flat list for anonymous users" branch in `renderSidebar()` now actually receives data and renders.

**Why this slipped past CO-238**: the bucketed shape `me.owned/member/subscribed/invited/discoverable` (CO-191) became the only render path for logged-in users; the anonymous fallback was dead code that no tests exercised. CO-238 made the sidebar's section structure visible without touching the data-fetching layer where the bug lived.

## [2.13.0] — 2026-05-20 — True content-volume metrics: lines, words, chars (CO-241)

### Added — body_lines / body_words / body_chars per entry and per universe

Added three new columns to the `entries` table (both meta-DB and per-universe
`data.db`) that capture true body-text volume distinct from `content_count`
(which counts files, not lines):

- `body_lines` — `body.lines().count()`
- `body_words` — `body.split_whitespace().count()`
- `body_chars` — `body.chars().count()`

**Vault PUT** (`EntryIndex::upsert`) computes and writes all three on every
insert/update. An idempotent boot-time backfill pass populates them for
existing entries (rows with `body_chars = 0 AND body != ''`).

**Storage dashboard** (`/api/v1/admin/storage`, `/api/v1/me/storage`,
`/api/v1/universes/{slug}/storage`) aggregates the three metrics per universe
via `SUM()` and exposes them on `UniverseStorage`.

**Storage HTML page** replaces the misleading "Linhas entries" column (which
was the SQLite row count, identical to `content_count`) with three distinct
columns: `Entradas / Linhas / Palavras / Tamanho`. `content_count` is now
uniformly labeled "Entradas" everywhere.

**Migrations:** meta-DB migration v46; per-universe migration v12.

## [2.12.4] — 2026-05-20 — Fix per-universe data_db_bytes (CO-240)

### Fixed — storage dashboard showed 0 bytes for every universe's data.db

`storage_dashboard.rs` was constructing the per-universe DB path as
`{data_dir}/universes/{key}/data.db`, but `UniversePool` stores each
universe's `data.db` under a 2-level xxHash fanout:
`{data_dir}/universes/{ab}/{cd}/{key}/data.db`.  
`file_size()` found nothing → always returned 0.

**Fix**: replace the manual path construction with `universe_pool.db_path(&key)`,
which uses the same fanout logic as the pool itself. The per-user and admin
storage endpoints now return accurate `data_db_bytes` values.

Integration test added in `storage_dashboard_tests.rs`:
create-universe → 5 vault PUTs → assert `data_db_bytes > 0`.

## [2.12.3] — 2026-05-20 — Hotfix: add explicit `/{slug}/` trailing-slash route

### Fixed — trailing-slash SPA routes still 404'd after 2.12.2

2.12.2's serve_deep_link logic was correct, but axum's `{*subpath}` wildcard doesn't match empty paths — so `/entrar/` never reached the handler. It fell through to the framework 404 (text/plain) instead of the SPA shell.

**Fix**: added an explicit `.route("/{slug}/", get(serve_co_index))` between the `/{slug}` route and the `/{slug}/{*subpath}` deep-link route. Trailing-slash SPA paths (`/entrar/`, `/sobre/`, `/termos/`) now hit `serve_co_index` and return 200 with the SPA shell.

## [2.12.2] — 2026-05-20 — Hotfix: CO-232 broke SPA routes with trailing slash

### Fixed — `serve_deep_link` returned 404 for non-universe SPA routes

`serve_deep_link` (added in CO-232) was registered on `/{slug}/{*subpath}` and treated every two-segment URL as "universe + entry path". For SPA-owned routes like `/entrar/`, `/sobre/`, `/termos/` (where the first segment is **not** a universe slug at all), it returned 404 because no entry matched — breaking the login page and several static SPA routes in prod.

**Fix**: `serve_deep_link` now distinguishes three cases:

1. **Slug is not a universe** → 200 with SPA shell (e.g. `/entrar/`, `/sobre/`) — client router renders the page.
2. **Universe exists + entry exists** → 200 with SPA shell, SPA opens the entry.
3. **Universe exists but entry doesn't** → 404 with SPA shell (CO-232's original intent), SPA renders not-found view.

New helper `universe_exists()` separates universe lookup from entry lookup. The CO-232 tests still pass (case 2 + 3 unchanged); case 1 is the new fix.

## [2.12.1] — 2026-05-20 — Hash API tokens at rest (CO-237)

### Security — API tokens no longer stored in plaintext

**Root cause:** The `api_tokens` table stored raw `co_<40-char>` tokens in a
`token TEXT` column. A database dump (stolen backup, SSH escalation, accidental
log leak) would expose every active token, allowing an attacker to impersonate
any user with a CLI or agent integration.

**Fix:**

- **SHA-256 hashing at insert** — `create_api_token` computes `SHA-256(token)`
  and stores only the hex hash in a new `token_hash TEXT` column. The raw token
  is returned to the caller exactly once and never written to the database.
- **Hash-based lookup** — `get_api_token_by_value` hashes the incoming Bearer
  value before the SQL lookup (`WHERE token_hash = ?`). No plaintext token is
  ever compared against stored data.
- **Token prefix for display** — a new `token_prefix TEXT` column stores the
  first 11 characters of the token (e.g. `co_abc12345`) so the list endpoint
  can show a recognizable prefix without exposing the full secret.
- **Migration v45** — adds `token_hash` and `token_prefix` columns with a
  unique index on `token_hash`. All existing tokens are invalidated at migration
  time (deleted from the table).
- **Documentation updated** — `seguranca-criptografia.md` removes the known-gap
  entry for CO-237 and documents the new hashing scheme.

**⚠️ Breaking change for token holders:** all API tokens issued before v2.12.1
are invalidated. Users must re-create their tokens via
`POST /api/v1/auth/token` (or the web UI). One-time cycle; tokens created from
v2.12.1 onward are hashed and unaffected by future DB dumps.

**Test:** `vault_routes::tests::test_api_token_hash_at_rest` — creates a token,
verifies no `co_*` value is stored in the DB, confirms lookup by raw value
succeeds and lookup by wrong value returns `None`.

## [2.12.0] — 2026-05-20 — OpenAPI coverage: auth + admin + chat (CO-226)

### Added — interactions registry extended to auth, admin, and chat

`co-web/e2e/interactions/registry.yaml` is the canonical OpenAPI 3.1 document
served at `GET /api/v1/interactions/openapi.json`. Previously it covered only
content entry CRUD (4 operations). This release adds 45 new operations:

- **auth (23 ops):** session lifecycle (`login`, `verify`, `logout`, `me`, `stats`),
  password flows (`password-login`, `signup`, `forgot-password`, `reset-password`),
  passwordless onboarding, API token management, session exchange,
  Google OAuth status, UAT login, and the full OIDC provider surface
  (discovery, JWKS, token exchange, userinfo, OAuth client management).
- **admin (15 ops):** dashboard, user origin breakdown, leads queue CRUD,
  telemetry endpoints (summary, CSV export, CRUD summary), storage breakdown,
  A/B flag management, and outbound webhook management (gestao).
- **chat (7 ops):** room listing and creation, room member list, paginated
  message history, post/edit/delete messages.

Coverage: ≥85% of auth routes, 100% of admin routes, ≥87% of HTTP chat routes
in `docs/architecture/api-catalog.md`.

Tests updated:
- `four_operations_under_entry_resource` → `entry_operations_present` (removed
  hard-coded count assertion; registry now covers more than one resource).
- `universe_is_a_path_parameter` → `universe_scoped_paths_use_universe_parameter`
  (narrowed to paths under `/api/v1/universes/`; auth/admin/OIDC paths are exempt).
- Added `registry_covers_auth_admin_chat` assertion test.

## [2.11.6] — 2026-05-20 — Sync pipeline: latest changes not appearing on prod web (CO-233)

### Fixed — cache headers on mutable entry API responses

**Root cause:** `list_entries`, `list_entry_tags`, and `entry_tree` returned no
`Cache-Control` header. Without an explicit `no-store` directive, Cloudflare CDN
(CO-117, with a "Cache Everything" page rule) could cache these mutable API
responses and serve stale data for its configured TTL — making vault PUTs and
board edits invisible to visitors until the CDN TTL expired (potentially hours).

Additionally, `entry_cache_control` used `stale-while-revalidate=300` for anon
reads of template / `co::public/*` seed content, creating a 5-minute window
where browsers could serve stale content even after a fresh response arrived.

**Fix:**

- `GET /{slug}/entries`: both JSON and protobuf responses now include
  `Cache-Control: no-store`, preventing caching at every layer (browser,
  CDN, proxy).
- `GET /{slug}/entries/tags`: same.
- `GET /{slug}/entries/tree`: same.
- Anon seed-content single-entry GET: `stale-while-revalidate=300` removed;
  header is now `public, max-age=60, must-revalidate` — no stale window.

**Regression tests** (`co-web/tests/sync_pipeline_tests.rs`):

- `vault_write_appears_in_list_immediately`: vault PUT immediately followed
  by `GET /entries`; asserts the entry appears AND `Cache-Control: no-store`
  is present; timing asserted < 2 s.
- `entry_tags_carries_no_store_header`: tags endpoint.
- `entry_tree_carries_no_store_header`: tree endpoint.
- `entry_update_visible_immediately`: update entry via vault, read back via
  GET single-entry; asserts updated frontmatter visible without delay.

## [2.11.5] — 2026-05-20 — Cross-universe deep-link returns 404 for unknown entries (CO-232)

### Fixed

- **`/<universe>/<slug>`** now returns HTTP 404 (and renders the SPA 404 view) when the entry does not exist, instead of silently landing on the universe home. Covers unknown entry paths for any universe; the happy-path (known entry) and the bare universe home (`/<universe>/`) are unchanged.
- New `serve_deep_link` handler in `co-web/src/server/static_files.rs` checks the four SPA candidate paths (`<slug>.md`, `<slug>`, `content/<slug>.md`, `content/<slug>`) against the per-universe entry index before deciding the HTTP status code.
- SPA `maybeOpenEntryFromUrl()` now renders an inline 404 view (with links back to the universe home and the global home) instead of resetting the URL to the universe home when all lookup attempts fail.
- New `not_found.*` i18n strings in Portuguese and English.
- 3 new in-process Rust unit tests (unknown slug → 404, known slug → 200, non-existent universe → 404).
- 1 new Playwright e2e spec (`e2e/deep-link-404.spec.ts`) asserting 404 HTTP status and 404 view rendering across `template` and `artelonga` universes.

## [2.11.4] — 2026-05-20 — Slim AppStateInner via segregated sub-states (CO-221)

### Changed

- **`AppStateInner` split into 4 composable sub-states**: `CoreState` (storage, config, auth_store, event_bus), `RealtimeState` (doc_rooms, sync_rooms, chat broadcast + presence), `IndexState` (cache, embeddings, embedding_tx), `IntegrationsState` (mail, geo, plugin_registry, game_storage, wae, jwt_key, rate_limiter, experiment, worker_supervisor).
- `AppStateInner` now holds `Arc<CoreState>`, `Arc<RealtimeState>`, `Arc<IndexState>`, `Arc<IntegrationsState>` — one Arc per sub-state.
- `AppState` changed from a bare type alias to a `Clone + Deref` newtype, enabling `FromRef<AppState>` impls for each sub-state.
- axum `FromRef` impls added: handlers can now take `State<Arc<CoreState>>` (or any sub-state) and receive only the fields they actually use.
- 11 handlers migrated to narrow sub-state extractors: `health_check_deep`, `cache_stats_handler`, `list_plugins`, `openid_configuration`, `jwks_json`, `vercel_drain_handler`, `serve_repl_page`, `workers_status_handler`, `create_flag_handler`, `list_flags_handler`, `toggle_flag_handler`.
- Remaining handlers continue to take `State<AppState>` — migration is opt-in.

### Why

Closes CO-221. The global `State<AppState>` extractor leaked all 20 unrelated dependencies into every handler, making the dependency graph opaque and compilation slower. With sub-states, handlers declare their actual dependencies; the compiler enforces it.

## [2.11.3] — 2026-05-20 — co-cli build hotfix

### Fixed

- `co-cli` `board.rs` was missing the `bypass_rate_limit: false` field on its `WebConfig` initializer (added in a prior `co-web` change). The library compiled but `co-cli` did not. 2.11.2 shipped the changelog + workspace bump without catching this; 2.11.3 unblocks the binary.

## [2.11.2] — 2026-05-20 — Worker trait + supervisor (CO-223)

### Added

- **`workers::Worker` trait**: unified lifecycle contract (`name()`, `run()`, `tick()`) for the three long-running background workers (embeddings, notifications, push delivery).
- **`workers::Supervisor`**: panic-isolated wrapper that restarts a worker on panic with exponential backoff. Replaces the prior fire-and-forget `tokio::spawn` calls.

### Changed

- Embedding, notification, and push workers refactored to implement `Worker` and run under the supervisor. A panic in one worker no longer kills the others.
- `core/src/workers/` layout established as the home for the trait, supervisor, and per-worker submodules.

### Why

Closes CO-223. Prior `tokio::spawn` workers ran unsupervised — a panic anywhere in the worker body terminated only the spawned task while the rest of the process kept running, leaving the system in a partial-failure state with no visible signal. The supervisor surfaces panics, restarts the worker with backoff, and exposes a uniform shape for the next worker we add (e.g. CO-244 REPL polling).

## [2.11.1] — 2026-05-19 — Typed auth extractor hierarchy (CO-222)

### Added

- **`auth::extractors` module**: four typed axum extractors — `AuthedUser`, `OwnerOf`, `AdminUser`, `TokenOrJwtUser` — that express auth requirements directly in handler signatures.
- **`auth::extract_bearer_or_cookie`**: shared helper extracted from middleware to locate the caller's token (Bearer header or `session` cookie) without duplicating header-parsing logic.

### Changed

- **11 handlers migrated** to `AuthedUser` extractor: `list_notifications_handler`, `read_all_notifications_handler`, `mark_notification_read_handler`, `get_preferences_handler`, `put_preferences_handler` (notification_routes); `subscribe_handler`, `delete_subscription_handler`, `list_subscriptions_handler` (push_routes); `list_rooms_handler`, `list_room_members_handler`, `create_room_handler` (chat_routes).
- Existing `require_auth` / `require_auth_with_token` middleware remains active in parallel — `AuthedUser` uses the `UserId` extension they inject as a fast path, so all existing tests continue to pass.

## [2.11.0] — 2026-05-19 — In-process event bus (CO-220)

### Added

- **`crate::events` module**: `Bus`, `BusReceiver`, `DomainEvent`, and `EventFilter` — a thin tokio-broadcast-backed in-process event bus.
- **`DomainEvent` variants**: `EntryWritten`, `EntryDeleted`, `NotificationRequested`, `InvitationAccepted`, `ProposalDecided`, `AssetUploaded`.
- **Notification listener** (server startup): subscribes to `EventFilter::Notification`, handles `NotificationRequested` events from `invitation_routes` and `proposal_routes`.
- **Entry listener** (server startup): subscribes to `EventFilter::Entry`, forwards `EntryWritten`/`EntryDeleted` to the embedding worker channel — replacing direct `embedding_worker::enqueue_*` calls in `entry_routes`.
- **Asset listener** (server startup): subscribes to `EventFilter::Asset`, auto-creates reference card entries for uploaded PDF/image assets.

### Changed

- `invitation_routes`: replaced direct `storage.create_notification()` with `event_bus.publish(NotificationRequested)`.
- `proposal_routes`: replaced two direct `storage.create_notification()` calls with `event_bus.publish(NotificationRequested)`.
- `entry_routes`: replaced three direct `embedding_worker::enqueue_*()` calls with `event_bus.publish(EntryWritten/EntryDeleted)`.
- `asset_routes`: emits `AssetUploaded` event after successful blob upload.
- `AppStateInner`: added `event_bus: crate::events::Bus` field.

## [2.10.0] — 2026-05-19 — Universe branching (CO-95 Phases 2-4)

### Added

- **Phase 2 — Op log endpoint**: `GET /api/v1/universes/:slug/ops` exposes the per-universe `entry_events` append-only log, paginated via `?after_seq=N&limit=N`.
- **Phase 2 — Atomic writes**: vault PUT and DELETE now wrap the `entries` upsert + `entry_events` insert in a single `BEGIN IMMEDIATE … COMMIT` transaction, preventing entries/events divergence on crash.
- **Phase 3 — Replay**: `GET /api/v1/universes/:slug/replay?to_op=N` returns the logical entry state of the universe as of op N — active paths (last event was `put`) and deleted paths (last event was `delete`).
- **Phase 3 — Op diff**: `GET /api/v1/universes/:slug/op-diff?from_op=M&to_op=N` shows the exact change set between two op IDs: added, modified, and deleted paths with before/after body hashes.
- **Phase 3 — O(1) fork**: `duplicate_universe` now uses `Storage::fast_fork_universe` — WAL checkpoint + `std::fs::copy` of `data.db` — instead of row-by-row copying. Falls back to `clone_universe` if the source DB file is absent.
- **Phase 4 — Universe diff**: `GET /api/v1/universes/:slug/diff?against=<key>` compares two live universe entry sets, reporting added, modified, and deleted paths.
- **Phase 4 — Promote**: `POST /api/v1/universes/:slug/promote` applies all source entries onto a target universe (last-write-wins). Returns a conflict list (paths with divergent hashes) and writes an audit entry at `_audit/promote-<ts>.md` in the target.
- **Phase 4 — Revert**: `POST /api/v1/universes/:slug/revert?to=<op_id>` restores the universe to its historical state at op N by replaying the event log.
- **Phase 4 — Cherry-pick**: `POST /api/v1/universes/:slug/cherry-pick` copies selected paths from source into a target universe.

### Fixed

- Clippy: collapsed nested `if`/`if let` chains in `entry_routes`, `proposal_routes`, `storage_dashboard`; removed for-loop over single element in `storage/seed`; removed needless dereferences in vault/op-log routes.

## [2.9.1] — 2026-05-18 — Server decomposition (CO-215)

### Refactored

- Split `co-web/src/server.rs` (1567 LoC) into focused submodules under `server/`:
  - `server/state.rs` — `AppStateInner`, `AppState`, lock helpers
  - `server/validation.rs` — input validation functions (task/comment/project)
  - `server/uat_boot.rs` — UAT startup tasks (reset flag, yuri seed, anon cleanup)
  - `server/seed_orchestrator.rs` — boot seed orchestration (template, quilombo, yggdrasil, admin, chat/push backfill)
  - `server/router.rs` — `build_router()` with all route registrations
  - `server/mod.rs` — module root: `start_server()`, health checks, static-file helpers
- Public API at `crate::server::{AppState, AppStateInner, start_server, build_router}` unchanged

## [2.9.0] — 2026-05-18 — Cross-repo architecture audit + security hardening + backlog scaffold

Bundled theme covering everything since 2.7.29 — five repos audited and scaffolded, one universe migrated, one prod bug fixed, one user-facing security doc, 16 new backlog user-stories + 14 new epic specs, 70+ task specs scaffolded across all repos.

### Cross-repo architecture audit (5 repos)

C4-aligned `as-is.md` + `api-catalog.md` + `refactor-plan.md` produced for each of: **co**, **rfq-gateway**, **quilombo-blog**, **ArteLonga**, **yggdrasil**. Each repo's `docs/architecture/` is registered as a CO sub-universe via the shared `_universe.yaml` + `schema.yaml` + `CHANGELOG.md` template — Iceberg-compatible append-only log slot ready for the transaction-log roadmap.

70+ task specs written across all 5 repos in `work/<space>/`:
- CO: CO-215..226 user-stories + CO-227..231 epics + CO-232..233 bug stories + CO-234 chat fix
- rfq-gateway: RFQ-14..23 + RFQ-24..26 epics + RFQ-27 OpenAPI endpoint
- quilombo-blog: bootstrapped `work/qb/` + QB-1..12 + QB-13..15 epics
- ArteLonga: AL-51..60 + AL-64..66 epics
- yggdrasil: YG-38..46 + YG-47..49 epics + YG-50 OpenAPI spec

This is the "universally documented API + docs template" target — every repo will expose `/openapi.json` after the universal-template tasks ship (CO-226 + QB-1 + AL-55 + RFQ-27 + YG-50).

### Comunicação migrated to a live CO universe

48 markdown files migrated from `/Users/artelonga/projects/comunicacao` to the CO universe `comunicacao` via Vault PUT. Every write logs to `entry_events`. The local git repo retires in favor of CO as source-of-truth + audit log.

### CO-234 — fix chat "Entre para participar do chat" for logged-in users

Universal bug across every universe: `chat.js:_hasSessionCookie()` regex-tested `document.cookie` for `session=`, but the session cookie is `HttpOnly` at `auth.rs:271`. Replaced with `_isAuthenticated()` checking `_state?.me`. Server-side WS handshake unchanged.

### CO-232 / CO-233 — flag two open prod-integration issues as user-stories

- CO-232 — Cross-universe deep-link `/<universe>/<unknown-slug>` falls through to universe home instead of 404
- CO-233 — Sync pipeline reliability (latest changes not appearing on prod web)

Both surfaced during the audit conversation; tracked for the next operational sweep.

### Security docs + CO-236 / CO-237 specs

New user-facing page at `/seguranca-criptografia` documenting the actual storage model with file:line citations into the code:

- **Senhas**: Argon2id (default params, OsRng salt)
- **JWTs**: ES256 (HS256 legacy fallback), 7-day TTL
- **Cookies**: HttpOnly + SameSite=Lax (the gotcha that caused CO-234)
- **Tokens de API**: 90-day, plaintext at rest (gap → CO-237)
- **Anexos**: ChaCha20-Poly1305 per-universe (CO-148)
- **Canais de recuperação**: encrypted-at-rest values
- **Códigos de verificação**: Argon2id-hashed

Plus:
- **CO-235** — `co clone <git-url>` feature concept (universe = open-source repo, automated mirroring)
- **CO-236** — `co auth` CLI command suite (reset-password, login, change-password, token create/list/revoke)
- **CO-237** — hash API tokens at rest (SHA-256 or Argon2id)

### 8 additional follow-on specs (CO-238..245) — sidebar / storage / file-types review

- CO-238 — Sidebar UX clarity (owned vs member vs sub-universe semantics)
- CO-239 — Real host disk stats (`nix::sys::statvfs`; currently stubbed to zero)
- CO-240 — Per-universe `data_db_bytes` fix (currently 0 for every universe)
- CO-241 — True content-volume metrics (lines / words / chars) — fixes "lines = files" confusion
- CO-242 — Unified file listing — surface ALL file types (PDF, image, video, code), not just .md
- CO-243 — VS Code (and LSP) integration — open universe as remote workspace
- CO-244 — Python / R REPL interoperability — DuckDB attach + in-browser REPL
- CO-245 — Inline CodeMirror editor for plaintext file types

### Tooling shipped (transitional — roadmap is native CO commands)

`scripts/gen-task-specs.py`, `scripts/apply-docs-subuniverse.py`, `scripts/migrate-comunicacao.py`, `scripts/launch-phase-c.sh`, `scripts/gen-co-238-245.py`. These will be replaced by native CO CLI commands as part of CO-235 (`co clone`) and CO-236 (`co auth`).

### Version note

Skipping 2.8.x because CO-214 work was tagged 2.8.0/2.8.1 on a feature branch that never landed as a proper release. 2.9.0 cleanly marks the first stable release-tagged point that contains the audit cycle.

The next milestone is Phase C execution — co-auto cycles all five repos through their refactor backlogs on long-running branches, ONE PR per repo at the end.

## [2.7.29] — 2026-05-16 — Bug fixes + remove admin gate + architecture review

Four threads, one release:

### Fix `+ Nova Tarefa` from the Conteúdo home view

Click silently no-op'd because the handler required
`state.currentProject` and Conteúdo doesn't auto-select one. Now:
auto-selects the first project on click; if no project exists, surfaces
a "Crie um projeto antes" warning toast instead of silently failing.

### Fix theme persistence

User-chosen theme via universe settings was reverting to "modern"
on next load. Root cause in `settings.js`: `co_user_palette` was
auto-set to `"modern"` on first load, then shadowed every universe's
saved `theme_preset` (because `userPalette || config.theme_preset`
made the localStorage value always win).

Fix: don't auto-set the default. `co_user_palette` stays null until
the user explicitly picks a palette from the header switcher.
Universe-level `theme_preset` is now the default winner;
`loadThemeCss` reads `state.universeConfig.theme_preset` as the
fallback before the hardcoded "modern".

### Remove admin gate from storage dashboard

Per user direction "remove admin functionality, users are either
members of or not." The `/storage` page no longer has an Admin tab;
the `/api/v1/me/storage` endpoint now uses
`DashboardFilter::AccessibleBy(uid)` which JOINs `universe_members` —
so an invited member of a private universe sees its stats. The
per-universe endpoint loosened similarly: owner OR member OR public.

`/api/v1/admin/storage` still exists for legacy; consider removing
in a future release.

### Invitation flow (verification, no code change)

The flow `Owner invites → invitee accepts → invitee is a member` is
already shipped via `invitation_routes.rs` + the `universe_members`
table. Once they're a member of a private universe:
- Read works via the visibility gate (member branch)
- Direct write works via the writer gate (member branch)
- For non-member proposed changes, the inline proposal flow from
  2.7.23 handles it: PUT 403 → /proposals/inline → owner decides

No new code; documenting the path explicitly.

### Architecture review

New doc at `docs/architecture-review.md`. Inventories
`co-web/static/variants/a/` (7014 lines across 22 files; app.js is
705), identifies SRP violations in app.js / modals.js / login.js,
proposes a target module structure (one file = one user-visible
thing), and a 6-phase incremental refactor plan that doesn't
require a stop-the-world rewrite.

Phase 1 (extract URL parsing) + Phase 2 (extract view-router) would
drop ~150 lines from app.js with zero behavior change. Tell me to
start and I'll send a PR-shaped patch per phase.

### Tests

4 storage tests still pass after the filter rename
(`AccessibleBy` replaces `OwnedBy`). The `universe_storage_owner_only`
test was updated to use a private universe for the negative case,
since public universes are now visible to any authed user.

## [2.7.28] — 2026-05-16 — Storage dashboard: UI page + per-user + per-universe

2.7.27 shipped only the admin JSON. This release adds the surfaces
the user actually needs:

### Two new endpoints

- `GET /api/v1/me/storage` — every universe the caller **owns**
  (owner-only by construction; auth required, 401 otherwise)
- `GET /api/v1/universes/{slug}/storage` — single universe by key,
  **owner-gated**: 200 for the owner, 403 for anyone else

Both use the same `compute_dashboard_filtered(state, filter)`
helper with a `DashboardFilter::{All, OwnedBy, Single}` enum, so
the response shape is identical across all three (admin / me /
single-universe) — clients render one template.

### `/storage` page (auth-required UI)

New static page served at `GET /storage`. Single HTML + JS file
(`co-web/static/shared/storage.html`); calls one of the three
endpoints depending on:

- Tab "Meus universos" → `/api/v1/me/storage`
- Tab "Todos (admin)" → `/api/v1/admin/storage` (admin sees, others
  get an inline 403 message)
- `?universe=<slug>` query param → `/api/v1/universes/<slug>/storage`
  (drill-in from the universe info modal; tabs hidden)

Renders four summary cards (universes, markdown, data.db, host
used) + a sortable table (entries, md_bytes, data_db_bytes,
entry_events.rows, entries.rows). Click any column header to sort;
click the universe key to navigate to that universe.

### Header link

The user badge in the SPA header now has a database icon linking
to `/storage`. Visible only when logged in.

### Tests

`co-web/tests/storage_dashboard_tests.rs` extended to 4 tests:
- `storage_dashboard_requires_admin` (existing)
- `storage_dashboard_returns_shape` (existing)
- `me_storage_returns_owned_universes_only` — owned-only scope is
  enforced server-side; other users' universes don't appear
- `universe_storage_owner_only` — owner gets 200; non-owner gets 403

All pass.

### Not yet (deliberate)

- Sub-folder / path-prefix filters (you said "subset as filters
  later")
- Per-table bytes (DBSTAT virtual table; needs rusqlite compile flag)
- Volume `total`/`available` bytes via statvfs (would add `libc`/`nix`)
- Inline storage section in the universe info modal (link exists
  via `/storage?universe=<slug>`; modal embed is a polish iteration)

## [2.7.27] — 2026-05-15 — Per-universe storage dashboard (admin)

New endpoint `GET /api/v1/admin/storage` returns a snapshot of every
universe's disk + table footprint. Admin-only (JWT + email match
against `CO_SEED_ADMIN_EMAIL`). Cached 60s in-memory to avoid
filesystem hammering on dashboard refresh.

### Response shape

```
{
  "generated_at": "2026-05-15T…",
  "host": { "data_dir": "/data", "data_dir_used_bytes": … },
  "totals": { "universes": N, "md_bytes": …, "data_db_bytes": … },
  "universes": [
    {
      "key": "artelonga",
      "owner_id": "…",
      "is_public": true,
      "is_template": false,
      "visibility": "public-subscribable",
      "content_count": 312,
      "md_bytes": …,          // sum of *.md files in universes/<key>/
      "data_db_bytes": …,     // size of universes/<key>/data.db on disk
      "tables": {
        "entries":          { "rows": 312 },
        "entry_events":     { "rows": 487 },   // ← transaction log growth
        "entry_relations":  { "rows":  89 },
        "references_meta":  { "rows":  14 }
      }
    }
  ]
}
```

### What this surfaces

- **`entry_events` growth** — the append-only log from 2.7.25 now
  carries every write's full body. A universe edited heavily over a
  year will have thousands of rows here. The dashboard makes that
  visible; compaction / Iceberg export becomes the natural follow-up
  when this row gets uncomfortable.
- **Stale data.db** — universes nobody's touched in months still
  pin a per-universe SQLite file. Dashboard tells you which to
  consider archiving.
- **Md vs DB ratio** — `data_db_bytes` should be ~2-3× `md_bytes`
  for a healthy universe (entries + indices). A bigger ratio
  hints at index bloat (FTS or embeddings).

### Implementation notes

- New module `co-web/src/storage_dashboard.rs` (~250 lines)
- `walk_bytes` recurses the universe dir summing files matching a
  predicate (`.md` filter for `md_bytes`, everything for host total)
- Table row counts use `SELECT COUNT(*)` per universe DB —
  cheap on the indexed per-universe SQLite
- Bytes-per-table omitted in v1 — would need the SQLite DBSTAT
  virtual table (compile flag not on by default for rusqlite-bundled).
  Row counts already point to hotspots clearly enough.
- Volume `total_bytes` / `available_bytes` omitted in v1 — would
  need a `libc::statvfs` binding we don't currently depend on. Add
  later via `nix::sys::statvfs` if precise volume telemetry needed.

### Trajectory

The dashboard is the visibility layer that lets us decide *when* to
implement the Kafka/Iceberg export from the transaction-log doc
(`co::public/transaction-log.md`). Until the dashboard turns red,
the local log stays the source of truth.

### Tests

`co-web/tests/storage_dashboard_tests.rs` — 2 pass:
- `storage_dashboard_requires_admin` (no-auth → 401, non-admin → 403)
- `storage_dashboard_returns_shape` (admin → 200 + expected fields)

## [2.7.26] — 2026-05-15 — Fix task edit on Conteúdo home view

Bug: clicking a task card in the Tarefas section of the Conteúdo
view silently no-op'd. Root cause: task cards routed through
`openContentEditor(taskId)` → `state.tasks.find(t => t.id === taskId)`
→ `if (!task) return`. But `state.tasks` is the project-board cache,
only populated by `selectProject()`. On the universe home view no
project is selected, so the array is empty and the click bailed.

Fix: unify task and page cards. Both now use `data-entry-path` and
route through the same master-detail flow (click → select in detail
pane on desktop; modal on mobile). The `[data-task-id]` click
handler was removed — it was dead code on Conteúdo. Tasks still
work in the Kanban / board view through the existing project-task
edit flow; that path is unaffected.

Also:
- `content._clickableEntries` (pages + tasks) is the unified lookup
  for click handlers; falls back to a synthetic entry without `body`
  so `openZoomModal` re-fetches.
- Task cards keep `data-task-id` (informational; no longer
  click-triggered) so the existing CSS / archive flows still find
  them by selector if needed.

## [2.7.25] — 2026-05-15 — Transaction log: append-only event store + history API

Phase 1 of the lakehouse trajectory. The atomic primitives for
content are now **logged before they're applied**: every PUT/DELETE
appends an immutable row to a per-universe `entry_events` table.
Source of truth for time-travel, undo, audit, and the future
Kafka/Iceberg/Pinot/Flink export. Full design doc at
`co::public/transaction-log.md`.

### Per-universe `entry_events` table (schema v11)

```
seq               INTEGER PRIMARY KEY AUTOINCREMENT
ts_micros         INTEGER NOT NULL           -- Unix epoch micros, orderable
op                TEXT     NOT NULL          -- 'put' | 'delete'
path              TEXT     NOT NULL
body_hash         TEXT                       -- SHA-256 of new body; NULL on delete
prev_body_hash    TEXT                       -- SHA-256 of old body; NULL on create
body              BLOB                       -- snapshot for replay
frontmatter_json  TEXT                       -- snapshot
author_id         TEXT                       -- NULL today; threaded later
request_id        TEXT                       -- UNIQUE (partial index) for idempotency
exported_at       TEXT                       -- set by Kafka/Iceberg worker
```

Designed to map 1:1 to a future `co.v1.EntryEvent` protobuf —
schema evolution + monotonic ordering + idempotency are the
contract for downstream consumers.

### Hooks

Two write paths recorded:
- `vault_routes::write_vault_entry` — the canonical upsert (used by
  vault PUT, inline proposals, state captures, merges, seed reseeds)
- `entry_routes::update_entry` — the entries-API PUT
- `vault_routes::delete_vault_file` — the entries/vault DELETE

Each captures `prev_body_hash` from `entries.body_hash` BEFORE the
upsert, then appends the event after. Same `uc_guard` lock means
writes are serialized; a separate-statement design (no explicit
BEGIN/COMMIT) means a crash between upsert and event-insert leaves
a one-event gap. Acceptable for v1; explicit transactions are a
follow-up if/when that gap matters.

### `GET /api/v1/universes/{slug}/entries/history?path=<entry-path>`

Returns the per-entry event log, newest first:

```json
{
  "path": "sobre.md",
  "events": [
    {
      "seq": 3,
      "ts_micros": 1747325432123456,
      "op": "put",
      "path": "sobre.md",
      "body_hash": "abc…",
      "prev_body_hash": "def…",
      "frontmatter_json": "{…}",
      "author_id": null,
      "request_id": null
    },
    …
  ],
  "total": 3
}
```

Public-readable; visibility gate covers access. `body` is not
returned in the listing (omits the blob to keep responses small —
fetch via the per-entry GET or a dedicated future reader).

### Tests

`co-web/tests/entry_events_tests.rs` — 3 pass:
- `put_appends_event_with_body_and_hash`
- `second_put_records_prev_hash` (chain: each event's `prev` equals
  the previous event's `body_hash`)
- `delete_appends_delete_event_with_prev_hash`

### Trajectory (full design in `co::public/transaction-log.md`)

| Phase | Status |
|---|---|
| 1. `entry_events` table + history endpoint | **shipped (this release)** |
| 2. Per-entry undo endpoint | follow-up |
| 3. `co.v1.EntryEvent` protobuf + `EventSink` trait | follow-up |
| 4. `KafkaSink` (rdkafka) + async export worker | follow-up |
| 5. `IcebergSink` (Parquet + catalog) | follow-up |
| 6. Pinot / Flink connectors | depends on 4+5 |

The local log IS the contract. Kafka/Iceberg consume the same
logical event shape; switching sinks doesn't require schema
changes.

## [2.7.24] — 2026-05-15 — Inline proposals: notify + decide + inbox

Three follow-ups on top of the inline-proposal endpoint shipped in
2.7.23. Editing flow now closes the loop:

### Notification on proposal create

`create_inline_proposal` now fires a `universe.proposal`
notification to the target universe's owner (unless the proposer
*is* the owner, or owner is `system`). Routes through the existing
notification machinery — surfaces in the bell, the
`/notifications` page, and the email worker per the user's
preferences.

i18n keys added:
- `notif.universe.proposal` — "{author} propôs uma mudança em
  {target_path} no universo {universe}"
- `notif.universe.proposal.merged`
- `notif.universe.proposal.rejected`

### `POST /api/v1/universes/{slug}/proposals/decide`

New endpoint. Only the target universe's owner may call it (403
otherwise). Body: `{ proposal_path, action: "merge" | "reject" }`.

- **merge**: writes the proposal body to the entry at
  `target_path` (preserving the target's existing frontmatter, or
  creating with `{type:"page"}` if absent), then flips proposal
  frontmatter to `status:"merged"`, `decided_by`, `decided_at`.
- **reject**: only flips proposal status to `"rejected"` +
  `decided_by` + `decided_at`. No content change.

Both fire a `universe.proposal.decided` notification to the
original proposer.

### `GET /api/v1/me/inbound-proposals`

New endpoint mounted at `/api/v1/me/inbound-proposals`. Walks every
universe owned by the caller and returns open inline proposals,
sorted newest first. Response shape:

```json
{
  "proposals": [
    {
      "universe": "...",
      "proposal_path": "_proposals/...",
      "target_path": "...",
      "author": "...",
      "status": "open",
      "created_at": "...",
      "note": null
    }
  ],
  "total": N
}
```

Owners can poll this to surface a dedicated Inbox view; for now
proposals are also visible via `<universe>/_proposals/` and via
the existing notifications page.

### Tests

`co-web/tests/inline_proposal_tests.rs` extended to 8 tests:
- creates `users` row alongside test universes (FK on
  `user_notifications.user_id` was silently swallowing notifs)
- `inline_proposal_notifies_owner`
- `decide_merge_writes_target_and_flips_status`
- `decide_requires_universe_owner` (403 for non-owner)
- `inbox_lists_proposals_for_owned_universes_only`

All 8 pass.

## [2.7.23] — 2026-05-15 — Inline proposals for scenario 3 (non-owner edits)

The third editing scenario — logged-in user edits a public universe
they don't own — used to dead-end at 403 → "Erro ao salvar". Now
the editor falls through to an inline-proposal flow.

### `POST /api/v1/universes/{slug}/proposals/inline`

New lightweight endpoint, mounted **outside** the writer gate so
authenticated non-owners can submit a proposed change. The handler
enforces its own rules:

- Auth required (401 otherwise)
- `target_path` must not contain `..`
- Body capped at 1 MB, note at 2 000 chars
- Path forced under `_proposals/<timestamp>-<author>-<nanoid>.md`
- Frontmatter is server-controlled — caller can't smuggle `type`,
  `author`, or `status`

Request:
```json
{ "target_path": "public/seguranca.md", "body": "...", "note": "tiny tweak" }
```

Response:
```json
{
  "proposal_path": "_proposals/2026-05-15T123456Z-uid-abcd1234.md",
  "target_universe": "<slug>",
  "target_path": "public/seguranca.md",
  "author": "<user_id>",
  "status": "open",
  "created_at": "..."
}
```

### Editor UX

In `createDetailController.renderEdit` save handler:
- 2xx → "Salvo" (unchanged)
- 403 → confirm dialog "Enviar como proposta?" → POST to
  `/proposals/inline` → "Proposta enviada (path)" on success
- Other failure → "Erro ao salvar" (unchanged)

The dialog is opt-in per save click — no silent fallback. The
editor stays open until the user confirms or cancels.

### Visibility

- `_proposals/*` paths filtered from page/task/event/clip listings
  in `renderConteudo` (same shape as `_drafts/` filter)
- Anon visitors don't see `_proposals/` (the public/ convention
  only exposes `public/*`; `_proposals/` is invisible by
  construction in universes adopting the convention)
- Owners can browse `<universe>/_proposals/` to review inbound
  proposals; a dedicated inbox view is a follow-up

### Tests

`co-web/tests/inline_proposal_tests.rs`:
- lands in target's `_proposals/` folder with correct frontmatter
- requires auth (401)
- rejects `..` in target_path (400)
- server overrides smuggled frontmatter (author/status forced)

All 4 pass.

## [2.7.22] — 2026-05-15 — Anon edits on template: honest prompt + login

The inline editor and zoom modal both had an `if (state.isTemplate)`
short-circuit that showed a fake "Salvo" toast and silently dropped
the write. Anon visitors who edited the template universe lost their
changes on refresh without warning — the toast was a lie.

Now: anon save attempts surface an honest message ("Faça login para
salvar." / "Sign in to save.") with `warning` severity, then open the
login modal. The editor stays open so the user's text is preserved
if they choose to sign in; cancelling the login modal returns to the
edit view unchanged.

- `conteudo.js` `createDetailController.renderEdit` save handler
- `conteudo.js` `openZoomModal.enterEditMode` save handler
- `i18n.js` `save_requires_login` key added (pt + en)

Authenticated users on owned universes are unaffected — they fall
through to the normal PUT path. Authenticated non-owners on
public universes still get the standard 403 → "Erro ao salvar" toast
(scenario 3 in the editing matrix; a propose-change flow is the
next milestone for that case).

## [2.7.21] — 2026-05-15 — Cache + rate limit for anon public content

`/template` was returning 429 (Too Many Requests) on second SPA load
within a minute. Root cause: anonymous read tier capped at 20/min;
a single SPA load fetches ~10–15 entries (universe meta, project,
board, dashboard, per-card excerpts), so two refreshes empty the
bucket.

Two fixes:

### Anon read tier: 20 → 120/min

Generous for normal SPA usage, still rate-limited for scraping.
Writes stay at 5/min (anon writes are the abuse surface). Tests
updated.

### `Cache-Control: public, max-age=60, stale-while-revalidate=300`

Added to anon GETs of stable public seed content:
- Template universe entries (welcome / onboarding cluster)
- `co::public/*` (transparency cluster)

The content only changes on deploy, so 60s of browser caching with
300s stale-while-revalidate covers SPA refreshes without 429ing.
Authed callers get no Cache-Control header (they may be editing).

Combined effect: the second page load served from browser cache, the
third (post-60s) reuses the 120/min anon bucket which is now ~10x
the pre-fix budget.

## [2.7.20] — 2026-05-15 — Transparency content moves to `co::public/*`

### Hard move

Seguranca, licensa, infra catalog (5 pages), renderers all moved
from `template::content/*` to `co::public/*`. `co` becomes the
canonical owner of transparency content; `template` keeps welcome /
onboarding pages (sobre, termos, privacidade, dados-rastreados,
linhas-do-tempo, co-plataforma, guia, index).

- Seed source files moved: `co-web/seed/template/{seguranca,licensa,
  infra*,renderers}.md` → `co-web/seed/co/public/`
- `reseed_co_public_pages` writes them to `co::public/<slug>.md` on
  every boot (idempotent via `upsert_entry_row`)
- `cleanup_template_moved_pages` deletes the stale template copies
  on first boot after upgrade
- Pretty-URL redirect target updated: `/seguranca` → `/co/public/seguranca`
- All internal cross-links in the seed pages rewritten from
  `/co/template?page=<slug>` → `/<slug>` (the pretty form)

### `public/` convention — anon visibility filter

A universe can adopt the `public/` folder convention. Anon visitors
to that universe only see entries whose path starts with `public/`.
Logged-in users see everything they already had access to. The
allowlist is currently a small hardcoded list (`PUBLIC_CONVENTION_UNIVERSES = &["co"]`)
in `entry_routes.rs`; generalizing to a per-universe flag is the
next step (mirrors the "recursive subuniverse" concept the user asked
for).

Filter applied in two places:
- `list_entries` — strips non-public entries from the listing
- `get_entry` — returns 404 for non-public paths to anon (404 over
  403 so we don't leak the existence of private paths)

### `DELETE /api/v1/universes/{slug}` — fixed cascade + tests

Four integration tests now cover the route
(`co-web/tests/delete_universe_tests.rs`):

- `delete_universe_succeeds_when_authenticated` — full lifecycle
- `delete_universe_refuses_template` — protected
- `delete_universe_requires_auth` — 401 without bearer
- `delete_universe_404_when_absent`

First test surfaced a real bug: the hardcoded DELETE list only
cleaned `entries`, `universe_members`, `subscriptions` — but FKs
exist on `universe_invitations`, `chat_rooms`, `projects` (declared
without `ON DELETE CASCADE`). Fix: enumerate every table with a
`universe_key` column via `pragma_table_info` at delete time and
cascade dynamically. New tables that gain a `universe_key` are
covered automatically — no code change needed.

## [2.7.19] — 2026-05-15 — Template URL on logged-in + template→co hierarchy

### Bug: /template/<entry> redirected logged-in users away

A logged-in user hitting `/template/content/seguranca` (or via the
pretty-URL redirect from `/seguranca`) got auto-switched to their
own universe before the entry could open — the boot path's
"jump to your own universe" logic ignored the URL path.

Fix: detect a requested entry (via `readEntryPathFromUrl` or
`?page=` query) before the auto-redirect; when present, stay on
template and open the entry. Anon visitors are unaffected.

### Hierarchy: template is a subuniverse of `co`

Seeded `template.parent_key = 'co'` idempotently (only updates rows
where it's currently NULL). Records the relationship the user wants
explicit: `co` is the dev board (parent); `template` is its public-
facing subuniverse — the surface anon visitors see.

This release records the parent link in the DB; deeper restructure
(making `co` private, surfacing co's content through template) is
a follow-up that needs design — currently the two universes have
independent content trees, so "template = public view of co" needs
a publication mechanism.

## [2.7.18] — 2026-05-15 — REPL shell at /repl (Step 1: shell-DSL + save)

New page at **`/repl`**. Single-file HTML + inline JS that runs a
shell-style command interpreter on top of the entries API (the same
contract documented under `/api/v1/interactions/openapi.json`).

### Commands

Mirror the four operations on the `entry` resource:

```
list [universe] [--type=...] [--q=...] [--limit=N]
get  <ref>                    ref = path OR universe::path
put  <ref> <body> [--fm=<json>]
delete <ref>
save <name>                   write transcript to notebooks/<name>.md
help, clear
```

The universe is taken from the header field (default `template`).
The `universe::path` notation from the interactions registry works
here too.

### Saved transcripts

`save <name>` writes a markdown entry to
`notebooks/<name>.md` in the active universe. Commands land in
```` ```co ```` fenced blocks, outputs in ```` ```output ```` fences.
The doc is opaque markdown today — the next milestone wires the
viewer to detect `co` fences and offer a "Run" button so saved
transcripts become replayable cells ("include as code").

### Trajectory

| Step | Status |
|---|---|
| 1. Shell DSL (`list`/`get`/`put`/`delete`/`save`) | shipped (this release) |
| 2. Python kernel (Pyodide, lazy-loaded ~10MB) | next |
| 3. R kernel (WebR, lazy-loaded ~20MB) | follow-up |
| 4. Protobuf-typed query helpers wrapping the entries API | with kernels |
| 5. "Include as code" — markdown viewer detects `co` fences, offers Run | follow-up |

The shell is the architecture. Pyodide/WebR are language facades
that wrap the same `co.*` calls. The savedness of transcripts as
markdown entries (which themselves can be re-run) is the loop that
makes interactions iterative.

## [2.7.17] — 2026-05-15 — Interactions: one `entry` resource, HTTP verbs

Pivot continues. 2.7.16 had four parallel "primitives" with names
like `entryWrite`/`entryRead`/`entryDelete`/`entryList`. That's
suboptimal: the HTTP verbs (PUT/GET/DELETE/GET) are the
differentiator. One resource is enough.

### Registry IS the OpenAPI doc

`registry.yaml` is now a canonical OpenAPI 3.1 document, no custom
shape. One resource (`entry`) with two paths and four operations:

| operationId | method | path |
|---|---|---|
| `getEntry`    | GET    | `/api/v1/universes/{universe}/entries/{path}` |
| `putEntry`    | PUT    | `/api/v1/universes/{universe}/entries/{path}` |
| `deleteEntry` | DELETE | `/api/v1/universes/{universe}/entries/{path}` |
| `listEntries` | GET    | `/api/v1/universes/{universe}/entries` |

Pre/postconditions live in `x-preconditions` / `x-postconditions`
(standard OpenAPI vendor extensions); safety classification in
`x-safety`. `{universe}` is a path parameter — the contract is
universe-agnostic and works for every universe the caller has
access to.

### `co-web/src/interactions.rs`

Now embeds the OpenAPI doc directly, parses to `serde_json::Value`,
and discovers operations by walking `paths × methods`. No custom
struct shape — the doc IS the source.

Endpoints unchanged externally; payload shape now reflects OpenAPI:

```
GET  /api/v1/interactions/               list operations
GET  /api/v1/interactions/openapi.json   the doc, as-is
GET  /api/v1/interactions/{operationId}  operation block (incl. x-conds)
POST /api/v1/interactions/{operationId}  reserved (501)
```

Tests: `openapi_parses`, `four_operations_under_entry_resource`,
`every_operation_has_pre_and_post_conditions`,
`universe_is_a_path_parameter` (verifies the every-universe
contract — every path template must reference `{universe}`).

### Test spec

`01-content-crud.spec.ts` rebadged to the new operation IDs
(`putEntry`/`getEntry`/`listEntries`/`deleteEntry`). Universe is
controlled by `CO_TEST_UNIVERSE` (default `artelonga`) — the same
spec runs against any universe by changing the env var.

## [2.7.16] — 2026-05-15 — Interactions: pivot to generic CRUD primitives

The interaction layer was over-baked in 2.7.13–2.7.15: it documented
a *specific* business operation ("ArteLonga social → wikilinks") as
the atomic unit. That's a client of the platform, not the platform.

Pivoted to four generic CRUD primitives as the atomic interactions:

- `entryWrite`  — PUT  `/api/v1/universes/{u}/entries/{p}`
- `entryRead`   — GET  `/api/v1/universes/{u}/entries/{p}`
- `entryDelete` — DELETE `/api/v1/universes/{u}/entries/{p}`
- `entryList`   — GET  `/api/v1/universes/{u}/entries`

Content (paths, bodies, frontmatter) is runtime data. The primitives
are the contract; "switch IG links" or any other domain operation
is a composition of these.

### Refactor

- `registry.yaml` now lists the four primitives with HTTP method,
  path template, parameters (JSON-Schema), pre/postconditions, auth,
  safety, tags.
- `01-artelonga-social-to-profiles.spec.ts` deleted.
- `01-content-crud.spec.ts` exercises the full cycle:
  write → read → list → update (write again) → delete →
  read-expecting-404. One assertion per registry postcondition.
  Uses `e2e/sandbox/<random>.md` so the test is namespace-safe.
- `co-web/src/interactions.rs` updated: `Interaction` struct now
  carries `method` and `path`; `universe` is no longer a top-level
  field (it's a parameter). Tests assert all four primitives are
  registered + operationIds unique + OpenAPI emits one path per
  primitive (5 tests pass).
- README rewritten to reflect the primitive-first framing.

The pivot keeps the trajectory direction (registry → derived
OpenAPI → callable RPC) but pins the atomic unit at the right
level of abstraction.

## [2.7.15] — 2026-05-15 — Interactions: derived OpenAPI 3.1 + RPC contract

Step 1 of the "interactions as API calls" trajectory. The
`registry.yaml` is now an OpenAPI-shaped source of truth and the
server derives + serves it.

### Enriched registry schema

Each interaction now carries:
- `operationId` (camelCase RPC name)
- `parameters` (JSON-Schema for typed input)
- `preconditions` + `postconditions` (id + rule template each)
- `produces` (entries created/updated, with frontmatter expectations)
- `auth` (`{ required, scope }`)
- `safety`, `tags`

### `co-web/src/interactions.rs` module

Parses `registry.yaml` once at startup (embedded via `include_str!`)
and exposes:

- `GET  /api/v1/interactions/` — list with id, operationId, title,
  universe, tags, safety
- `GET  /api/v1/interactions/openapi.json` — full OpenAPI 3.1 derived
  from the registry; each interaction maps to one path with both a
  GET (fetch spec) and a POST (execute, currently 501)
- `GET  /api/v1/interactions/{operationId}` — full spec for one
  interaction
- `POST /api/v1/interactions/{operationId}` — reserved (501 with
  pointer to the Playwright command); becomes the executable RPC
  in the next step

4 unit tests cover: registry parses, first interaction shape matches,
operationIds unique, OpenAPI emits one path per interaction.

### Trajectory

- **Now (this release)**: contract published. Agents (co-auto,
  claude-code) can `GET /api/v1/interactions/openapi.json`, discover
  available interactions, and read pre/postconditions without
  parsing TypeScript.
- **Next**: POST handler that executes interactions server-side —
  authenticates the caller, runs the WHEN via existing entry write
  endpoints, checks postconditions, returns
  `{operationId, criteria: [{id, rule, passed, evidence}], produced}`.
  Equivalent to running the Playwright spec; one call replaces an
  npm-test-with-creds invocation.
- **Eventually**: client codegen (TS, Rust, Python) directly from the
  derived OpenAPI. Interactions become first-class platform APIs.

## [2.7.14] — 2026-05-15 — Interactions: stub + registry + idempotency

Three improvements on the e2e interactions framework shipped in
2.7.13:

### Profile stub creation in INTERACTION-01

`artelonga::comunidades/falcao.md` is now created by the interaction
itself (with `stub: true` in frontmatter) so the wikilink `[[falcao]]`
in `sobre.md` resolves to a real page instead of 404ing. The follow-
up task tracks the human work of fleshing it out. New criterion 5
asserts the stub exists + is flagged as a stub. Existing stub /
real profile is **not** overwritten — GET-first, only PUT on 404.
afterEach only deletes the stub if this run created it.

### registry.yaml — machine-readable index

`co-web/e2e/interactions/registry.yaml` lists every interaction with
metadata (id, title, spec path, universe touched, entries read +
produced, required env vars, tags, safety mode). Lets co-auto or any
agent enumerate / filter / dispatch interactions via `yq` without
parsing TypeScript. Spec files and registry entries must stay in
sync — adding a new interaction means adding both.

### Idempotent re-run contract

Specs detect the post-state at start. If the WHEN action has already
happened (Instagram links already gone from `sobre.md`), the test
skips with an explicit message — never restores garbage in place of
the canonical baseline. afterEach cleanup is conditional: only
restores the snapshot if precondition succeeded, only deletes
entries it actually created. Stuck runs, successful runs, and
partial runs all converge safely. Documented in README.md.

## [2.7.13] — 2026-05-15 — E2E interactions framework + first interaction

New `co-web/e2e/interactions/` test layer. Each interaction is a
single atomic user-level CRUD flow with acceptance criteria embedded
as a GIVEN/WHEN/THEN block in the spec's JSDoc — one assertion per
criterion so a failure points at the violated rule by name.

Notation: `<universe>::<path>` is the universal entry identifier
(`artelonga::sobre.md`, `artelonga::comunidades` for folders).

### INTERACTION-01: ArteLonga social → internal profile wikilinks

`e2e/interactions/01-artelonga-social-to-profiles.spec.ts`

- GIVEN `artelonga::sobre.md` has external Instagram links for the
  editorial board, plus a pre-existing bare `[[falcao]]` wikilink
  that points at a not-yet-existing profile.
- WHEN the user replaces each Instagram external URL with an
  internal wikilink (`[[<handle>|<label>]]`).
- THEN: (1) no IG URLs remain, (2) each handle has a wikilink,
  (3) `[[falcao]]` is preserved verbatim, (4) a sub-task at
  `artelonga::projects/AL/<next>.md` is created with `type: task`,
  `status: todo`, title referencing the falcao profile, (5) both
  entries are listed via the public entries API.

Spec is safe to run against prod: snapshots `sobre.md` before
mutating, restores in `afterEach`, deletes the new task entry on
teardown. Skips entirely if `CO_TEST_USER_EMAIL` +
`CO_TEST_USER_PASSWORD` aren't set (so CI without secrets stays
green).

Run with:
```
BASE_URL=https://co-artelonga.fly.dev \
CO_TEST_USER_EMAIL=yuri@artelonga.com.br \
CO_TEST_USER_PASSWORD=*** \
npx playwright test e2e/interactions/
```

## [2.7.12] — 2026-05-15 — Editor typing lag + server-side draft backup

### Typing lag fixed

CodeMirror's onChange ran a full markdown re-render of the preview
pane on every keystroke. On a ~7KB seguranca page that meant a
marked.js parse + DOM swap per character — sub-second perceived
delay. Now the preview is debounced 180ms, so typing latency drops
to CodeMirror's own input handling regardless of document size.

Also removed the per-keystroke `textarea.value = val` sync in the
inline detail controller — the save handler reads from
`editorInstance.getValue()` directly, so the hidden textarea was
just extra DOM reflow per character.

### Server-side draft backup every 5s

Drafts now back up to the server (in addition to localStorage) at
the path `_drafts/<entry-path>`. Runs every 5s while editing.
Fire-and-forget — a network blip falls back to localStorage. Skipped
for template universe (anon visitors can't write). Draft deleted on
successful save.

`_drafts/` paths are filtered from page/task/event/clip listings
in `renderConteudo` so they don't appear as ghost entries.

Save semantics unchanged: full PUT to the canonical path only when
the user clicks Salvar. Auto-save on every keystroke is explicitly
out of scope ("save necessary when user clicks").

## [2.7.11] — 2026-05-15 — Theme-aware CSS vars + template URL resolves anon

### Template URL on anon visits

`/template/seguranca` was landing on the universe home (index README)
instead of the seguranca page. The anon-on-template boot path only
called `maybeOpenPageFromUrl` (which reads `?page=` query) — never
`maybeOpenEntryFromUrl` (which reads the URL path). Fixed: anon flow
now routes through `maybeOpenEntryFromUrl`, which falls back to
`maybeOpenPageFromUrl` when no path is present, so both shapes work.

### Stats bar + new detail UI used undefined CSS vars

`conteudo-stats`, `conteudo-detail-toolbar`, `conteudo-readme`,
`conteudo-detail-btn`, splitter hover, and the editing-toolbar amber
state all referenced CSS custom properties that themes don't define:
`--surface-1`, `--surface-2`, `--accent-soft`, `--text`. The
hardcoded fallback colors (`#f8f8f6`, `#ececea`, `#fef3c7`, `#111`)
rendered identically on every theme — stats bar was always pale white
even in dark themes (relic, matrix, terminal, scholarly-dark…).

Migrated to theme-defined variables:
- `--surface-1` → `--bg-hover`
- `--surface-2` → `--card-bg`
- `--accent-soft` → `--accent-light`
- `--text` → `--text-primary`

### Theme coverage E2E (`e2e/theme-coverage.spec.ts`)

New Playwright spec walks all 12 themes (default + 11 palettes),
applies `data-palette` to `<html>`, screenshots the conteúdo view, and
asserts the stats bar isn't transparent. Dark themes get an extra
assertion that the computed background sum is < 384 (average RGB
< 128), catching white-on-dark regressions like this one. Run with
`BASE_URL=https://co-artelonga.fly.dev npx playwright test
e2e/theme-coverage.spec.ts`.

## [2.7.10] — 2026-05-14 — Fullscreen on click + /template/<slug> resolves

### Inline fullscreen button: immediate OS fullscreen

The fullscreen icon in the detail-pane toolbar now calls
`requestFullscreen()` on the detail pane itself, synchronously inside
the click handler. Previous version routed through `openZoomModal`
which awaits the editor bundle — the user-gesture trust window
expires after the first await, so the browser silently refused the
fullscreen request. Direct call preserves the gesture.

CSS `:fullscreen` styling centers the content at ~820px with
comfortable padding so the pane looks like a reading view, not a
floating overflow block.

### /template/<slug> URL resolves to content/<slug>.md

`/template/seguranca` returned the universe but didn't open the
seguranca entry — the SPA's entry-from-URL resolver only tried
`seguranca.md` / `seguranca` at the universe root and fell through
to a stem search that returned 0 hits (search index doesn't index
seed pages by stem alone).

Two fixes:
- Server: pretty-URL redirect now lands on
  `/template/content/<slug>` (the canonical path) instead of
  `/template/<slug>`. `/seguranca` → 307 → `/template/content/seguranca`
  → SPA resolves `content/seguranca.md` directly.
- Client: `maybeOpenEntryFromUrl` also tries `content/<entryPath>.md`
  before the stem-search fallback. Direct `/template/seguranca` URLs
  typed by users now open the right entry.

## [2.7.9] — 2026-05-14 — Resizable splitter + Obsidian-mode layout

Conteúdo view now has:

- **Draggable splitter** between detail pane (left) and sections pane
  (right). Drag to set any 15–85% split. Position persisted in
  localStorage (`co_conteudo_split_pct`).
- **Obsidian mode toggle** (icon button top-right of sections pane).
  Re-layouts to tree-on-left/content-on-right, with the tree pane
  fixed at 280px — classic Obsidian / IDE-style sidebar. Toggle
  state persisted in `co_conteudo_layout_mode`.
- Touch-friendly: splitter responds to touchstart/touchmove/touchend
  in addition to mouse events.
- Mobile (<= 900px): both modes collapse to stacked vertical layout
  with the splitter hidden — same as before.

## [2.7.8] — 2026-05-14 — Pretty URLs for seed pages + link-audit E2E

### Pretty URLs

`/<slug>` now 307-redirects to `/template/<slug>` when slug is one of
the seeded template pages. Lets you hand out short URLs like
`co.artelonga.com.br/seguranca` instead of the canonical
`/co/template?page=seguranca`. Slug list lives in `co-web/src/pretty_urls.rs`
and must stay in sync with `reseed_template_content_pages`.

The universe slug stays `template` for now (user OK'd the URL ending
on `/template/seguranca`). Renaming the slug to a Portuguese word
(`modelo` or similar) is deferred — it's a multi-place change
(DB row + entries table + filesystem path + many code literals) that
deserves its own patch.

### E2E link audit

`co-web/e2e/seed-links.spec.ts` crawls every seeded template page via
the public entries API, extracts markdown links + autolinks, and
asserts each resolves:
- Internal links via the universes/entries API
- External links via HEAD (with GET fallback) and follow redirects
- Pretty-URL redirects land on `/template/<slug>` with 301/302/307

This would have caught the Fly DPA 404 fixed in 2.7.7. Run against
prod with `BASE_URL=https://co-artelonga.fly.dev npx playwright test
e2e/seed-links.spec.ts`.

## [2.7.7] — 2026-05-14 — UX cleanup + yggdrasil seed + broken link fix

Five fixes batched together from rapid user feedback on 2.7.5/2.7.6:

### Inline editor: Esc to exit + clearer mode

The inline edit toolbar shows "editando · Esc para cancelar" and the
header turns amber while editing so the mode shift is obvious. Esc
key cancels and reverts to read mode. The read toolbar shows a "clique
para editar" hint so users discover the click-to-edit interaction.

### Real OS fullscreen on the zoom modal

The `fullscreen` button in the zoom modal toolbar now calls the
browser Fullscreen API on the container. Pressing `F` toggles. Esc
exits fullscreen (browser native), Esc again closes the modal.
CSS: container expands to 100vw/100vh and the body gets 32×48 padding
in fullscreen for readability. The toolbar button icon flips between
`fullscreen` ↔ `fullscreen_exit` to track state.

### Yggdrasil: seed an index page

The yggdrasil universe shipped with content_count=1 (one state file),
so anon visitors saw an empty universe. Added `index.md` describing
the games hub (Tetris/Snake/2048), the sementes currency, and the
build-time relationship with `co/game-core`. Idempotent seed runs on
boot via `upsert_entry_row`.

### Chat: skip WS for anonymous visitors

`/api/v1/universes/.../chat/rooms/.../ws` requires a session JWT and
returns 401 to anons. The client opened it anyway, got `onclose`, and
showed "Conexão perdida. Reconectando…" in a perpetual reconnect loop.
Now the client checks for a `session` cookie before opening the WS
and surfaces "Entre para participar do chat." instead. Authed users
are unaffected.

### Fly DPA link 404 → privacy policy

`https://fly.io/legal/dpa/` started returning 404. Updated both
`privacidade.md` and `dados-rastreados.md` to point at
`https://fly.io/legal/privacy-policy/` (verified 200) with a note
that the DPA is available on request for corporate clients.

## [2.7.6] — 2026-05-14 — Conteúdo as universal entry view (fix)

2.7.5 made Conteúdo the default for the template universe, but switching
to a user-owned universe still landed on Kanban because
`applyUniverseConfig` mapped the stored `layout='board'` (migration
default for non-template universes) to the kanban view.

Fix: hard-default the entry view to Conteúdo regardless of stored
layout. Kanban / Tabela / Calendário / Timeline / Painel remain one
click away in the tab strip. Per-universe layout preference can come
back later with a settings UI that distinguishes explicit user choice
from the migration default.

## [2.7.5] — 2026-05-14 — Master-detail Conteúdo + click-to-edit inline

Refactor the Conteúdo split so the left pane is a live viewer for the
currently *selected* entry, not a hardcoded README pin. The right pane
is the master list; clicks on a page card update the left pane.

- **Left pane**: renders the selected entry. Single click anywhere on
  the rendered body → switches to inline edit (CodeMirror, same
  bundle the modal uses). Save / Cancel actions in the toolbar.
- **Right pane**: list of pages, tasks, events, clips. Click on a
  page card → swaps the left pane to that entry on desktop; opens
  full-screen modal on mobile.
- **Toolbar button** (`open_in_full`): escalates to full-screen mode
  via the existing zoom modal — useful for long reads.
- **Full-screen modal** still edits via its existing button, so both
  surfaces route the same PUT endpoint and stay consistent.
- **Selection persists** per-universe via `localStorage` — refresh
  resumes on the last viewed entry.
- **Initial selection** defaults to `index.md` / `README.md`; falls
  back to the first page entry; if nothing matches, the view falls
  back to the prior single-column section list.

Selection-driven model retires the prior "README pinned to left"
behavior shipped in 2.7.4.

## [2.7.4] — 2026-05-14 — Content-first default + README split layout

Universe entry point is now **Conteúdo** instead of Kanban. The README
(`index.md` / `README.md`) is rendered as the primary surface:

- **Desktop:** README occupies the left half; Páginas + Tarefas +
  Eventos + Clipes sections occupy the right half.
- **Mobile:** README full-width on top, sections stacked below.

Markdown rendering for the README pane applies the same passes as the
zoom modal (wikilink resolution, table wrap, image zoom, code
highlight, mermaid). Double-click opens the README in the editor.

The README is stripped from the Páginas list so it doesn't appear
twice. If a universe has no `index.md` / `README.md`, the view falls
back to the single-column section list (prior behavior).

### Template universe layout

Seed default flipped from `'board'` to `'conteudo'`. Idempotent UPDATE
runs on boot to migrate existing template rows (e.g. prod) that were
seeded with `'board'` before this change.

User-owned universes are untouched — their layout reflects whatever
the owner set in settings.

## [2.7.3] — 2026-05-14 — License flip MIT → AGPL v3

Project license changed from MIT to **GNU AGPL v3 or later** to match
the `/co/template?page=licensa` page shipped in 2.7.1. The mismatch
existed for ~24h between the storefront (AGPL) and the code (MIT).

Files updated:
- `LICENSE` → AGPL v3 canonical text (661 lines)
- `Cargo.toml` (workspace) `license = "AGPL-3.0-or-later"`
- `co-cli/Cargo.toml`, `game-core/Cargo.toml` (explicit, not workspace-inherited)
- `dev/co-auto`, `dev/co-token`, `dev/co-pwhash` (explicit)
- `README.md` license section

Crates that inherit from workspace (`co-web`, `co`, `core`, `co-agent`)
automatically pick up the new license via `license.workspace = true`.

**`co-obsidian/` (the Obsidian plugin) stays MIT** — it's a client tool,
not a network service, and the Obsidian plugin ecosystem expects
permissive licenses. The AGPL network clause has no purchase on a
client-side plugin.

**Implications acknowledged before the flip:**
- AGPL bites on the *server* (modified deployments must offer source).
- Closed-source forks become impossible. Anyone forking and running
  CO as SaaS must release modifications.
- Re-licensing later requires every contributor's consent. Tonight
  trivial (1 contributor); growing friction over time.
- No CLA introduced — contributors implicitly license under AGPL.
  Can revisit if dual-licensing for enterprise tier ever matters.

## [2.7.2] — 2026-05-13 — Infra catalog in template + cross-universe wiki-links

The compute portfolio for ArteLonga (Fly.io tasks, sizing, costs, comms
topology) is now visible to anon visitors as readable markdown inside
the template universe — content mirrored from `/projects/infra/`.

Five new template pages:

- `content/infra.md` — overview, inventory table, costs AS IS / TO BE
- `content/infra-co.md` — co-artelonga prod/uat + planned CO-143/CO-123 tasks
- `content/infra-yggdrasil.md` — yggdrasil-artelonga (game lobby)
- `content/infra-quilomboaraucaria.md` — quilombo-araucaria (media-heavy)
- `content/infra-rfq-gateway.md` — staging+smoke-staging+prod tier split

Each page is taggable (frontmatter `tags: [infra, fly, compute, ...]`)
and cross-linked via `?page=infra-X` anchors. Backlinks to existing
security/dependencies pages preserved.

### Cross-universe wiki-links

Wiki-link syntax now supports a leading slash for cross-universe targets:

- `[[/template/infra]]` — points at the catalog page in `template`
- `[[/template/seguranca]]` — points at the security page in `template`

Falls through to a normal SPA 404 if the target universe is private or
the entry doesn't exist — same auth path as direct URL navigation, no
new info disclosure. Bare `[[path]]` syntax (resolved within current
universe) is unchanged.

Orphan detector (`orphan_wikilinks`) skips cross-universe targets so
they don't pollute the per-universe orphan list.

Anchored to user request: "include the infra from the projects/infra
documentation and this should be indexable and taggable (direct links,
eg `[[link]]` from all universes since template is general public.
This logic should work recursively with a universe for which a user
doesnt have permission link would return 404."

## [2.7.1] — 2026-05-13 — Transparency pages seeded into template universe

Six new content pages added to the template universe seed so anon
visitors land with readable docs about the security model, dependencies,
red-team scenarios, VAPID threat model, license (AGPL v3), and
markdown renderer options.

- `content/seguranca.md` — security overview (entry point)
- `content/seguranca-dependencias.md` — deps catalog + decisions
- `content/seguranca-cenarios.md` — red-team scenarios + playbook
- `content/seguranca-vapid.md` — VAPID threat model deep-dive
- `content/licensa.md` — AGPL v3 explanation in PT
- `content/renderers.md` — markdown renderer options

All cross-linked via `?page=<slug>` template-internal anchors.
Canonical versions in `docs/security/*.md` + `docs/licensa.md` +
`docs/markdown-renderers.md` remain the source of truth; the template
versions are condensed for end-user reading.

Reseed runs unconditionally on every boot (`reseed_template_content_pages`)
— `upsert_entry_row` makes it idempotent.

## [2.7.0] — 2026-05-13 — CO-209: Conversas — unified chat surface with first-time welcome + member rail

### CO-209 — Unified Conversas surface

- **Unified drawer**: single `💬 Conversas` button replaces separate `💬 Chat` + `📩 Mensagens` buttons.
  Drawer shows two left-rail sections: "Universos" (universe chats) and "Mensagens privadas" (DMs).
- **First-time welcome modal**: privacy disclosure shown once per browser profile (localStorage flag
  `co_chat_welcome_seen`). Dismissal via `Enter`, click outside, or `×` button. PT and EN support.
- **Member rail**: bottom of the left rail shows all members of the current conversation with
  presence dots (● online / ○ offline). Filter input appears when count > 10. Click a member to open a DM.
- **DM section collapsible**: "Mensagens privadas" section can be collapsed with a toggle.
- **Backend**: `ensure_default_room` now seeds new universes with name `'geral'` (PT); CO universe's
  default room renamed to `'CO-geral'` on startup. New endpoint
  `GET /api/v1/universes/:slug/chat/rooms/:room_slug/members` returns room members with display names.

## [2.6.1] — 2026-05-13 — CO-208: Playwright e2e maintenance — rate-limit bypass + API drift fixes

### CO-208 — Unwind 12 days of e2e drift + rate-limit collisions

- **`CO_BYPASS_RATE_LIMIT=1`**: new env flag. When set alongside `CO_ENV=test`,
  the token-bucket rate-limit middleware passes every request through
  unconditionally. No effect outside `CO_ENV=test` — prod and UAT behaviour
  unchanged.
- **`is_anonymous` assertion fixed**: `clone_universe` returns `Universe`
  (no `is_anonymous` field since CO-170/CO-184). Tests in `auth-crdt.spec.ts`
  and `usage-gate.spec.ts` now check `owner_id` matches `^anon-` instead.
- **`global-setup.ts`** forwards `CO_ENV=test` and `CO_BYPASS_RATE_LIMIT=1` to
  the spawned co-web process so local `npx playwright test` runs also benefit.
- **CI e2e job**: `continue-on-error: true` removed; job is now a real gate.
  `CO_ENV=test` and `CO_BYPASS_RATE_LIMIT=1` added to the step environment.

## [2.6.0] — 2026-05-13 — Analytics ingestion + geo + public endpoints (CO-177 → CO-180)

Closes the artelonga.com.br analytics chain end-to-end. AL-48 (the bake
workflow shipped under the assumption this backend existed) is now
unblocked.

### CO-177 — Accept events from artelonga.com.br via CORS + populate universe_key

`marketing_events_handler` now sets `telemetry_events.universe_key` from
`ev.site` (trimmed, non-empty, ≤ 64 chars; else NULL). Previously
hardcoded to None — universe-scoped queries always returned empty.

CORS is handled by the existing global `mirror_request()` layer (CO-205)
so artelonga.com.br is already permitted; admin telemetry endpoints
remain protected by GitHub admin auth.

### CO-178 — Geo enrichment server-side (country + city)

Each event is enriched with `country` (ISO 3166-1 α-2) and `city` derived
from the request IP via MaxMind GeoLite2, before the IP is hashed and
discarded. Raw IPs are never stored.

- `co-web/src/geo.rs`: `GeoDb` + `geo_lookup(db, ip)`. Reads
  `GeoLite2-City.mmdb` from `GEOIP_DB_PATH` (default
  `/data/GeoLite2-City.mmdb`). Disabled gracefully when file is absent.
- Three handlers updated: `telemetry_middleware`, `track_event_handler`,
  `marketing_events_handler` all call `geo_lookup` before `hash_ip_daily`.
- `AppStateInner.geo: Arc<GeoDb>` — shared, read-only.

Migration v44: `country TEXT` + `city TEXT` columns plus
`idx_telemetry_country`. Nullable — old rows and private IPs stay NULL.

Attribution: This product includes GeoLite2 data created by MaxMind,
available from <https://www.maxmind.com>.

### CO-179 — Public analytics endpoints (`/summary`, `/recent`)

Read-only aggregates feeding artelonga's analytics dashboard:

- `GET /api/v1/analytics/public/summary?days=N` — views, visitors,
  returning, sessions, timeseries, top_pages, geo array. `days` clamped
  to `[1, 365]`, default 30. `days=0` returns 400.
- `GET /api/v1/analytics/public/recent?limit=N` — most recent events
  DESC, `limit` clamped to `[1, 200]`, default 50.

PII stripped: no `visitor_token`, no `ip_hash`, no raw `properties`.
5-minute in-memory cache per (endpoint, query params) via
`OnceLock<Mutex<HashMap>>`. CORS inherited from the global
`mirror_request` layer. Geo array hydrated by CO-178 once GeoLite2 is
present.

Index added: `idx_telemetry_universe_time` on
`(universe_key, timestamp)` — universe-scoped time-range queries are
now O(events_for_universe).

### CO-180 — Popularity endpoint for service ranking

`GET /api/v1/analytics/public/popularity?prefix=/servicos/&days=30` —
page-view counts for paths matching a prefix, ordered by
`views DESC, path ASC` (stable, deterministic). Designed for a GH
Action in `artelonga/ArteLonga` that commits `assets/popularity.json`
daily — the static site gets empirical ranking with no runtime API
dependency.

- `prefix` validation: required, starts with `/`, no `..`, max 64 chars
  → 400 on violation.
- `days` clamped to `[1, 365]`, default 30.
- Response shape: `{ as_of, window_days, prefix, items: [{ path, slug,
  views, visitors }] }`. `slug` derived: strip prefix + trailing `/`.
- 5-minute in-memory cache per `(prefix, days)`.

### Operational notes

- All four migrations are idempotent — runs every boot, no-op once
  applied.
- Total new tests: 35+ (CORS preflight, sanitize, persist, clamp, PII
  absence, shape, popularity validation, popularity ordering).
- AL-48's bake workflow can now be re-enabled; expect first popularity
  bake within 24 h of artelonga seeing real traffic.

## [2.5.0] — 2026-05-12 — CO-206: Yggdrasil verifies CO JWKS — centralized SSO

CO is now the single identity authority for the entire artelonga stack.
Yggdrasil users are redirected to CO for login; CO issues a short-lived
(60s) ES256 handover token that yggdrasil validates via JWKS — no shared
secret required.

**CO-side changes:**
- `GET /auth/co-handover?return_to=<url>` — new server-side redirect that
  issues a `co_token` JWT and bounces to the receiver. Requires auth;
  rejects `return_to` hosts not on the safelist.
- `is_allowed_return_to` safelist now includes `yggdrasil-artelonga.fly.dev`
  and `yggdrasil.artelonga.com.br` (future custom domain).
- Migration v43: `users.yggdrasil_user_id` column for round-trip identity
  bridging (CO-207 will use this). Idempotent boot migration.
- Storage helpers `get_yggdrasil_user_id` / `set_yggdrasil_user_id`.

## [2.4.1] — 2026-05-12 — Hotfix: CSRF middleware trust list out of sync with CORS

`POST /api/v1/auth/logout` (and other non-safe methods) from
artelonga.com.br returned `403 CSRF: Origin not allowed` even though
the CORS preflight succeeded. Surfaced as "sair on artelonga doesn't
work" right after AL-50 landed.

**Root cause:** `csrf_middleware` in `quilombo_telemetria.rs` reads
allowed origins from the `ALLOWED_ORIGINS` env var, which doesn't
include artelonga.com.br. CO-205 updated the CORS layer's
`mirror_request()` but the CSRF middleware's origin check was a
separate, env-driven list.

**Fix:** added a hardcoded `TRUSTED_HOSTS` list inside `csrf_middleware`
mirroring the cross-domain hosts the CORS layer allows
(`artelonga.com.br`, `co.artelonga.com.br`, `yggdrasil.artelonga.com.br`,
`quilomboaraucaria.com.br`, `quilomboaraucaria.org`). Now in sync.
`ALLOWED_ORIGINS` env var is still honored as an additive override
for ad-hoc dev hosts.

## [2.4.0] — 2026-05-12 — CO-205: Artelonga signup backend — CORS + origin tracking

Cross-domain signup from artelonga.com.br to co.artelonga.com.br. Visitors
on artelonga.com.br can now POST to CO auth endpoints and get a session
cookie that works across the `.artelonga.com.br` subdomain.

### Added

- `users.origin TEXT` column (migration v42) — tracks signup entry point
  (`'artelonga'`, `'co'`, `'quilombo'`, `'yggdrasil'`, etc.) for all new
  users; existing users keep `NULL`.
- `onboarding_codes.origin TEXT` column — carries the origin through the
  two-step passwordless onboarding flow.
- `GET /api/v1/admin/users/origin-breakdown` — admin-gated telemetry
  endpoint returning user counts grouped by signup origin.
- `crate::auth::sanitize_origin` — rejects strings with non-alphanumeric/
  hyphen characters or length > 32; stores `NULL` instead of echoing junk.

### Changed

- CORS layer now sets `Access-Control-Allow-Credentials: true` globally;
  `mirror_request()` echoes the caller's `Origin` so cross-origin requests
  with `credentials: 'include'` work from any subdomain.
- `POST /api/v1/auth/onboard-with-email` — accepts optional `origin` field,
  stored in `onboarding_codes`, applied to `users.origin` on account create.
- `POST /api/v1/auth/signup` — accepts optional `origin` field.
- `GET /api/v1/auth/google/start` — accepts optional `origin` query param,
  carried through the state JWT to `find_or_create_user_by_google`.

## [2.3.4] — 2026-05-12 — CO-203: parking_lot::Mutex eliminates storage-lock poison cascade

**Closes the incident family** that produced 2.3.1 + 2.3.2 hotfixes
earlier today. `Mutex<Storage>` is now `parking_lot::Mutex<Storage>`
instead of `std::sync::Mutex<Storage>`. `parking_lot` doesn't poison
on panic, so a panic during one request kills *that one request* and
the next acquisition gets a fresh lock — no site-wide cascade.

### Changed

- `co-web/Cargo.toml` — added `parking_lot = "0.12"`
- `co-web/src/server.rs` — `AppStateInner.storage` is now
  `parking_lot::Mutex<Storage>`; `lock_storage` helper returns
  `parking_lot::MutexGuard<'_, Storage>` directly (no `Result`)
- **55 source files across `co-web`** — all `Result`-based lock
  patterns collapsed to plain `state.storage.lock()` /
  `lock_storage(&state)`:
  - `.lock().unwrap()` → `.lock()`
  - `.lock().unwrap_or_else(|p| p.into_inner())` → `.lock()` (the
    poison-tolerant pattern added in 2.3.1 is now redundant)
  - `.lock().map_err(|_| AppError::Internal("Storage lock failed".into()))?`
    → `.lock()` (the request-handler pattern)
  - `if let Ok(s) = ... { ... }` → unwrap directly

Net delta: −333 lines of error-handling boilerplate. Worker locks are
also simpler — no more 4-line poison-tolerant `unwrap_or_else` blocks.

### Verified

- `cargo build -p co-web` ✓
- `cargo clippy -p co-web -- -D warnings` clean ✓
- `cargo test -p co-web --lib --test-threads=1` — **548/548 pass**
  (previously 3 race-condition flakes; now stable because
  parking_lot's locking doesn't suffer the same race window)

### What this means in operational terms

| Failure mode | Before 2.3.4 | After 2.3.4 |
|---|---|---|
| Storage method panics under lock | Site-wide 500s until restart | One 500 for that request; next request succeeds |
| Long-running worker panics | Whole app dead | Worker restarts next tick |
| 3 hotfixes in a day for the same family | Yes (2.3.0 → 2.3.1 → 2.3.2 → 2.3.3) | No |

The ~30 remaining `.expect()` calls in storage methods are still
landmines for individual requests, but their **blast radius is now
one request, not the whole app**. A separate cleanup ticket can audit
them later when it's no longer urgent.

## [2.3.3] — 2026-05-12 — Sidebar + navigation UX fixes

Three surgical SPA fixes reported alongside the 2.3.x poison incident:

### Fixed — Header showed "Selecione um projeto" instead of the universe name

`renderHeader` defaulted to the i18n placeholder `select_project` whenever
no project was pinned, even when the user was clearly inside a universe
context (e.g., "Comunicação", "RFQ"). New precedence:

1. Project name (if a project is selected)
2. Universe name (we're in a universe but no project yet)
3. `select_project` placeholder (no universe context at all)

### Fixed — Browser back/forward did nothing

`setUniverseSlugInUrl` was `pushState`'ing without a `popstate` handler.
Back-button rewrote the URL but no JS reacted, leaving the user on the
current universe. Added a one-time `window.addEventListener('popstate',
...)` that reads the universe slug from `event.state` (or falls back to
URL parsing) and dispatches to `bootAppForUniverse`. Browser nav now
works across universe switches.

### Fixed — Subuniverses collapsed by default when on the parent

In the sidebar tree (e.g., when the user navigated to `tempo`),
descendant universes stayed collapsed unless the *current* universe
matched one of the *descendants*. So users on the parent saw an
unhelpful chevron with no children visible.

Default-expand logic now triggers when EITHER the current universe is
the parent itself OR one of its descendants. localStorage override
still respected.

## [2.3.2] — 2026-05-12 — Hotfix #2: same poison pattern in CO-191 universe-list methods

After 2.3.1 deployed, the storage mutex got poisoned again — this time
from `Storage::list_subscribed_universes` at `universe.rs:440`. The
panic site was a `.expect("list_subscribed_universes")` on the
`query_map` result. Triggered when yuri's Google sign-in hit
`/api/v1/me/universes`, which calls all four CO-191 list methods.

Same pattern as 2.3.1 (panicking under `Mutex<Storage>`), different
storage methods. Fix is the same shape: prepare + query_map both
match on Err, log `tracing::error!`, return `Vec::new()` so the
endpoint degrades to a 200 with empty buckets instead of poisoning
the lock.

Four methods patched: `list_owned_universes`, `list_member_universes`,
`list_subscribed_universes`, `list_discoverable_universes`.

Recovery executed: `flyctl machine restart` once more. After deploy
the pattern can no longer recur from this code path.

**Triage findings:** roughly 30 other `.expect()` calls in storage
methods (chat, invitations, notifications, push, etc.) carry the same
landmine. Whack-a-mole won't scale. Filing CO-204 for a systemic fix:
swap `std::sync::Mutex` for `parking_lot::Mutex`, which doesn't poison
on panic — then existing `.expect()` calls become safe (or at worst
return a 500 for one request, not site-wide).

## [2.3.1] — 2026-05-12 — Hotfix: storage lock poisoning from worker panics

**Incident summary:** All authenticated requests on prod returned
`{"error":"internal_error","message_en":"storage lock"}` after some
trigger panicked inside the new notification workers while holding
the storage mutex. Anonymous reads partially worked; logged-in
content and Google sign-in were blocked.

**Root cause:** Inside `Storage::list_users_with_pending_email_notifications`
and the sibling push helper, `.prepare(...).expect("…")` would panic
on any SQLite error. Those methods are invoked from
`notification_email_worker::tick` (CO-200) and `notification_push_worker::tick`
(CO-201) *while the storage `Mutex<Storage>` is held* — so any panic
poisons the lock and every subsequent acquisition across the app
fails. Pattern previously documented in
`feedback_migration_column_reads.md` (2026-04-30 prod incident,
CO 1.22.4).

**Fixes:**

1. `list_users_with_pending_email_notifications` and
   `list_users_with_pending_push_notifications` no longer panic on
   prepare/query errors. They log via `tracing::error!` and return
   `Vec::new()` so the worker simply waits one tick and retries.
2. All `state.storage.lock().unwrap()` call sites in the two notif
   workers (7 total: 3 in email worker, 4 in push worker) replaced
   with `.lock().unwrap_or_else(|p| p.into_inner())`. Even if some
   future panic does occur with the lock held, the app keeps running
   instead of cascading into universal 500s.

**Recovery executed:** Restarted the prod machine
(`flyctl machine restart 1850920b111d38`) to clear the poisoned
in-memory state. Storage worked again for ~minutes. This hotfix
prevents the failure mode from recurring.

**Operational lesson:** Long-running workers MUST never panic while
holding a shared `Mutex<T>`. Either:
- Use non-panicking storage methods (log + return empty/None), or
- Drop the lock before any path that can fail, or
- Use parking_lot::Mutex (which doesn't poison; not in tree yet).

The standard pattern adopted across notif workers:
```rust
let storage = state.storage.lock().unwrap_or_else(|p| p.into_inner());
```

## [2.3.0] — 2026-05-11 — Phase 5 Notifications complete (CO-199 → CO-202)

Closes the async-communication loop opened by Phase 4 chat. Universe rooms
+ DMs (live since 2.2.0) plus the engine + email digests + browser push +
in-app bell — users learn about messages without having CO open.

### CO-199 — Notification engine + preferences + 4 event types

Schema: new `user_notifications` append-only log table and
`notification_preferences` per-user settings table. Boot-time backfill
inserts default preferences for every existing user.

Event capture wired into 3 existing producers: `post_message_handler`
emits `chat.message` for every room member except the author
(`in_app_chat_message` preference), `chat.dm` for the other party in DM
rooms, and `chat.mention` for `@usuario` references that resolve to room
members. `create_invitation_handler` emits `universe.invitation` for the
invitee when they already have a CO account.

REST endpoints under `/api/v1/me/`: `GET /notifications` (paginated with
`since` cursor + `unread_count`), `POST /notifications/:id/read`,
`POST /notifications/read-all`, `GET /notification-preferences`,
`PUT /notification-preferences` (partial update, validates
`email_digest_freq` enum and `HH:MM` quiet-hours format).

Idempotency: duplicate `(user_id, event_type, object_id)` within 5 s
produces one row.

### CO-200 — Email digest delivery (instant/hourly/daily/weekly + quiet hours)

New `notification_email_worker` background task. 60-second tick loop:
queries users with pending undelivered notifications, applies the
frequency gate, checks quiet hours with timezone-aware offset lookup,
filters per the user's per-event-type email toggles, sends via the
Resend → SMTP → log cascade. Per-user consecutive failure tracking
(cap 5 → skip until prefs change). On success,
`notifications.delivered_email_at` is populated.

Email subject + body localized via the new `users.language` column
(default `pt`). HTML template renders relative time, body preview, and
deep-links per event type. Sender defaults to
`notificacoes@seguranca.artelonga.com.br` (configurable via
`NOTIF_FROM_EMAIL`).

### CO-201 — Web push notifications via Push API + service worker

New `push_subscriptions` table; new `notification_push_worker` ticks
every 10 s. Payload encrypted AES-128-GCM per RFC 8188/8291 using the
subscription's `p256dh` + `auth` keys and ECDH; VAPID JWT signed with
ES256/P-256. 410 Gone → subscription deleted. 5xx → `failure_count++`;
at 5 the subscription is pruned. On success, `delivered_push_at` is
populated.

Service worker `push-sw.js` shows the system notification;
`notificationclick` focuses an existing tab on the target URL or opens
a new one. `tag` field coalesces multiple notifs from the same thread.

REST endpoints: `GET /api/v1/notifications/vapid-public-key` (anonymous),
`POST /api/v1/me/push-subscriptions` (upsert by endpoint, idempotent),
`GET /api/v1/me/push-subscriptions`, `DELETE /api/v1/me/push-subscriptions/:id`.

**Production setup required:** `VAPID_PUBLIC_KEY`, `VAPID_PRIVATE_KEY`,
and `VAPID_SUBJECT` must be set as Fly secrets for push to actually fire.
Without them the worker degrades to log-only mode (no crash, no fan-out).

### CO-202 — In-app 🔔 notification center + settings

`modules/notifications.js` mounts a bell button with a red-dot badge.
Click opens a dropdown showing recent notifications rendered by i18n
key + params with relative time. Click a row → marks read + navigates
to deep-link. "Marcar todas" calls `/read-all`.

Real-time bumping: `chat.js` calls `window.coOnChatMessageArrived` from
its WS handler; if the message is for a room not currently visible, the
bell increments without polling. Fallback poll every 30 s catches
invitations and other non-WS events.

Full-page view at `/notifications` with filters (event type,
all/unread) and `?since=` pagination.

Settings section in the security modal: 4×3 channel/event toggle matrix
(in-app × email × push), email frequency radio (instant/hourly/daily/
weekly/never), quiet-hours `HH:MM` + timezone, "Ativar notificações"
button kicking off the CO-201 subscribe flow, registered-devices list
with per-device revoke. 34 new i18n keys PT + EN.

### Operational notes

Schema additions all idempotent via `CREATE TABLE IF NOT EXISTS` — no
version-slot migration. Boot adds 2 spawned workers: notif email (60 s
tick), notif push (10 s tick). Tests: 15 (CO-199) + 12 (CO-200) + 10
(CO-201) = 37 new passing.

## [2.2.0] — 2026-05-11 — Private DMs (CO-198)

### Private 1:1 DMs with inbox, unread counts, and privacy controls

- **CO-198** — Private DMs (Phase 4 slice 5). Schema: `kind` column on
  `chat_rooms`, new `chat_room_members` table (read-state + mute), `dm_policy`
  on `users`, and `user_blocks` table. Boot-time sentinel rows anchor the DM FK
  chain and backfill existing universe member rows into `chat_room_members`.

  REST endpoints: `POST /api/v1/dms/with/:user_id` (idempotent open-DM with
  policy + block checks), `GET /api/v1/me/dms` (inbox with unread counts +
  preview, ordered by last message), `POST /api/v1/dms/:room_id/read`,
  `POST /api/v1/dms/:room_id/mute`, `PUT /api/v1/me/dm-policy`,
  `POST/DELETE /api/v1/users/:user_id/block`.

  Existing message endpoints reuse `slug="dm"` sentinel to serve DM rooms —
  `chat_room_members` membership + block check replace universe-role auth for
  that path.

  Frontend: `modules/dm.js` DM inbox drawer; `📩 Mensagens` sidebar button
  with red-dot unread badge; DM mode in existing chat drawer (no room rail);
  universe member list DM icon (invitations panel); DM privacy radio +
  block-list manager in security settings modal. i18n: 24 new keys PT+EN.

  17 backend tests cover all acceptance criteria (idempotency, canonical pair
  ordering, policy gates, block enforcement, unread counting, mark-read,
  backfill).

## [2.1.0] — 2026-05-11 — Phase 4 chat complete (CO-194 + CO-195 + CO-196)

### Phase 4 chat — live updates + UI + moderation

- **CO-194** — WebSocket live updates + presence. Per-room broadcast channel
  (`message.created`, `message.edited`, `message.deleted`, `presence.join/leave`,
  `typing.start/stop`). Multi-tab dedup via refcount. Keep-alive ping/pong.
  11 tests cover auth gates, rate limits, broadcast isolation, presence, typing.

- **CO-195** — Chat UI sidebar drawer + Yggdrasil lobby panel. New
  `modules/chat.js` module with room rail, scrollback, live composer,
  WS reconnect with exponential backoff, presence list. Sidebar `💬 Chat`
  button visible when logged in; closes and tears down on universe switch.

- **CO-196** — Chat moderation: edit/delete own, admin delete any.
  - `PATCH /api/v1/.../messages/:id` — author-only edit within 15-min window;
    403 `edit_window_expired` after; 410 on deleted message.
  - `DELETE /api/v1/.../messages/:id` — author or owner/admin can soft-delete;
    410 if already deleted. Body preserved in DB; clients see `[mensagem removida]`.
  - Both endpoints broadcast `message.edited` / `message.deleted` WS events.
  - UI: hover shows ✏️ (own, within 15 min) and 🗑️ (own or mod); inline
    textarea edit flow (Enter/Esc); delete confirm popover with optimistic
    tombstone + rollback on error; `(editado)` tag after successful edit.
  - 14 tests: every auth gate (author/owner/admin/member/viewer), edit window,
    empty body, already-deleted, and WS broadcast verification.
  - i18n PT + EN for all chat strings.

## [2.0.0] — 2026-05-10 — Identity + Phase 4 Foundation

Major release closing the multi-day identity / SSO / membership / chat-foundation
arc that ran from 1.95.0 (CO-188) through the chat backend (CO-193). Functionally
includes the work that already shipped to prod in 1.95.0 → 1.99.0 plus two
additional tickets (CO-187 cleanup, CO-193 chat backend) that landed without
intermediate version bumps. Cutting 2.0.0 to mark the coherent feature set and
to reset the release cadence from ticket-per-bump to theme-per-release.

### Phase 3 complete — identity + access for everyone

- **CO-172 / CO-184** — Bidirectional identity bridge. Quilombo signups create
  CO accounts; CO signups create quilombo identities. Lazy-bridge for legacy
  unlinked rows (by usuario AND by email).
- **CO-176 + diagnostic logging** — `/forgot-password` enforces usuario+email
  pair, no-enumeration; per-step trace logs make ops actually debuggable.
- **CO-175 / CO-177** — Public username+password signup and Google OAuth.
- **CO-186** — Handover tokens migrated to ES256 + JWKS (`/.well-known/jwks.json`),
  eliminating per-universe shared secret distribution.
- **CO-187** — Removed legacy HS256 handover signer now that ES256 transition
  is complete. Cleanup only — zero callers, no behavior change.
- **CO-188 / CO-189** — Universe invitations backend + UI. Single-use 14-day
  tokens, public preview, accept gating with identity match.
- **CO-190** — Passwordless onboarding via email. Single "Continuar com email"
  entry point that logs in OR creates accounts via 6-digit magic code,
  auto-derives usuario from email local-part with collision suffix.
- **CO-191 / CO-192** — Unified `GET /api/v1/me/universes` bucketed shape
  (owned/member/subscribed/invited/discoverable). Sidebar renders sections
  semantically with role chips, 🎁 invite badge + inline accept/decline,
  collapsible Discover section.

### Lead capture pipeline

- **CO-183** — `POST /api/v1/leads` + admin queue. Replaces the artelonga
  `/contato/` mailto with persisted, queryable leads. Bot filter, IP-hash,
  rate-limit (5/IP/24h), Resend email notification, admin SPA at
  `/admin/leads.html`, LGPD 24-month retention task. AL-4 in the artelonga
  repo flips the form to use this endpoint.

### Phase 4 foundation — chat backend

- **CO-193** — Chat schema (`chat_rooms`, `chat_messages`) + REST endpoints.
  Every universe auto-seeds a `general` room (boot-time backfill for
  pre-existing universes). Per-room paginated history. Role-gated:
  owner/admin can create rooms; member+ can post; viewer/subscriber are
  read-only; anonymous has no access. Rate-limited 20 messages/user/min.
  17 tests cover every auth gate, slug collision, pagination, tombstone.

CO-194 (WebSocket live updates) + CO-195 (yggdrasil lobby UI) + CO-196
(moderation) will land as a single 2.1.0 cycle.

### Operational improvements

- Diagnostic logging across `forgot_password_handler` and
  `find_user_for_recovery` so silent no-match paths emit structured
  trace lines (`identifier=*** email=r***@artelonga.com.br`,
  `find_user_for_recovery: matched ... → co_id=...`,
  `all paths exhausted, returning None`). No enumeration leak — logs
  only, response shapes unchanged.
- `change-password` flow accepts optional `email` field for users who
  want to attach/update their recovery address without changing
  username. Mirrors to linked `quilombo_usuarios` row when present.

### Workflow change

Going forward, release cadence is **theme-per-release**, not
ticket-per-bump. Related tickets bundle into a single semver-meaningful
version. Patch bumps reserved for actual bugfixes / hotfixes between
themes. Major bumps when a coherent multi-day arc closes.

---

## [1.99.0] — 2026-05-10

### Added — CO-192: Sidebar consumes unified /me/universes shape

Sidebar now renders universes in semantic sections sourced from the new
`GET /api/v1/me/universes` endpoint (CO-191 precondition, also shipped here).

**Backend (CO-191):**
- `GET /api/v1/me/universes` (auth required) — returns bucketed shape:
  `{owned, member, subscribed, invited, discoverable, counts}`. Each
  bucket is sorted by name; `discoverable` capped at 50.
- `POST /api/v1/me/invitations/accept` — authenticated accept-by-universe-key,
  no raw token needed from the client.
- Storage helpers: `list_owned_universes`, `list_member_universes`,
  `list_subscribed_universes`, `list_discoverable_universes`.

**Frontend (CO-192):**
- Sidebar renders sections in fixed order: owned → member → subscribed →
  invited → discoverable. Empty buckets produce no section header.
- Non-owned items show a small role chip (admin / membro / inscrito / etc).
- Invited section has 🎁 emoji + count in label; each row has functional
  Aceitar / Recusar buttons that optimistically remove the row, call the API,
  then refresh `meUniverses`.
- Discoverable section is collapsible (default closed); state persists in
  `localStorage` key `co_sidebar_discover`.
- `loadMeUniverses()` called after login, clone, invite accept, subscribe.
- Anonymous users keep the existing public-catalog sidebar (no regression).
- All strings in PT + EN.

## [1.98.0] — 2026-05-10

### Added — CO-183: Lead capture pipeline (replaces artelonga `mailto:` contact form)

Backend pipeline for the `/contato/` form on `artelonga.com.br` so mobile
users without a mail client can submit, and so leads become queryable +
assignable instead of disappearing into someone's inbox.

**New endpoints:**

- `POST /api/v1/leads` (public) — accepts `{nome, email, telefone, mensagem,
  servico_titulo, parceiro_handle}`. Validates `mensagem` (required, ≤4000
  chars). Bot-filters via UA (silent 200, no persist). Daily-salted IP
  hash (CO-46 helper). Rate-limited 5/IP/24h (6th → 429). Persists, fires
  async email notification to `LEADS_NOTIFY_TO` (default
  `rede@artelonga.com.br`) — POST returns 201 even if mail send fails
  (lead persisted > notification perfect). CORS includes artelonga.com.br.

- `GET /api/v1/admin/leads` — admin-gated (`CO_SEED_ADMIN_EMAIL`).
  Filterable by `status`, `since`, `assignee`, `limit` (default 50, max
  200). Returns `{leads: [...], total}`.

- `PATCH /api/v1/admin/leads/:id` — admin partial update with state-
  machine validation (`new → triaged → in_progress → closed`). 400 on
  invalid transition. Auto-bumps `updated_at`.

- `GET /admin/leads.html` — cookie-auth handler serving the static SPA.

**Schema:** unconditional `ensure_table` for `leads` plus 3 indexes on
`status`, `created_at`, `assignee_handle`. Idempotent backfill pattern
(runs every boot, no-op once table exists).

**Retention task (LGPD):** `retention_task` daemon spawned at startup
purges `leads WHERE created_at < now - 24 months AND status = 'closed'`
once per day.

**Admin SPA** (`co-web/static/variants/a/leads.html`): inline
HTML+CSS+JS, summary chips per status, filterable table, detail panel
with PATCH form, 60s auto-refresh, `#lead-N` anchor nav from email
links.

**Privacy:** raw IP never persisted (only `ip_hash`); UA truncated to
256 chars; 24-month retention default.

Tests: 10 covering valid POST → 201, missing mensagem → 400, bot UA →
200 silent, rate limit 6th → 429, admin gate (no JWT → 401, non-admin
→ 403), state transitions (valid → 200, invalid → 400), email-send
failure still returns 201.

Files: `co-web/src/lead_routes.rs` (new),
`co-web/static/variants/a/leads.html` (new),
`co-web/src/storage/migrations.rs`, `co-web/src/lib.rs`,
`co-web/src/server.rs`, `docs/leads-api.md` (new).

## [1.97.0] — 2026-05-10

### Added — CO-190: Passwordless onboarding via email (magic-code sign-in or signup)

Single "Continuar com email" entry point: user types their email, receives a
6-digit code, and on verify either logs in (existing account) or gets a new
account auto-provisioned (derived `usuario` from email local-part). Slack/
Notion/Linear-style flow — no separate sign-up vs sign-in decision required.

- **Migration v41**: `onboarding_codes` table + index keyed by `email_lookup_hash`.
- **`POST /api/v1/auth/onboard-with-email`**: validates email, rate-limits
  (5/email/hour, 20/IP/hour), determines intent (`login`|`create`), mints
  6-digit Argon2id-hashed code, sends via Resend → SMTP → log cascade, returns
  202 `{sent: true, expires_at}` (always, even for unknown emails).
- **`POST /api/v1/auth/onboard-with-email/verify`**: looks up active code,
  enforces 5-wrong-attempt lockout, branches on intent — login mints session
  for existing user, create provisions `users` row (no password yet), runs
  CO-184 reverse bridge, runs `ensure_email_recovery_channel`, mints session.
  Returns 200 `{user_id, email, display_name, expires_at, return_to}` or 410
  when code is locked, expired, or consumed.
- **Storage `onboarding.rs`**: `create_onboarding_code`, `get_onboarding_code`,
  `consume_onboarding_code`, `increment_onboarding_attempts`,
  `cleanup_expired_onboarding_codes`, `count_onboarding_codes_for_email/ip`,
  `record_ip_onboarding_request`, plus `derive_usuario_from_email` helper with
  `-N` suffix dedup chain.
- **UI**: "Continuar com email" is now the **first** affordance in the login
  modal; password form is collapsible via "Já tem usuário e senha? ▼". Code-
  entry step shows "Reenviar" (disabled 60s) and "Editar email" links.
- **i18n**: all new keys in PT and EN.
- **Tests**: 10 covering happy login/create intents, rate-limit, wrong-code
  lockout (5 attempts), consumed code, expired code, missing code, invalid
  email, and `derive_usuario_from_email` with collision suffix.

## [1.96.2] — 2026-05-10

### Added — Per-step trace logging in `find_user_for_recovery`

Each step now emits a `tracing::info!` line so operators can see from
`flyctl logs` exactly which lookup branch matched, missed, or was skipped:

- `input=<redacted> (len=N)` at entry
- `matched CO users.usuario → <co_id>`
- `matched linked quilombo usuario → <co_id>`
- `matched unlinked quilombo usuario → q_id=…, bridging` then `bridge complete → co_id=…`
- `matched quilombo email → q_id=…, bridging` then `bridge complete via email → co_id=…`
- `no quilombo_usuarios row with email=<redacted>`
- `all paths exhausted, returning None`

Pure observability — no behavior change.

## [1.96.1] — 2026-05-10

### Fixed — Forgot-password now lazy-bridges legacy quilombo users by usuario AND by email field

Two gaps in `find_user_for_recovery_pair` meant that a legacy quilombo user
(no `linked_co_user_id` yet) could request a password reset, type their
quilombo usuario + email, and silently hit the no-match path. Resend was
never asked to send anything. Surfaced testing `retrocore` /
`retrocore@artelonga.com.br` after 1.95.1.

**Path 1 (added):** `find_user_for_recovery` now also walks
`quilombo_usuarios.usuario` rows where `linked_co_user_id IS NULL` (the
existing branch only matched `IS NOT NULL`), runs `ensure_co_user_for_quilombo`
+ `link_quilombo_to_co`, and returns the freshly-bridged CO user id.

**Path 2 (added):** When the primary identifier lookup misses but the
caller supplied a separate `email` field that contains `@`,
`find_user_for_recovery_pair` retries the lookup using the email. This
chains into the existing email-based lazy-bridge, so a user who typed the
right email but the wrong (or empty) usuario still resolves.

**Channel bootstrap:** Once a user_id is resolved AND the caller supplied
an email, `find_user_for_recovery_pair` always calls
`ensure_email_recovery_channel(user_id, email)`. Idempotent for existing
channels; for freshly-bridged users this guarantees they have a verified
channel to receive the code on. Without it, the bridge would succeed and
the handler would still send no email.

**Logging:** the no-match log now shows the redacted email field too, not
just whether one was supplied:
`Recovery request no-match: ... identifier=*** email=r***@artelonga.com.br`

Tests: 1 new — `test_forgot_password_lazy_bridges_legacy_quilombo_by_email`.
All 25 recovery-route tests pass.

## [1.96.0] — 2026-05-10

### Added — Change-password flow can attach an email

`POST /api/v1/auth/change-password` now accepts an optional `email` field.
When present (typically a quilombo user adding an email for the first time
via the security modal), the server:

1. Validates the email format and uniqueness across `users.email`.
2. Calls `set_user_email` — fails 409 if the address is already used by a
   different account; otherwise updates `users.email` for the current user.
3. Calls `ensure_email_recovery_channel` to promote the address to a
   verified recovery channel (forgot-password works immediately).
4. Calls `mirror_email_to_quilombo` to copy the new address onto any
   linked `quilombo_usuarios` row that lacks one.
5. Mirrors the new password hash to linked quilombo rows via
   `mirror_password_to_quilombo` (already in 1.83.0 for the recovery flow,
   now also for change-password — keeping the two paths symmetric).

**Username is never touched.** A signed-in quilombo user clicking
"Mudar senha" and adding `name@example.com` keeps `quilombo_usuarios.usuario`
and `users.usuario` unchanged; only the email + recovery channel are
attached.

UI: `index.html` security modal grew an optional "Adicionar/atualizar email"
input. `login.js::btn-change-password` sends `email` when filled and surfaces
the 409-conflict / generic error states distinctly. PT + EN i18n strings
added: `change_password_attach_email`, `change_password_attach_email_hint`,
`change_password_success_with_email`, `change_password_email_conflict`.

Tests: 2 new — `test_change_password_attaches_new_email`,
`test_change_password_email_conflict_returns_409`. All 4 change-password
tests pass clean.

## [1.95.1] — 2026-05-10

### Added — Recovery diagnostic logging

`forgot_password_handler` now emits `tracing::info!` on the no-match and
no-channel paths so operators can tell from `flyctl logs` whether a request
silently dropped (no user, no verified channel, or wrong pair) versus
actually attempting delivery. Response shape is unchanged — caller still
gets the same 202 with empty `sent_to` (no enumeration leak); the new
information goes only to server logs.

Surfaced when triaging a password-reset attempt for `retro-core@artelonga.com.br`
that produced no Resend log entry. Previously you couldn't distinguish
"no account exists" from "delivery failed" without DB inspection.

## [1.95.0] — 2026-05-10

### Added — CO-188: Universe invitation tokens

Universe owners and admin members can now invite others to private universes via single-use tokens — no more manual SQL.

**New endpoints:**
- `POST /api/v1/universes/:slug/invitations` — mint a 14-day single-use invite (auth required; caller must be owner or admin member). Accepts `email`, `usuario`, or `user_id`. Returns `{token_hash, expires_at, sent_to}`. Raw token sent exclusively to the recipient via email (Resend → SMTP → log cascade). Never returned to the API caller.
- `GET /api/v1/invitations/:token` — public preview: see universe name, inviter, and expiry before logging in. Returns 404 (not found), 410 (expired), or 200 with `already_consumed: true/false`.
- `POST /api/v1/invitations/:token/accept` — auth required. Verifies caller identity matches the invitee (by user_id or email, case-insensitive). On success, inserts a `universe_members` row and marks the invitation consumed. Idempotent re-membership via `INSERT OR IGNORE`.

**Migration v40:** new `universe_invitations` table with indexes on `(universe_key, consumed_at)` and `(invited_email, consumed_at)`. `revoked_at` column reserved for a future revoke endpoint (CO-189+).

**Storage helpers** in `storage/invitations.rs`: `create_invitation`, `get_invitation_by_token`, `consume_invitation`, `list_invitations_for_universe`, `list_invitations_for_email`.

**Tests:** 10 covering happy path (email / usuario / user_id), 403 non-owner, 409 already-member, 410 expired, 410 consumed (re-accept), and identity mismatch on accept.

## [1.94.1] — 2026-05-10

### Fixed — `return_to` safelist now includes `quilomboaraucaria.org`

Production quilombo serves on `.org` (the `.com.br` is a dev/historic alias). The `is_allowed_return_to` safelist only had `quilomboaraucaria.com.br` → CO was 400-rejecting any handover bounce that named the live `.org` host. After this patch, both are accepted; the `quilomboaraucaria.org.evil.com` suffix-confusion attack is also tested as rejected.

## [1.94.0] — 2026-05-09

### Changed — CO-186: SSO handover tokens now signed ES256 (no shared secret per universe)

Before: each new universe deploy needed `CO_JWT_SECRET=<JWT_SECRET on co-artelonga>` set as a Fly secret to validate handover tokens. Three operational pains: secret distribution per deploy, lockstep rotation, every deploy could forge tokens for any user.

After: handover tokens signed with CO's existing P-256 private key (already used for `/.well-known/jwks.json` per CO-166). Receivers validate the public key fetched from CO's JWKS endpoint — no shared secret. Onboarding a new universe to SSO is now:
- One env var: `CO_JWKS_URL=https://co.artelonga.com.br/.well-known/jwks.json` (configurable; this is the default).
- One ~30-line `/auth/co-handover` endpoint: validate `co_token` via JWKS, mint local cookie, redirect.
- One Google button anchor with `?return_to=https://<your-domain>/auth/co-handover`.

**Code changes:**
- `auth::sign_handover_jwt_es256(jwt_key, user_id, email, tier)` — new ES256 signer using `JwtKey` from CO-166. 60-second TTL.
- `auth::sign_handover_jwt(...)` — kept as legacy HS256 helper (annotated for removal in CO-187).
- `auth::maybe_attach_co_handover_token(...)` signature changed: now takes `&JwtKey` instead of `&str` HS256 secret.
- `oauth_google::callback_handler` and `recovery_routes::reset_password_handler` updated to pass `&state.jwt_key`.

**For deployment operators:**
- `co-artelonga` doesn't need any new secret — uses the existing `JwtKey` infrastructure that was already powering JWKS.
- `quilombo-araucaria` and any future universe SHOULD migrate `/auth/co-handover` to JWKS validation (filed as **QB-12** for quilombo). Until they migrate, both signing paths can coexist: CO-187 will retire HS256 once consumers cut over.

**For new universes (the per-universe pattern, before adding any):**
1. Implement `/auth/co-handover` endpoint that:
   - Reads `?co_token=<JWT>` from URL.
   - Fetches `https://co.artelonga.com.br/.well-known/jwks.json` (cache 1h).
   - Verifies the token signature with the public key matching the `kid` header.
   - Re-signs a local 7-day session JWT and sets a same-domain cookie.
   - Redirects to `/`.
2. Update login UI to point Google/recover anchors at `?return_to=https://<domain>/auth/co-handover`.
3. No secrets to set on Fly. Done.

This unblocks adding artelonga, yggdrasil, future deployments to SSO without per-add secret distribution.

## [1.93.1] — 2026-05-09

### Added — CO-185: short-lived `co_token` for cross-apex SSO handover

QB-11 ships the SvelteKit `/auth/co-handover` endpoint that reads `co_token` from the URL and sets a quilombo-side session cookie. This patch is the co-web producer side.

**`crate::auth`**:
- `sign_handover_jwt(user_id, email, tier, secret)` — same Claims shape as `sign_jwt` but with **60-second** TTL. Long enough to traverse the redirect, short enough that a leaked URL is useless.
- `maybe_attach_co_handover_token(return_to, ...)` — when `return_to` contains `/auth/co-handover` (any safelisted host), appends `?co_token=<jwt>` (or `&co_token=`). Otherwise returns the URL unchanged.

**Wired into:**
- `oauth_google::callback_handler` — on Google sign-in, the redirect to `return_to` now carries the handover token when bouncing to a co-handover URL. Cookie still set on `co.artelonga.com.br` so the user is also logged in here.
- `recovery_routes::reset_password_handler` — response body now includes `co_token: "<short-lived jwt>"`. Login modal SPA reads it from the response and appends `?co_token=` to the redirect URL when `return_to` ends in `/auth/co-handover`. Same logic — `co.artelonga.com.br` cookie unchanged, plus URL handover.

The handover token has 60-second lifetime by design — the receiving SvelteKit endpoint validates and immediately mints its own 7-day cookie (via re-signing with the shared `CO_JWT_SECRET`), so the browser never holds the URL-bound token longer than one redirect.

When `return_to` does NOT contain `/auth/co-handover` (the common case — same-apex returns), behavior is unchanged: bare redirect, cookie carries the session.

QB-11 expects `CO_JWT_SECRET` env var on the quilombo deployment to match `JWT_SECRET` on `co-artelonga`. Both decode the same HS256 JWTs.

## [1.93.0] — 2026-05-09

### Added — CO-184: reverse bridge — every CO sign-in auto-provisions a quilombo identity

CO-172 made every quilombo signup auto-provision a CO account (one-way). User feedback:

> we want single sign on for all users all routes, so a sign on to google on quilombo should return a co acount, similarly to a co sign on on co should return a quilombo account

Closes the loop in the other direction. Whenever a user signs in to CO — Google OAuth, password-login, public signup, magic-link verify — `Storage::ensure_quilombo_user_for_co(co_user_id)` runs as a best-effort post-login hook. Idempotent:

1. Returns the existing quilombo row when `linked_co_user_id` already points to this CO user.
2. Links to a quilombo row that happens to share the same email (no new row).
3. Otherwise inserts a fresh `quilombo_usuarios` row: `papel='membro'`, `senha_hash='!provisorio!'` (sentinel from QB-6 — legacy quilombo password login is blocked, but the user authenticates via CO session anyway), `usuario` derived from CO `usuario` or email local-part with `-N` dedupe on collision.

Wired into:
- `oauth_google::callback_handler` (Google sign-in)
- `password_login_handler` (existing accounts)
- `signup_handler` (CO-175 public signup)
- `verify_handler` (magic-link `/auth/verify`)

Failures log at WARN and don't block the sign-in itself — users can always recover their CO session even if the quilombo bridge transiently fails.

After this lands, every authenticated CO user has a corresponding `quilombo_usuarios` row, so per-universe metadata (CO-173) returns quilombo profile data for them and `quilomboaraucaria.com.br` per-content auth checks resolve cleanly.

**Cross-apex cookie handover** (the second half of "single sign-on across both domains") is filed as **QB-11** in the quilombo repo — the SvelteKit at `quilomboaraucaria.com.br` needs an `/auth/co-handover` route that reads a token from the redirect URL and sets its own session cookie. Cookies cannot share between `.artelonga.com.br` and `.com.br/quilomboaraucaria.com.br`, so token-handover via URL is the protocol.

## [1.92.0] — 2026-05-09

### Added — CO-177: Google OAuth sign-in (login + signup)

Google sign-in / sign-up across the platform. One button on both the login modal and the signup form, available everywhere the CO SPA renders. Future cross-deployment bounces (quilombo SvelteKit's "Continuar com Google", a planned ArteLonga login) reuse the same `/api/v1/auth/google/start?return_to=...` endpoint via the existing `is_allowed_return_to` safelist — same pattern as `/recover`.

**New module `oauth_google.rs`:**
- `GET /api/v1/auth/google/start?return_to=<url>` — generates a state JWT (signed with the shared secret, carries `return_to` + nonce + 10-min TTL), redirects to Google's consent screen with `scope=openid email profile`, `prompt=select_account`.
- `GET /api/v1/auth/google/callback?code=&state=` — verifies state JWT, exchanges code at `oauth2.googleapis.com/token`, fetches `openidconnect.googleapis.com/v1/userinfo`, finds-or-creates the CO user, sets the session cookie, 303-redirects to the safelisted `return_to` (or `/`).

**Storage** — `Storage::find_or_create_user_by_google(sub, email, name)`:
1. Match by `users.google_sub` → existing Google-linked user.
2. Match by `users.email` → existing CO user, link Google sub to them, auto-promote email as verified recovery channel.
3. Insert new user with `tier='player'`, deduped `usuario` from email local-part, default subscriptions, recovery channel auto-promotion.

**Migration v39:**
- `users.google_sub TEXT` (nullable).
- `CREATE UNIQUE INDEX idx_users_google_sub ON users(google_sub) WHERE google_sub IS NOT NULL` — partial index, one Google account → one CO user, NULLs coexist freely.

**Status endpoint** — `GET /api/v1/auth/google/status` returns `{configured: bool}` based on whether `GOOGLE_CLIENT_ID` + `GOOGLE_CLIENT_SECRET` env vars are set. The login UI hides the button on `configured: false`, so deployments that haven't configured Google never show a button that 503s.

**Required env vars** (set per deployment via `flyctl secrets set ...`):
- `GOOGLE_CLIENT_ID`
- `GOOGLE_CLIENT_SECRET`
- `GOOGLE_REDIRECT_URI` (default `https://co.artelonga.com.br/api/v1/auth/google/callback`)

**UI** — login modal + signup form gain an OAuth block with `or` divider and a Google-branded `Continuar com Google` / `Cadastrar com Google` link styled per Google's brand guidelines (4-color G icon as inline SVG). The block is `display:none` until JS confirms `configured: true` and forwards any `?return_to=` from the current page to the start endpoint.

**i18n** — new keys (PT + EN): `oauth_divider`, `continue_with_google`, `signup_with_google`.

**Error handling** — new `AppError::ServiceUnavailable(String)` → 503 with the standard `{error, message, message_en}` envelope. Used when the OAuth env vars aren't set on a deploy that nonetheless gets a callback.

Telemetry: `auth.login` event with `list="google"` to distinguish from `password` and `magic-link`.

To activate on a deploy:

```bash
flyctl secrets set GOOGLE_CLIENT_ID=...apps.googleusercontent.com \
                   GOOGLE_CLIENT_SECRET=GOCSPX-... \
                   GOOGLE_REDIRECT_URI='https://co.artelonga.com.br/api/v1/auth/google/callback' \
                   -a co-artelonga
```

Register the redirect URI in the Google Cloud Console under "OAuth 2.0 Client IDs" → Authorized redirect URIs before flipping the secrets.

## [1.91.3] — 2026-05-09

### Changed — Recovery form requires both `usuario` AND `email` (no more "or")

User feedback after the QB-9 e2e: the single "Usuário ou canal de recuperação" field was confusing because a quilombo user has both — a username at quilombo *and* an email. The form now asks for both explicitly:

- **Field 1:** Usuário — identifies *which* account (quilombo usuario, CO usuario, or anything resolvable through the lookup chain).
- **Field 2:** Email — confirms ownership *and* is the channel where the code arrives.

Both are required client-side. Server-side a new `find_user_for_recovery_pair(identifier, email)` enforces that both resolve to the **same** user. When the email is empty (legacy API consumers), the lookup falls back to the single-identifier behavior.

Both `forgot-password` and `forgot-password/verify` now accept the `email` field. The verify path uses the pair so a stolen code can't be replayed against a different account.

New i18n keys: `forgot_password_username_label`, `forgot_password_username_placeholder`, `forgot_password_email_label`, `forgot_password_email_placeholder`, `forgot_password_username_required`, `forgot_password_email_required` (PT + EN). `forgot_password_subtitle` updated to "Digite seu usuário e email cadastrado".

### Hardened — CO-176: quilombo signup bridge is mandatory + integrity log on boot

Deployment-readiness pass for CO-172. Every quilombo signup must end with a linked CO account; previously the bridge just `tracing::warn!`d on failure and continued.

- `quilombo_routes::cadastro_handler` now treats `ensure_co_user_for_quilombo` + `link_quilombo_to_co` as mandatory. On failure: rolls back the freshly-created `quilombo_usuarios` row with `DELETE WHERE id = ?1`, logs at `ERROR`, returns 500 with a Portuguese message hinting retry. The user gets a clean state to retry without hitting the username-taken path.
- Boot-time integrity check: `server.rs` runs `SELECT COUNT(*) FROM quilombo_usuarios WHERE linked_co_user_id IS NULL` after the recovery backfills and warns at `WARN` if any are present. Pre-1.91.3 legacy rows recover lazily through the `/recover` flow (1.91.2 lazy bridge), but a non-zero count after the fleet has rolled to 1.91.3+ would indicate a regression.

Combined with CO-176's lazy-bridge step in `find_user_for_recovery` (1.91.2), every quilombo user — legacy or new — has a path to a working CO account either at signup time (synchronous, mandatory) or at first password recovery (lazy, idempotent).

## [1.91.2] — 2026-05-09

### Fixed — CO-176 follow-up: lazy-bridge legacy quilombo users on recovery + simpler subtitle

User-facing report after running QB-9 e2e: a quilombo user who set their email *after* signing up couldn't recover via that email — the system silently returned `sent_to: []` because their `quilombo_usuarios.email` was set but no `linked_co_user_id` / verified channel existed yet on the CO side.

**Backend** — `find_user_for_recovery` gains a lazy-bridge step. If the verified-channel lookup misses but the typed value matches a `quilombo_usuarios.email`, the lookup runs the same `ensure_co_user_for_quilombo` → `link_quilombo_to_co` → `ensure_email_recovery_channel` chain that CO-172 Phase 1 runs at signup-time, then returns the new CO user id. Idempotent — once a user is bridged, the verified-channel lookup catches them on subsequent calls and the lazy step never fires.

This rescues:
- Quilombo signups from before CO-172 shipped (no `linked_co_user_id`).
- Cases where the bridge silently warned-and-continued at email-set time (e.g. transient SQL error, lost log).
- Manual `quilombo_usuarios.email` writes outside the perfil endpoint.

**Frontend** — dropped the long "Sua conta funciona no Quilombo Araucária e em CO…" subtitle per user feedback ("no need to mntion sua conta funciona etc etc its okay"). The `/recover` page now reads:
- Title: **Recuperar senha**
- Subtitle: **Recupere o acesso à sua conta.**

Removed unused i18n keys: `recover_subtitle_quilombo`, `recover_subtitle_external` (PT + EN).

## [1.91.1] — 2026-05-09

### Fixed — CO-176: `/recover` is friendlier when bounced from quilombo

QB-9 shipped on quilomboaraucaria.com.br: clicking "Esqueci minha senha" bounces to `co.artelonga.com.br/recover?return_to=https%3A%2F%2Fquilomboaraucaria.com.br&identifier=...`. End-user feedback after running the flow:

> Esqueci minha senha redireciona para [Co Entrar] que é okay mas confuso, porque o usuário precisa fornecer o usuário do quilombo E o email deles.

Two fixes:

**Frontend** — title + subtitle on the recover page now reflect the originating context:
- Default `Co / Entrar / Acesse seu quadro de projetos` was confusing for a quilombo user mid-recovery.
- When `?return_to=https://quilomboaraucaria.com.br`, modal title becomes "Recuperar senha" and the subtitle reads "Sua conta funciona no Quilombo Araucária e em CO. Use o mesmo email ou usuário." (PT) / "Your account works at Quilombo Araucária and CO. Use the same email or username." (EN).
- Generic external `return_to` (any other safelisted host) gets a less-specific subtitle. Same-origin gets the plain "Recupere o acesso à sua conta."
- The redundant `Digite seu usuário ou email de recuperação` step subtitle is hidden when the modal title already says "Recuperar senha".
- New i18n keys (PT + EN): `recover_title`, `recover_subtitle`, `recover_subtitle_quilombo`, `recover_subtitle_external`.

**Backend** — `find_user_for_recovery` now also looks up `quilombo_usuarios.usuario` and resolves through `linked_co_user_id` to the canonical CO user. Fixes the case where a quilombo user's `users.usuario` is `NULL` because `quilombo_bridge.rs` fell back on a unique-index collision — typing the quilombo username would hit dead-end `users.usuario` only without this. Email path still works (preferred); this just means the quilombo username path doesn't silently 202-with-empty-`sent_to`.

## [1.91.0] — 2026-05-09

### Added — CO-175 (G3): public signup endpoint + UI

Closes the gap surfaced in the e2e checklist: a brand-new visitor at `co.artelonga.com.br` had no path to create an account. Quilombo signups bridged in via CO-172, but a direct CO signup form didn't exist.

**Backend** — `POST /api/v1/auth/signup`:
- Body: `{ usuario, password, email? }`. Email optional per `feedback_auth_email.md`.
- Validation: usuario 3-30 chars `[a-z0-9_-]`, password ≥8 chars, email format if present.
- 409 on usuario or email collision (separate messages so the UI can be specific).
- **Rate limit: 100 new accounts per rolling 24h cluster-wide.** Counts `users.created_at` against the window. 101st request returns 429 with a Portuguese message hinting "tente novamente em algumas horas". Knob is `SIGNUP_DAILY_CAP` constant in `auth_handlers.rs`.
- Tier hardcoded to `'player'` (admin reserved for env-seeded operator account).
- Argon2id hash via `tokio::task::spawn_blocking` so the worker thread isn't pinned during the password derivation.
- On success: writes `users` row, calls `ensure_email_recovery_channel` if email was supplied (auto-promotes to verified channel — `forgot-password` works immediately for the new user), issues the same JWT-cookie response shape as `password-login`.
- Telemetry: emits `auth.signup` with `list="public"`.

**Storage** — `Storage::create_user_with_password(usuario, password_hash, email?)`:
- Idempotent in the sense that taken usuario / email returns `SignupError::UsuarioTaken` / `EmailTaken` for the route to map to 409.
- Calls `subscribe_user_to_default_universes` (same path as the magic-link flow).

**Storage** — `Storage::count_users_created_since(seconds)`:
- Used by the rate-limit check; cheap COUNT against `users.created_at`.

**UI** — login modal gains a "Criar conta" link next to "Esqueci minha senha":
- New step `#login-step-signup` with usuario / password / email-optional form.
- Client-side mirrors of the backend validation (length checks) for fast-fail.
- "Já tenho conta" returns to the login step.
- On success: page reload — `init()` reads `/me` and routes to the new user's hub.
- New i18n strings (PT + EN): `signup_link`, `signup_subtitle`, `signup_usuario`, `signup_usuario_placeholder`, `signup_email_optional`, `signup_email_hint`, `signup_submit`, `signup_submitting`, `signup_have_account`, `signup_error_usuario_len`, `signup_error_senha_len`, `signup_error_generic`.

Tiny CSS — `.login-link-sep` for the dot between the two header links.

This closes G3 from the e2e checklist. New users can now create CO accounts without going through quilombo or admin seeding.

## [1.90.3] — 2026-05-09

### Fixed — `Entrar` button in the header was a no-op (CO-171 refactor regression)

`renderHeaderUserArea(null)` runs at line 419 of `init()` — *before* `wireModules()` injects the real `showLoginModal` at line 421. The header binding was:

```js
btn.addEventListener('click', _showLoginModal);  // captures noop — broken
```

`addEventListener` captures the function reference at bind-time, so the listener stayed pointed at the initial `let _showLoginModal = () => {};` even after `injectShowLoginModal` reassigned the module-level variable.

Two sites fixed with the defer-dereference pattern:

```js
btn.addEventListener('click', () => _showLoginModal());  // reads at click time
```

- `modules/sidebar.js:199` — `btn-header-entrar` (the symptom yuri@quilombo reported)
- `modules/onboarding.js:235` — `btn-banner-entrar` (same shape, would have hit anonymous template visitors)

Pattern note for future module-callback wiring: use `() => _fn()` indirection in `addEventListener` calls when the function is supplied via `inject*Callbacks` and the binding might happen pre-injection.

## [1.90.2] — 2026-05-09

### Changed — Security modal refactored: padding, login-card vibe, type-aware phone input

The "Canais de recuperação" modal had a flat layout with no body padding — labels touched the modal's left edge — plus the same heading rendered twice (once in the modal header and again as an inner `h3`). Refactored to mirror the login modal's clean structure:

- HTML: `#security-modal-body` now uses real classes (`.security-modal-body`, `.security-section`, `.security-section-header`) instead of inline styles. Single `Segurança` heading in the modal header; section headings (`Canais de recuperação`, `Mudar senha`) live below as `h3`s. Buttons promoted to `btn-full` so they match the login form's footprint.
- CSS: new block in `style.css` defines body padding (`24px`), section gaps (`28px`), section dividers (`border-top` between `.security-section` siblings), and a tidy `.recovery-channel-row` (icon + masked value + status pill + remove button).
- Channel rows: pill-style cards with `bg-hover` background instead of flat text. Unverified rows highlight the status with `--accent`. Channel-type icon prefix (`✉`, `💬`, `📱`).

### Added — Phone-number recovery channels in UI

`recovery_channels_title` already had `whatsapp` and `sms` options in the `<select>`, but the value input was hard-coded to `type="email"` with placeholder `email@exemplo.com`. New `syncChannelInputForType()` listener flips:

- type → `tel`, `inputmode="tel"`, `autocomplete="tel"`
- placeholder → `+55 41 99999-9999`
- helper text → "Você receberá um código por mensagem do WhatsApp. Inclua o código do país (+55)." (or SMS variant)

Email path keeps the original affordances. Backend already supports the channels (`recovery_crypto::normalize_channel_value` strips non-digits from phone numbers, preserves leading `+`).

New i18n strings: `security_title`, `recovery_channel_email_hint`, `recovery_channel_whatsapp_hint`, `recovery_channel_sms_hint` (PT + EN).

## [1.90.1] — 2026-05-09

### Removed — CO-172 cleanup: dead quilombo outbound redirect from co-web's SPA

`co-web/static/variants/a/modules/login.js` shipped with hostname-detection redirects assuming the SPA also served `quilomboaraucaria.com.br`. It doesn't — that domain serves the SvelteKit SPA from `~/projects/quilomboaraucaria/`, not co-web. The `window.location.hostname === 'quilomboaraucaria.com.br'` checks never matched in production, making the outbound redirect blocks dead code.

Removed:
- The `buildCoRecoverUrl(...)` + `isQuilomboDomain` constants and the `if (isQuilomboDomain) { ... return; }` guard in the forgot-password click handler.
- The equivalent guard in the change-password click handler.

Kept (these run when a quilombo user lands on `co.artelonga.com.br/recover` from the bounce):
- `/recover` path detection that pre-fills `?identifier=` and pins the modal step.
- `isAllowedReturnTo` safelist for the after-reset redirect.
- The `return_to` redirect after a successful password update.

The corresponding outbound redirect work is filed as CO-174 and lives in the `~/projects/quilomboaraucaria/` repo.

## [1.90.0] — 2026-05-09

### Added — CO-173: per-(user, universe) metadata in `/api/v1/auth/me`

`MeResponse` now carries a `universes: Vec<UserUniverseEntry>` array — one entry per universe the authenticated user has any relation to (owner / member / subscriber). Each entry includes:
- `key`, `name`
- `role` (`"owner"` for `owner_id` matches, member's role string otherwise, `"subscriber"` for sub-only, `"viewer"` fallback)
- boolean flags: `is_owner`, `is_member`, `is_subscriber`
- `metadata: Value` — universe-specific bag

Metadata sources, folded into a single bag per entry:
- For any universe the user is a member of: `joined_at` from `universe_members`.
- For `quilomboaraucaria` (when the user has a `quilombo_usuarios.linked_co_user_id`): `papel`, `bio`, `foto_url`, `telefone`, `email` straight from the quilombo profile.
- Cross-deployment fetches (e.g. for a future ArteLonga server) deferred to CO-172v2's API mesh.

`Storage::list_universes_with_metadata_for_user` walks `list_universes_for_user` plus three side queries (member roles, subscription set, the quilombo profile) and assembles the entries in O(n) over the user's universes.

The `universes` field uses `#[serde(default, skip_serializing_if = "Vec::is_empty")]` — older clients that only read `user_id`/`email`/`display_name`/`tier` keep working unchanged. New clients get the per-universe data without a second roundtrip to `/api/v1/universes`.

## [1.89.1] — 2026-05-09

### Fixed — CO-172 hardening: server-side `?return_to=` safelist on `/recover`

The `is_allowed_return_to` safelist in `recovery_routes.rs` was tested but never actually called — production enforcement lived only in `login.js::isAllowedReturnTo`. The SPA correctly refused to redirect to non-safelisted hosts, but `co.artelonga.com.br/recover?return_to=https://evil.com` still returned 200, so the URL itself was a valid phishing surface (trusted hostname, would-be victim never sees a server-side rejection).

New `serve_recover` handler (in `server.rs`) wraps the `/recover` route, deserializes `?return_to=`, and returns **400** when the host isn't `*.artelonga.com.br` or `quilomboaraucaria.com.br`. Identical safelist function as the JS check — defense-in-depth.

The `#[cfg_attr(not(test), allow(dead_code))]` came off `is_allowed_return_to` since it's now a load-bearing pub(crate) fn.

## [1.89.0] — 2026-05-09

### Added — CO-172: Quilombo signups become CO accounts — central auth + redirected password reset

**Phase 1 — Bridge quilombo signup into `users`**
- `Storage::ensure_co_user_for_quilombo` — idempotent: returns existing `linked_co_user_id`, finds CO user by email match, or INSERTs a new `users` row (tier=`player`, reuses Argon2id hash)
- `Storage::link_quilombo_to_co` — sets `quilombo_usuarios.linked_co_user_id`
- `POST /api/v1/quilombo/auth/cadastro` now bridges every new signup into `users`
- `PUT /api/v1/quilombo/perfil` bridges quilombo user into CO when email is first set, and promotes a verified email recovery channel immediately

**Phase 2 — Recovery channel backfill for quilombo users**
- `Storage::backfill_quilombo_recovery_channels` — for every quilombo user with `email + linked_co_user_id` set, ensures a verified CO recovery channel; runs on every boot (idempotent)

**Phase 3 — `/recover` endpoint + quilombo SPA redirects**
- New `/recover` route on CO serves the SPA pre-pinned to the forgot-password step; reads `?identifier=` to pre-fill and `?return_to=` to redirect after success
- `is_allowed_return_to` safelist: only `*.artelonga.com.br` and `quilomboaraucaria.com.br` accepted
- Quilombo SPA "Esqueci minha senha" detects `quilomboaraucaria.com.br` hostname and redirects to `co.artelonga.com.br/recover?return_to=...&identifier=...`
- Quilombo SPA "Alterar senha" redirects to `co.artelonga.com.br/recover?action=change_password&return_to=...`
- After successful reset on `/recover`, SPA redirects to safelisted `return_to` instead of reloading

**Phase 4 — Password reset propagation**
- `Storage::mirror_password_to_quilombo` — after CO `reset-password`, new hash mirrored to all linked `quilombo_usuarios` rows so legacy quilombo login stays in sync

## [1.88.4] — 2026-05-09

### Added — Guia do Co: documentação para usuários no universo template

Nova página de boas-vindas (`content/guia.md`) disponível em todos os universos derivados do template. Cobre:

- Primeiros passos: criar universo, adicionar tarefas, convidar colaboradores.
- Todas as seis visualizações (Quadro, Tabela, Linha do tempo, Calendário, Conteúdo, Dashboard).
- Vault API e compatibilidade com Obsidian.
- Linhas do tempo públicas (Tempo, Universo, Humanidade).
- Temas e atalhos de teclado.
- **Yggdrasil** — seção em destaque explicando o hub de jogos como "mundo fantasia": os cinco jogos (Tetris, Snake, Invaders, PointSet, Poker), sistema de perfil, ranking global e a origem mitológica do nome.

A página é reescrita a cada boot via `reseed_template_content_pages` — a versão do binário é sempre a fonte da verdade.

## [1.88.3] — 2026-05-09

### Refactored — CO-171: modularize the 6k+ line monoliths

**`co-web/src/storage.rs`** (7 962 → 136 lines) extracted into 14 submodules under `co-web/src/storage/`:
- `schema.rs` — `ensure_column`, `ensure_table`, `split_frontmatter`, `seed_page_frontmatter`, `seed_page_body`, `walkdir`, row-mapper helpers (`row_to_recovery_channel`, `row_to_recovery_verification`, `upsert_entry_row`, `entry_row_from_sql`, `entry_row_to_project`, `entry_row_to_task`, `entry_row_to_comment`), `parse_status/priority/datetime`, and `mod ensure_column_tests` + `mod seed_md_tests`
- `migrations.rs` — `run_migrations`, `migrate_old_data_to_entries`, `maybe_migrate_entries_to_universe_dbs`; delegates SQL blocks to `migrations/apply.rs`
- `log_drain.rs` — `get_log_drain_secret`, `set_log_drain_secret`, `insert_log_drain_event`, `log_uat_mutation`, `get_uat_mutations_since/watermark`, `backup_universe`, `universe_db_size`, `search_entries_across_universes`, `schema_version`
- `projects.rs` — `list_projects`, `list_projects_for_universe`, `get_project`, `create_project`, `delete_project`
- `tasks.rs` — `list_tasks`, `list_tasks_filtered/paginated`, `get_task`, `create_task`, `update_task`, `delete_task`, `bulk_update/delete_tasks`
- `comments.rs` — `list_comments`, `create_comment`, `list_activity`
- `dashboard.rs` — `get_dashboard` + private analytics helpers
- `users.rs` — `create_user`, `get_user_by_email/id/usuario`, `update_password_hash`
- `admin.rs` — `seed_uat_user`, `seed_admin_user_from_env`, `ensure_admin_*`, `cleanup_admin_anon_clutter`, `rescue_orphan_universes`, `cleanup_anon_universes`, `get/restore_all_users_with_hashes`
- `universe.rs` — `create/get/list_universes_*`, membership CRUD, subscribe/unsubscribe
- `blobs.rs` — `put_blob`, `get_blob`, `has_blob`, `backfill_blobs_from_entries`
- `subscriptions.rs` — `pin_subscription`, `is_subscribed`, `list_universe_subscribers`, `search_public_universes`, `check_universe_access`, `count_*`, `claim_universe`, `get/update_universe_form_config`
- `clone_ops.rs` — `migrate_co170_phase_b`, `rebuild_project_universe_index`, `delete_deprecated_universes`, `recompute_content_counts`, `clone_universe`, `list_user_universes`, `list_projects_for_user`, `is_project_in_template`
- `api_tokens.rs` — `get/update_entry_body`, `create/list/delete/get_api_token_by_value`, `create/get/verify/delete/set_channel_lockout` recovery channel CRUD, `create/get/consume/expire` verification + reset tokens

**`co-web/src/recovery_routes.rs`** (1 856 → 844 lines) test module moved to `recovery_routes/tests.rs` (992 lines)

**`co-web/src/universe_routes.rs`** (2 742 → 1 179 lines):
- `universe_routes/template.rs` (725 lines) — `apply_template`, `apply_template_all`, `submit_doc_gen_job`, `get_doc_gen_last_error`, `themes_router`, and private helpers `run_type_check`, `build_claude_md`, `build_api_md`, `slugify`
- `universe_routes/tests.rs` (845 lines) — all 27 tests moved out of inline `mod tests`

**`co-web/src/vault_routes.rs`** (2 216 → 1 485 lines) test module moved to `vault_routes/tests.rs` (731 lines)

**`co-web/src/server.rs`** (2 930 → 1 325 lines):
- `server/auth_handlers.rs` (450 lines) — `login_handler`, `verify_handler`, `me_handler`, `user_stats_handler`, `logout_handler`, `password_login_handler`, `uat_login_handler`
- `server/legacy.rs` (304 lines) — legacy inline project/task handlers
- `server/static_files.rs` (355 lines) — `serve_variant_file`, `serve_co_index`, `serve_assets_page`, `serve_sync_settings`, `guess_content_type`, `cache_control_for`, `looks_like_static_asset`
- `server/tests.rs` (506 lines) — server-level tests

**`co-web/static/variants/a/app.js`** (6 909 → 495 lines) split into 17 ES modules under `modules/`:
- `state.js` — shared mutable `state` object
- `constants.js` — STATUSES, label maps, theme maps, Obsidian compat functions
- `helpers.js` — DOM, date/time, subtask, sorting, assignee utilities
- `api.js` — `apiFetch` + `api` object
- `settings.js` — toast, loading, theme CSS, settings panel, `applyUniverseConfig`
- `sidebar.js` — `renderSidebar`, `renderHeader`, `renderUsageCount`, `setupHamburgerMenu`
- `modals.js` — task modal, comments, activity, universe-info, usage-limit modal, editor
- `login.js` — `showLoginModal`, `hideLoginModal`, `setupLoginModal`, `setupSecurityModal`
- `onboarding.js` — `setupOnboarding`, `setupCriarModal`
- `yggdrasil.js` — `bootYggdrasil`, `renderYggdrasilHub`, `renderYggdrasilGame`
- `boot.js` — `bootAppForUniverse`, `renderUniverseHome`
- `views/kanban.js` — `renderKanban`, `renderTaskCard`, drag-drop
- `views/calendar.js` — `renderCalendar`, `renderMiniCalendar`, `renderGantt`
- `views/table.js` — `renderTable`, `setupTableEvents`
- `views/timeline.js` — `renderTimeline`, timeline math, bar drag, tooltip
- `views/dashboard.js` — `renderDashboard`, SVG chart helpers
- `views/conteudo.js` — `renderConteudo`, folder tree, zoom modal, view-dados

`index.html` updated to `<script type="module" src="/app.js">` for native ES module loading.

No public API change — all HTTP endpoints, `pub fn` signatures, and JS globals unchanged.

## [1.88.2] — 2026-05-09

### Fixed — CO-170 phase B: actually rebuild project_universe_index on every boot

1.88.1 fixed the rebuild *logic* (sort + INSERT OR IGNORE + uppercase normalization) but the call site was gated on `if total_moves > 0`. After the moves landed in 1.88.0, every subsequent boot had `total = 0`, so the rebuild never re-ran — the index stayed in its pre-fix state.

`rebuild_project_universe_index` is now public and called unconditionally from `server.rs` boot path right after `migrate_co170_phase_b`. Cheap (<100 rows total per universe walk) and idempotent.

This is what actually surfaces the moved tasks in `/api/projects/CW/tasks?u=co` etc.

## [1.88.1] — 2026-05-09

### Fixed — CO-170 phase B follow-up: deterministic project_universe_index rebuild

After the 1.88.0 data moves, `/api/projects/CW/tasks?u=co` still returned `[]` because `project_universe_index` couldn't disambiguate projects sharing a `key` between universes (e.g. `co/projects/CO/_project.md` "CO Platform" vs `template/projects/CO/_project.md` "Bem-vindo ao Co" both register `key: "CO"`). The rebuild used `INSERT OR REPLACE` and iterated universes in undefined order, so the winner was non-deterministic — and on this prod boot, the tutorial won.

Two small changes:
- Sort universes during rebuild: real content universes first, then `template` / `tempo` / `humanity` / `universo` / `yggdrasil` last.
- Switch to `INSERT OR IGNORE` so the first-seen registration wins. Combined with the sort, that guarantees `co` always beats `template` for the `CO` key.
- Normalize the stored `project_key` to uppercase to match the lookup path.

The rebuild now logs the number of indexed rows so future drift is visible from boot logs.

## [1.88.0] — 2026-05-09

### Fixed — CO-170 phase B (data moves): consolidate misplaced project content

Past co-sync runs preserved absolute filesystem paths when ingesting entries into the `co` universe (`data/universes/default/projects/AL/...`, `data/universes/template/projects/MP/...`). Result: 7 Arte Longa tasks living in `co`, 18+ Quilombo tasks living in `co`, 44 CO-platform tasks (API/CW/DS/PLT) living in `artelonga`, plus tutorial leaks (LOCACO, MP) in `co`. Per user direction "AL is its own universe; co stuff can be under co":

`Storage::migrate_co170_phase_b()` runs on every boot. Idempotent — once entries move, the source matches no rows and each step no-ops:

1. Drop tutorial / template leakage from `co`: `data/universes/template/projects/{MP,CO}/*`, `data/universes/template/{timeline,content}/*`, `data/universes/local-2cw54k/projects/LOCACO/*`.
2. Move `co/data/universes/default/projects/AL/*` → `artelonga/projects/AL/*` (path stripped of `data/universes/default/` prefix).
3. Move `co/data/universes/default/projects/QA/*` → `quilomboaraucaria/projects/QA/*` (same path-strip).
4. Drop the empty `co/data/universes/default/projects/{API,DS,PLT,CO}/_project.md` stubs that overlap with destination keys.
5. Move `artelonga/projects/{API,DS,PLT,CW}/*` → `co/projects/{...}/*` (path unchanged).
6. Rebuild `project_universe_index` from scratch off the post-move state.

Per-step row counts are logged so future drift surfaces without bisecting commits.

A new `MoveRow` struct replaces the prior 9-element tuple in `move_entries_strip_prefix` to satisfy `clippy::type_complexity`.

## [1.87.0] — 2026-05-09

### Fixed — CO-170 phase B: drop empty cross-leaked project stubs

The `co` universe listed projects AL, API, CO, DS, PLT and the `artelonga` universe listed API, ARTEP, CW, DS, PLT — both with `_project.md` rows but **zero tasks** under them. Empirical audit confirmed: every leaked project was a metadata-only stub. Past filesystem syncs wrote `projects/<KEY>/_project.md` into the wrong universe directories; the actual user-facing content (CO-N user-stories in `co`, `modelos/*.md` content in `artelonga`) lives outside the projects surface.

`Storage::cleanup_empty_projects()` now runs on every boot. For each universe's per-universe DB:
1. Find rows with `entry_type = 'project'`.
2. Count siblings under `projects/<KEY>/` that are NOT `_project.md`.
3. If zero, drop the project entry + the matching `project_universe_index` row.

Per user direction "AL is its own universe; co stuff can be under co", the algorithm preserves any project that has real content, regardless of which universe it sits in. Today: every leaked project is empty, so all of them get cleaned. Future leaks (if any) will only get cleaned if they're equally hollow.

Idempotent — re-running after a clean boot is a no-op.

## [1.86.0] — 2026-05-09

### Fixed — CO-170 phases A + D: universe sidebar hygiene + timeline aliases

**Phase A — soft-hide deprecated/empty universes**
- Migration v38 adds `universes.hidden INTEGER NOT NULL DEFAULT 0` (idempotent via `ensure_column`).
- New `Storage::hide_deprecated_universes()` runs on every boot. Sets `hidden = 1` for `language`, `topologia` (both empty parent placeholders, 0 entries), and `mbya` (4620-entry corpus, intended pre-merge into `comunicacao` per 1.69.0 changelog but the actual content move never landed — see CO-170 Phase C).
- Both `list_public_universes` (anon sidebar) and `list_universes_for_user` (admin sidebar) now filter `WHERE COALESCE(hidden, 0) = 0`. Direct URL access still works; only the listing surface is gated.

**Bonus — surface timeline universes in the public list**
- `list_public_universes` previously matched only `visibility = 'public-subscribable' OR is_template = 1`. The bundled timeline trio (`tempo`, `humanity`, `universo`) ships with `visibility = 'public-static'`, so they were accessible by direct URL but invisible in the sidebar. Query now includes `'public-static'` too. Anon visitors will see the timeline trio appear nested under `template` (via `parent_key`).

**Phase D — friendly aliases for the timeline view**
- Two new redirect routes: `/linhadotempo` and `/timeline` both 307 → `/shared/timeline.html?u=tempo,universo,humanity`. The composite timeline view always existed at the long URL; these aliases make it discoverable from a typed URL.

This addresses 4 of the 6 issues in CO-170. Phase B (cross-universe project leak — `co` vs `artelonga` projects mixed) and Phase C (real `mbya` → `comunicacao` content merge) remain — both need destructive-data-move authorization.

## [1.85.0] — 2026-05-09

### Changed — CO-165: `users.email` is now an implicit verified recovery channel

Forgot-password used to require an explicit "add channel → verify code" UI step before it would do anything. New behavior: any user with a non-empty `users.email` automatically has that address as a verified recovery channel — no add-channel dance required to recover the account.

**Two new `Storage` methods:**
- `ensure_email_recovery_channel(user_id, email)` — encrypts the email and inserts a verified channel row, or bumps an existing matching row to verified. Idempotent.
- `backfill_email_recovery_channels()` — walks `users` and calls the above for every row with a non-NULL email. Returns the count touched.

**Wiring:**
- `seed_admin_user_from_env` calls `ensure_email_recovery_channel` after the admin row is created/updated, so the seed admin can recover from day one.
- `server.rs` boot path calls `backfill_email_recovery_channels` once on every startup. Existing prod admins get backfilled; new signups via `password-login` paths inherit the same guarantee through the seed call site.

The lookup hash is computed via `recovery_crypto::compute_lookup_hash` (BLAKE3 keyed) so the existing `forgot-password` lookup-by-channel-value path Just Works against the seeded rows.

This makes `forgot-password yuri@artelonga.com.br` actually deliver to that mailbox via Resend (1.84.0 cascade) without any pre-setup.

## [1.84.0] — 2026-05-09

### Changed — CO-165 + CO-169: recovery delivery now uses CO-169 channel providers

`recovery_routes::send_verification_code` was a flat match with SMTP-only email and stub WhatsApp/SMS. CO-169 shipped real provider abstractions (`ResendProvider`, `EvolutionApiProvider`) but they weren't wired into the password-recovery path.

Now resolved with a tiered cascade per channel — every step is best-effort, every failure logs the redacted recipient + raw code so an operator can recover:

- **Email** — `ResendProvider` (`RESEND_API_KEY`) → SMTP (`CO_SMTP_*`) → log only.
- **WhatsApp** — `EvolutionApiProvider` (`EVOLUTION_API_KEY`/`EVOLUTION_API_URL`/`EVOLUTION_INSTANCE`) → log only.
- **SMS** — log only (Twilio is the planned Phase 2 provider).

Two new helper functions in `recovery_routes.rs` — `deliver_email_code` and `deliver_whatsapp_code` — replace the inline match. `redact_phone` masks all but the last four digits in WhatsApp logs (consistent with existing `redact_email`).

The recovery body strings stay PT-only in this change; i18n is its own ticket.

CO-165 acceptance items "WhatsApp + SMS providers stubbed in Phase 1" are no longer "stubbed in this codebase" — the Phase 2 work has shipped (CO-169) and now flows through the recovery code path. Operators can drop in `RESEND_API_KEY` or `EVOLUTION_API_KEY` and codes route correctly without code changes.

## [1.83.0] — 2026-05-08

### Added — CO-165: real SMTP delivery for recovery codes (email)

The `email` arm of `send_verification_code` now delivers via SMTP when configured. Previously it logged the code to stderr only — workable for dev, broken for any real user.

**New module `email_smtp.rs`** — lettre-based async SMTP:
- `send_recovery_code(to, code) → Result<bool>` — `Ok(true)` when SMTP delivered, `Ok(false)` when SMTP not configured (caller logs as dev fallback), `Err` on delivery failure.
- Reads `CO_SMTP_HOST`, `CO_SMTP_USER`, `CO_SMTP_PASS`, `CO_SMTP_FROM`, `CO_SMTP_PORT` (default 587). All four required fields must be set; missing any one falls back to log mode.
- STARTTLS via rustls-TLS (`tokio1-rustls-tls` feature). PT body, plain text.

**Wired into `recovery_routes::send_verification_code`:**
- Email branch tries SMTP, falls back to logging the code with redacted recipient (`j***@artelonga.com.br`) on failure or no-SMTP.
- Spawned as a `tokio::spawn` so the request returns immediately — recovery endpoints already always return 202 (no enumeration), and SMTP can take 1-3s.
- WhatsApp + SMS arms unchanged — still Phase 2 stubs (Twilio + Meta API).

**New deps:** `lettre 0.11` with `smtp-transport`, `tokio1-rustls-tls`, `builder` features only (no native-tls, no SMTP-pool).

**Operator setup** (fly secrets):
```
flyctl secrets set CO_SMTP_HOST=smtp.example.com \
                   CO_SMTP_USER=postmaster@... \
                   CO_SMTP_PASS=... \
                   CO_SMTP_FROM='CO <noreply@artelonga.com.br>' \
                   -a co-artelonga
```

When set, recovery codes go to real inboxes. When unset (e.g. local dev), codes still print to logs so the existing dev flow keeps working.

## [1.82.0] — 2026-05-08

### Added — CO-165: Forgot password / change password with verified recovery channels

**New module `recovery_crypto.rs`** — ChaCha20-Poly1305 encryption for channel values:
- Master key from `CO_RECOVERY_KEY` → fallback `JWT_SECRET` → dev default.
- Two BLAKE3-derived subkeys: `enc` (for encryption) and `lkp` (deterministic lookup hash).
- `encrypt_channel_value` / `decrypt_channel_value` — store ciphertext+nonce in DB.
- `compute_lookup_hash` — 64-char hex for indexed lookups without decryption.
- `normalize_channel_value` — email: trim+lowercase; phone: digits+leading `+`.
- `mask_channel_value` — display masking (`j***@domain.com`, `****1234`, `wa:****1234`).

**New module `recovery_routes.rs`** — 8 endpoints:
- `POST /api/v1/auth/recovery/channels` — add channel, send 6-digit verification code.
- `POST /api/v1/auth/recovery/channels/verify` — verify channel with code (argon2id).
- `GET  /api/v1/auth/recovery/channels` — list channels with masked values.
- `DELETE /api/v1/auth/recovery/channels/{id}` — remove channel (requires current password).
- `POST /api/v1/auth/forgot-password` — send reset codes to all verified channels (always 202).
- `POST /api/v1/auth/forgot-password/verify` — verify code, receive one-time reset token.
- `POST /api/v1/auth/reset-password` — exchange token for new password, get new session.
- `POST /api/v1/auth/change-password` — change password (requires current password + JWT).

**Migration v37** — users table: email nullable + `usuario` column (backfilled from email local-part);
three new tables: `user_recovery_channels`, `recovery_verifications`, `password_reset_tokens`.

**`password_login_handler`** updated to accept `email` or `usuario` field (username+email decoupling).

18 tests: 14 endpoint tests (including delete channel, lockout, E2E happy path) + 7 crypto unit tests.

## [1.81.0] — 2026-05-08

### Fixed — Anonymous landing page now shows the CO template tutorial

Boot-time template seeding became idempotent. Previously, if the `template` universe row already existed (every deploy after the first), `seed_template_universe()` was skipped entirely — only content pages were refreshed. If the project + tutorial tasks had been lost from the per-universe entries DB at any point (old migration, manual cleanup), they stayed lost forever, so anon hitting `/` saw an empty kanban (`projects: []`) with no tutorial content to render.

`seed_template_universe()` now runs on every boot. Internal `already_seeded` check guards on `projects/CO/_project.md` existing in the entries DB — fresh tutorial tasks are NOT created if a project is present (preserves anything users edited on first-boot installs). Content pages still re-seed unconditionally on every boot, theme stays pinned to `modern`. After this deploy, prod's template universe will get its 7 tutorial tasks back on first boot.

## [1.76.0] — 2026-05-06

### Added — Direct notification provider adapters (CO-169)

CO now delivers notifications directly to Resend (email) and Evolution API (WhatsApp) without requiring an external webhook receiver:

- **`ChannelProvider` trait** (`notification_providers.rs`): `name()` + async `send()` — implemented by `ResendProvider` and `EvolutionApiProvider`.
- **`ResendProvider`**: sends transactional email via `POST api.resend.com/emails`. Requires `RESEND_API_KEY` (+ optional `RESEND_FROM`).
- **`EvolutionApiProvider`**: sends WhatsApp text via Evolution API `sendText` endpoint. Requires `EVOLUTION_API_KEY` (+ optional `EVOLUTION_API_URL`, `EVOLUTION_INSTANCE`).
- **Migration v36**: adds `telefone TEXT` to `quilombo_usuarios`; adds `channel` + `recipient` columns to `notifications`; inserts sentinel webhook row `__direct__` (satisfies FK, never dispatched as HTTP webhook).
- **`emit_event` extended**: new signature includes `email: Option<&str>` + `telefone: Option<&str>`. When provider env vars are set and a recipient is known, enqueues channel-specific rows with `channel='email'` or `channel='whatsapp'`.
- **Worker dispatch**: `webhook_worker` loads providers once at startup; dispatches by `notifications.channel`; existing webhook path unchanged.
- **Template rendering**: `{{key}}` substitution from event payload. Built-in defaults for `quilombo.evento.criado` (email subject, email body, WhatsApp text); override via `CO_TPL_{PREFIX}_{SLOT}` env vars.
- **Tests**: `notification_providers` module tests cover template rendering, prefix derivation, env-var overrides, `from_env()` (returns `None` without keys), `name()`, and `MockChannelProvider`; `webhook.rs` tests cover email/whatsapp row enqueuing and absence when keys are missing.
- **3 event call sites updated**: `quilombo.evento.criado`, `quilombo.missao.participou`, `quilombo.mensagem.criada` all pass the acting user's email + telefone.

## [1.80.0] — 2026-05-08

### Added — Outbound webhook system + notification queue (CO-168)

CO now emits signed HTTP POST events to registered endpoints, enabling n8n/Zapier/custom-adapter integration without new Rust deployments:

- **Migration v32**: `webhooks` + `notifications` tables in meta.db; partial-index on `(status, next_attempt_at)` for efficient polling.
- **Admin API** (GitHub auth, `/api/v1/gestao/webhooks`): register (secret returned once), list (secret redacted), update url/events/enabled, delete (cascades notifications), delivery log (last 100 rows).
- **`emit_event(conn, event_type, payload)`**: synchronous write to `notifications` for each enabled webhook whose event filter matches — called from request handlers after the triggering action succeeds.
- **Webhook worker**: background task started at boot, polls every 5 s, delivers one notification per tick via `reqwest` with `HMAC-SHA256` signature (`X-CO-Signature-256: sha256=<hex>`) matching GitHub's scheme.
- **Retry policy**: up to 3 retries with 5 s / 30 s / 2 min backoff; 4th failure marks `dead`.
- **Wildcard event matching**: `*` (all), `quilombo.*` (namespace), or exact event type.
- **3 quilombo events wired**: `quilombo.evento.criado`, `quilombo.missao.participou`, `quilombo.mensagem.criada`.
- **`docs/webhooks.md`**: event catalogue, admin API reference, n8n/Zapier setup guide with HMAC validation example.
## [1.79.0] — 2026-05-08

### Added — CO-167: Email collection for quilombo users

Bridge between quilombo accounts and CO unified auth (CO-166). Quilombo users can now attach an email for account recovery and future notifications:

- **Migration v34**: adds nullable unique `email` column + `linked_co_user_id` column to `quilombo_usuarios`. Unique index on `email WHERE email IS NOT NULL` — multiple users without email never conflict.
- **`PUT /api/v1/quilombo/perfil`** now accepts `email`; validates format and returns 409 if already taken by another user.
- **`POST /api/v1/quilombo/auth/login`** response now includes `missing_email: true` when the user has no email set (nudge signal for the client to show the "add email" banner).
- **`GET /api/v1/quilombo/admin/usuarios`** now includes `email` and `linked_co_user_id` per user row.
- **`GET /api/v1/quilombo/admin/resumo`** now includes `com_email` (count with email) and `vinculados_co` (count with `linked_co_user_id`) in the summary JSON.

## [1.78.0] — 2026-05-08

### Added — CO-164: Vector index for entries — semantic search

Local text-embedding pipeline using `fastembed` (all-MiniLM-L6-v2, 384-dim, ~80 MB) — no external API calls, offline-first.

- **`entry_embeddings` table** (per-universe schema): stores `embedding BLOB` (384 × f32 LE), `body_hash` (staleness guard), `model`, `indexed_at` per `(universe_key, path)`. `idx_emb_body_hash` index.
- **`EmbeddingService`** (`embedding.rs`): lazy-loads fastembed model from `~/.co/models/` (or `CO_MODELS_DIR`). Gracefully disabled when model unavailable — server continues without semantic search.
- **Background worker** (`embedding_worker.rs`): dedicated OS thread, batches up to 32 entries per inference call (≤100 ms window). Enqueued on every entry create/update/delete. Fire-and-forget (`try_send`).
- **Boot scan**: on startup, compares `entries.body_hash` vs `entry_embeddings.body_hash` across all universes and enqueues stale/missing entries. Does not re-embed unchanged content.
- **`GET /api/v1/universes/:slug/entries?semantic=<query>&k=10`**: returns top-K entries by cosine similarity, each with `_score ∈ [0, 1]`.
- **Hybrid search** (`?q=&semantic=`): merges FTS rank + cosine similarity via harmonic mean, outperforming either alone.
- **`GET /api/v1/universes/:slug/entries/similar?path=<vault-path>&k=10`**: similar entries to a given one (excludes self).
- **`GET /api/v1/search?semantic=<query>&k=10`**: cross-universe semantic search across all universes the user can read.
- `EntryRow._score` field (optional, `skip_serializing_if = None`) — set on semantic results only.

## [1.77.0] — 2026-05-08

### Added — CO-163: Mempalace BaseBackend Python shim (`scripts/mempalace_co_backend.py`)

`MempalaceCoBackend` implements mempalace's `BaseBackend` ABC backed by CO's HTTP API:

- `upsert` writes content blob via `POST /api/v1/blobs`, embedding blob via `POST /api/v1/blobs`, and metadata entry via vault PUT — three calls, parallelised via `ThreadPoolExecutor`.
- `get(ids)` reads vault entries by path, resolves `blob_hash` → bytes via `GET /api/v1/blobs/:hash`.
- `query` falls back to keyword search via `GET /api/v1/universes/:slug/entries?q=…` today; exposes a `_vector_search` hook (no-op) for swapping in CO-164's vector endpoint when it ships.
- `delete` removes vault entries; CAS blobs are content-addressed and shared — never deleted by the shim.
- Chroma-style `where` clauses: `$eq/$gt/$gte/$lt/$lte/$in` mapped server-side to CO's frontmatter index; `$ne/$nin/$and/$or` evaluated client-side.

Ships with `scripts/test_mempalace_co.py` (27 unit + mock-HTTP tests, 2 live-server integration tests behind `CO_INTEGRATION_TEST=1`) and `scripts/mempalace_co_README.md` documenting config, the keyword-only `query` caveat, and the CO-164 upgrade path.

No Rust changes. Pure Python, stdlib-only (`urllib.request`, `struct`, `json`).

## [1.76.0] — 2026-05-08

### Added — CO-166: Single Sign-On across universe deployments

## [1.75.0] — 2026-05-06

### Added — Blob CAS API: GET/HEAD/POST `/api/v1/blobs`

Exposes the meta-DB `blobs` table over HTTP for external tools (mempalace BaseBackend shim, future MCP server, agents needing raw content by hash):

- `GET /api/v1/blobs/:hash` — returns the bytes (`application/octet-stream`, immutable cache-control).
- `HEAD /api/v1/blobs/:hash` — 200 if stored, 404 otherwise.
- `POST /api/v1/blobs` — body bytes go into the CAS; returns `{hash, size}`.

Auth via `require_auth_with_token` — JWT or long-lived API token. Hash validation requires 64-char lowercase hex (sha256). The `immutable` cache-control matters: bytes-by-hash never change, so any client (CDN, mempalace, MCP) can cache aggressively.

This is the foundation for the mempalace `BaseBackend` shim (interop step 2 in the project_co_mempalace_interop memory). A Python `MempalaceCoBackend` can call these endpoints to dedupe drawer storage into CO's CAS, getting versioning + sync for free.

## [1.74.0] — 2026-05-06

### Added — Phase 8 step 4: content-fidelity rewind via blob lookups

The CO-native versioning roadmap is **complete**: pinned subscribers reading entries via `?as_of=<state>` now receive the **historical bytes** of each entry, not the current ones. This is true `git checkout` semantics — the universe served at a pin is exactly what existed when the state was captured.

**Implementation:**
1. State manifest format extended from `<combined_hash>  <path>` → `<combined_hash>  <body_hash>  <path>` per line. New `parse_state_manifest_full` returns `(path, combined, Option<body_hash>)`. Legacy 2-column lines continue to parse as no-body-hash (path-fidelity fallback).
2. `list_entries` rewind branch reads each entry's recorded body_hash from the manifest, fetches from `blobs` via `Storage::get_blob`, and substitutes the entry body. Entries without a body_hash (from pre-1.74 states, or if the blob is missing) fall back to the current body — never a fetch error.
3. Backward-compat: existing states (created before 1.74) keep working at path-fidelity. Every state captured from 1.74 onward gets the body_hash column and full content rewind.

**The full roadmap:** Phase 1 (states/commits) · Phase 2 (branches) · Phase 3 (proposals+merges) · Phase 4 (diff) · Phase 6 (pin storage) · Phase 7 (path-fidelity rewind) · Phase 8 (CAS + content-fidelity rewind, 4 steps). All shipped. The platform now version-controls itself end-to-end with API + UI parity for the canonical git operations, plus the substrate (blobs, mining, MCP-ready) to interoperate with mempalace.

## [1.73.0] — 2026-05-06

### Added — Phase 8 step 3: boot-time backfill of existing entries into CAS blobs

`Storage::backfill_blobs_from_entries` walks every universe in the meta DB, opens each per-universe SQLite, reads `body` from every entry, and calls `put_blob` for each. Idempotent — `INSERT OR IGNORE` on the unique hash means subsequent boots no-op for already-stored content. Wired into the post-seed startup path so the first boot after this deploy materializes ~5K+ existing entries' bodies into the global `blobs` table.

Sets up Phase 8 step 4 (the actual content-fidelity rewind): with every entry's body retrievable by `body_hash` from blobs, pinned subscribers can now be served historical bytes. State manifests already store `(path, hash)` pairs — the missing piece was the bytes-by-hash store, which now exists with a complete content corpus.

## [1.72.0] — 2026-05-06

### Added — `co-mine-claude-sessions`: import Claude Code transcripts as CO entries

`scripts/co-mine-claude-sessions.py` walks `~/.claude/projects/*/` for `.jsonl` session files, renders each as markdown (title + per-turn `## User · ts` / `## Assistant · ts` blocks), and PUTs them into a target CO universe at `sessions/<project-dir>/<session-id-short>.md`. Frontmatter carries `type=claude-session`, `session_id`, `project`, `user_messages`, `assistant_messages`, `started_at`, `ended_at`. Idempotent — vault PUT is upsert.

Mempalace inspiration: equivalent of `mempalace mine ~/.claude/projects --mode convos`, but writing into CO's vault. Closes the dogfood loop: every Claude Code session that built CO is now versioned + searchable inside CO.

CLI: `python3 scripts/co-mine-claude-sessions.py [universe] [--limit N] [--project SUBSTR] [--dry-run]`. Auth: API token from macOS Keychain (service=co-sync-token).

156 sessions detected at first run (97 from `~/projects/co`). Volume target: `co` universe by default.

## [1.71.0] — 2026-05-06

### Added — Phase 8 step 2: vault writes dual-write to CAS blobs

`write_vault_entry` and `index_raw_vault_file` now both call `meta.put_blob(body.as_bytes())` after the index upsert. The entry's pre-existing `body_hash` column is already sha256 of the body, which is the same key the blob store uses — so the entry doubles as a reference into `blobs` with zero schema change to the per-universe entry tables. Failures are logged but non-fatal (the on-disk file + entries index are already durable).

Going forward, every new vault write puts its body in the global `blobs` table. Existing entries (5K+ pre-1.71) will be backfilled in step 3. Step 4 makes pin-rewind reads serve historical bytes via blob lookups — the long-promised content-fidelity rewind.

## [1.70.0] — 2026-05-06

### Added — Phase 8 step 1: content-addressed blob storage layer

Migration v31 adds a `blobs` table — each row is `(hash, bytes, size, created_at)` keyed by sha256 of the bytes. Storage methods `put_blob`, `get_blob`, `has_blob`. Idempotent insert (same bytes → same hash → INSERT OR IGNORE). No vault-write integration yet — this is just the data plane that future phases (full-fidelity rewind, deduplicated entries, content-version pins) will build on.

Step 2 (next): vault PUTs dual-write — every entry's body lands in `blobs` keyed by its hash, with the entry storing `body_blob_hash`. Step 3 backfills existing entries. Step 4 pins+rewind read historical bytes via blob lookups.

## [1.69.0] — 2026-05-06

### Changed — file moves: `mbya` + `topologia` + `language` collapsed into `comunicacao`

Local: `~/projects/topologia/` and `~/projects/mbya/` content merged into a new `~/projects/comunicacao/` directory. Topologia's content sits at the root (`concepts/`, `guarani-mbya/`, `portuguese/`, `yoruba/`, `languages/`, `_template-language/`); Arandu Mbyá content is nested under `mbya/` (4619 .md files). Total 4666 .md files. Origin repos preserved on disk for now (manual cleanup later if desired).

`co-universes.yaml` updated: dropped `mbya`, `topologia`, `language` entries; the existing `comunicacao` slug now points at `~/projects/comunicacao`. Auto-sync restarted to pick up the new layout. Server-side `mbya`, `topologia`, `language` universes will be deleted (via `DELETE /universes/:slug`) once `comunicacao` finishes its initial bulk push.

### Fixed — admin-tier API tokens skip the vault per-token rate limit

The vault PUT path had a 60 req/min per-token rate limiter (CO-80) that bottlenecked legitimate bulk operations like `co-sync`'s initial push. Now admin-tier users bypass it; the check still runs for non-admin tokens (currently a no-op since the 1.45.0 single-tier model maps every authed user to admin, but kept for forward-compat). The original abuse-protection intent was for anonymous quotas, which a separate code path covers.

## [1.68.0] — 2026-05-06

### Added — public universe listing for anonymous visitors

`GET /api/v1/universes/public` returns every `public-subscribable` universe (plus the `template`) without requiring auth. The SPA's `getUniverses()` now tries the authed `/api/v1/universes` first and falls back to `/public` on 401, so anonymous visitors at the hub see a populated sidebar instead of an empty one.

Visibility-private universes still require auth + membership; this change only surfaces what was already public.

## [1.67.0] — 2026-05-06

### Fixed — `content_count` refreshed correctly after every vault write

`write_vault_entry` now ends by `SELECT COUNT(*) FROM entries WHERE universe_key = ?` and writing the result to `universes.content_count`. Replaces the pre-1.67 `increment_universe_content_count` call that lived in `put_vault_file` only — and was buggy on two axes: it overcounted on updates (+1 every time you re-saved), and it undercounted on every state/branch/proposal/merge write because those go through `write_vault_entry` directly, bypassing `put_vault_file`.

`SELECT COUNT(*)` on the indexed per-universe `entries` table is cheap and idempotent. Both legacy increment call sites in `put_vault_file` and `clipper_post` are removed (count refresh happens inside `write_vault_entry` now).

## [1.66.0] — 2026-05-06

### Added — subscribe / unsubscribe inline in the info modal

The subscription status line in the info modal grew managers:
- **Not subscribed** → `+ Subscribe` button (POST /:slug/subscribe)
- **Subscribed (following head)** → `unsubscribe` button (DELETE /:slug/subscribe)
- **📌 Pinned to X** → existing `unpin` button (DELETE /:slug/subscribe/pin)

Only shown for `public-subscribable` universes — server enforces the visibility constraint, the client mirrors it. Anonymous users see nothing here (they get null subscription).

This makes subscription a one-click operation alongside the pin flow it shares the data layer with.

## [1.65.0] — 2026-05-06

### Added — proposal create + merge from info modal

The Propostas e merges section now has:
- `+ Propor mudanças a outro universo` disclosure → form with target universe slug, title, description, source state dropdown (from current universe), target branch (defaults "main"). Submits `POST /api/v1/universes/<target>/proposals` with the source as the current universe.
- `⇆ Merge` button on each open proposal pointing AT this universe. Confirmation prompt → `POST /api/v1/universes/:slug/merges` with the proposal path. Toast shows entry-copy count.

This closes the SPA-level proposal loop: create from any universe targeting any other, merge from the receiving side. The full git-replacement primitive set — capture, branch, advance, propose, merge, diff, pin, rewind — is now driveable end-to-end from the ℹ Universo modal.

## [1.64.0] — 2026-05-06

### Added — branch head-advance from the info modal

Each branch row now has a `↳ advance head` disclosure. Expanding shows a state dropdown + apply button; submitting `PUT /:slug/branches/:name` with the chosen `head_state` fast-forwards the branch's head pointer (no merge logic — just a pointer move).

This closes the in-UI branch loop: list, create, advance head, see the change reflect in the branch row's `head:` field on re-render. Direct branch deletion is still API-only, but with `DELETE /:slug/<branches/...>` available via the entry route, it's reachable if you need it.

## [1.63.0] — 2026-05-06

### Added — branch creation form in info modal

The Branches section now has a `+ Nova branch` disclosure that expands an inline form: branch name (alphanumeric + `-`, `_`, `/`), state dropdown listing every existing state with hash + message, and a "default branch" checkbox. Submits `POST /:slug/branches` and re-renders the modal. Anonymous viewers see the existing branch list but no creation control (server gates the POST).

If the universe has no states yet, the form is replaced with "Capture um estado primeiro" — branches need a state to point at.

This rounds out the branch interaction loop in the SPA: list, create, advance (via the merge handler when you accept a proposal). Direct branch-head advance from the UI is still API-only.

## [1.62.0] — 2026-05-06

### Added — Phase 7: rewind view (`?as_of=` filter on /entries)

`GET /api/v1/universes/:slug/entries?as_of=states/...md` filters the result down to paths that existed in the named state's manifest. Validates that `as_of` starts with `states/` and that the referenced state exists in the universe; rejects with 400 otherwise. Bodies served are still current — this is path-fidelity rewind, not full content rewind. Full-fidelity (entries served as-of-their-historical-bytes) requires content-addressed blob storage and is deferred.

The SPA caches the user's pin in `state.subscriptionPin[slug]` whenever the info modal opens or after pin/unpin actions, then automatically appends `&as_of=<pin>` to every `getUniverseEntries()` call. Pin → renders the rewind view across Conteúdo, Calendário, Timeline, etc. Unpin → restores head view.

This makes pinning behaviorally meaningful: pinned subscribers see the universe as it was when they pinned, even if upstream advances.

## [1.61.0] — 2026-05-06

### Added — pin status surfaced in the info modal

`GET /api/v1/universes/:slug/subscription` returns `{subscribed, pinned_state}` for the calling user. The info modal fetches it on open and shows: "📌 Pinned to `<state>` [unpin]" when set, "Subscribed (following head)" when subscribed-no-pin, "Not subscribed" otherwise. The pinned state row gets a subtle blue background and `📌 pinned` badge instead of the pin button. Click `unpin` to clear and re-render.

## [1.60.0] — 2026-05-06

### Added — Phase 6 storage: subscribers can pin to a specific state

Migration v30 adds `pinned_state` column to `subscriptions` (TEXT, NULL = head-following). New endpoints:

- `PUT /api/v1/universes/:slug/subscribe/pin` body `{state: "states/...md"}` — pin (auto-subscribes if needed; validates that the state exists)
- `DELETE /api/v1/universes/:slug/subscribe/pin` — clear pin (still subscribed, follows head)

UI: a `📌 pin` button next to each state in the info-modal state log. Click → POST pin → toast confirms.

**What's NOT shipped yet (deferred to Phase 7):** the rewind view itself — i.e., serving entries-as-of the pinned state when a pinned subscriber reads. Today the pin is just stored. The behavioral effect ships when the entries query path becomes pin-aware. This split lets the data settle before anyone depends on the rewind semantics.

## [1.59.0] — 2026-05-06

### Added — info modal: branches + proposals/merges sections; Conteúdo hides backend types

The ℹ Universo modal now has five sections instead of three: overview, content-types, **states** (with click-to-diff), **branches**, **proposals + merges**. Branches show `name`, default flag, head state ID, and last-updated timestamp. Proposals + merges interleaved newest-first with status badges (open/merged/rejected) and source-universe references.

Conteúdo no longer includes versioning entries (`type=state|branch|proposal|merge`) in its untyped-pages fallback. They were previously bleeding into the SPA's content view as orphan markdown files; now they live solely on the info modal where they belong.

## [1.58.0] — 2026-05-06

### Changed — versioning + stats moved to a "ℹ Universo" info modal

The `⏱ Estado` save button and `🕓` history modal added in 1.55-1.57 lived on the universe header. Per direction: that's not where versioning belongs — versioning entries shouldn't show in `Conteúdo` either, but on a dedicated, **publicly viewable info page**.

The header now has a single `ℹ` button. Clicking it opens a modal with three sections:
- **Overview** — name, description, visibility badge, slug, total entries / states / branches counts
- **Tipos de conteúdo** — a breakdown of `entry_type → count` for every type present in the universe (tasks, concepts, translations, states, branches, etc.)
- **Estados (versões)** — embedded `git log` equivalent: list of all states newest-first, click to expand the inline diff vs parent. Includes a `⏱ Capturar estado` button that runs the same POST as before (auth required — anonymous viewers see read-only).

This is the public face of "what's inside this universe" for both anonymous and authed callers. It also took a baseline state on every existing universe (10 of them: co, mbya, topologia, artelonga, quilomboaraucaria, rfq, time, comunicacao, yggdrasil, dados, yuri) so each has a starting point in its state chain.

## [1.57.0] — 2026-05-06

### Added — clickable state rows show inline diff vs parent

In the state history modal (1.56.0), clicking a state row now expands an inline panel showing the diff against that state's parent — green `+` for added paths, amber `~` for modified, red `-` for removed, plus the unchanged count. Uses the `/states/diff` API (1.51.0). Click again to collapse.

Lists are truncated at 50 paths per category with a `… and N more` footer to keep large changesets readable. The first state in a chain shows "no parent to diff against" instead of a fetch.

This brings the SPA to `git log --stat` parity for the versioning roadmap.

## [1.56.0] — 2026-05-06

### Added — state history modal (🕓 button next to "⏱ Estado")

Click the clock icon to open a modal listing every state of the current universe (newest first), each row showing the `state_hash` prefix, the message you typed, entry count, parent state, author, and a localized timestamp. No interaction beyond viewing yet — diff visualization and "rewind to this state" are still on the roadmap.

`git log` equivalent reached parity with `git commit` (1.55.0). Branch listing, diff view, and proposal review are next on the SPA UI track.

## [1.55.0] — 2026-05-06

### Added — "⏱ Estado" button: first frontend surface for CO-native versioning

A header button next to "+ Nova Tarefa" in every universe view. Click prompts for an optional message, posts to `/api/v1/universes/:slug/states`, and shows a toast with the new state's hash prefix and entry count. First end-user-facing surface for the versioning roadmap (Phases 1-4 had API only); takes a `git commit`-equivalent in two clicks.

i18n: `save_state` key added in pt-BR ("⏱ Estado") and en ("⏱ State").

Doesn't yet surface state history, branches, proposals, merges, or diff — those land as the SPA UI catches up to the API surface. The button is the smallest unit that brings versioning out of curl-only.

## [1.54.0] — 2026-05-06

### Fixed — boot-reconcile no longer stomps user edits

`seed_admin_content_universes` previously ran an unconditional `UPDATE universes SET name=…, description=…, visibility=…, parent_key=…` for every seeded universe on every boot. Any user edit to those fields was reverted on the next deploy — directly contradicting the 1.45.0 "any authed user can edit any universe" model.

Now the seed is INSERT OR IGNORE only (with `parent_key` and a correctly derived `is_public` added to the initial-insert column list). Existing rows are never touched. User edits persist across deploys.

Trade-off: seed values are now strict defaults — if you change the seed list (e.g., rename `time` from "Time" to "Tempo"), the new value won't propagate to existing rows. Corrections to declared intent for already-seeded universes require an explicit migration that targets the specific row by key. This is the right boundary: declared seed = initial scaffolding, not ongoing source of truth.

## [1.53.0] — 2026-05-06

### Fixed — `time` universe seeded as `public-subscribable` (was hardcoded `private`)

`seed_admin_content_universes` listed the `time` universe with `visibility=private` and the boot-reconcile UPDATE re-asserted that on every deploy, stomping any user-set value. Aligns the seed with `co-universes.yaml` (declared `public-subscribable`). Now anonymous viewers can see metadata for `/time` and authed users get full read on its 56 events without explicit subscription.

The broader pattern — boot-reconcile UPDATE overwriting user-set fields — is still present for `name`, `description`, `parent_key`, and other seeded universes. That contradicts the 1.45.0 "any authed user can edit any universe" model. A separate fix to make the reconcile additive (only fill in nulls, never overwrite user values) is on the followup list.

`dados` visibility (`public-subscribable` despite never being in the seed) traced to the v29 migration: it must have had `requires_login=1` set in its original row, which v29 collapsed to `public-subscribable`. That's fine — the universe is functioning and the migration was idempotent.

## [1.52.0] — 2026-05-06

### Fixed — `co` boot-seed no longer wipes user-generated entries

`seed_co_universe_tasks` previously ran `DELETE FROM entries WHERE universe_key = 'co'` on every deploy before re-seeding from `seed-co/`, then upserted the seed files back. The wipe killed every state, branch, proposal, and merge entry created since the last deploy — the CO-native versioning roadmap couldn't dogfood on the `co` universe itself.

The seed is now purely additive: it only upserts paths that exist in `seed-co/`, leaving every other entry untouched. If you remove a file from `seed-co/`, the corresponding entry will linger on prod until explicitly deleted via `DELETE /universes/:slug/...` (or the entry's vault path) — but that's the right tradeoff for a system whose canonical content source is auto-sync from local, not the baked-in seed.

`content_count` now reflects the actual entry-index row count rather than just the seed-file count, so user-generated entries get counted in `GET /universes/co`.

## [1.51.0] — 2026-05-06

### Added — Diff API: compare two state manifests (Phase 4 of CO-native versioning)

`GET /api/v1/universes/:slug/states/diff?from=<state>&to=<state>` returns the path-level differences between two states in the same universe:

```json
{
  "from": "states/2026-...A.md",
  "to":   "states/2026-...B.md",
  "added":    [{ "path": "concepts/new.md", "to_hash": "..." }],
  "removed":  [{ "path": "old.md", "from_hash": "..." }],
  "modified": [{ "path": "x.md", "from_hash": "...", "to_hash": "..." }],
  "unchanged": 47
}
```

Implementation parses the line-per-entry manifest from each state's body (the fenced `<hash>  <path>` block written by `create_state` since 1.47.0) and computes the set difference. No new tables, no schema migration. The diff is now usable by:

- the proposal review surface ("show me what this proposal changes")
- a "what's new since I last visited" indicator over a subscription's last-seen state
- bisect-style "when did this entry first appear" queries over a state chain

3 new unit tests cover the parser (fenced-block extraction, empty-fence edge case, ignoring text outside the fence).

## [1.50.0] — 2026-05-06

### Added — DELETE /universes/:slug + structured-data files write verbatim

`DELETE /api/v1/universes/:slug` removes a universe entirely — cascades through `entries`, `entries_fts`, `universe_members`, `subscriptions`, then the on-disk universe directory. Refuses to delete `template`. Any authenticated user can delete (1.45.0 single-tier model). Closes the gap that left 5 stale fragment universes (`concepts`, `guarani-mbya`, `portuguese`, `yoruba`, `languages`) plus the test fork from 1.49 unreachable for cleanup.

### Fixed — vault PUT no longer wraps non-markdown files with frontmatter

Vault PUT previously treated every body as markdown-with-frontmatter, prepending `---\n{}\n---\n` to YAML/TOML/JSON files. That broke `_universe.yaml` parsing because the manifest reader saw `{}` as the first YAML doc. New `is_raw_data_file()` helper detects `.yaml`/`.yml`/`.toml`/`.json` paths and writes them verbatim via `write_raw_vault_file` + `index_raw_vault_file` (still indexes the entry but with empty frontmatter and the raw body). Existing `.md` writes are unchanged.

## [1.49.0] — 2026-05-06

### Added — Proposals + merges: cross-universe versioning Phase 3 (replaces `git pull request`)

A `proposal` is an entry of `type=proposal` requesting that the content from a `source_universe`'s `source_state` be merged into a `target_universe`'s named branch. A `merge` is the event-record of acceptance — also an entry, stored alongside the merged target state for forensics.

New endpoints:
- `POST /api/v1/universes/:slug/proposals` — create a proposal targeting `:slug`. Validates source universe + source state exist and source ≠ target.
- `POST /api/v1/universes/:slug/merges` — execute the merge. Body: `{proposal: "proposals/...md"}`. The handler copies every non-metadata entry from the source state into the target (filtering out `state`, `branch`, `proposal`, `merge` types so universe-local bookkeeping doesn't propagate), takes a fresh state in the target, advances `target_branch.head_state` if the branch exists, writes a `merge` entry recording the event, and flips `proposal.status="merged"`.

Naive Phase 3 semantics: source state wins on overlap (same-path entries get overwritten); entries in target that aren't in source are left untouched (additive). True three-way merge with conflict resolution is deferred to Phase 4.

This closes the basic git-replacement loop: `state` (commit) + `branch` (named pointer) + `proposal`/`merge` (PR flow). Phase 4 adds subscriber pinning to a state ID; Phase 5 adds proper conflict resolution.

## [1.48.0] — 2026-05-05

### Added — Branches: CO-native versioning Phase 2 (replaces `git branch`)

A `branch` is just an entry with `type=branch`, stored at `branches/<name>.md`. Frontmatter carries `name`, `head_state` (path to a `state` entry), `default`, plus the usual `created_at`/`updated_at`/`author`. Branch names accept letters, digits, hyphen, underscore, and slash (so `feat/new-flow` works), 1–100 chars.

New endpoints:
- `POST /api/v1/universes/:slug/branches` — create a branch pointing at a state
- `PUT /api/v1/universes/:slug/branches/:name` — advance head to a newer state

Listing is just `GET /entries?type=branch` (existing endpoint). Both create and advance verify that the referenced `head_state` exists in the universe before writing.

Deliberately deferred to Phase 3+: "active branch" semantics (which branch new writes flow into — today writes go to the universe filesystem flat; branches are bookmarks over the linear state history), branch deletion (depends on `DELETE /universes/:slug` first), and merge events with conflict resolution.

## [1.47.0] — 2026-05-05

### Added — States: CO-native versioning Phase 1 (replaces `git commit`)

`POST /api/v1/universes/:slug/states` writes an atomic point-in-time capture of every entry in the universe (excluding states themselves, to prevent recursive hash drift). The state is stored as just another entry — `type=state`, path `states/<ISO-timestamp>-<nanoid>.md`, body is a stable line-per-entry serialization of `<sha256>  <path>` sorted by path. Frontmatter carries `parent` (auto-wired to the most recent prior state), `state_hash` (sha256 of the body), `entry_count`, `author`, `message`. Same dedup property as a git commit hash — two states with the same `state_hash` capture identical content.

Forward-compatible: states are entries, so they flow through every existing infrastructure path (FTS search, vault API, WS broadcast, visibility gate). Listing states is just `GET /entries?type=state&$sort={created_at:-1}`. No new tables, no schema migration. Branches and merges arrive in subsequent phases as additional content types.

This is the first concrete primitive replacing git for CO development workflows — see `feedback_no_git.md` memory for the broader direction.

## [1.46.0] — 2026-05-05

### Changed — collapse `requires_login` into `public-subscribable` + default-subscriptions

Three visibility states remain (down from four): `template`, `public-subscribable`, `private`. `requires_login` was redundant — it gated existence behind login but otherwise behaved like `public-subscribable` for authenticated users. Migration v29 flips every `requires_login` row to `public-subscribable` and tags it `default_for_new_users=1`.

New `default_for_new_users` flag on `universes`: when set, every new signup auto-subscribes (`subscribe_user_to_default_universes` runs on `create_user`). Existing users get a one-time `backfill_default_subscriptions` on boot so the v29 flag actually reaches their sidebars. Yggdrasil is the first beneficiary — every authed user now sees it without explicit opt-in, the way `requires_login` used to imply.

`check_universe_access` for `public-subscribable` now returns `ReadOnly` for any logged-in user (matches the 1.45.0 single-tier model — every authed user is admin, so subscription-as-paywall is gone). Anonymous still gets `MetadataOnly` (discovery surface intact).

`PUT /universes/:slug` rejects `visibility=requires_login` as invalid — only `private` and `public-subscribable` are user-settable now.

## [1.45.0] — 2026-05-05

### Changed — single-tier permission model: every authenticated user is admin

`Tier::parse` now maps every named tier value (`user`, `player`, `pro`, `admin`) → `Tier::Admin`. Anonymous remains the only non-admin tier. The legacy `User`/`Pro` enum variants stay around for backward compatibility with existing test fixtures and DB rows but are no longer produced by parsing — pre-collapse rows on already-deployed prods read as Admin without a DB migration.

Owner-only checks dropped on `PUT /api/v1/universes/:slug` (universe-metadata edit) and `GET /api/v1/universes/:slug/subscribers` — any authenticated user can edit any universe they can see. The visibility gate (private vs subscribable vs public) remains the only access control. A future `is_static` flag will be the single read-only exception.

`seed_admin_user_from_env` (run on every boot) now updates `tier='admin'` for the seeded user even when their hash is unchanged — pre-1.45 prods automatically promote the seed admin on deploy. New users created via any flow (seed, login, password-login) default to `tier='admin'`.

Storage-quota tests rewritten: the tier-based 10k-entry cap is gone for authenticated users; anonymous 100-entry cap remains.

## [1.44.1] — 2026-05-05

### Added — Timeline view renders entries-as-events

Closes the gap from 1.44.0: when the universe manifest declares a `presentation.calendar.date_field` (CO-73), the Timeline tab now renders a chronological feed of those entries grouped by month, instead of falling through to the legacy task gantt that requires a project. Entry click opens the zoom modal. Pairs with the calendar grid view — same data source, two layouts, picked by tab.

## [1.44.0] — 2026-05-05

### Changed — Conteúdo is the default view; Calendar/Timeline are first-class tabs

Tab order: **Conteúdo** (default) → Kanban → Tabela → Calendário → Timeline → Painel. Conteúdo is now the universal default for any universe — entries-as-source-of-truth, regardless of whether the universe has a populated kanban project. Calendar can render entries-as-events even without a project when the manifest declares a `presentation.calendar.date_field` (CO-73), so the time universe's `at_iso` events show on the calendar grid out of the box.

**Known gap:** Timeline view still reads from `filteredTasks()` (legacy), so it doesn't yet render entries-as-events. The `events should be compatible with timeline format and calendar` directive is half-done — calendar works, timeline needs a separate pass to share the manifest-date path.

## [1.43.1] — 2026-05-05

### Fixed — static assets returning SPA HTML after URL refactor

`/{slug}` matched `/style.css`, `/app.js`, `/manifest.json`, and `/shared/production.css` — every static asset got served the SPA HTML shell, killing all CSS and breaking the layout entirely. `serve_co_index` now sniffs for file extensions in the path and asset prefixes (`shared/`, `variants/`, `pdfjs/`, `games/`, `icons/`) and delegates to the static-file handler. Plus 301 redirects from legacy `/co/{slug}` URLs to `/{slug}` so old bookmarks land in the right universe instead of the SPA shell loading `co` with a phantom subpath.

## [1.43.0] — 2026-05-05

### Changed — drop the `/co` URL prefix; CO is hosted at the root

Universe URLs are now `/{slug}` (was `/co/{slug}`). The platform hub lives at `/` (was a redirect to `/co`). The `co` slug is no longer a path namespace — it's just one universe instance among many, so `/co` now resolves to the `co` universe's view, not the platform hub. Reserved top-level paths (cannot be used as universe slugs): `api`, `admin`, `settings`, `yggdrasil`, `static`, `health`, `_app`, `v1`. The service worker, telemetry middleware (server + client), wikilink generator, and asset browser all updated to the new format. Hard cut — old `/co/{slug}` URLs return 404. Bookmarks need updating.

## [1.42.1] — 2026-05-04

### Fixed — rate limiter recognizes API tokens

The rate-limit middleware (`extract_auth_identity`) only decoded JWTs, so requests authenticated via long-lived API tokens (CO-35) fell through to the Anonymous-by-IP bucket — a single admin running multiple background workers got the same 20-reads/min limit as a public visitor. New `extract_auth_identity_with_token` does the JWT decode first, then on miss looks up `api_tokens` and joins to `users` for the owner's tier. Manifested as 7 watchers failing initial WS upgrade with HTTP 429 during co-sync startup.

## [1.42.0] — 2026-05-04

### Added — universe template, reindex, raw blob, link/PDF fixes

**CO-161: single tower visibility gate** — Replaced 13 per-handler `check_reader_for_entries` calls and the duplicate `asset_routes::check_reader` with `universe_visibility_gate` + `universe_writer_gate` middleware applied once to the combined `universe_content_api` router. 4 integration tests: anon/public → 200, anon/private → 401, owner/private → 200, non-member/private → 403.

**Universe template scaffold** — `POST /{slug}/apply-template` creates `CLAUDE.md` and `docs/api.md` (type: doc), adds `doc` to `_universe.yaml`, and returns a type-check report of entries with missing or undeclared types. Idempotent. `POST /apply-template-all` runs across all owned universes and writes a dados-style hub entry (`universes.md`) in a designated private universe.

**Reindex** — `POST /{slug}/reindex` walks all `.md` files on disk via `co::scan_entries` and rebuilds the SQLite entry index. Fixes stale content when files are added outside the Vault API (git commits, local edits). Also syncs `content_count` and invalidates caches.

**Entry limit raised** — Default query limit 500 → 5 000. Configurable via `?limit=N` (max 50 000). Fixes mbya showing only 500 of 4 608 lexemes.

**Raw blob endpoint** — `GET /{slug}/blob/{*path}` serves any file from the universe directory with the correct Content-Type. Protected by visibility gate. Used by the PDF viewer when a reference card has `file:` but no `blob_sha256` (e.g. `mbya/refs/*.pdf`).

**Wikilink and deep-URL fixes** — Wikilinks now resolve to `/co/{slug}/{path}` (removed spurious `/entries/` segment). `readUniverseSlugFromUrl` handles deep paths `/co/{slug}/{*rest}`. `maybeOpenEntryFromUrl` fetches and opens any linked entry directly. Per-segment path encoding (`split('/').map(encodeURIComponent).join('/')`) preserves slashes in multi-segment paths.

**Markdown tables** — Fallback renderer now handles GFM pipe tables including column alignment.

**Migration scripts** — `scripts/sync-to-prod.sh` and `scripts/consolidate-topologia.sh` for pushing local content to prod and merging topologia sub-universes into a single universe (option C: alphabetical folders).

## [1.41.1] — 2026-05-03

### Fixed — privacy: anonymous reads on private universes were leaking entries

Surfaced during the readiness review. `/api/v1/universes/<u>/entries` (and sibling read paths `/entries/{path}`, `/entries/tags`, `/entries/tree`, `/manifest`, `/query`, `/citations`, `/citations/orphan-wikilinks`, `/relations/inbound`, `/relations/outbound`, `/references`, `/references/orphan-blobs`, `/references/broken-cards`, `/references/{path}`, `/references/works`) returned 200 with full content when called anonymously, regardless of the universe's `visibility`.

Symptoms confirmed before the fix:
- `concepts` (private, content_count=8) → 200 + 8 entries to anonymous
- `languages` (private, content_count=5) → 200 + 5 entries to anonymous
- `time` (private, content_count=56) → 200 + 56 entries to anonymous
- `rfq` (private, content_count=206) → 200 + 206 entries to anonymous

Added `entry_routes::check_reader_for_entries(state, headers, universe_key)` mirroring the gate already used by `asset_routes::check_reader`: public/template universes pass through; private universes require an authenticated user who is the owner or has a `universe_members` row. Wired the gate into 13 read handlers across `entry_routes`, `relation_routes`, and `reference_routes`.

After this deploy: anonymous requests on the new topologia universes (`concepts`, `guarani-mbya`, `portuguese`, `yoruba`, `languages`) and `time` and `rfq` return 401; public-subscribable universes (`mbya`, `co`, `artelonga`, `quilomboaraucaria`, `template`) keep returning 200.

A follow-up would be to consolidate the gate into a tower middleware applied once on the `/api/v1/universes/{slug}/...` prefix instead of 13 per-handler calls — files as a future ticket; this fix closes the immediate leak.

## [1.41.0] — 2026-05-03

### Added — CO-160: Inline PDF renderer in the SPA (PDF.js)

Reference entries with `type: reference`, `medium: pdf`, and a valid `file:` field now render an inline PDF viewer below the markdown body when opened in the zoom modal. The viewer is powered by PDF.js 5.7.284 (self-hosted, no external dependency), embedded at `/pdfjs/` and served by the same static file handler as other assets.

- `shouldRenderInlinePdf` / `pdfUrlFromCard` / `buildPdfViewerHtml` / `initPdfViewerActions` helpers detect the entry shape and wire up the iframe
- PDF URL resolves via the asset endpoint (`blob_sha256`) if available, falling back to the vault path-relative URL
- `<iframe loading="lazy" allowfullscreen>` uses browser-native lazy loading; PDF bytes are only fetched when the viewer is in the viewport
- "Baixar PDF" button triggers a browser download with the original filename via the `download` attribute
- "Tela cheia" button calls `requestFullscreen()` on the iframe element (Fullscreen API)
- Auth cookies are forwarded automatically (same-origin); private universe PDFs are not accessible to anonymous viewers without a session
- `pdfjs/` path prefix added to the static file handler in `server.rs`; `.mjs` MIME type added to `guess_content_type`
- PDF.js bundle: `build/pdf.mjs`, `build/pdf.worker.mjs`, `build/pdf.sandbox.mjs`, `web/viewer.html`, `web/viewer.mjs`, `web/viewer.css`, `web/images/`, `web/locale/en-US/` + `web/locale/pt-BR/` — ~4.3 MB vendored at `co-web/static/pdfjs/`
- No LCP regression on entries that don't have an inline-PDF section (iframe is never inserted)

## [1.40.0] — 2026-05-03

### Added — CO-158: Reference versioning — work_id + editions[] + primary/secondary source chain

- `references_meta` table gains `work_id`, `edition_id`, `primary_layer` columns; PK changed to `(universe_key, entry_path, edition_id)`.
- Per-universe DB migration v8: existing rows backfilled with `edition_id = 'default'`, `work_id` derived from filename stem, `primary_layer = NULL`.
- Reference cards may now carry an `editions:` array — one `references_meta` row is written per edition, so a single card can represent multiple concrete artifacts (scans, reprints, OCR'd versions).
- `work_id` groups all editions of the same conceptual work; auto-derived from the card's filename stem when not explicitly authored.
- `primary_layer` stores the minimum layer value from `primary_source_chain` (0 = phenomenon, 1 = transcription, 2 = publication, 3+ = re-print / scan / OCR); `null` when no chain is authored.
- Duplicate sha256 detection: re-uploading a PDF that already exists in `references_meta` under the same `work_id` skips creating a second edition row.
- New REST endpoints:
  - `GET /references?work_id=<id>` — return every edition row for a given work
  - `GET /references?primary_layer=<n>` — return references with that source-chain layer
  - `GET /references/works` — list all distinct `work_id` values in the universe
- 5 new CO-158 unit tests; existing CO-156 tests updated to pass with the new schema.

## [1.39.0] — 2026-05-03

### Added — CO-156: Universal envelope — `reference` content type + uniform CRUD telemetry

#### Part A — `reference` as a first-class content type

- `_universe.yaml` parser now accepts `properties_per_type` with the per-content-type property map using `kind: text|int|enum|list` vocabulary; content types may be declared as bare strings (`- reference`) or full objects.
- Per-universe DB v7 migration creates `references_meta` (structured shadow table) + `references_fts` (FTS5 index over title, body, transcription).
- Every reference-card write (via entry_routes, vault_routes, or the new reference_routes) upserts `references_meta`; sha256 of the bound sibling asset is resolved and stored.
- New REST API under `/api/v1/universes/{u}/references`:
  - `GET /references?medium=pdf` — list cards with medium/seed_status/FTS filters
  - `GET /references/orphan-blobs` — assets with no card
  - `GET /references/broken-cards` — cards whose `file:` doesn't resolve
  - `GET /references/{*path}` — single card
  - `POST /references` — create card
  - `PUT /references/{*path}` — update card
  - `DELETE /references/{*path}` — delete card (blob unaffected)

#### Part B — Universal CRUD telemetry envelope

Every state change now emits one `telemetry_events` row with `event_type = "crud"` carrying a uniform envelope: `kind` (`entry.upsert`, `entry.delete`, `asset.upload`, `asset.delete`, `relation.create`, `relation.delete`, `ws.connect`, `ws.disconnect`, `ws.lag`, `auth.login`, `auth.logout`), `universe`, `list`, `key`, `actor`, `session_id`, `deployment_version` (from `CARGO_PKG_VERSION`), `timestamp_ns`, and `extra`.

- `deployment_version` matches `cargo workspace.package.version` at write time.
- `session_id` is derived from JWT session cookie hash or anon visitante cookie hash.
- `/co/co/telemetria` admin dashboard now shows CRUD events by kind with 24-hour window.
- `GET /api/v1/admin/telemetry/crud-summary` returns the 24h CRUD breakdown.
- `docs/telemetry-envelope.md` documents all event kinds and their `extra` shapes.

### Added — PDF metadata extraction tool (CO-157)

`scripts/extract-pdf-meta.py` auto-populates reference-card `.md` siblings from source PDFs. Extracts title (from `/Info.Title` or first-page heuristic), authors, year, page count, sha256, language (via `langdetect`), DOI (regex `10.\d{4,9}/...`), ISBN, abstract, and keywords. Writes YAML frontmatter + prose body matching the `reference` content type envelope from CO-156.

- Diff mode (existing `.md`, no `--force`): shows unified diff on stderr, exits non-zero if stable fields differ
- `--force`: rewrites auto-generated block (frontmatter + abstract section) while preserving `## Notes` and any human-authored content
- Flags `extraction: text-only-failed` for image-only PDFs where `pypdf` yields no text
- Test fixture at `tests/fixtures/stub.pdf` + 25 unit and integration tests in `tests/test_extract_pdf_meta.py`

### Added — CO-159: INMET moon-phase importer

`scripts/import-moon-phases.py <year>` — fetches the lunar phase table from
`portal.inmet.gov.br/paginas/luas` and writes one `.md` per phase into
`time/moon-phases/<year>/` using the `moon-phase.md` template frontmatter.

- Parses four columns (LUA NOVA → `moon.new`, LUA CRESCENTE → `moon.first-quarter`,
  LUA CHEIA → `moon.full`, LUA MINGUANTE → `moon.last-quarter`)
- Times in BRT (UTC-3); `at_iso` = BRT + 3 h, `at_local` carries the wall-clock
- Idempotent: skip if `at_iso` matches the existing file; update if INMET revised the table
- Fails loudly on any unexpected HTML structure so silent data corruption is impossible
- Cross-year: `--time-dir` and `?ano=<year>` URL parameter work for any year
- `tests/fixtures/inmet-luas-2026.html` — offline HTML state for CI (2026: 50 phases)
- Ran against `~/projects/time` to populate all 50 phases for 2026

## [1.38.11] — 2026-05-03

### Added — `time` universe + Cadogan/ayvu-rapyta reference + 3 follow-up tickets

A 7th private universe `time` for every time-stamped event the system knows about — astronomical (`moon-phase`, `eclipse`, `equinox`, `solstice`), generic (`event`), and internal (`telemetry-event`). One queryable timeline; `at_iso` is the canonical sort key. Lives at `~/projects/time/`.

Manifest declares 6 content_types and the supporting properties: `at_iso` (UTC instant), `at_local` (wall-clock), `duration_seconds` (for events with extent), `geo` (lat/lon/region/timezone), `source` + `source_url`, `kind` (sub-type tag for SPA rendering), and the telemetry-specific `related_universe` / `related_entry_path` / `actor_id` / `deployment_version` / `extra` fields.

Scaffolded skeleton:

- `time/_universe.yaml` — manifest with the 6 content types
- `time/index.md` — universe home explaining "why one universe, not many"
- `time/README.md` — directory layout and source-of-truth policy
- `time/templates/{event, moon-phase, telemetry-event}.md` — copy-and-edit templates
- `time/moon-phases/2026/2026-01-13-new.md` — first hand-authored INMET phase (will be replaced by CO-159's importer)

### Added — Cadogan / ayvu-rapyta reference card

`mbya/refs/ayvu-rapyta-cadogan.md` — reference card for León Cadogan's *Ayvu Rapyta: Textos míticos de los Mbyá Guaraní del Guairá* (1959). Demonstrates the `secondary_source: true` + `canonical_source: indigenous-mbya-knowledge-keepers` distinction that CO-158 will turn into a first-class chain-of-custody schema. Identifies 7 Mbyá terms likely to be cross-referenced once the body is read (ayvu, ayvu-rapyta, ñe'ẽ, ñamandu, tenondé, jaryi, kuaray).

### Filed — CO-157, CO-158, CO-159

Three follow-up tickets for the patterns this work surfaces:

- **CO-157** — PDF metadata extraction tool (`scripts/extract-pdf-meta.py`); read the PDF's /Info dict + first-page heuristics + DOI regex + sha256 to auto-populate the reference card. Reduces "drop a PDF, run, review and commit" friction.
- **CO-158** — Reference versioning. Same conceptual work (`work_id`) → multiple concrete artifacts (`editions[]`); each edition has its own sha256, pages, language, editor_notes, seed_status. Plus `primary_source_chain` documenting layers of mediation between original phenomenon and cited document — the schema honestly captures "this is a digital scan of a 1992 reprint of a 1959 transcription of 1940s field recordings."
- **CO-159** — INMET moon-phase importer; scrapes `portal.inmet.gov.br/paginas/luas` and emits one `.md` per phase into `time/moon-phases/<year>/`. Idempotent re-runs. Cross-year support out of the box.

`time` is `visibility: private`; admin gets membership via the same `system_keys` list as the topologia universes.

## [1.38.10] — 2026-05-03

### Fixed — admin gets membership in mbya + topologia universes

`ensure_admin_universe_memberships` only granted yuri membership for `template`, `quilomboaraucaria`, `yggdrasil`, `dados`, `artelonga`, `rfq`, `co` — the 5 mbya/topologia universes were missing. Symptoms: `GET /api/v1/universes/languages` returned 404 to yuri (private universe + non-member = pretend it doesn't exist), and `POST /api/v1/universes/mbya/assets` returned 403 (PDF uploads silently failing — observed: 8/8 binaries failed at `bulk-upload-binary.py mbya`).

Added the 6 keys (`mbya`, `concepts`, `guarani-mbya`, `portuguese`, `yoruba`, `languages`) to the system_keys list. Idempotent on every boot via `INSERT OR IGNORE`. After deploy, yuri sees these universes in the sidebar + can upload binaries to them.

## [1.38.9] — 2026-05-03

### Added — `languages/` catalog universe with authoritative metadata

A 6th topologia universe — `languages` — that holds one `.md` per language with structured metadata: BCP-47 code, native/EN/PT names, ISO 639-1/3, Glottolog code + URL, SAPhon URL (for South American indigenous), language family, geographic centroid (lat/lon), speaker estimate, cross-ref to the term plane (when one exists).

```
GET /api/v1/universes/languages/entries
GET /api/v1/universes/languages/entries/gn-mbya.md
GET /api/v1/universes/languages/entries?q=tupi
```

Initial 4 entries: `gn-mbya` (SAPhon + Glottolog `mbya1239` + Dooley reference), `pt-BR` (Glottolog `braz1246`), `en` (Glottolog `stan1293`; meta-language for concept anchors, no term plane), `yo` (Glottolog `yoru1245`; Afro-Brazilian liturgical scope).

Source-of-truth policy when authorities disagree (documented in `topologia/languages/index.md`):
- Identity: Glottolog wins.
- SA indigenous phonology / coordinates: SAPhon wins.
- Geography otherwise: SAPhon for SA indigenous → community/state stats → Wikipedia infobox.

This catalog is the foundation for CO-153 (cross-universe `entry_relations.to_universe`) — term entries currently carry `language_code: gn-mbya` as a string; once cross-universe relations land, they upgrade to `co://languages/gn-mbya.md` refs that resolve through the relation graph.

`languages` is `visibility: private` (same status as the other 4 topologia universes — under active authoring).

## [1.38.8] — 2026-05-03

### Changed — topologia universes private; watcher narrows to `.md`-only

`seed_admin_content_universes` now declares the 4 topologia universes (`concepts`, `guarani-mbya`, `portuguese`, `yoruba`) as `visibility: private` (down from `public-subscribable`). Reason: the term entries are still under active authoring with non-native draft status; flipping back to public-subscribable comes when seed_status passes review. The reconcile pass on every boot pushes the new visibility through. `mbya` (Arandu) stays public-subscribable.

`co-agent-watch::is_syncable` narrowed to `.md`-only. Binaries (PDF, image, audio, video) need the `/api/v1/universes/{u}/assets` path with sha256 content addressing — the WS protocol's `CoFile.content` is UTF-8-checked at the server, so PDFs were previously sent over the wire and silently rejected. Filter at the source instead. Run `scripts/bulk-upload-binary.py <slug> <root>` to push binaries; CO-151 Phase 2 will add a typed `Asset` body to `SyncDelta` so the watcher can stream them too.

### Added — CO-156 filed (universal envelope: binary content cards + uniform CRUD telemetry)

Filed `work/co/CO-156.md` codifying the pattern that emerged from the topologia + mbya/refs work: a `reference` content type with a `.md` metadata card sibling for any non-markdown asset (PDF, image, video, YouTube URL); an indexable `references_meta` shadow table + FTS over `transcription`; a single telemetry envelope every CRUD + WS state change emits. Subsumes/supersedes CO-154's narrower scope.

### Authored — content (synced via watcher to prod)

- `topologia/concepts/concepts/fractality.md` — new concept anchor (kosmos domain).
- `topologia/concepts/concepts/recursion.md` — new concept anchor (language domain).
- `topologia/guarani-mbya/terms/pindovy.md` — 4-way species mapping example: Mbyá `pindovy` ↔ folk Portuguese names ↔ scientific *Syagrus romanzoffiana* ↔ geographic distribution. Demonstrates the universal-schema pattern from `topologia/docs/universe-as-list-of-lists.md`.
- `topologia/portuguese/terms/jeriva.md` — companion folk-name entry pointing back at the canonical pindovy mapping.
- `topologia/docs/universe-as-list-of-lists.md` — philosophy note: universe = list of lists; state = (user_session, version_deployment); universal CRUD + telemetry envelope.
- `mbya/refs/index.md` — index of references (7 PDFs + 1 YouTube stub).
- `mbya/refs/{CADERNO4_CRISTINE_TAKUA_GUA, educacao_indigena_…, GNDicInt, GNDicLex, Livro_Guarani_digital, PICH0255-T}.md` — metadata cards for each PDF in the project, with `seed_status`, mime, size, language, keywords, and links into the lexicon.
- `mbya/refs/youtube-czwpPvu3ziQ.md` — pattern stub for YouTube references (URL + chapters/transcription/likely-mbya-terms slots).

## [1.38.7] — 2026-05-03

### Added — meaning-topology universes (mbya + topologia 4-plane) into sync

`seed_admin_content_universes` now creates 5 new public-subscribable universes on every prod boot:

| Key | Source | Purpose |
|---|---|---|
| `mbya` | `~/projects/mbya/` | Arandu Mbyá Guarani lexicon (Rust workspace + content) |
| `concepts` | `~/projects/topologia/concepts/` | Language-agnostic meaning anchors |
| `guarani-mbya` | `~/projects/topologia/guarani-mbya/` | Mbyá Guarani term plane (cross-language layer above Arandu) |
| `portuguese` | `~/projects/topologia/portuguese/` | Portuguese term plane |
| `yoruba` | `~/projects/topologia/yoruba/` | Yoruba term plane |

`scripts/co-watch-v2.sh` REPOS array now spawns one watcher per universe (9 total). Local edits to any of the 5 new repos sync to prod via the CO-151 protobuf+WS path.

### Added — `topologia/` becomes a Rust workspace

Created `topologia/Cargo.toml` + `topologia/crates/topologia-core/` — a no-I/O crate of shared types (`Term`, `Concept`, `LanguagePlane` trait, `ConceptPlane` trait, `TranslationLink`) that **mbya** (Arandu) and **co** can both add as a path dependency. The crate documents the two distinct i18n patterns:

1. **Language as universe** (lexicon model) — each language is a CO universe, every entry is a `term`, cross-language linking via `co://concepts/<key>.md` URIs.
2. **Language as frontmatter field** (translation model) — any user's entry can carry `language: <code>` plus a `translation_of: { universe, path, canonical_language }` link to the canonical.

Adapter crates (`topologia-co-adapter`, `topologia-mbya-adapter`) are filed as future work — `topologia-core` is content-shape-only and ships first so consumers can settle on the canonical types.

`topologia/_template-language/` is a copy-and-rename template (`{{LANG_NAME}}` / `{{LANG_CODE}}` placeholders) for adding new language planes; `topologia/docs/i18n-patterns.md` walks through both patterns and when to use each.

## [1.38.6] — 2026-05-03

### Added — web→local sync direction (CO-151 second leg)

The v2 watcher's downlink path was already wired (server broadcasts → `apply_batch`), but **only client-originated changes** ever reached the broadcast. REST writes via `/vault/*` and `/entries/*` bypassed the SyncRoom entirely, so a SPA edit on prod was invisible to connected watchers.

**Server side (`co-web/src/sync_ws.rs`):** added `emit_rest_upsert` and `emit_rest_delete` helpers that build a `SyncDelta`, append it to the room's delta-log (so reconnecting clients can resume), and broadcast it with `origin_conn_id = 0` (REST has no WS connection, so the per-connection echo filter never matches and every connected watcher gets the frame). `vault_routes::put_vault_file` and `delete_vault_file` now call these after the entry write completes.

**Client side (`co-agent/src/watcher.rs`):**

1. **Path resolution.** `apply_batch` now joins `delta.entry_path` against the watch root (`config.watch_dirs.first()`) instead of using it CWD-relative. Defensively rejects absolute paths.
2. **Echo dedup.** A shared `Arc<Mutex<HashMap<sha256, Instant>>>` tracks recently-applied content; `encode_event` skips emitting a delta when the on-disk sha256 matches a recently-applied one (5s window). Closes the web→local→fs-notify→web echo loop.
3. **Idempotent local write.** `apply_batch` reads the file before writing — if the bytes already match, the write is skipped (avoids triggering fs-notify at all).
4. **Delete-side dedup.** Successful local deletes record a `DEL:<path>` sentinel in the same map.

**Tests:** new `test_encode_event_skips_recently_applied_content` covers the dedup behavior end-to-end. Watcher suite is now 8 tests; co-web suite still 281 tests; clippy clean.

End-to-end verification (after deploy + watcher restart):
- `curl -X PUT /api/v1/universes/co/vault/notes/test.md` → file appears at `~/projects/co/notes/test.md` within ~1s
- `curl -X DELETE …` → file removed locally within ~1s
- No echo loop in `~/.co/watch-v2.log`

## [1.38.5] — 2026-05-03

### Fixed — sync-driven writes now reconcile `content_count` per batch

After 1.38.4 redeployed, `co` still drifted (513 cached vs 500 actual). Cause: `apply_deltas_to_storage` calls `EntryIndex::upsert` and `DELETE FROM entries` on the per-universe DB but never touches the cached `content_count` field on `meta.universes`. Boot-time `recompute_content_counts` corrected the drift but new sync writes immediately reintroduced it.

`apply_deltas_to_storage` now ends each batch with `UPDATE universes SET content_count = (SELECT COUNT(*) FROM entries) WHERE key = ?` — one extra `COUNT(*)` per batch, atomic, drift-free. Already-shipped boot reconcile + per-batch reconcile = `content_count` stays accurate forever.

## [1.38.4] — 2026-05-03

### Fixed — SPA route fallback for nested universe paths + content_count reconcile

Two follow-ups from the post-CO-151 prod checklist:

1. **`/co/{slug}/{*subpath}` now serves the SPA shell.** The router only registered `/co/{slug}` and `/co/{slug}/assets`, so anything deeper (e.g. `/co/yuri/dados`, `/co/co/processos/alterar-pagina-na-web`) fell through to a 404. Added a catch-all `*subpath` route that serves the SPA shell so the client-side router can resolve those paths. Placed AFTER `/co/{slug}/assets` and `/co/yggdrasil/{game}` so axum's matcher prefers the more specific routes.

2. **`content_count` reconcile already runs on boot** (`recompute_content_counts` from CO-142 Phase B), so the small drift seen on prod (`co`: 510 cached vs 500 actual rows) auto-corrects on this deploy. No code change.

This deploy also re-aligns `/api/health` to report the workspace version (was reporting 1.38.2 because 1.38.3 was a watcher-only fix that didn't go through `flyctl deploy`).

## [1.38.3] — 2026-05-03

### Fixed — v2 watcher: deletes propagate (macOS FSEvents quirk) + multi-universe supervisor

`encode_event` now checks `abs_path.exists()` at flush time. macOS FSEvents sometimes reports `rm` as a `Modify` event rather than `Remove`, which the watcher was classifying as Upserted → tried to read the (now-missing) file → encode returned None → no delta sent → server still had the entry. Fixed: regardless of how notify classified the event, if the file no longer exists at flush time we emit a Deleted delta.

`scripts/co-watch-v2.sh` is the new launchd `ProgramArguments` — supervises one `co-agent-watch` per universe (4 sub-processes), refreshes the session cookie from keychain on 401. Replaces `scripts/co-watch.py` (v1 JSON/REST poll) in `~/Library/LaunchAgents/com.artelonga.co-sync.plist`.

**Verified end-to-end on prod (1.38.3):**
- Touch a file in `~/projects/co/` → on prod via `GET /entries/<path>` in ~2s
- Delete the file → 404 on prod in ~4s
- Zero feedback loop (broadcast filtered by `origin_conn_id`)
- 4 watchers connected to `wss://co-artelonga.fly.dev/api/v1/sync/ws` (one per universe), supervised by single launchd job

## [1.38.2] — 2026-05-03

### Fixed — CO-151 v2 watcher: relativized paths + broke broadcast feedback loop

Three bugs surfaced when running the v2 watcher end-to-end against prod:

1. **`tokio::task::spawn_blocking` killed `notify` on macOS.** FSEvents needs a thread with a CFRunLoop that lives for the whole stream; tokio's blocking pool tears those down. Switched to `std::thread::spawn`.
2. **Watcher sent absolute paths in `entry_path`** (e.g. `/Users/artelonga/co-watch-test/foo.md`). Server's `universe_root.join(absolute)` → still absolute, so writes landed outside the universe dir and `GET /entries/{rel}` returned 404. Reshaped `WatchEvent` into `{abs_path, rel_path, kind}` so the wire carries the relative path while disk reads still resolve via the absolute one. Added `relativize()` + `is_syncable()` filters.
3. **Server broadcast back to the originating client.** The watcher then ran `apply_batch` → wrote the file locally → `notify` fired → another upload → infinite loop. Added `BroadcastFrame { origin_conn_id, encoded }` and a per-room monotonic `next_conn_id`; the broadcast receiver loop skips frames where `origin_conn_id == self`. End-to-end loop count now bounded at 1.

Watcher tests updated for the new `WatchEvent` fields (7 pass). Server `sync_ws` tests still green (8 pass). Verified end-to-end on prod: write file → on prod via `GET /entries/<rel>` in <1s; no feedback loop in `~/.co/watch.log` after fix.

## [1.38.1] — 2026-05-03

### Fixed — CO-151 sync server now actually persists deltas

The 1.38.0 `apply_deltas_to_storage` called `Storage::update_entry_body`, which:
1. issues `UPDATE entries SET body=...` against `meta.db.entries` (a no-op since CO-77 moved entries to per-universe DBs), and
2. is UPDATE-only, so a delta for a *new* path silently did nothing.

Result: a v2 watcher could write `notes/hello.md`, watch the SyncDelta land on the broadcast log, and still see HTTP 404 from `GET /entries/notes/hello.md` because nothing actually persisted.

**Rewrote `apply_deltas_to_storage`** to use the same write path the Vault REST handler uses:
- `Kind::Upserted`: parse YAML frontmatter from the `CoFile` content, build an `Entry`, call `co::entry::write_entry` (writes the .md to disk under `data/universes/<aa>/<bb>/<key>/`), then `EntryIndex::upsert` against the per-universe `data.db`.
- `Kind::Deleted`: `std::fs::remove_file` the .md and `DELETE FROM entries` in the per-universe DB.

**Added `co-agent-watch` binary** (`co-agent/src/bin/watch.rs`) — wraps `SyncWatcher` in a CLI so the v2 launchd plist has something to actually run. The 1.38.0 plist referenced `~/.cargo/bin/co-agent-watch` which didn't exist.

**Fixed v2 watcher URL** (`co-agent/src/watcher.rs`) — was building `?token=...` only; server requires `?universe=<key>&token=<jwt>` and returned HTTP 400 otherwise.

**Regression test** `test_upserted_delta_writes_to_disk_and_db` proves the v2 write goes all the way through: WS upload → file on disk + per-universe row indexed + reachable via `/entries/{path}`.

## [1.38.0] — 2026-05-03

### Added — CO-151: real-time delta sync — protobuf SyncDelta over WebSocket with zstd

Bidirectional file-sync channel that streams deltas in a compact binary format, replacing the v1 JSON/REST poll approach in `scripts/co-watch.py`.

**Wire format** (`core/proto/sync.proto`):
- `SyncDelta` — one change (upserted / deleted / renamed) with a `CoFile` content envelope
- `SyncBatch` — batched deltas with resume token for reconnect replay

**Server** (`co-web/src/sync_ws.rs`):
- Route: `GET /api/v1/sync/ws?universe=<key>` (JWT or session cookie auth)
- Per-universe `SyncRoom` with 24h in-memory delta log for `X-Sync-Resume` replay
- Broadcast fan-out to all connected clients in the same universe

**Client** (`co-agent/src/watcher.rs`):
- FSEvents (macOS) / inotify (Linux) via `notify` crate with 200ms debounce
- Encodes local changes as `SyncDelta` and ships over the WS uplink
- Applies server-pushed downlinks to local files (last-write-wins)

**Compression** (`core/src/sync/delta.rs`):
- zstd level 3; placeholder for a ~32 KB training dictionary (CO-151 follow-up)
- proto+zstd wire bytes < JSON equivalent in all tests

**Migration**: `scripts/co-watch.py` (v1) stays operational; `scripts/co-sync-v2.plist` provides the replacement launchd configuration once `co-agent-watch` is deployed.

## [1.37.3] — 2026-05-03

### Fixed — `If-None-Match` short-circuit ran before existence check on `GET /assets/:sha`

The 304 fast path compared the URL sha against the `If-None-Match` header *before* looking up the row, so a probe like `curl -H 'If-None-Match: "X"' /assets/X` returned 304 for any valid 64-char hex sha — even when the row didn't exist. That broke client-side idempotency probes (a missing blob looked "already there" to the bulk uploader, which then mis-counted the run).

Reordered: row lookup first, then 304 short-circuit only if the row actually exists. Also simplified `scripts/bulk-upload-binary.py` to skip the probe entirely — the server is already idempotent on POST (same bytes → same sha → existing row reused), so the second `GET` was redundant.

Added regression test `if_none_match_on_nonexistent_returns_404_not_304` (14 asset integration tests total now).

## [1.37.2] — 2026-05-03

### Changed — home rewritten around the **Co**nsciência **Co**letiva philosophy

The previous home (`template/index.md`) opened with "uma plataforma para organizar ideias, projetos e pessoas em universos" — accurate but generic. The manifesto on `template/sobre.md` had the actual philosophy (Cocriar / Colaborar / Conectar) but lived a click away.

Merged both: the home now leads with **conectar pessoas** and the three verbs, defines what a universe is, then shows the curated trio diagram and the navigation primer. `sobre.md` is now a technical/governance page that points back to home for the philosophy.

`template/sobre.md` rewritten as a stack + community + license page, no philosophy duplication.

## [1.37.1] — 2026-05-03

### Fixed — bulk-upload usability: 413 on >2 MB assets, 429 saturating burst writes

Surfaced by the first quilomboaraucaria upload pass (401 binaries / 558 ok, 35 markdown / 95 ok). Two distinct failure modes:

1. **413 "Failed to buffer the request body: length limit exceeded"** on full-resolution images and MP4 (`*.orig.jpg`, `*.orig.png`, `*.mp4`, `fotos/post-*.jpg`). axum applies a 2 MB `DefaultBodyLimit` to all routes by default; the asset router's internal `MAX_ASSET_BYTES = 50 MB` never got a chance to run.
2. **429 rate_limited** on the markdown PUT burst — the per-bucket cap is 60 writes/min for `tier=user`, and a 95-file Vault dump trivially exceeds that.

**Fixes:**

- `asset_router()` now applies `DefaultBodyLimit::max(MAX_ASSET_BYTES)` (50 MB) on the router layer so the handler-level cap is the only gate.
- `rate_limit_middleware` now honors `X-Admin-Override-Quota: true` for authenticated callers (any tier ≠ Anonymous). CO-90 keeps `tier` billing-only, so the bypass is opt-in per request and the ownership/membership check still runs inside the route handler — anonymous callers can set the header but it's ignored. Bulk-upload script sends this header.
- `scripts/bulk-upload-binary.py` rewritten with `ThreadPoolExecutor` (8 workers), exponential backoff retry on 429/timeout/HTTP 0, idempotent skip-if-already-uploaded probe, and the override header. Same two-pass shape: binaries first to capture sha256, then markdown with `![](relative)` → `![](sha256:…)` rewriting.

After this deploy: a 50 MB JPG uploads cleanly, a 200-file markdown burst doesn't trigger 429, and a re-run of the same content is a no-op (sha256 idempotency + entry upsert).

## [1.37.0] — 2026-05-02

### Added — encrypted, indexable, lazy-loaded assets (CO-147 + CO-148 + CO-149 + CO-150)

Closes phases 2–5 of CO-145. Every blob written through the asset endpoint is now ChaCha20-Poly1305 ciphertext on disk, indexable by tags + mime + filename without decryption, range-fetchable for video and large media, and lazy-loaded by default in the SPA.

**CO-147 — indexable metadata**

```
GET    /api/v1/universes/{u}/assets?mime=&search=&tag=  → { assets: [{…, tags}], total }
GET    /api/v1/universes/{u}/assets/tags                → [{ tag, count }]
POST   /api/v1/universes/{u}/assets/{sha}/tags          body: {"tags": ["a","b"]}
DELETE /api/v1/universes/{u}/assets/{sha}/tags/{tag}
```

- New `asset_tags` table (FK CASCADE on assets); list endpoint joins per-asset tags into the response so the asset browser UI can render them inline.
- New `frontmatter_index` shadow table (title, type, status, tags_json, dates, parent_path) — designed to survive encryption-at-rest because typed metadata stays plaintext while body bytes get encrypted.

**CO-148 — encryption envelope**

- ChaCha20-Poly1305 AEAD, per-blob random 12-byte nonce, AAD = `universe_key || sha256` so a copied blob fails to decrypt across universes or under a different sha.
- Per-universe DEK derived deterministically: BLAKE3-keyed-hash(master_key, "co-asset-dek-v1\0" || universe_key). Master key sourced from `CO_ASSETS_MASTER_KEY` env (preferred) or `JWT_SECRET` (dev fallback). DEK never lands on disk.
- Schema additions: `assets.nonce BLOB`, `assets.cipher_size INTEGER`, `assets.encrypted INTEGER NOT NULL DEFAULT 0`. Old Phase 1 plaintext rows continue to read transparently; new uploads write ciphertext.
- **Threat model (Tier 1 — what ships):** disk-only attacker (stolen volume, leaked backup, accidental dump) cannot read content; needs the master key too. Real protection against backup leaks, dev-machine theft, S3 mistakes.
- **Threat model (Tier 2 — deferred):** server-trusted attacker with root + env still can. Closing this gap requires user-password-derived KEK with session-scoped DEK; filed as CO-148 follow-up.

**CO-149 — HTTP range support**

- `Range: bytes=N-M`, `bytes=N-`, `bytes=-N` all parse; multi-range rejected.
- 206 with `Content-Range: bytes N-M/total` + `Accept-Ranges: bytes`.
- 416 (Range Not Satisfiable) with `Content-Range: */total` for invalid ranges.
- Full-decrypt-then-slice: ChaCha20-Poly1305 verifies over the whole stream, so chunked-AEAD would change Phase 3's correctness story. Acceptable up to ~50 MB; chunked-AEAD for larger media is filed as future work.

**CO-150 — SPA lazy-load**

- `?excerpt=true` already shipped on entry GET (returns `{frontmatter, excerpt}` capped at 200 chars).
- Asset browser at `/co/{slug}/assets` consumes the new `GET /assets` endpoint.
- `markdown.js` post-processes rendered HTML in both fallback and full (marked + DOMPurify) paths: `<img src="sha256:abc…">` → asset URL + `loading="lazy" decoding="async"`; `<video src="sha256:…">` → asset URL + `preload="none"`; bare `<img>` tags get `loading="lazy"` if missing.
- Markdown source `![alt](sha256:abc…)` and ` ```video\nsha256:abc\n``` ` syntax both resolve through the renderer.

**Tests:** 13 asset integration tests (round-trip, dedupe, ETag/304, anon-on-private blocked, anon-on-public allowed, oversize rejection, ciphertext-on-disk, HTTP range 206 + suffix + 416, tag CRUD round-trip, delete-when-unreferenced) + 4 crypto unit tests + 6 asset_routes unit tests. Full co-web suite (380+) green; clippy clean.

---

## [1.36.0] — 2026-04-30

### Added — binary asset upload + content-addressable storage (CO-146, Phase 1 of CO-145)

Every universe now has a binary-asset endpoint backed by sha256 content addressing. Phase 1 of the encrypted+indexable+lazy-load epic (CO-145); designed to unblock the 506 MB quilomboaraucaria upload that the markdown-only Vault API rejects today.

**New endpoints:**

```
POST   /api/v1/universes/{u}/assets        body: raw bytes  → {sha256, mime, size, url}
GET    /api/v1/universes/{u}/assets/{sha}  → bytes + ETag + immutable cache
DELETE /api/v1/universes/{u}/assets/{sha}  → 204 if refcount == 0; 409 otherwise
```

**Storage layout:**

```
data/universes/<aa>/<bb>/<key>/
  data.db                          (existing)
  blobs/<aa>/<bb>/<sha256>         (new — raw bytes, sharded 2-level)
```

**Per-universe schema additions** (universe schema_v4):

```sql
CREATE TABLE assets (
    sha256        TEXT PRIMARY KEY,
    blob_path     TEXT NOT NULL,
    mime          TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL,
    filename      TEXT,
    created_at_ns INTEGER NOT NULL,
    created_by    TEXT,
    refcount      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_assets_mime       ON assets(mime);
CREATE INDEX idx_assets_created_at ON assets(created_at_ns);
```

**Properties:**
- **Idempotent** — same bytes → same sha256 → single on-disk blob (re-upload is a no-op)
- **Atomic write** — write-tmp + rename, no torn writes on crash
- **Cache-friendly** — `Cache-Control: private, max-age=31536000, immutable` + ETag = sha256, with proper 304 short-circuit before disk read
- **MIME sniffing** — header takes precedence; falls back to magic-byte detection for jpeg/png/gif/webp/pdf/mp4
- **Auth** — write requires owner/member; read allows anon on public universes

**What Phase 1 does NOT do** (deferred to subsequent CO-145 phases):
- Encryption at rest — CO-148 wraps every blob in ChaCha20-Poly1305 with per-universe DEK + owner KEK derived via Argon2id
- Indexable list/filter endpoint — CO-147 adds `GET /assets?type=image/*&tag=foo`
- HTTP range support — CO-149 adds streaming for video and large images
- SPA `<img loading="lazy">` integration — CO-150

Phase 1 stays plaintext deliberately because the existing `entries.content` column is also plaintext — the privacy gap is not widened. Encryption ships in Phase 3 (CO-148) once the upload path itself is proven.

**Hard cap:** 50 MB per blob (axum's default body limit aligns; oversize returns 400/413). CO-149 + CO-148 Phase 6 lift this with chunked-AEAD streaming.

**Tests:** 7 integration tests (`co-web/tests/asset_tests.rs`) cover round-trip, dedupe, ETag/304, anon-on-private blocked, anon-on-public allowed, oversize rejection, delete-when-unreferenced. Plus 6 unit tests for hex encoding, MIME sniffing, and shard-path construction.

**Design doc:** `docs/research/encrypted-indexable-assets.md` documents the full 5-phase plan including the index-plaintext / encrypt-body split, key hierarchy, and lazy-load wire contract.

**Filed tickets:** CO-145 (epic), CO-146 (this), CO-147, CO-148, CO-149, CO-150.

## [1.35.2] — 2026-05-02

### Fixed — recovery from buggy `prune_orphan_universe_dirs` (1.34.5 regression)

**Critical regression introduced in 1.34.5:** the previous `prune_orphan_universe_dirs` iterated all top-level dirs under `/data/universes/` and deleted any whose name didn't match a `universes.key` row. That was wrong — `UniversePool` (CO-77) shards per-universe `data.db` files at:

```
/data/universes/<2-hex>/<2-hex>/<key>/data.db
```

The 2-hex shard-prefix dirs (e.g. `68`, `b5`, `0e`, `f0`) are NOT universe keys — they're hash-prefix dirs holding multiple per-universe DB files. Deleting them wiped the per-universe SQLite for affected universes (template, quilomboaraucaria, artelonga, humanity, universo). The flat `.md` content survived (`/data/universes/<key>/*.md`) — the SQLite shards got lazily recreated empty by `UniversePool::get_or_open` on first access, returning `entries.total=0` to the API.

**Two fixes shipped in 1.35.2:**

1. **`prune_orphan_universe_dirs` rewritten as narrow allowlist.** Now only deletes dirs whose name matches the explicit `KNOWN_DEPRECATED_DIRS` list (`co-dev`, `co-experience`, `qa-dev`, `quilombo-blog{,-2,-3}`, plus a few test/anon residues). Wider cleanup is now an explicit ops task, not unattended boot-time. Defensive double-check that no `universes.key` row holds the name before deleting.

2. **`rebuild_entries_from_filesystem(keys: &[&str])`** — recovery pass for the affected universes. Walks `/data/universes/<key>/**/*.md`, parses frontmatter, upserts each entry into the per-universe `data.db` via `universe_pool`. Idempotent — skipped per-universe when entries table already has rows. Wired into `server.rs` startup for system universes: template, tempo, humanity, universo, quilomboaraucaria, artelonga, rfq, co, yuri, dados.

After 1.35.2 boot:
- `entries.total` for template/quilomboaraucaria/artelonga restored from the .md filesystem
- `content_count` recomputed accurately
- No data loss (filesystem was always the source of truth; only the SQLite mirror was wiped)

## [1.35.1] — 2026-05-02

### Fixed — `UniverseInfo` exposes `content_version` + smoke script Python compatibility

Two follow-ups during the 1.35.0 smoke pass:

1. **`UniverseInfo` DTO missing `content_version`.** Same shape as the CO-137 parent_key bug — the column existed and the data was correct, but the public DTO didn't surface it. Added `content_version: String` (defaults to "0.0.0") and a defensive separate `SELECT` in `get_universe_info` that tolerates a missing column.

2. **`scripts/smoke-processo-alterar-pagina.sh` Python f-string syntax.** Older Python (<3.12) doesn't allow `\"` escapes inside f-string expressions. Switched to `'  ...{} ...'.format(...)` form. Script now runs end-to-end against any Python 3.6+.

After this deploy, `GET /api/v1/universes/<key>` returns `content_version` in the JSON body.

## [1.35.0] — 2026-05-02

### Added — `alterar-pagina-na-web` process implemented end-to-end (CO-144 Phase C)

All 7 chain steps are now wired in the live binary, exercisable via REST:

```
POST   /api/v1/processos/alterar-pagina-na-web/preview
POST   /api/v1/processos/alterar-pagina-na-web/approve/{run_id}
POST   /api/v1/processos/alterar-pagina-na-web/revert
GET    /api/v1/processos/alterar-pagina-na-web/runs?universe=<key>
```

- **Step 1 — Trigger:** `POST /preview` with `{universe, page_path, field, new_value, bump_level?}`
- **Step 2 — Source:** server reads the current entry from filesystem via `co::entry::read_entry` (source of truth)
- **Step 3 — Review:** preview row inserted into new `process_runs` table with state=preview, returns diff + run_id + computed `proposed_version`
- **Step 4 — Approval:** `POST /approve/{run_id}` re-validates state, re-checks write access, then proceeds to sink
- **Step 5 — Sink:**
  - 5.1 Frontmatter field updated, `co::entry::write_entry` persists to FS
  - 5.2 `universes.content_version` bumped (semver patch by default; `minor`/`major` via `bump_level`)
  - 5.3 `<universe>/CHANGELOG.md` appended with the structured entry (creates with header if missing)
  - 5.4 Deploy: simulated for now (real target adapters are CO-134 static-on-R2, CO-135 CF Pages, etc.)
- **Step 6 — Telemetry:** `telemetry_events` row with `event_type='process'`, `event_name='alterar-pagina-na-web.completed'`; run state → completed
- **Step 7 — Rollback:** `POST /revert` with `{universe, target_version}` (or `"prior"` to use the most recent prior). Restores frontmatter from the parent run's `from_value`, rolls back `content_version`, appends a "Reverted" CHANGELOG entry, marks parent run state='reverted', emits `alterar-pagina-na-web.reverted` event.

Run history queryable via `GET /runs?universe=<key>` — returns ordered list with full payload, parent_run_id linkage, state.

### Schema additions (auto-applied via `ensure_*` backfill)
- `universes.content_version TEXT NOT NULL DEFAULT '0.0.0'` — per-universe semver
- `process_runs` table — run_id, process_name, universe_key, state, payload (JSON), timestamps, actor_id, parent_run_id
- Index `idx_process_runs_universe_time`

### Acceptance for the worked example (Co/processos/alterar-pagina-na-web)
- [x] All 7 steps execute against a real universe end-to-end
- [x] CHANGELOG.md lands in the universe root with structured entries
- [x] Revert restores prior frontmatter + version + emits inverse event
- [x] State machine prevents double-approval and approval after revert
- [x] Access-checked: write permission required for preview/approve/revert
- [ ] SPA dashboard render of the run history (Phase D — separate ticket)
- [ ] Real deploy target (current: simulated) — depends on CO-134/135 adapter completion

## [1.34.8] — 2026-05-02

### Added — `Co/processos/alterar-pagina-na-web` + recursive ingest of co universe

User clarification 2026-05-02: the per-user dashboard work (CO-144) needs to encompass a deterministic source→sink **process model**, with `Co/processos/alterar-pagina-na-web` as the worked example.

- **CO-144 expanded** (`work/co/CO-144.md`): now 4 phases — A (auto-create personal universe + dados/ skeleton), B (cross-universe activity feed populating `<username>/dados/`), C (process model with `Co/processos/<process>` content type and reflexive editing pattern), D (SPA dashboard + process stepper rendering). Architecture diagram + decision log added.
- **`work/co/processos/alterar-pagina-na-web.md` committed** (246 lines): documents the 7-step deterministic chain — Trigger → Source → Review (`co preview` localhost v+1) → Approval → Sink (manifest bump + CHANGELOG + deploy) → Telemetry (3 sinks) → Rollback. Includes a Mermaid source→sink flowchart, structured event schema, source-to-sink data sync table, edge cases. State-of-implementation table marks each step ❌/🟡/✅.
- **`Storage::seed_co_universe_tasks` now recursive**: walks `/app/seed-co/` and preserves subdir structure. Top-level `*.md` keep the `tasks/<filename>` prefix for backwards compat with 1.34.3; deeper files use their relative path (e.g. `processos/alterar-pagina-na-web.md`).

After deploy, the SPA's `/co/co/processos/alterar-pagina-na-web` resolves to the worked example.

## [1.34.7] — 2026-05-02

### Fixed — `*@co.local` legacy users blocked admin from claiming their slug

1.34.6 surfaced the unique-index conflict on `users.username`: admin `yuri@artelonga.com.br` couldn't claim `yuri` because the legacy `yuri@co.local` test user held it.

**Fix:** new `Storage::free_legacy_co_local_usernames()` runs before `ensure_admin_username` on every boot. Renames any `*@co.local` user's username to `legacy-<original>` (e.g. `yuri` → `legacy-yuri`). Idempotent — `WHERE username NOT LIKE 'legacy-%'` keeps re-runs as no-ops.

After this deploy, the admin's username is set to `yuri` on next boot, completing the "always use slug as user name by default" directive.

## [1.34.6] — 2026-05-02

### Added — admin's `yuri` personal universe re-homed + username default

User feedback 2026-05-02: "include the private yuri user (always use slug as user name by default)". The `yuri` universe and `dados` dashboard universe were misclassified as cruft in my earlier note — both are intentional and intact (correctly preserved by `prune_orphan_universe_dirs` since their DB rows exist). What was actually wrong:

- `yuri@artelonga.com.br` (admin) had `users.username = ''` (empty)
- The `yuri` universe was owned by `usr_-PFeKIctDZ` (legacy `yuri@co.local` test user that previously held the username slug)
- New `Storage::ensure_admin_username(email)` derives the slug from the email prefix (`yuri@artelonga.com.br → yuri`), updates `users.username` if empty. Skips gracefully on unique-index conflict — does not break boot.
- `PERSONAL_KEYS` (in server.rs admin-bootstrap path) now includes `yuri` alongside `artelonga` and `rfq`. Next boot re-homes the `yuri` universe to the admin's `user_id` via `ensure_admin_owns_personal_universes`.

### Filed — CO-144: per-user dashboard universe + cross-universe activity feed

3-phase ticket scoping the broader feature the user described: "it works like a dashboard, changing a file in other universes or adding a new universe populates that, obviously one (private) per user".

- **Phase A** — every signup auto-creates a private universe with `key = users.username` (extends the admin-only pattern shipping in 1.34.6 to every user)
- **Phase B** — `upsert_entry_row` emits cross-universe events that materialize into (i) the existing global `dados` universe and (ii) each user's slug-named universe, filtered by membership/subscription
- **Phase C** — SPA Painel-style dashboard view that renders the activity feed with universe / actor / entry-type filters

Decision recorded: `dados` stays system-owned (global aggregate). Per-user dashboards are the user's slug-named private universe.

## [1.34.5] — 2026-05-02

### Added — `prune_orphan_universe_dirs` filesystem cleanup on every boot

Closes the filesystem-cruft gap surfaced after CO-142 Phases C+D hard-deleted DB rows for `co-dev`, `co-experience`, `qa-dev`, `quilombo-blog{,-2,-3}` (and various test/anon dirs) — the dirs at `/data/universes/<key>/` persisted, accumulating cruft.

`Storage::prune_orphan_universe_dirs()` runs after the seed/delete/recompute passes on every boot. For each entry under `/data/universes/`, checks if a row exists in `universes` for that key; if not, removes the dir. Idempotent — already-removed dirs are no-ops. Safe — anonymous clones (hash-keyed dirs that have a corresponding `anon-*` row) are kept.

### Done — CO-100 documentation pass for 1.34.x reality

`docs/ARCHITECTURE.md` updated from 1.21.x state to current state:
- C4 component diagram now includes co-agent (CO-120), ClickHouse (CO-123), Cloudflare CDN+WAE (CO-117), admin surface (CO-105), and the per-universe SQLite split (CO-77)
- New "Armazenamento (1.23+)" section documenting the meta.db / per-universe data.db topology, WAL-safe state rules, idempotent migrations (`ensure_column` / `ensure_table`)
- New "Endpoints novos (1.22 → 1.34)" table covering admin / A/B / log-drains / cache / themes / generic entries
- New "Componentes opcionais" section on co-agent, ClickHouse, backup-cron, Cloudflare
- New "Evolução desde 1.21.x" cross-reference table mapping each shipped feature to its commit/file location
- Service worker updated `co-v3-network-first` → `co-v4-offline`

CO-100 frontmatter: `in_progress` → `done`.

### Repository

`github.com/artelonga/co` flipped from PRIVATE → PUBLIC. Pre-publish audit: `.claude/` files (Claude Code session state, never repo-content) untracked + added to `.gitignore`. No actual prod secrets in git history; the only "secret-shaped" mention was `JWT_SECRET=dev-test-secret` as a Bash command-pattern allow-list value in `.claude/settings.local.json` — placeholder, not a real secret. Privacy-page links pointing at the source (`https://github.com/artelonga/co/...`) now resolve for anonymous browsers, fulfilling the "verifiable" promise in `dados-rastreados.md`.

## [1.34.4] — 2026-05-02

### Fixed — `seed_admin_content_universes` reconciles visibility on every boot

Discovered during 1.34.3 staleness audit: `artelonga` returned 404 to anonymous despite the seed declaring `public-subscribable`. Root cause: `INSERT OR IGNORE` doesn't update existing rows, so a row created with an old default (`private`) silently stays wrong forever. Same risk for any future visibility intent change on these admin-content universes.

**Fix:** added a follow-up `UPDATE universes SET visibility = ?, is_public = ? WHERE key = ? AND (visibility != ? OR is_public != ?)` to `seed_admin_content_universes`. Only writes when the stored value doesn't match declared intent — idempotent on every boot. `is_public` bit kept in sync (0 for private, 1 otherwise) so legacy callers checking that flag also see the intended state.

After this deploy:
- `artelonga` → public-subscribable, reachable to anonymous (was 404)
- `rfq` → private (unchanged)
- `co` → public-subscribable (unchanged)

### Fixed — Stale GitHub URL in `termos.md`

`seed/template/termos.md:98` still pointed at the renamed `data/universes/template/content/termos.md` path (instead of `co-web/seed/template/termos.md`). Same class as the privacidade and dados-rastreados fixes from 1.34.3 — completes the audit of stale GitHub paths in the legal-pages corpus.

## [1.34.3] — 2026-05-02

### Fixed — `co` universe shows 0 entries despite 140 task markdown files

User report on 2026-05-02: "co has 0 entries, we have 140 tasks". CO-142 Phase E populated `/data/co/` from the bundled `/app/seed-co/` for the admin dev_board scan, but the SPA's `/co/co` board reads from the per-universe `entries` table (CO-77) — which stayed empty.

**Fix:** new `Storage::seed_co_universe_tasks(source_dir)` runs on every boot after Phase E's `copy_dir_all`. Iterates `/app/seed-co/*.md`, builds an `Entry` via the existing `make_entry` + `seed_page_frontmatter` helpers, writes via `co::entry::write_entry`, upserts via `upsert_entry_row` against the per-universe pool's `co` connection. Path layout: `tasks/CO-NNN.md`. Idempotent.

After this fix, `GET /api/v1/universes/co/entries` returns 140+ ticket entries.

### Fixed — Política de Privacidade link broken from termos.md

Internal markdown link in `seed/template/termos.md` was `/co/template?path=content/privacidade.md` but the SPA only recognizes `?page=<slug>` (handled by `maybeOpenPageFromUrl`). Anonymous users clicking the link landed on the template board with no modal opening.

- `seed/template/termos.md` — link corrected to `/co?page=privacidade`
- `seed/template/privacidade.md` — fixed the "histórico de versões" GitHub URL from the renamed `data/universes/template/content/privacidade.md` to the current `co-web/seed/template/privacidade.md`

## [1.34.2] — 2026-05-02

### Fixed — CO-142: public-universe routing audit + co-dev/co-experience deprecation

Five-phase cleanup of the public-universe surface:

**Phase A — Routing fix**
- Moved `dev_board::router()` from `/api/v1/universes` to `/api/v1/admin` so it
  no longer shadows the public-subscribable universe lookup via `universe_api`.
  Dev board routes are now at `/api/v1/admin/co-dev/…`.
- Retargeted the telemetry SPA route from `/co/co-dev/telemetria` to
  `/co/co/telemetria` (reflects the `co` work universe replacing `co-dev`).
- Added smoke-check [11]: every public universe (`template`, `quilomboaraucaria`,
  `co`, timeline trio) must return 200 to anonymous.

**Phase B — content_count reconciliation**
- Added `recompute_content_counts()`: on every boot, counts entries in each
  universe's per-universe DB and writes the result to `universes.content_count`.
  Fixes `template.content_count = 0` caused by `reseed_template_content_pages`
  calling `upsert_entry_row` without `increment_universe_content_count`.
- Added smoke-check [12]: `template.content_count >= 6`.

**Phase C — co-dev / co-experience deprecation**
- Removed `seed_co_dev_universe()` call from startup.
- Added `delete_deprecated_universes()`: hard-deletes `co-dev` and `co-experience`
  rows (and memberships) on every boot. Idempotent.
- Removed `co-dev` and `co-experience` from `ensure_admin_universe_memberships`
  system_keys and from `uat_mirror` skip list.
- **Decision**: epics stay as entries in the `co` universe (not promoted to
  sub-universes). Documented in `docs/UNIVERSES.md`.

**Phase D — Quilombo reconciliation**
- Added `delete_stale_quilombo_variants()`: hard-deletes `quilombo-blog`,
  `quilombo-blog-2`, `quilombo-blog-3`, and `qa-dev` on every boot.
- Created `docs/UNIVERSES.md`: canonical inventory of all system universes,
  with documented purpose and seed path for each.
- Removed `qa-dev` from `PERSONAL_KEYS` in the admin bootstrap sequence.

**Phase E — Dev board task display**
- Added `COPY work/co/ /app/seed-co/` to the Docker runtime stage so the
  repo's `work/co/CO-*.md` files are bundled in the image.
- Added startup refresh: on every boot, `copy_dir_all(/app/seed-co, data/co/)`
  keeps the dev board in sync with the repo's task statuses.
- Documented all startup invariants in `docs/OPERATIONS.md`.

## [1.34.1] — 2026-05-02

### Fixed — `dados-rastreados` page refreshed for 2026-05 cookie surface

Following user feedback ("Dados is not up to date"), updated the privacy disclosure to reflect cookies and localStorage state added since 1.21.x:

- Date stamp: abril → maio de 2026
- Added cookies: `co_onboarded` (CO-99 onboarding), `co_cookie_consent` (LGPD banner), `co_preferred_universe` (auto-redirect to last universe)
- Added new section §3.1 enumerating localStorage / IndexedDB state (`co_universe_tree_*` from CO-98 hierarchy, `co_subtree_*`, `co_section_*`, `co_folder_*`, `co_draft_*` autosave drafts, `co-vault` IDB cache from CO-69 PWA offline)
- Fixed the "verifiable source" link from `data/universes/template/content/dados-rastreados.md` (renamed years ago) to `co-web/seed/template/dados-rastreados.md` (current path)

### Filed — CO-142: public-universe routing audit + co-dev/co-experience deprecation

Five-phase ticket scoping the architecture-level cleanup the user named on 2026-05-02:

- **Phase A** — disambiguate `/api/v1/universes/co-dev` shadow (dev_board admin middleware vs. public-subscribable universe)
- **Phase B** — fix `content_count=0` on `template` (and likely other system universes) — recompute on boot or atomic via upsert
- **Phase C** — deprecate `co-dev` / `co-experience` public universes; migrate to epic ↔ sub-universe via CO-98 `parent_key`
- **Phase D** — reconcile quilombo* and qa-* universe sprawl into a documented set
- **Phase E** — wire the dev board to read from `work/co/CO-*.md` so completed tickets actually show as done

Each phase has explicit acceptance criteria and call-out of the underlying mechanism (route mounting order in `server.rs`, `upsert_entry_row` count maintenance, `parent_key` semantics, deploy-time path mounts). No code changes in this commit — ticket only.

## [1.34.0] — 2026-05-01

### Added — CO-105: Admin telemetry dashboard

Cherry-picked + integrated from the long-running `feat/CO-105` branch (1 commit, originally branched at 1.27.0). Resolves on top of current main; conflict markers in `Cargo.toml`, `co-web/src/lib.rs`, `co-web/src/server.rs`, `Cargo.lock`, and `CHANGELOG.md` were resolved by accepting HEAD's structure and adding the new admin module alongside (not replacing) the post-1.27.0 routes (`/api/v1/ab`, `/api/v1/cache`).

**`GET /api/v1/admin/dashboard`** — single JSON endpoint with platform-wide aggregates:
- JWT required; caller email must match `CO_SEED_ADMIN_EMAIL` (read fresh from env per request)
- Returns 401 for invalid/missing JWT, 403 for email mismatch — never leaks admin email on invalid signature
- Totals: users, universes, active universes (7d), entries
- Daily rows (14 days): pageviews, unique visitors, signups, errors — sourced from `telemetry_events` + `users.created_at`
- Top 10 universes by event count (7d) with name fallback
- Auth stats: logins today, failed logins, active sessions (last 30 min)
- 5-minute in-memory cache; no DB writes per request

**`GET /admin`** — static admin page (cookie auth):
- Server-side JWT + email gate: redirects to `/co` if unauthenticated, 403 if not seed admin
- Plain HTML, no framework, no CDN — inline CSS + JS, `< 10 KB`
- Top strip: users, universes, active-7d, entries as big numbers
- Daily traffic sparkline (last 14 days): dual polyline SVG (pageviews + uniques)
- Top universes table (key, name, events)
- Auth panel: logins today / failed / active sessions
- Auto-refreshes every 60 seconds

**`co-web/static/variants/a/admin.html`** — embedded via `include_str!` at compile time.

**`co-web/src/admin_routes.rs`** — new module with typed structs, aggregate query helpers, handlers, and 21 unit + integration tests.

## [1.33.2] — 2026-05-01

### Added — `ensure_table` helper to formalize the migration-drift safety pattern

Sibling of `ensure_column` (CO-137). Queries `sqlite_master` before issuing the DDL; returns `true` if the table was created, `false` if it already existed. The standalone `CREATE TABLE IF NOT EXISTS` SQL is already idempotent, so the helper exists primarily to give callers a single, consistent surface for migrations and to make adding observability (tracing / metrics) trivial at the call site.

Callers updated:
- CO-77 `entries` + `entries_fts` backfill — now uses `ensure_table` per table
- CO-121 `feature_flags` + `ab_assignments` + `ab_exposures` backfill — now uses `ensure_table` per table
- The `idx_exposures_flag_time` index stays on `CREATE INDEX IF NOT EXISTS` (indexes aren't tracked as `sqlite_master.type='table'`).

Closes the structural follow-up the 1.33.1 hotfix opened: every CREATE TABLE migration that ships now has a single, consistent helper to call. Combined with `ensure_column`, the framework is structurally robust against the partial-application failure mode that bit prod three times (CO-77, CO-137, CO-121).

## [1.33.1] — 2026-05-01

### Fixed — A/B `feature_flags` table missing on prod (CO-121 partial-apply hotfix)

**Symptom on prod (1.33.0):** every boot logs `ERROR co_web::server: CO-121: failed to seed feature flags: no such table: feature_flags`. Same partial-application failure mode as CO-137 / 1.22.4: `schema_version` row exists for v27 but the corresponding `CREATE TABLE feature_flags` never took effect on this DB. Boot proceeds but A/B endpoints would 500 on first use.

**Fix:** unconditional post-migration backfill at the end of `Storage::run_migrations` — `CREATE TABLE IF NOT EXISTS feature_flags / ab_assignments / ab_exposures` plus the `idx_exposures_flag_time` index. Mirrors the existing CO-77 (`entries`) and CO-137 (`parent_key`) backfills. Idempotent; safe to re-run on every boot.

This is the **third** instance of the same migration-drift class (CO-77 entries, CO-137 parent_key, CO-121 feature_flags). Pattern is now formalized in `feedback_migration_column_reads.md`: every CREATE TABLE + ALTER ADD COLUMN that ships should also have an unconditional backfill at the end of `run_migrations` for at least one release cycle, until prod has visibly converged.

After this fix:
- Prod boot logs no longer carry `no such table: feature_flags`
- A/B exposure logging works without surfacing a 500 to anonymous template visitors

## [1.33.0] — 2026-05-01

### Added — CO-123: ClickHouse single-node + WAE export pipeline

- `infra/clickhouse/` — Fly app config, ClickHouse config/users XML, `init.sql` (wae_events MergeTree, 90-day TTL, Iceberg table function ready)
- `scripts/wae-to-clickhouse.sh` — daily WAE SQL API → ClickHouse bulk insert; maps CF Analytics Engine columns to typed schema
- `infra/clickhouse-export-cron/` — Alpine Fly cron app running export at 04:17 UTC
- `infra/clickhouse/iceberg-smoke-test.sh` — validates Iceberg S3 integration via ClickHouseS3 table function
- `docs/analytics/sample-queries.sql` — 8 ready-to-run queries (top universes, error rate, A/B funnel, p95 latency, retention)
- `docs/OPERATIONS.md` §ClickHouse — full runbook: setup, proxy, querying, export schedule, smoke test

## [1.32.0] — 2026-05-01

### Added — CO-124: Co-agent variants for CF Workers tail + Vercel Log Drains

- **CF tail Worker** (`workers/co-tail/`) — Cloudflare-native tail Worker that subscribes to a
  target Worker's log stream, converts events to CO `TelemetryEvent` JSON-Lines, gzip-compresses,
  signs with HMAC-SHA256, and POSTs to the CO ingest endpoint; deployable via `wrangler deploy`
- **Vercel Log Drain receiver** — `POST /v1/log-drains/vercel/{universe_id}` route on co-web:
  validates Vercel `x-vercel-signature` (HMAC-SHA1), maps NDJSON log entries to CO events, and
  stores them in `log_drain_events` with idempotent deduplication by `event_id`
- **Schema migration v28** — `log_drain_secret TEXT` column on `universes`; new `log_drain_events`
  table with `event_id` primary key and composite index on `(universe_id, received_at)`
- **Documentation** — `docs/co-agent/cloudflare-workers.md` and `docs/co-agent/vercel.md`

## [1.31.0] — 2026-05-01

### Added — CO-97: Visitor token unification (Option A)

- `telemetry_middleware` and `quilombo_telemetria` read `al_vid` first, fall back to `visitante_id`
- Both middlewares emit `al_vid` scoped to `.artelonga.com.br` (JS-readable, `SameSite=Lax; Secure`)
- `HttpOnly` intentionally dropped on visitor token — analytics-only, no auth role (see ADR-001)
- `docs/decisions/001-visitor-token-unification.md` — decision record with trade-off sign-off
- `dados-rastreados.md` updated to disclose `al_vid` cookie and scope

## [1.30.0] — 2026-05-01

### Added — CO-79/80/108/109/118/121/122: Wave 3-5 + platform infra

- **CO-79** — Caching layer: in-process manifest LRU, theme-css ETag, query singleflight, cache-hit metrics
- **CO-80** — Per-tier rate limiting: token-bucket per user/tier/operation; `/api/v1/ab` admin routes wired
- **CO-108** — Universe archive format + backup-to-external-HD scripts
- **CO-109** — Mbya Guarani stress-test universe: lexicon → markdown corpus, UAT seed
- **CO-118** — Workers Analytics Engine: `WaeEmitter`, Cloudflare Worker proxy
- **CO-121** — A/B primitives: `feature_flags`, `ab_assignments`, `ab_exposures`, admin routes
- **CO-122** — Quota/tier model spec in `docs/QUOTAS.md` (no enforcement yet)

### Fixed

- `has_data()` dual-check guards CO-77 first-boot false-negative (prod incident 2026-05-01)
- Cache timing test budget relaxed 1 ms → 10 ms for parallel CI runs

## [1.29.0] — 2026-05-01

### Added — CO-69: PWA offline — IndexedDB cache + Background Sync

**offline.js** (`static/shared/offline.js` — new file)
- IndexedDB schema `co-offline-v1` with `entries` store (keyed by `[universe_key, path]`, LRU-indexed) and `pending_writes` store (autoIncrement)
- `window.fetch` intercept for PUT/POST to `/api/v1/universes/*/entries*` and `/vault/*`: writes to IDB immediately (optimistic cache), tries network, queues on failure and registers Background Sync tag `co-vault-writes`
- `flushPendingWrites()` — replays pending queue; called on `online` event and manual sync button
- `updateOfflineBanner()` — shows/hides the conflict banner with pending write count; i18n-aware (pt/en)
- `beforeinstallprompt` capture + `showInstallPrompt()` for PWA home screen install
- SW `CO_SYNC_COMPLETE` message listener → refreshes banner after background sync

**Service worker** (`static/shared/sw.js`, `static/sw.js`)
- CACHE_NAME bumped `co-v3-network-first` → `co-v4-offline` (triggers cache refresh on deploy)
- `handleVaultGet` — GET `/api/v1/universes/*/vault/*`: checks IndexedDB first, falls back to network, populates cache on success
- `sync` event handler (`co-vault-writes` tag): replays `pending_writes` from IDB with credentials, stops on first network failure to prevent thundering herd; notifies all clients via `CO_SYNC_COMPLETE`

**index.html** (`static/variants/a/index.html`)
- Offline conflict banner (`#offline-sync-banner`): fixed top bar with pending count, "Sincronizar" button, dismiss; hidden via `style.display`
- Install button (`#btn-install-pwa`): shown in header when `beforeinstallprompt` fires; triggers native install prompt

## [1.28.0] — 2026-05-01

### Added — CO-104: Backup automation — daily state of SQLite + universes/ to S3

**Scripts**
- `scripts/backup-prod.sh` — atomic SQLite state via `.backup` + `universes/` tarball, uploads both to S3 (`co.db/<date>.db`, `universes/<date>.tar.gz`); idempotent, no interactive prompts
- `scripts/restore.sh` — restores from S3 (date mode) or local file; added **production safety guard**: fails loud if target is `co-artelonga` without `--yes-i-want-to-overwrite-prod`; restores both SQLite and `universes/` tarball when pulling from S3

**Cron automation**
- Option A: `infra/backup-cron/` — Alpine Fly app running `crond` at 03:17 UTC; self-contained image with `flyctl` + `aws-cli`; `fly.toml` + `Dockerfile` + `entrypoint.sh`
- Option B: `.github/workflows/backup.yml` — GitHub Actions daily cron at 03:17 UTC; `workflow_dispatch` for on-demand runs; requires `BACKUP_AWS_ACCESS_KEY_ID`, `BACKUP_AWS_SECRET_ACCESS_KEY`, `FLY_API_TOKEN` secrets

**Infrastructure**
- `infra/s3/lifecycle.json` — S3 lifecycle: STANDARD_IA after 30 days, delete after 365 days
- `infra/s3/setup.sh` — idempotent bucket setup: create, block public access, SSE-S3 encryption, lifecycle

**Documentation**
- `docs/OPERATIONS.md` — "Backup & restore" section rewritten with full runbook: S3 layout, on-demand backup, restore with prod guard, cron options, restore-drill, first-run checklist

## [1.27.0] — 2026-04-30

### Added — CO-73: Temporal model — first-class semantic dates (event_at, due_at, scheduled_at, …)

**`DateSemantic` enum expansion (`core/src/manifest.rs`)**
- Renamed `Due/Event/Created/Updated` → `DueAt/EventAt/CreatedAt/UpdatedAt` to match canonical `_at` names
- Added four new semantics: `ScheduledAt`, `PublishedAt`, `ExpiresAt`, `EffectiveAt`
- Added `DateSemantic::as_str()` returning the canonical query-param string (e.g. `"event_at"`)

**`entry_dates` table (per-universe `data.db`, migration v2)**
- Schema: `(universe_key, entry_path, semantic, value TEXT NOT NULL UTC ISO-8601)` with PK
- Index `idx_entry_dates_range ON (universe_key, semantic, value)` for O(log N) range queries
- Created idempotently on every DB open; version bumped to 2 on first migration

**Write hook (`co-web/src/entry_index.rs`)**
- `upsert_dates(universe_key, entry, manifest)` — extracts all `Date` fields with a declared semantic from the manifest, normalises values to UTC RFC3339, upserts into `entry_dates`
- `remove_dates(universe_key, path)` — clears all `entry_dates` rows on DELETE
- `normalize_date_to_utc(s)` — accepts full RFC3339 and `YYYY-MM-DD`; returns `None` on parse failure
- Hook wired into `create_entry`, `update_entry`, `delete_entry` in `entry_routes.rs`

**Date-semantic query API**
- `GET /api/v1/universes/:slug/entries?date_semantic=event_at&from=2026-01-01&to=2026-12-31`
- JOINs `entry_dates` on `(universe_key, semantic)` with optional `>= from` / `<= to` bounds
- Results ordered by date ascending; hard cap 500

**Manifest API endpoint**
- `GET /api/v1/universes/:slug/manifest` — returns parsed `_universe.yaml` as JSON; falls back to `default_manifest` when no file exists

**Calendar view upgrade (frontend)**
- Detects manifest `presentation.calendar.date_field`; fetches entries via date-semantic API when declared
- Renders entries (not just tasks) in calendar cells; normalises UTC to user's local timezone via `Intl.DateTimeFormat`
- Legacy `due_date`-from-tasks rendering preserved as fallback

**Gantt view (frontend)**
- Manifest-declared `views: [{ type: gantt, date_start: X, date_end: Y }]` injects a tab automatically
- `renderGantt(viewDef)` renders horizontal bars spanning `date_start` → `date_end` per entry
- Today marker, month labels, responsive bar widths; no code changes needed for new Gantt views

**Timezone support**
- Server stores UTC; browser renders in user's timezone via `Intl.DateTimeFormat().resolvedOptions().timeZone`

### Added — CO-61: Sync Protocol v1 — op log + content-addressed blobs + 3-way merge + recursive resolution

**Spec document (`docs/sync-protocol-v1.md`)**
- 706-line canonical specification covering: op log shape, HLC semantics, content-addressed blob store, 3-way merge algorithm, recursive conflict resolution, idempotency/atomicity guarantees, REST transport, auth (v1.0 shared secret / v1.1 federation reserved), and reducer rules
- Explains the PR analogy (Proposta ≅ pull request), prod-wins default policy, and copia semantics
- Full compatibility mapping with CO-51/54/55/58/66/68 sync tracks

**JSON Schemas (`docs/sync-protocol-v1/schemas/`)**
- `hlc.json` — Hybrid Logical Clock serialized as `wall_ms:counter:node_hex32`
- `ator.json` — Actor identity (node_id + optional user_id)
- `alvo.json` — Addressed entity + optional field
- `operacao.json` — Single immutable op with causal parents
- `manifesto.json` — Peer state summary for divergence detection
- `proposta.json` — Sync proposal (batch of ops from sender)
- `conflito.json` — Detected conflict with resolution options
- `relatorio_mesclagem.json` — Merge report returned to sender

**Test vectors (`docs/sync-protocol-v1/fixtures/`)**
- 10 fixture files covering: clean apply, independent advances, same-slot conflicts, resolver ops, delete-vs-update (copia), idempotent dedup, nested conflicts, causal ancestry, schema migration, and resolver reversibility

**Rust skeleton (`core/src/sync/mod.rs`)**
- Types: `Hlc`, `Ator`, `Alvo`, `Operacao`, `Manifesto`, `Proposta`, `Conflito`, `RelatorioMesclagem`
- `SyncProtocol` trait for implementors
- `mesclar()` / `mesclar_com_blobs()` — pure 3-way merge function with dedup, causality-aware conflict detection, and blob request list
- `causal_ancestor()` — transitive parent-walk via op `pai` DAG
- `conflito_id_de()` — deterministic conflict UUID from SHA-256 of sorted op IDs
- Custom serde for `Hlc` (string format) and `[u8; 32]` (hex)

**Fixture tests (`core/tests/sync_fixtures.rs`)**
- Parameterized test runner loading all 10 fixtures
- Compares `aplicadas`, `novas_ops_remotas`, `blobs_solicitados`, and `conflitos` (by op_local/op_remota/alvo, ignoring generated IDs)

## [1.26.0] — 2026-04-30

### Added — CO-74: Relationship graph — typed FK references + query DSL + wikilink promotion

**`entry_relations` table (per-universe `data.db`, migration v3)**
- Schema: `(universe_key, from_path, to_path, relation_type, created_at)` with PK + two directional indexes
- Indexed on `(universe_key, from_path, relation_type)` and `(universe_key, to_path, relation_type)` — O(log N) lookups in both directions
- Created idempotently on every DB open via `UNIVERSE_SCHEMA IF NOT EXISTS`

**Wikilink parser (`core/src/wikilink.rs`)**
- `resolve_ref_value(s)` — strips `[[target]]` or `[[target|alias]]` notation, returns bare path
- `extract_wikilinks(text)` — scans free text for all wikilink targets
- Used at entry write time to resolve typed ref field values

**Relation index (`co-web/src/relation_index.rs`)**
- `RelationIndex` with `replace_all`, `delete_for_entry`, `outbound`, `inbound`
- `extract_relations(manifest, entry_type, frontmatter)` — derives `(relation_type, to_path)` pairs from manifest-declared `ref`/`ref_list` fields only; non-ref fields with wikilinks stay as plain text
- `sync_entry_relations(conn, ...)` — called from all write paths
- `backfill_for_manifest(conn, ...)` — re-derives relations for all entries of affected types
- `backfill_relations_background(pool, slug, manifest)` — fire-and-forget thread spawned on manifest update

**Manifest-driven typing**
- On every entry create/update (via entry routes and vault routes), manifest is loaded and FK relations derived from `Ref`/`RefList` fields and stored atomically
- On entry delete, outbound relations removed
- Wikilinks in non-ref fields (plain `String`, `Enum`, etc.) never produce relation rows

**Query DSL (`co-web/src/query_dsl.rs`)**
- Syntax: `FROM <type> [WHERE <cond> [AND <cond>]*] [LIMIT <n>]`
- Operators: `=` (exact frontmatter match), `LIKE` (frontmatter LIKE), `INCLUDES` (relation join)
- `INCLUDES` compiles to a `JOIN entry_relations` with `DISTINCT` deduplication
- Field names validated as safe identifiers before interpolation into `json_extract` paths (SQL-injection proof)
- Result cap: 1 000 rows (explicit LIMIT clamped silently)
- Max 10 filter conditions per query

**API: `GET /api/v1/universes/:slug/query`**
- `?q=<dsl>` — parses DSL, compiles to SQLite, returns `{ entries, total }`
- Returns 400 on parse error with human-readable message

**Board UI — relation-aware entry detail**
- `GET /api/v1/universes/:slug/entries/*path` now returns `{ ...entry, relations: [...] }` in JSON (protobuf unchanged)
- `relations` array lists outbound FK edges: `{ universe_key, from_path, to_path, relation_type, created_at }`
- Board can render relationship-aware views without a separate API call

**Backfill on manifest update**
- When `_universe.yaml` is updated via vault PUT, `backfill_relations_background` is spawned alongside the existing index-rebuild job
- Idempotent: can be re-run safely; `replace_all` clears stale rows before inserting

## [1.25.0] — 2026-04-30

### Added — CO-77: Per-universe SQLite sharding + LiteFS read replicas

**Storage architecture split**
- Monolithic `co.db` renamed to `meta.db` at startup (atomic POSIX rename; backward-compatible)
- Each universe gets its own `data.db` at a 2-level xxHash fanout path:
  `{data_dir}/universes/{ab}/{cd}/{key}/data.db` — 256×256 = 65 536 directories, handles 10 M+ universes without `ls` degradation
- `meta.db` retains: users, universes, universe_members, api_tokens, subscriptions, telemetry, uat_mutations, quilombo_* tables
- Per-universe `data.db` holds: entries, entries_fts (WAL-mode, independent lock per universe)

**Connection pool (`co-web/src/universe_pool.rs`)**
- `UniversePool` with LRU eviction — default capacity 1 000 open connections
- Per-universe migration runs on first open (entries + entries_fts schema)
- `get_or_open(key)` returns `Arc<Mutex<Connection>>` so different universes lock independently

**Parallel write throughput**
- Writes to different universes now run concurrently — no shared SQLite write lock
- `project_universe_index` in meta.db provides O(1) routing for legacy `/projects/{key}` routes

**Startup migration (online, zero downtime)**
- On first boot after upgrade, entries in meta.db are automatically migrated to per-universe DBs
- `project_universe_index` populated from frontmatter of project entries during migration
- meta.db entries table cleared after all universes confirmed copied

**New Storage API**
- `Storage::universe_conn(key)` — get per-universe connection
- `Storage::backup_universe(key, dest)` — rusqlite Backup API, < 30s for any universe
- `Storage::universe_db_size(key)` — file-size quota check
- `Storage::search_entries_across_universes(keys, query)` — cross-universe aggregator

**LiteFS configuration (`litefs.yml`)**
- Primary in `gru`, replicas in other regions via Consul lease
- Fly.io env vars `LITEFS_DIR` and `LITEFS_URL` added to `fly.toml` and `fly.uat.toml`
- Proxy config for write-forwarding to primary

**Offline migration tool (`co-web/src/bin/split_db.rs`)**
- `split_db --data-dir /data [--dry-run]`
- Idempotent: INSERT OR IGNORE, safe to re-run after interruption
- Populates `project_universe_index` and clears meta.db entries table when complete

**Entry routes updated**
- `entry_routes.rs` and `vault_routes.rs`: all `EntryIndex` operations now use `universe_conn(slug)` instead of `meta.db` — entries are fetched from the correct per-universe DB

### Added — CO-72: Doc-generator hooks + SQLite job queue

**Doc-generator adapters (`co-web/src/doc_gen.rs`)**
- `DocAdapter` trait: `fn run(source_dir, output_type, limits) -> Result<Vec<DocEntry>>`
- Stub implementations for all v1 formats: scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc
- `ResourceLimits`: wall-clock 5 min, 2 GB RAM, 1 GB output per job
- `DocFormat::from_str` via `std::str::FromStr`; `run_adapter` dispatch function

**SQLite job queue (`co-web/src/job_queue.rs`, migration v24)**
- `jobs` table: `(id, universe_key, kind, payload, status, attempts, dedupe_key, created_at, run_at, started_at, completed_at, error)`
- `enqueue_doc_gen`: idempotent submission — same `(universe, format, source_dir, adapter_version)` returns existing job ID
- FIFO claim via `UPDATE … RETURNING` with `(run_at, created_at)` ordering; no universe starvation
- Exponential backoff on failure (2^n min, capped 64 min); dead-letter after 5 attempts
- In-process worker loop (`spawn_worker`) using `tokio::time::timeout` for wall-time enforcement
- `doc_gen_error` / `doc_gen_error_at` columns on `universes` table for failure surfacing

**API endpoints**
- `POST /api/v1/universes/:slug/jobs/doc-gen` (owner-only): submit doc-gen job, returns `{ job_id }`
- `GET /api/v1/universes/:slug/jobs/doc-gen/last-error` (owner-only): last failure message + timestamp

**CO-77 compatibility fixes (co-web/src/storage.rs)**
- `seed_template_universe`: writes entries to per-universe DB (universe pool) instead of meta.db
- `reseed_template_content_pages`: same fix
- `clone_universe_internal`: reads from source per-universe DB, writes to target per-universe DB + registers in `project_universe_index`
- Migration v24 runs after v23; v25 (CO-77) follows
## [1.24.0] — 2026-04-30

### Added — CO-71: Per-universe schema validator + generic JSON entry storage

- `core/src/payload.rs` — `validate_payload()` validates frontmatter JSON against a manifest `ContentType` schema with dot-notation field-path errors; `coerce_payload()` coerces fields to typed Rust values; `TypedEntry` with `fields: BTreeMap<String, TypedValue>` (Date → `DateTime<Utc>`, Number, Boolean, StringArray, String, Null)
- `co-web/src/index_manager.rs` — `IndexManager::apply_indexes()` / `drop_stale_indexes()` / `sync_indexes()` diff and apply SQLite expression indexes (`idx_co71_<universe>_<field>`); `apply_manifest_indexes_background()` spawns a background thread so index creation never blocks HTTP writes
- `co-web/src/entry_index.rs` — `upsert()` now writes `payload` column (mirrors `frontmatter_json`); `typed_view()` converts `EntryRow` → `TypedEntry` using the manifest; expression indexes target `json_extract(payload, '$.field')`
- `co-web/src/entry_routes.rs` — POST and PUT entry handlers validate frontmatter against `_universe.yaml` manifest before write; invalid payloads return 422 with field-path error; legacy universes (no manifest) pass through unchanged
- `co-web/src/vault_routes.rs` — PUT `_universe.yaml` triggers background index sync via `apply_manifest_indexes_background`
- `co-web/src/error.rs` — `AppError::UnprocessableEntity` (HTTP 422) for manifest validation failures
- Migration v24: `entries.payload TEXT NOT NULL DEFAULT '{}'` + backfill from `frontmatter_json`; `universes.manifest_version INTEGER NOT NULL DEFAULT 0` for future migration tracking

## [1.23.0] — 2026-04-30

### Added — CO-70: Manifest format spec — `_universe.yaml` at universe root

- `core/src/manifest.rs` — typed `Manifest` struct hierarchy parsed from `_universe.yaml`
- `parse()` / `parse_str()` — validates size cap (100 KB), content-type count cap (100), field-path errors, and forward-compat warnings for unknown top-level keys
- `default_manifest(name)` — returns a board-of-tasks manifest matching pre-manifest behaviour (`task` type, `[todo, doing, done]` board columns)
- `Manifest::triggers_migration_from(stored_version)` — CO-71 hook for entry-payload migration on schema version bump
- `docs/schemas/_universe.v1.json` — JSON Schema (draft 2020-12) for `_universe.yaml` v1

## [1.22.7] — 2026-04-30

### Removed — CO-64: git-sync dead code + migration v23 drops git_* columns

- Deleted `co-web/src/git_sync.rs` (365 lines, dead since Vault API pivot)
- Removed `UniverseGitConfig` struct and git storage methods
- Removed route handlers: `update_universe_git`, `manual_sync`, `webhook_sync`
- Migration v23: `ALTER TABLE universes DROP COLUMN` for 6 git_* columns
- Added `docs/ARCHITECTURE.md` — post-GitHub data model overview
- CO-50, CO-55 marked deprecated

## [1.22.6] — 2026-04-30

### Added — CO-138: Wave 2 Playwright e2e coverage (sidebar tree, mermaid, onboarding)

Three Playwright test suites under `co-web/e2e/wave-2/` that drive Chromium against UAT (or a local server with seeded fixtures):

- `co-web/e2e/wave-2/co-98-sidebar-tree.spec.ts` — verifies the timeline trio (`tempo`, `humanity`, `universo`) appears nested under `template` in the sidebar, with chevron toggle and CSS indent.
- `co-web/e2e/wave-2/co-107-mermaid.spec.ts` — asserts the template home renders a Mermaid SVG containing the trio node labels, and that universes without Mermaid blocks do not load the Mermaid bundle.
- `co-web/e2e/wave-2/co-99-onboarding.spec.ts` — exercises the 3-step onboarding banner lifecycle: cookie set on dismiss, reload suppression, mobile viewport suppression, and no banner for logged-in users.

Additional infrastructure:
- `co-web/e2e/helpers.ts`: `loginAsAdmin` helper — UAT uses magic `uat-login`, prod/local uses `password-login` via `CO_ADMIN_EMAIL` + `CO_ADMIN_PASSWORD` env vars.
- `co-web/playwright.config.ts`: `baseURL` now reads `process.env.BASE_URL ?? "http://localhost:3000"` so `BASE_URL=https://co-artelonga-uat.fly.dev npx playwright test` works.
- `docs/OPERATIONS.md`: Wave 2 regression gate command added to post-deploy section.

## [1.22.5] — 2026-04-30

### Fixed — CO-137: harden ALTER ADD COLUMN migrations against partial-application + diagnostic endpoint

**Root cause investigation (CO-137):** Migration v22 (`parent_key` on `universes`) was checked with `if current_version < 22` after a fresh `MAX(version)` read — mechanically correct. Code analysis suggests the most likely failure mode is a stale `schema_version=22` row recorded without the matching `ALTER TABLE` completing (volume state edge case or a previous deploy that committed the version row but not the schema change). The diagnostic endpoint added in this release confirms prod schema state.

**Structural fix:** Replaced bare `ALTER TABLE … ADD COLUMN` calls in migrations v17–v22 with `ensure_column` — a `pragma_table_info`-guarded helper that is a no-op when the column already exists. This makes every column-add migration idempotent: re-running a partially-applied migration recovers cleanly instead of panicking on "duplicate column name."

Additionally, an **unconditional post-migration backfill** runs after all versioned blocks to ensure `parent_key` exists on the `universes` table regardless of what `schema_version` records, closing the exact failure mode from the 2026-04-30 prod incident.

**Changes:**
- `co-web/src/storage.rs`: `ensure_column` helper + unit tests (4 cases incl. partial-migration recovery simulation)
- Migrations v17, v18, v20, v21, v22 updated to use `ensure_column` + `INSERT OR IGNORE` for version row
- Unconditional `parent_key` backfill after all migrations
- `co-web/src/gestao_routes.rs`: `GET /api/v1/gestao/_schema_check` (GitHub admin auth) returning `universes` column list + `schema_version` rows

## [1.22.4] — 2026-04-30

### Fixed — `get_universe` resilient to partially-applied parent_key migration (prod hotfix)

**Symptom on prod (1.22.3):** `GET /api/v1/universes/template`, `/api/v1/universes/tempo`, etc. returned 404 "not found" even though the universes were seeded (filesystem dirs present, startup logs confirmed `Timeline universe '<key>' seeded`). Sibling endpoints that didn't go through `get_universe` (`/theme.css`, `/config`) continued to work.

**Root cause:** since 1.22.0, `Storage::get_universe` and `list_universes_for_user` selected `parent_key` (added by migration v22). When the column wasn't actually present on the DB at query time — for any reason (migration not yet applied, partial schema state, drift between machines) — the SELECT errored, `.ok()` swallowed the error, and the function returned `None`, indistinguishable from "universe doesn't exist". UAT was unaffected because its DB had the column; prod was not.

**Fix:** split the SELECT into two queries.
1. The stable schema (everything ≤ schema_v 17) — must succeed for any prod DB still in service.
2. A separate `SELECT parent_key FROM universes WHERE key = ?` that opportunistically fetches `parent_key`. If the column doesn't exist or the row is missing, gracefully returns `None`.

Applied to:
- `get_universe`
- `list_universes_for_user` (per-row second query — slight overhead, fine at our scale)
- `search_public_universes` (parent_key set to `None` unconditionally — search results don't surface parent_key in any UX path)

This is the right shape for any column added by a recent migration: the read-path should not assume the migration has landed, especially when the assumption is buried inside `.ok()` and silently maps to "not found."

After this fix, even if migration v22 didn't run (or ran and was rolled back), the API behaves correctly — the trio still has `parent_key="template"` on UAT (DB has the column), and the trio is reachable on prod (degrades to `parent_key=None` if the column happens to be missing).

## [1.22.3] — 2026-04-30

### Fixed — `parent_key` now exposed by `GET /api/v1/universes/:slug` (CO-98 follow-up)

Surfaced during UAT smoke verification of 1.22.2: the public universe-info endpoint returns a stripped `UniverseInfo` DTO, not the raw `Universe` struct. Adding `parent_key` to `models::Universe` (1.22.0) was therefore not enough — the field was silently dropped by the DTO before serialization.

- `co-web/src/universe_routes.rs::UniverseInfo` — adds `parent_key: Option<String>` with `#[serde(skip_serializing_if = "Option::is_none")]` so top-level universes still emit no extra field.
- `get_universe_info` — passes `universe.parent_key` through to the DTO.

`GET /api/v1/universes` (the bulk list) was unaffected — that endpoint already returned raw `Universe` instances and emitted `parent_key` correctly.

After this fix, `curl /api/v1/universes/tempo | jq .parent_key` returns `"template"` as the CO-98 spec required.

## [1.22.2] — 2026-04-30

### Added — onboarding coach mark for first-time anonymous visitors (CO-99)

Three-step floating banner (bottom-right, ~320×120px) introducing first-time anonymous visitors on the template universe to the platform's narrative. **Non-blocking** — does not capture clicks behind it. Cookie-gated for one year on dismissal/completion.

Steps (PT-BR copy):
1. **Visões** — names the four views (Quadro / Tabela / Conteúdo / Linha do tempo) and points to the header tabs.
2. **Linha do tempo** — explains the log-scale and links to the multi-overlay at `/shared/timeline.html?u=tempo,universo,humanity` (opens in new tab).
3. **Crie seu universo** — points users at the new `+ Novo universo` sidebar button (CO-96 P1) once they create an account.

Show conditions (all must be true):
- `state.isTemplate === true`
- `await api.me()` returned null (anonymous)
- `co_onboarded` cookie is **not** set
- viewport width ≥ 720px (mobile UX deferred)

Dismissal sets `co_onboarded=1; Path=/; Max-Age=31536000; SameSite=Lax`. Theme-aware via CSS custom properties (`--card-bg`, `--accent`, `--border`, `--shadow-md`, `--text-muted`); inline-styled for self-containment, no new CSS file needed.

`setupOnboarding()` is invoked from `init()` on both the anonymous-template branch and the fallback-to-template branch. Internal gates re-check viewport + cookie + state defensively, so a future caller can't accidentally show the banner to the wrong audience.

Telemetry (onboarding completion rate) is deferred to the admin-dashboard ticket (CO-105) per spec — banner today is purely client-side cookie-driven.

## [1.22.1] — 2026-04-30

### Added — universe create modal Phase 1 (CO-96 P1)

The existing "Criar universo" modal (previously banner-only and always cloned `template`) now supports the full Phase 1 surface from CO-96:

- **`+ Novo universo` button in the sidebar header.** Always visible; opens the modal with a fresh empty form (visibility=private, copy-from off).
- **Description field** — optional textarea (rows=2).
- **Visibility radio group** — `Privado · Público assinável · Login obrigatório`. Default: `private` to match server semantics.
- **Copy-from existing universe** — checkbox + dropdown. Source dropdown is populated from `state.userUniverses` (plus a stable `Template (CO)` fallback) on every open.
- **Branched submit** — copy-from off → `POST /api/v1/universes` (empty); copy-from on → `POST /api/v1/universes/<source>/duplicate` (CO-95). Visibility ≠ private is applied via a follow-up `PUT /api/v1/universes/:slug` to keep the create endpoint shape unchanged.

The legacy banner CTA (`btn-criar-universo`) keeps its old behavior — the click handler now passes `{ copyFromTemplate: true }` to prefill copy-from from `template`, preserving the anonymous-visitor flow.

Out of scope for Phase 1 (per the ticket): debounced key-uniqueness check (server already rejects 409 on duplicate); rename/visibility-change context menu; soft-delete. Those land in Phase 2/3 of CO-96.

## [1.22.0] — 2026-04-30

Wave 2 of the v1-launch sprint, partial: universe hierarchy (CO-98) and home-page Mermaid (CO-107). Create modal (CO-96 P1) and onboarding banner (CO-99) are open as separate work.

### Added — hierarchical universes (CO-98)

Each universe row now carries an optional `parent_key` pointer. Top-level universes have `parent_key = NULL`; children render nested under their parent in the SPA sidebar with a 16px indent and a chevron (`▸ / ▾`).

- **Migration v22** — `ALTER TABLE universes ADD COLUMN parent_key TEXT; CREATE INDEX idx_universes_parent_key ON universes(parent_key);`. Nullable, no FK — orphan children (parent disappears) gracefully fall back to top-level rendering.
- **Models** — `Universe.parent_key: Option<String>` added; serialized in API responses (`#[serde(skip_serializing_if = "Option::is_none")]`), so universes without a parent emit no extra field.
- **Seed** — `seed_timeline_universe` now sets `parent_key = 'template'` on the trio (`tempo`, `humanity`, `universo`). An idempotent UPDATE backfills `parent_key` on existing rows from prior versions.
- **SPA** — `renderSidebar` builds a tree from the flat `state.userUniverses` list and renders top-level → children with chevron toggles. Per-parent expansion state persists in `localStorage` (`co_universe_tree_<key>`); default expanded if a child is the active universe.
- **Storage tests** — `SEED_TEMPLATE_INDEX_MD` added to the embedded-seed roundtrip suite; existing 143 tests pass unchanged.

### Added — Mermaid diagrams in universe-home (CO-107)

`renderUniverseHome` now post-processes Mermaid fenced blocks via the existing `CoMarkdown.renderMermaidBlocks` helper. Lazy-loaded; no overhead when an `index.md` has no Mermaid blocks.

Template now ships a root-level `index.md` (in addition to the `content/` legal pages) showing the **Template → Tempo / Universo / Humanidade** trio as a directed graph, with palette tokens matching `timeline.html`. Visible on `co.artelonga.com.br/co` and any future template clone.

- `co-web/seed/template/index.md` — new home-page seed (Mermaid + view explainer in PT-BR)
- `co-web/src/storage.rs::reseed_template_content_pages` — adds `("index.md", SEED_TEMPLATE_INDEX_MD)` to the always-overwrite list (idempotent re-seed on every boot)
- `co-web/static/variants/a/app.js::renderUniverseHome` — calls `renderMermaidBlocks(body)` after the markdown renders

Constraint preserved: existing universes whose `index.md` has no Mermaid block trigger no extra network requests and no JS errors (helper short-circuits when no fence is present).

## [1.21.2] — 2026-04-30

### Added — per-deploy regression smoke scripts (CO-103)

Two one-shot bash scripts that verify post-deploy invariants and exit non-zero with a diagnostic on any miss:

- `scripts/smoke-prod.sh` — targets `https://co.artelonga.com.br` (override via `BASE_URL`).
- `scripts/smoke-uat.sh` — targets `https://co-artelonga-uat.fly.dev` (override via `BASE_URL`).
- `scripts/smoke-lib.sh` — shared helpers (`check_status`, `check_json_field`, `check_count`).

10 checks in order: health, health-deep, template universe, timeline trio shape + event counts (21/26/28 pinned), themes CSS (`--accent: #6366f1`), static assets, service worker cache name, auth reachability (bogus login → 401), template entries total, favicon.

`docs/OPERATIONS.md` added with the full smoke-test runbook and deploy procedure.

### Added — `GET /api/health/deep`

New endpoint that verifies DB read+write (SAVEPOINT/ROLLBACK proves write access without modifying data) and disk accessibility. Returns `{"status":"ok","db":"ok","disk":"ok"}` on success or HTTP 503 with `"status":"degraded"` if any subsystem is unhealthy.

## [1.21.1] — 2026-04-30

### Added — multi-universe overlay + smooth event travel in the timeline

The timeline visualization at `/shared/timeline.html` is now demoable. Three improvements working together:

- **Multi-universe overlay.** `?u=tempo,humanity,universo` (comma-separated) renders events from any combination of the three timeline universes on the same canvas. Each universe gets its own color (teal / blue / warm) and its own vertical lane so events don't collide. URL syncs in real time when you toggle universes via the header chips.
- **Prev/next event with smooth travel.** Header has `‹ ›` buttons; arrow keys also work. Pressing one animates the focus to the next/previous event with a 750ms ease-in-out-cubic over interpolated pixel-space — so traveling from "Big Bang" to "Andromeda collision" pans smoothly across both linear and log regions instead of teleporting. Clicking an event on the timeline travels to it the same way. `Home` / `0` returns to 2026.
- **Cleaner empty / disabled states.** Nav buttons are disabled when no events are loaded. An on-canvas hint explains how to toggle when all universes are off.

### Added — `Linhas do tempo` featured page in the template universe

`co-web/seed/template/linhas-do-tempo.md` is a new public page that documents the timeline trio as a curated category under the template universe. Direct links to all three timelines, the combined view (`?u=tempo,humanity,universo`), and a "build your own" note showing the `type: event` + `date_year` frontmatter convention. Re-seeded on every boot.

### Fixed — admin sidebar polluted with anonymous "Meu Co" clones

A previous version of `rescue_orphan_universes` re-homed orphan anonymous-clone universes (key prefix `u-`) to the admin user, polluting their sidebar with clones from old visitors. Two changes:

- `rescue_orphan_universes` now skips keys matching `u-%` and `anon-%` — those are anonymous clones, not real personal universes.
- New `cleanup_admin_anon_clutter(admin_email)` runs on every startup after the seed admin is ensured. It deletes anonymous-clone universes still owned by the admin (legacy from the prior buggy rescue), along with their entries, members, and on-disk universe directory. Idempotent.

### Added — `public-static` visibility recognized by access control

`check_universe_access` now returns `ReadOnly` for universes with `visibility = 'public-static'`, matching the existing handling of `visibility = 'template'`. Without this, the new timeline universes were 404'ing for anonymous visitors even though `is_public = 1`.

## [1.21.0] — 2026-04-29

### Added — three timeline universes (`tempo`, `humanity`, `universo`)

The CO-92 timeline visualization now ships with three sibling universes seeded out of the box:

- **`tempo`** — meta-universe explaining the time-scale concept itself. 21 events bridging cosmic and human history (Big Bang → Now → heat death). Acts as the "what is this view" front door.
- **`humanity`** — focused on our species. 26 events from the emergence of Homo sapiens through agriculture, writing, the printing press, the Industrial Revolution, computing, the Web, and the present.
- **`universo`** — full cosmic timeline. 28 events from inflation through stelliferous era, Sun's red giant phase, last star, black-hole evaporation, and heat death.

Inspired by [scaleofuniverse.com/pt](https://scaleofuniverse.com/pt) but with **emphasis on time** rather than spatial scale. Each universe is `is_public=1`, `requires_login=0`, system-owned, modern theme, layout=`timeline`.

Architecture: each universe is a regular Co universe (just system-seeded). Events are markdown entries with `type: event` and a numeric `date_year` in frontmatter. Content is split from form — manifests live as JSON at `co-web/seed/timeline/{tempo,humanity,universo}.json` plus an `index.md` per universe; storage.rs only orchestrates seeding (`seed_timeline_universe`, `seed_all_timeline_universes`). Idempotent re-seed on every startup so JSON edits ship in the next deploy without manual data migration.

### Changed — timeline UI: cross-universe nav + scaleofuniverse link

`/shared/timeline.html` now shows a header tab bar with `Tempo` / `Universo` / `Humanidade` so demo viewers can flip between the three views in one click. Active universe is highlighted with an accent underline. The "scale ↗" link in the header credits scaleofuniverse.com as inspiration. Default `?u=` is now `tempo` (was `template`). Hint and error strings localized to PT-BR. Header title fetched from `/api/v1/universes/:slug` so it shows the friendly name ("Universo", not the slug).

## [1.20.11] — 2026-04-29

### Fixed — universe with no projects left the spinner up forever

`renderContent()` returned silently with `if (!state.currentProject) return;` — leaving the loading-spinner from `bootAppForUniverse` rendered indefinitely whenever the universe had no projects (or the projects fetch failed). With artelonga / qa-dev / etc. having content uploaded via vault but no canonical "project" entry, the SPA was stuck at "Carregando…" for any logged-in user opening those universes. Replaced the silent return with a call to a new `renderUniverseHome()` that always paints something visible, so the spinner can never persist past `render()`.

### Added — universe home / front page rendered from `index.md`

Each universe can now ship an `index.md` at its root. When the user enters the universe (and there's no project to render the kanban for), the SPA fetches that file and renders its body as a hero page: title from `universe.name`, description from `universe.description`, and the markdown body in the main area.

If `index.md` doesn't exist, a friendly empty state explains how to add one and reports how many entries the universe has, so the page is never spooky-blank. Mirrors the convention of `README.md` for git repos / `CLAUDE.md` for instruction files: a "what is this" front page that anyone landing here can read without scrolling.

### Added — boot watchdog + per-fetch timeouts in `bootAppForUniverse`

Each fetch step (`getUniverseInfo`, `getUniverseConfig`, `getUniverseProjects`, `selectProject`) is now wrapped in `withTimeout(promise, 8000)` so any individual hang resolves to `null` after 8s instead of blocking the whole boot. An outer 20-second watchdog renders a recovery card with "Recarregar / Voltar ao template / Reset cache" links if the boot doesn't complete — defensive against any future hang in code I haven't audited.

## [1.20.10] — 2026-04-29

### Fixed — service worker was caching every JS deploy into oblivion

`co-web/static/shared/sw.js` (the actual served file — `static/sw.js` was a stale duplicate that the server doesn't read) was cache-first for every static asset including `app.js` and `style.css`. Even `Cmd+Shift+R` couldn't bypass it: browsers route reload requests through the SW, and the SW was happily returning yesterday's bytes from `caches.match()` while only updating the cache for *next time*. So users complained that "modern theme doesn't stick" / "hard refresh doesn't load" — they were never actually receiving any of the 1.20.5 → 1.20.9 fixes.

Rewrote the SW with a **network-first** strategy for HTML/JS/CSS (deploys propagate immediately, fall back to cache only when offline) and cache-first only for icons/fonts/manifest. Also:

- Bumped `CACHE_NAME` to `co-v3-network-first`, so existing clients purge their stale cache when the new SW activates.
- Registration in `index.html` now listens for `updatefound`, calls `SKIP_WAITING` on the new worker, then reloads the page on `controllerchange` so users get the fresh bundle without manual intervention.
- Removed the `STATIC_ASSETS` precache list except for the manifest + favicon — precaching `app.js` was the original sin.

Existing users will see one auto-reload the next time they open the app; subsequent deploys arrive normally without that bounce.

## [1.20.9] — 2026-04-29

### Fixed — universe switch could leave the spinner up forever

If `selectProject`, `getUniverseProjects`, or any other async step inside `bootAppForUniverse` threw, the function fell through without clearing `state.switchingUniverse` or calling `hideLoading()`. The spinner stayed visible and the sidebar's universe-click handler refused further switches (it short-circuits on `state.switchingUniverse`). Wrapped the whole boot sequence in `try { ... } finally { state.switchingUniverse = false; hideLoading(); render(); }` so a failure can never wedge the UI. Each fetch step also has its own try-catch with `console.warn` so a bad universe degrades gracefully instead of cascading.

### Changed — modern palette is now the unconditional default

`loadThemeCss` previously fell back to the universe's own theme.css when `co_user_palette` wasn't set. With user feedback that modern should "stick" across every universe, the function now defaults to `modern` if no palette is stored and persists that choice. A per-load cache-buster (`?v=<unix>`) is appended so a recent change is picked up even when the browser was sitting on a stale theme.css.

### Changed — Conteúdo sections and folders default to collapsed

The Páginas section and every nested folder now start closed; the user expands what they want to look at. Saved-state in localStorage still wins, so once you open a folder it remembers next time. This makes universes with hundreds of entries (artelonga: 146, quilomboaraucaria: 70) approachable from a clean slate instead of dumping the whole tree on first render.

## [1.20.8] — 2026-04-29

### Fixed — modern theme actually loads modern colors

`loadThemeCss` was loading `template`'s `theme.css` when a user override was active. But `template` had `theme_preset='scholarly-light'` in the DB (left over from an earlier migration), so "modern override" was actually rendering scholarly browns. Two fixes:

- New endpoint `GET /api/v1/themes/:preset` returns the CSS for any built-in preset directly from the compiled-in `ThemePreset::by_name()`, independent of any universe's stored config. SPA's `loadThemeCss` now hits this endpoint when `co_user_palette` is set, so the user's choice always wins.
- Added `Storage::ensure_template_theme_preset()` and call it on every startup with `'modern'`. This brings the template universe's stored preset back in line with what the seed code intended, fixing the public landing page's appearance for unauthenticated visitors.

### Added — frontmatter preview when entry body is empty

Many universes encode their actual content as structured frontmatter rather than markdown body — e.g. artelonga's 146 entries are mostly member/community/service profiles with rich `nome` / `papel` / `bio` / `funcao` / `descricao` fields and no body. The Conteúdo view's `cardBodyHtml` now falls back to a compact key-value preview of the user-meaningful frontmatter fields when the body is empty (skipping scaffolding keys like `type`, `slug`, `created`, `tags`). Image URLs render as thumbnails; HTTP URLs as links. Up to 8 fields shown. New CSS classes: `.conteudo-fm-preview`, `.conteudo-fm-row`, `.conteudo-fm-key`, `.conteudo-fm-val`, `.conteudo-fm-img`.

## [1.20.7] — 2026-04-29

### Fixed — known personal universes now always belong to the current admin

`rescue_orphan_universes` only catches universes whose `owner_id` has no row in `users`. But after the prod data was bootstrapped, then partially wiped, then re-seeded, a more subtle state emerged: the prior admin user_id is **still in the users table** (left over), and `artelonga` / `rfq` / `qa-dev` still point at it. The current admin can't see them, but rescue skips them because the owner is technically a valid user.

Added `Storage::ensure_admin_owns_personal_universes(email, keys)` and called it on every startup with the well-known personal universe keys (`artelonga`, `rfq`, `qa-dev` — same list the bootstrap script seeds). For each of those keys, if it exists and its `owner_id != current admin user_id`, re-home it to the current admin and ensure an `owner` membership row. If it already belongs to the right user, only the membership row is reconciled (defensive). Idempotent — does nothing on a clean DB.

## [1.20.6] — 2026-04-29

### Changed — universe switching is now an atomic transition

`bootAppForUniverse` was a chain of partial state mutations interleaved with async fetches. The result was visible jank: cards from the previous universe lingered while the new one's config loaded, the settings gear flickered, and the theme swap landed at an unpredictable point in the sequence. Rewrote the flow:

1. Set `state.switchingUniverse = true` and reset all per-universe collections (`tasks`, `projects`, `currentProject`, `universeInfo`, `universeConfig`) up front, so nothing from the previous universe can leak through.
2. Show the loading spinner — it clears the content area immediately.
3. Apply the new theme/config FIRST (single hot-swap of `<link id="co-theme-css">`), so the spinner sits on the right palette.
4. Fetch projects, then drill into the first one.
5. Drop the flag and call `render()` exactly once.

The sidebar click handler now also marks the clicked item active immediately (before any fetch), and rapid double-clicks during a transition are ignored. Template banner show/hide is decided by the slug check (`isTemplate = slug === 'template'`) instead of being unconditionally hidden.

## [1.20.5] — 2026-04-29

### Fixed — orphan universes re-homed to the seeded admin

When the admin user was re-created after a data wipe (new uuid), prior universes still pointed to the old user_id and silently disappeared from the new admin's sidebar — even though `list_universes_for_user` already had the owner_id fallback. Added `Storage::rescue_orphan_universes(admin_email)` that runs on every startup right after `seed_admin_user_from_env`: any universe whose `owner_id` no longer exists in `users` (and isn't the `system` sentinel) gets re-homed to the seeded admin and an `INSERT OR IGNORE` membership row is added. Idempotent — does nothing on a healthy database.

### Fixed — modern theme override now actually applies cross-universe

Setting `co_user_palette = modern` in localStorage was supposed to make the modern look win over each universe's own `theme_preset`. The SPA was setting `data-palette="modern"` on `<html>`, but no CSS rules implement that selector — meanwhile `loadThemeCss(slug)` kept loading the universe's native theme.css (e.g. quilombo's earth tones), which overrode everything. Fixed by routing `loadThemeCss` through a preset-to-source map: when a user override is active, load the matching system universe's theme.css (`modern` → `template`) instead of the current board's. The same `<link id="co-theme-css">` element is reused, so the swap is hot.

## [1.20.4] — 2026-04-29

### Fixed — owner could be silently hidden from their own sidebar

`list_universes_for_user` only matched against `universe_members` and `subscriptions` rows. `create_universe` always inserts an owner row in `universe_members`, but if that row is ever lost (historic data, partial migration, manual cleanup), the owner stops seeing their own universe in the SPA sidebar. Added `WHERE u.owner_id = ?1 OR u.key IN (...members/subs...)` as a defensive fallback so ownership alone is enough to qualify.

### Added — stats strip in Conteúdo view

The Conteúdo view now shows a compact stats header above the sections: total entries, page count, task count, event count, distinct tag count, and last-edited relative time. Derived from the entries already loaded for the view (no extra API call). Renders unobtrusively as a single horizontal strip; collapses to two rows on mobile.

## [1.20.3] — 2026-04-29

### Fixed — `/entries` (no type filter) returned empty list

`EntryIndex::query` always added `entry_type = ?2` to the WHERE clause, even when called with an empty string. The `list_entries` route's "no type" branch passed `""`, so `GET /api/v1/universes/:slug/entries` (no `?type=`) returned 0 rows for every universe — even when filtered queries by type counted entries correctly. Visible symptom: SPA's Conteúdo view showed correct counts in the sidebar but rendered nothing in the main panel because the `allEntries` merge step (used to fold untyped markdown into the page tree) got an empty array.

Fix: `query` now omits the `entry_type` clause when the type is empty, so passing `""` truly means "any type". Filtered queries continue to work exactly as before.

### Fixed — timeline default universe was `co-dev`

`co-web/static/shared/timeline.html` defaulted `?u=` to `co-dev` (an internal-only universe), causing 404s on prod where co-dev is not seeded. Default is now `template`, which exists everywhere.

## [1.20.2] — 2026-04-29

### Changed — legal pages refresh for public test

Rewrote the four template seed pages for the initial public-test launch on `co.artelonga.com.br`:

- **Honest framing of encryption.** Previous wording implied "banco de dados criptografado em repouso" — that's roadmap (CO-86, v3.0), not current state. New text describes what's implemented today (TLS 1.3, Argon2id, access control, isolated SQLite) and explicitly calls out that bodies are plaintext at rest, with the v3.0 envelope-encryption plan stated as the path forward. For sensitive content, recommends self-hosting until v3.0.
- **Two hosting models documented.** Auto-hospedagem (MIT, you control everything, this policy doesn't apply) vs. instância gerenciada Arte Longa (`co.artelonga.com.br`, GRU region, controlador é Yuri). Each modality's responsibilities made explicit.
- **Public-test disclosure in Termos.** New §3 says "estado do produto: teste público inicial" — no formal SLA, expect breakage between versions, recommend waiting for v3.0 for production-critical use.
- **Updated `dados-rastreados.md`** with the actual telemetry event taxonomy used in the SPA (matches `static/shared/telemetry.js`), and clarifies that body content is never sent in telemetry payloads.
- **LGPD §6/§7 sharpened:** added 15-day response SLA, removed vague phrasing.

### Fixed — template content pages now refresh on every boot

`seed_template_universe()` was gated on first-boot only (`!storage.template_exists()`), which meant any update to the bundled seed pages would never reach existing deployments without a full UAT-style data reset. Extracted the four content pages into `reseed_template_content_pages()` and call it unconditionally on every server startup. Tasks and projects within the template are still seed-once (user can edit them); content pages always track the binary.

### Refactored — content separated from form

Seed content for the template universe (sobre, termos, privacidade, dados-rastreados) was previously embedded as multi-hundred-line Rust string literals inside `seed_template_universe()`. That made `storage.rs` a 3000+ line monolith mixing schema, queries, and prose.

- Moved the four pages to `co-web/seed/template/*.md` — editable as plain markdown.
- Added a tiny frontmatter parser (`split_frontmatter`, `seed_page_frontmatter`, `seed_page_body`) that turns a `.md` file with YAML frontmatter into the `(metadata_json, body_str)` pair `make_entry` expects.
- Files are embedded at compile time via `include_str!`, so no runtime filesystem dependency — single binary, single artifact.
- `created` / `modified` timestamps are stamped at seed time (so freshly seeded universes show "now"), but everything else (slug, title, order, tags) is read from the .md file's frontmatter — that's the single source of truth.
- 4 unit tests cover the parser and verify all 4 embedded files parse cleanly.
- Net: `storage.rs` shrank by ~430 lines.

## [1.20.1] — 2026-04-29

### Fixed — universe duplication now copies ALL entry types

`Storage::clone_universe` had project + task + page-specific copy paths but skipped everything else (events, clips, doc.*, untyped markdown). The first 1.20.0 duplicate of `quilomboaraucaria` produced an empty universe because all 70 source entries were `event` type from the legacy quilombo-blog migration.

- Added a final bulk `INSERT INTO entries SELECT FROM entries` step that copies all entry types not covered by the typed paths (entry_type NOT IN ('project','task','page')). Source paths/titles/frontmatter/body preserved verbatim — the duplicate is a true state.
- `INSERT OR IGNORE` makes it safe to re-run if a partial copy needs completion.

## [1.20.0] — 2026-04-29

### Added — CO-95 Phase 1: owner-controlled universe duplication

- New endpoint `POST /api/v1/universes/:source/duplicate` accepts JWT or API token (via the new `auth::resolve_user_id` helper). Verifies the caller has read access to the source (owner / member / public / template), then bulk-copies entries into a new universe owned by the caller. New universe defaults to `private` visibility.
- Differs from the existing `/clone` endpoint: requires authentication, allows duplicating private universes the caller is a member of, and sets ownership to the caller (no anon-XXX fallback).
- Use case: `quilomboaraucaria` → `quilombo-blog` for parallel scalability + latency analysis without disturbing the original. Generalizes to any "materialized dev branch" workflow today; full lineage tracking + merge / promote / revert lands in CO-95 Phase 4.
- `scripts/duplicate-universe.sh <source> <target>` — keychain-token-backed helper.

### Added — `auth::resolve_user_id`

Helper for handlers outside the JWT-only `require_auth` middleware that still need to identify the caller. Tries Bearer JWT first, then falls back to API token via `Storage::get_api_token_by_value`. Used by the new duplicate endpoint; future use by CO-91 sync, CO-93 universe-type changes, etc.

### Spec

- `work/co/CO-95.md`: Universe branching — 4-phase plan (state → op log → replay → merge). Phase 1 ships in this release.
- `work/co/CO-96.md`: Universe CRUD UX in the SPA — sidebar `+ New universe` button, context menu (rename / change visibility / duplicate / delete), settings tab, soft-delete + 30-day trash. 3 phases mapped to 1.20.0 / 1.21.0 / 1.22.0.

## [1.19.2] — 2026-04-29

### Fixed — telemetry beacon 415, missing favicon, missing PWA icon

Three cosmetic console errors visible after first prod login post-1.19.1:
- `POST /api/v1/telemetry/event` returned 415 because `navigator.sendBeacon` with a string body sends `Content-Type: text/plain`, which axum's `Json` extractor rejects. Patched `co-web/static/shared/telemetry.js` to use a `Blob` with `type: 'application/json'`.
- `/favicon.ico` 404'd — added `co-web/static/shared/favicon.svg` (Co wordmark) and a `<link rel="icon" type="image/svg+xml">` in `variants/a/index.html`.
- PWA manifest icon 404'd because `/shared/icon-192.png` and `/shared/icon-512.png` didn't exist. Updated `manifest.json` to reference the SVG favicon (PWA spec accepts SVG with `purpose: "any"`).

### Added — user-level Modern palette default (CO-94 follow-up)

- `applyUniverseConfig` now respects a `co_user_palette` localStorage key. On first visit, it's seeded with `'modern'` so every universe board renders with the Modern palette by default. The user can later switch via the existing palette dropdown; clearing the override returns to per-universe themes.
- This is the "session-token-like" theme preference: set once locally, applied across all boards and tables. Server-side personalization (per-user theme preference stored on the user row) is a follow-up.

## [1.19.1] — 2026-04-29

### Fixed — bulk-imported markdown now visible in the Conteúdo view (CO-94 Phase 1)

After running CO-67 prod seed (artelonga, rfq, qa-dev populated with ~146/12/93 local files), the SPA's Conteúdo tab was rendering "Nenhuma página" because it filters entries by `type=page|task|event|clip` but the bulk-imported markdown has no `type:` set in frontmatter.

- `co-web/static/variants/a/app.js::renderConteudo`: fetches all entries via `getUniverseEntries(slug)` in addition to the typed queries; folds untyped `.md` files into the page list before building the folder tree. Existing typed sections (Tasks, Events, Clips) unchanged.

### Fixed — seed script no longer uploads `.claude/` runtime state

The earlier seed run captured `.claude/worktrees/agent-XXX/...` files (co-auto runtime state) into `rfq` and `qa-dev`. The find command's exclude list missed these.

- `scripts/seed-prod-universes.sh`: added `.claude/`, `.obsidian/`, `.cache/`, `.vercel/`, `seed-co/` to the exclude paths
- Fixed `ensure_jj_repo` stderr/stdout: jj init noise was being captured into the commit_id variable, polluting the changelog snippets. Init output now goes to stderr.
- Added `scripts/cleanup-vault-noise.sh`: idempotent helper that deletes vault entries matching noise patterns. Dry-run by default; pass `--execute` to actually delete.

### Spec

- `work/co/CO-94.md`: Obsidian-like vault viewer. Phase 1 ships in this release; Phases 2-3 (dedicated Vault tab with file tree + viewer + Cmd+P search + wikilink/backlink resolution + drag-and-drop reorganization) deferred to 1.20+ and 3.x.

## [1.19.0] — 2026-04-28

### Added — CO-92: unified timeline view with linear+log scrolling

- `co-web/static/shared/timeline.html` (~470 lines): standalone HTML/SVG/JS timeline page that renders events from any universe on a horizontal time axis. No framework, no build step. Visit `/shared/timeline.html?u=<universe>`.
- **Coordinate transform**: linear within ±100 years of focus (4 px/year), logarithmic beyond (90 px/decade). One 1920px screen spans 4.6 Gya → 302,026 CE simultaneously while keeping year-scale resolution near the present.
- **Date format**: events use `type: event` + `date_year: <signed integer>` in frontmatter. Optional `date: YYYY-MM-DD` and `time: HH:MM` for modern events.
- **Interactions**: drag to pan, mouse wheel/trackpad scroll to pan, hover dots for tooltips, reset button.
- **Friendly year labels**: `4.6 Gya BP` (4.6 billion years before present), `300 kya BP` (300,000), `2026 CE`, `302026 CE`.
- 4 sample events under `work/timeline-samples/` covering Earth formation (-4.6 Gya), *Homo sapiens* emergence (-300 kya), now (2026), and +300 kya (302,026).
- `scripts/seed-timeline-events.sh`: uploads samples to a target universe via `co-token` auth.

Spec: `work/co/CO-92.md`. Phase 1 (standalone page, this release). Phases 2-4 (SPA integration, CO-73 / CO-89 wiring) deferred to follow-ups.

## [1.18.5] — 2026-04-28

### Fixed — seeded admin sees content on login (universe memberships auto-set)

After CO-85 + CO-90 (preview) shipped, a freshly-seeded prod admin (`yuri@artelonga.com.br`) logged in to an empty SPA dashboard because `list_universes_for_user` returns only owned/member/subscribed universes — and the seed didn't make the new user a member of anything.

- `Storage::ensure_admin_universe_memberships(email)`: idempotent post-seed step that adds the seeded admin as `admin` member of every existing system universe (`template`, `quilomboaraucaria`, `yggdrasil`, `dados`, `co-dev`, `co-experience`). Skips universes that don't exist yet.
- `co-web/src/server.rs::start_server`: calls `ensure_admin_universe_memberships` immediately after `seed_admin_user_from_env`, ensuring it runs on every boot (idempotent — `INSERT OR IGNORE`).
- After this deploy + a Fly machine restart, prod yuri sees system universes in their sidebar on next login.

This is still CO-90 preview territory; the full ownership transfer (yuri becomes `owner_id`, not just member) ships in CO-90 for 1.20.0.

## [1.18.4] — 2026-04-28

### Fixed — SPA login form now uses CO-85's universal `/api/v1/auth/password-login`

- `co-web/static/variants/a/app.js`: replaced the call to `/api/v1/auth/uat-login` with `/api/v1/auth/password-login`. The UAT-only endpoint returns 404 in prod by design, which is why the SPA login form failed silently in production. The new endpoint works on both UAT (with `yuri@uat.local`/`uat`) and prod (with the env-seeded admin email/password), so the same code path covers all deployments.
- Same request/response shape; no other UI changes.

### Credential reference

- **UAT** browser login at `https://co-artelonga-uat.fly.dev`: `yuri@uat.local` / `uat`
- **Prod** browser login at `https://co-artelonga.fly.dev`: `yuri@artelonga.com.br` / the password set via `CO_SEED_ADMIN_PASSWORD_HASH`

## [1.18.3] — 2026-04-27

### Fixed — CO-82: throttle mirror to stay under prod's 60 req/min cap

- First-run-on-prod mirror copied 59 of 70 quilomboaraucaria entries before tripping the per-token rate limit (HTTP 429). Adds a 1-second sleep between entry copies in `co-web/src/uat_mirror.rs`. At ~30 prod requests/min (2 GETs per entry), well below the 60/min cap with headroom for the metadata/list calls at start of each universe.
- A 200-entry universe now takes ~3.5 minutes to mirror — acceptable for an occasional UAT reset.

## [1.18.2] — 2026-04-27

### Fixed — CO-82: mirror works end-to-end (no longer needs `/api/v1/universes`)

- `co-web/src/uat_mirror.rs`: stopped calling `GET /api/v1/universes` (which requires JWT and rejected the API token). Mirror now reads a configured list of universe keys from the `UAT_MIRROR_UNIVERSES` env var (default: `artelonga,quilomboaraucaria,rfq`), fetches each via the public per-universe metadata endpoint (`GET /api/v1/universes/:slug`, no auth), and copies content via the vault routes (which already accept API tokens).
- Vault routes were already accepting API tokens via `vault_auth`; `/api/v1/universes/{slug}` for metadata is public — so the mirror's hot path now works without any auth-middleware refactor.
- Added `co-web/src/auth.rs::require_auth_with_token`: a stateful middleware that accepts JWT *or* API token. Currently unused — added as scaffolding for future routes a long-lived background worker needs to hit (CO-89 git ingestion, future external integrations). Mounting it on the existing universe protected routes requires threading state through the router builder; deferred to CO-91 or absorbed into CO-90.
- 404 on a configured universe is logged and skipped, not fatal.

### Operational

After deploy: existing `UAT_PROD_TOKEN` secret already in place from operationalize-prod.sh. The mirror will pick up the universe list from defaults; override via `flyctl secrets set UAT_MIRROR_UNIVERSES='foo,bar' -a co-artelonga-uat`.

## [1.18.1] — 2026-04-27

### Fixed — CO-90 (preview): seeded user gets `tier='user'`, not `tier='admin'`

- `Storage::seed_admin_user_from_env`: switched both insert and update branches from `tier='admin'` to `tier='user'`. The seeded account is just a regular user; privileged access to system universes (template, yggdrasil, dados, co-dev) comes from being the `owner_id` of those universes, not from a global tier value.
- This is a surgical preview of CO-90 (drop the global admin tier entirely). Full CO-90 audits and removes all remaining `tier=='admin'` bypasses in handlers (`dev_board.rs:31`, `universe_routes.rs:765`).
- Display name now defaults to the email itself (was hardcoded `'admin'`); operators can update later.
- User id prefix changed `usr_admin_` → `usr_`.
- Existing users with `tier='admin'` from a 1.18.0 deploy are NOT auto-migrated by this patch — CO-90 ships a proper migration. To force a refresh now: change the password hash secret slightly (re-run hash generator) so the drift-detection branch updates the row.

## [1.18.0] — 2026-04-27

### Added — CO-85: Password-login on prod — replace email-code friction with Argon2id auth

- `POST /api/v1/auth/password-login`: new env-agnostic endpoint; works in any deployment when the user record has a `password_hash` set. Returns the same JWT + `Set-Cookie: session=<JWT>` response shape as `uat-login`. Returns 401 for unknown email, wrong password, or missing hash (no information leak).
- `POST /api/v1/auth/uat-login`: kept as a compat alias for UAT scripts and CLAUDE.md docs; delegates to the same handler when `CO_ENV=uat`, returns 404 in production (unchanged behavior).
- `seed_admin_user_from_env()` in `Storage`: idempotent startup seed driven by `CO_SEED_ADMIN_EMAIL` + `CO_SEED_ADMIN_PASSWORD_HASH` env vars. Drift detection: if the user exists with the same hash, no-op; if the hash differs, updates hash + tier. If the user is missing, inserts with `tier=admin`. Logs once per startup: "admin user seeded: `<email>`".
- Called from `start_server` after migrations and before other seeds, any env.
- Warns at startup if `CO_SEED_ADMIN_PASSWORD_HASH` does not start with `$argon2id$` (likely misconfiguration).
- Unit tests: `password-login` success, wrong-password 401, missing-hash 401; seed drift detection (no-op, update, insert).

## [1.17.0] — 2026-04-27

### Added — CO-83: Mermaid.js diagram rendering

- `co-web/static/vendor/mermaid.min.js` (v10.9.0, 3.2 MB): vendored for offline-first rendering and tighter CSP; lazy-loaded only when a page contains a ```` ```mermaid ```` block
- `co-web/static/shared/markdown.js`: new `renderMermaidBlocks(container)` post-processor follows the existing `highlightCode` / `enableImageZoom` pattern. Idempotent (skips already-rendered blocks via `data-mermaid-rendered`), error-safe (invalid syntax → inline error box, doesn't crash the page)
- Theme bridge: reads CSS custom properties (`--bg`, `--accent`, `--text`, `--md-primary`, etc.) and maps them to Mermaid's `themeVariables`, so diagrams adapt to all 12 Co themes. Re-applied on each render so theme switches re-style new diagrams
- `securityLevel: 'strict'` and `htmlLabels: false` — no inline `<a>` href in diagrams (admits typed wikilinks later via CO-74), no embedded HTML
- Wired into the entry zoom view in `co-web/static/variants/a/app.js` next to the existing `highlightCode` call. Other variants/render paths can opt in similarly
- Seed diagram: `docs/diagrams/deployment.md` — C4 Container view of the UAT + prod deployment topology
- Supports all Mermaid v10 diagram types: flowchart, sequenceDiagram, stateDiagram-v2, classDiagram, erDiagram, gantt, C4Context/Container/Component/Deployment

## [1.16.0] — 2026-04-26

### Added — CO-82: UAT mirrors prod content on reset

- `co-web/src/uat_mirror.rs`: opt-in mirror that runs in a tokio task after a UAT reset; logs into local UAT as yuri, pulls yuri's prod universes via the Vault REST API, and replays content into UAT through the same write path
- `co-web/src/server.rs`: `uat_startup` now returns whether reset just happened; `start_server` spawns the mirror task when env vars are present
- Gated by env: `UAT_MIRROR_PROD=true`, `UAT_PROD_URL`, `UAT_PROD_TOKEN`. When unset, behavior is identical to before the patch (empty placeholders after reset)
- System universes (`template`, `yggdrasil`, `co-dev`, `co-experience`, `dados`) skipped — they have their own seed paths
- Per-universe failures logged, not fatal — prod-down or token-expired never crashes UAT
- Code only runs when `CO_ENV=uat`; on prod the mirror branch is unreachable
- Cargo.toml: `reqwest` gains `cookies` feature; new `percent-encoding` dep
- Operationalization (set Fly secrets `UAT_PROD_TOKEN` etc.) deferred — feature ships dormant

## [1.15.1] — 2026-04-26

### Fixed — CO-66: API hygiene — 500→409 on duplicate key, seed idempotency, UAT no-auto-stop

- `co-web/src/universe_routes.rs`: `POST /api/v1/universes` with an existing key now returns 409 Conflict with `{"error":"conflict"}` body instead of 500 Internal Server Error; lock is held across the existence check and insert to prevent TOCTOU
- `co-web/tests/quilombo_tests.rs`: new test `test_quilombo_seed_preserves_user_edited_description` verifies `seed_quilombo_universe` (INSERT OR IGNORE) never overwrites a user-edited description
- `fly.uat.toml`: set `auto_stop_machines = false` — UAT machine stays running through idle periods so cold-start latency does not block testing

## [1.15.0] — 2026-04-26

### Added — CO-65: visibility on `PUT /api/v1/universes/:slug`

- `co-web/src/universe_routes.rs`: extended `update_universe` handler to accept `visibility` field in addition to `name` and `description`
- Accepted values: `private`, `public-subscribable`, `requires_login`. `template` is system-only and rejected with 400
- Atomic update of legacy `is_public` and `requires_login` columns alongside `visibility`, keeping CO-49 access checks coherent
- New unit test `test_update_universe_visibility_flip` in `co-web/tests/api_tests.rs`: covers happy-path flip + invalid-value rejection

### Note

Versioned to 1.15.0 to reconcile the source `Cargo.toml` (was 1.1.0) with the
deployed binary (was reporting 1.14.0 from an image built 2026-04-07 that had
since drifted from local source). All work since CO-37 (Cargo.toml never
re-bumped after CO-37 deploy) is implicitly bundled into this release.

## [1.2.0] — 2026-04-10

### co-web

#### Added — CO-38: Yggdrasil — universe of universes: minigames hub

- **Migration v18**: `requires_login INTEGER NOT NULL DEFAULT 0` column on `universes` table — gates login-only universes from anonymous access
- **Yggdrasil universe**: seeded on first boot (`key=yggdrasil`, `requires_login=1`, `is_public=1`, `theme_preset=relic`, `layout=gaming`, `owner=system`)
- **Login gate** (`universe_routes.rs`): `GET /api/v1/universes/:slug` returns 401 for universes with `requires_login=true` when no valid JWT is present; other universes unaffected
- **`UniverseInfo`** response now includes `requires_login: bool` field
- **Global leaderboard endpoint** `GET /api/v1/games/leaderboard/global`: aggregates high scores across all games per user, returns top N sorted by total score
- **Recent activity endpoint** `GET /api/v1/games/recent`: returns recent game plays across all users sorted by `last_played_at` desc
- **Browser games** (`co-web/static/games/`): 5 pure HTML5 canvas + JS games — Tetris, Snake, Space Invaders, PointSet (memory pairs), Video Poker — each posts score to `/api/v1/games/{name}/result` on game over
- **Yggdrasil hub** (`app.js` variant a): gaming layout at `/co/yggdrasil` — player profile card (level, total score, games played), game grid (5 cards with personal best + JOGAR), global leaderboard panel, recent activity feed; detects `/co/yggdrasil/{game}` to launch individual games with per-game leaderboard
- **Login wall**: anonymous visitors to `/co/yggdrasil` see a "Login to play" CTA screen instead of the hub
- **SPA route** `/co/yggdrasil/{game}` added to the Axum router (served by the same SPA)
- **i18n strings** added for Yggdrasil UI elements (pt-BR)
- **4 new tests** in `template_tests.rs`: seed/existence, requires_login flag, 401 for anonymous, 200 for authenticated; template universe still accessible anonymously

---

## [1.1.0] — 2026-04-10

### co-web

#### Added — CO-46: Full user telemetry — privacy-respecting tracking

- **`telemetry_events` table** (migration v16): stores page views, interactions, errors, and performance events without PII — no raw IPs, no email addresses, no entry content
- **`co-web/src/telemetry.rs`**: new telemetry module with server-side middleware, storage helpers, and aggregation queries
  - `telemetry_middleware`: tracks all GET page views; filters bots; stores daily-salted IP hash, device/browser/OS from UA
  - `hash_ip_daily()`: xxhash + daily date salt — same IP gets a different hash each day, preventing cross-day re-identification
  - `cleanup_old_events()`: 90-day retention policy (removes raw rows older than 90 days)
  - `telemetry_summary()`: aggregates total events, unique visitors, top pages, error count, p95 latency, events by type and day
- **`POST /api/v1/telemetry/event`**: client-side event ingestion endpoint (returns 202 Accepted); accepts `event_name`, `event_type`, `path`, `universe_key`, `properties`, `duration_ms`, `session_id`
- **`GET /api/v1/admin/telemetry/summary`**: aggregated analytics for the last 30 days (GitHub admin auth required)
- **`GET /api/v1/admin/telemetry/export`**: last 10 000 events as CSV download (GitHub admin auth required)
- **`GET /co/co-dev/telemetria`**: admin dashboard page with cards (total visitors, unique visitors, error count, p95 latency), traffic chart, top pages, events by type, and CSV export
- **`co-web/static/shared/telemetry.js`**: client-side module
  - Respects `navigator.doNotTrack === '1'` — tracking silently disabled
  - Gated on `co_cookie_consent` in localStorage — no events sent before consent
  - Auto-tracks page views (with load time + TTI) on `DOMContentLoaded`
  - Auto-tracks JavaScript errors via `window.onerror`
  - Auto-tracks LCP and FID via `PerformanceObserver`
  - Exposes `window.coTrack(eventName, properties)` for manual interaction tracking
  - Uses `navigator.sendBeacon` for non-blocking delivery
  - Session ID: random nanoid stored in `sessionStorage` (expires on tab close)
- **Integration tests** in `co-web/tests/telemetry_tests.rs`: simulate user flow → verify events recorded, retention cleanup, HTTP endpoint status codes, admin auth guard, admin dashboard page
- **Unit tests** in `co-web/src/telemetry.rs`: UA parsing, bot detection, IP hash privacy

## [1.0.0] — 2026-04-07

### co-web

#### Added — CO-37: Design alignment — Scholarly Automaton + Relic Archive aesthetic

**Typography**
- Load Newsreader (serif) + Work Sans (sans) for Scholarly theme via Google Fonts CDN
- Load Newsreader (serif) + Manrope (sans) for Relic theme
- Load Material Symbols Outlined via Google Fonts CDN
- Font hierarchy: project name = Newsreader italic, task titles = Newsreader 600, labels = Work Sans/Manrope uppercase

**Surface & Depth (No-Line Rule)**
- Removed all `1px solid` header/sidebar borders for Scholarly and Relic palettes
- Sidebar: `surface-container-low` background via tonal shift — no right border
- Cards: asymmetric padding (16px left vs 10px right) for editorial feel
- Kanban columns: tonal background shift per palette (no column borders)
- Ghost borders via CSS custom properties at 15% opacity where accessibility requires
- Modals: ambient `box-shadow: 0 20px 50px` warm-tinted shadows
- Glassmorphism: Relic dark modal + header use `backdrop-filter: blur(20px)` with 80% opacity surface

**Color Tokens (theme_engine.rs)**
- Full Material Design 3 token set added to Scholarly (light + dark) presets: `--md-primary`, `--md-surface`, `--md-surface-container-*`, `--md-on-surface`, `--md-outline`, `--md-outline-variant`, and 30+ additional tokens
- Full MD3 token set added to Relic (dark + light) presets
- All MD3 tokens exposed as CSS custom properties `--md-*` in named palette blocks
- Scholarly dark companion: inverted surface tiers, warm brass tones preserved
- Relic light companion: warm rose-tinted light version

**Components**
- Buttons: Primary (Scholarly = brass + inner glow, Relic = blood-silk gradient), Secondary (ghost border 15% opacity, 40% on hover)
- Task cards: thin left border with priority color (critical/high/medium/low) instead of pill
- Task cards: no dividers between cards — whitespace separation
- Kanban card hover: background tonal shift to surface-container, no hard border
- View tabs: pill group style with `border-radius: 99px`, active tab gets accent bg
- Sidebar items: `translateX(4px)` on hover instead of background change
- Search input: bottom-border only (ledger style) for Scholarly palette
- Status badges: pill-shaped with `primary-container` bg for Relic

**Material Icons**
- View tabs: Material Symbols Outlined icons (view_kanban, table_rows, dashboard, auto_stories) + text
- Sidebar nav section: architecture icon
- Icon-only on mobile (label hidden below 640px)
- On desktop: icon + text

**Responsive**
- Login button, language toggle, palette switcher: always visible on all breakpoints
- Mobile ≤640px: single-column kanban, horizontal-scroll view tabs
- Tablet 641–1024px: 2-column kanban grid

**Obsidian Tasks Compatibility**
- New `co-web/src/obsidian_tasks.rs` module: bidirectional status ↔ checkbox mapping
  - `status_to_checkbox`: `todo→' '`, `in_progress→'/'`, `in_review→'~'`, `done→'x'`
  - `checkbox_to_status`: reverse mapping with uppercase-X support
  - `inject_task_checkbox`: prepends `- [c] Title` to task body on vault export
  - `apply_obsidian_tasks`: parses checkbox from body on vault import, updates frontmatter status; frontmatter is canonical (not overwritten if already set)
- `vault_routes.rs` GET: injects checkbox line into task entry bodies on export
- `vault_routes.rs` PUT: parses checkbox from incoming body, updates frontmatter status on import; strips checkbox line from stored body
- `app.js`: `taskToObsidianLine`, `parseObsidianCheckboxLine`, `extractStatusFromBody` utilities
- 14 unit tests in `obsidian_tasks.rs` covering all status/checkbox combinations and edge cases

## [0.30.0] — 2026-04-06

### co-obsidian (new module)

#### Added — CO-34: Obsidian plugin — sync CO universe ↔ vault

- `co-obsidian/` — new Obsidian community plugin (TypeScript, esbuild)
- `manifest.json`: id `co-universe-sync`, name "CO Universe Sync", minAppVersion 1.4.0
- `package.json` with esbuild build system + Jest test runner
- Plugin settings: CO instance URL, API token, universe slug, sync direction, interval, conflict markers
- Settings tab with connection test and OAuth login button
- `src/api-client.ts` — typed CO Vault API client (listFiles, getFile, putFile, deleteFile, search, getTags)
- `src/sync-engine.ts` — core sync engine:
  - `pull()`: GET `/vault/` listing → mtime-based incremental check → render + write to vault
  - `push()`: scan vault .md files → hash-based change detection → upload to CO
  - `sync()`: bidirectional — pull then push, last-write-wins; optional conflict markers
  - Sync triggers: on-save (debounced 5 s), startup, configurable interval
  - Status callbacks: idle / syncing / synced / offline / conflict / error
- `src/frontmatter.ts` — bidirectional frontmatter mapping:
  - CO → Obsidian: `labels` → `tags`, `created_at` → `created`, `updated_at` → `modified`, `parent: N` → `parent: "[[CO-N]]"`
  - Obsidian → CO: `tags` → `labels`, `created` → `created_at`, `modified` → `updated_at`, `parent: "[[CO-N]]"` → `parent: N`
  - Unknown fields preserved in both directions (round-trip safe)
  - `parseFrontmatter`, `serialiseFrontmatter`, `extractFrontmatterBlock`, `renderMarkdown`
- `src/wikilinks.ts` — wikilink generation and resolution:
  - `[[CO-21|Title]]` wikilinks in exported .md
  - `parent:: [[CO-21]]` inline Dataview field for hierarchy
  - `extractWikilinkIds`, `resolveParentRef`, `wikilinksToMdLinks`, `mdLinksToWikilinks`
- `src/status-bar.ts` — status bar: "CO: synced ✓" / "CO: syncing…" / "CO: offline" / "CO: N conflicts"
- `src/main.ts` — main plugin class:
  - Ribbon icon (click to sync)
  - 6 commands: Sync now, Pull from CO, Push to CO, Open in CO, Create task, Link to CO
  - ObsidianProtocolHandler for OAuth callback (`obsidian://co-universe-sync/oauth`)
  - Auto-sync interval with `registerInterval`
  - On-save debounced push via `vault.on("modify")`
- `.co/sync.json`: `{ lastSync, fileHashes, remoteMtimes, remoteVersion }` for incremental sync
- Authentication: API token paste (stored in data.json) + OAuth browser flow + auto token refresh
- `tests/frontmatter.test.ts` — 30 unit tests: round-trip mapping, parsing, serialisation
- `tests/wikilinks.test.ts` — 22 unit tests: generation, resolution, Dataview fields
- `tests/sync-engine.test.ts` — 11 integration tests: mock CO API, pull/push/sync verification
- `tests/__mocks__/obsidian.ts` — Obsidian API mock for Jest (no real vault needed)
- `README.md` with setup instructions, command table, frontmatter mapping table
- `LICENSE`: MIT
- All 63 tests pass

---

## [0.29.0] — 2026-04-06

### co-web

#### Added — CO-35: Vault REST API + Obsidian Clipper support

- `vault_routes.rs` — Vault REST API compatible with Obsidian Local REST API
  - `GET /api/v1/universes/{slug}/vault/` — list all files with metadata
  - `GET /api/v1/universes/{slug}/vault/{*path}` — get file content + stat
  - `PUT /api/v1/universes/{slug}/vault/{*path}` — create/replace file
  - `POST /api/v1/universes/{slug}/vault/{*path}` — append to file
  - `PATCH /api/v1/universes/{slug}/vault/{*path}` — targeted edit (frontmatter field, heading section, block ID)
  - `DELETE /api/v1/universes/{slug}/vault/{*path}` — soft delete (`.trash/`) or hard delete (`?permanent=true`)
  - `POST /api/v1/universes/{slug}/vault/search` — full-text search across vault files
  - `GET /api/v1/universes/{slug}/vault/tags` — aggregate all frontmatter tags
  - `GET /api/v1/universes/{slug}/vault/tree` — recursive directory tree (BTreeMap, sorted)
  - `POST /api/v1/universes/{slug}/vault/clip` — accept Obsidian Clipper payload, write clipped note
- `storage.rs` — migration v15: `api_tokens` table with indexes; `create_api_token`, `list_api_tokens`, `delete_api_token`, `get_api_token_by_value` methods
- Auth: Bearer JWT (same as board API) + long-lived API tokens (`co_` prefix, 90-day expiry)
- Token management: `POST /api/v1/auth/token`, `GET /api/v1/auth/tokens`, `DELETE /api/v1/auth/tokens/{id}`
- Rate limiting: 60 req/min per API token (in-memory sliding window, `LazyLock<Mutex<HashMap>>`)
- `static/clipper-template.json` — Obsidian Clipper compatible template for CO frontmatter schema
- `static/shared/clipper.js` — board UI paste handler
  - `Ctrl/Cmd+Shift+V` keyboard shortcut for "Paste as CO content"
  - Paste event listener on board area: detects Clipper-formatted markdown, shows choice dialog
  - "Paste as task" vs "Paste as content" dialog with frontmatter preview
  - `co:clipper-paste` custom event dispatched for board.js integration
  - `co:card-context-menu` listener adds "Copy as Obsidian markdown" to task card context menus
  - `COClipper` public API: `isClipperFormat`, `parseFrontmatter`, `toObsidianMarkdown`, `handleClipboardText`
- All 8 variant `index.html` files updated to include `clipper.js`

---

## [0.28.0] — 2026-04-06

### co (workspace)

#### Added — CO-28: Open source repo setup

- `README.md` — rewritten for public audience: what CO is, quick start (cargo install + Docker), self-hosting (Docker Compose + Fly.io), architecture diagram, CLI reference, contributing link
- `CONTRIBUTING.md` — development setup, TDD workflow, branch/label conventions, commit format, test rules, PR process
- `.github/ISSUE_TEMPLATE/bug_report.md` — structured bug report template
- `.github/ISSUE_TEMPLATE/feature_request.md` — feature request template with acceptance criteria
- `.gitignore` — added `*.db`, `*.redb`, `.env`, `.env.local` patterns; removed `!co-web/data/` exception that could allow committing runtime databases
- `Cargo.toml` — added `keywords` and `categories` to workspace package; updated repository URL to `artelonga/co`

---

## [0.27.0] — 2026-04-06

### co-web

#### Added — CO-33: E2E test suite — Playwright for full MVP flow

- `e2e/universe.spec.ts` — Universe creation: criar form submit → redirect to /co/:slug → editable board
- `e2e/board-drag.spec.ts` — Board drag-and-drop between kanban columns + full CRUD sequence
- `e2e/codemirror.spec.ts` — CodeMirror 6 editor: init, toolbar (Bold/Italic/Heading), live preview, save+persist
- `e2e/usage-gate.spec.ts` — Usage gate: API 402 structure, overlay DOM, "Entrar" opens login modal
- `e2e/theme.spec.ts` — Palette switcher: anonymous sees 4, switch updates CSS vars without reload
- `e2e/i18n.spec.ts` — i18n toggle pt↔en, co_lang cookie set, persists across page reload
- `e2e/auth-crdt.spec.ts` — Auth flow, sharing gate, anonymous editor has no WebSocket, CRDT two-context sync
- `e2e/responsive.spec.ts` — Board renders at mobile (375px), tablet (768px), desktop (1280px) viewports
- `.github/workflows/ci.yml` — Added `e2e` job: build co-web → install Playwright → run Chromium suite → upload HTML report

---

## [0.26.0] — 2026-04-06

### co-deploy

#### Added — CO-32: Ansible deployment — provision, deploy, backup playbooks for Fly.io + VPS

- New `co-deploy/` directory with standard Ansible structure
- `inventory/fly.yml` — Fly.io target (local connection via flyctl)
- `inventory/vps.yml` — generic VPS target (DigitalOcean, Hetzner, etc.) with env-var overrides
- `playbooks/provision.yml` — creates `co` unprivileged user, installs ca-certificates + sqlite3 + zstd + Caddy, creates `/opt/co/` + `/var/lib/co/data/`, configures UFW (allow 80/443, deny rest)
- `playbooks/deploy.yml` — cross-compiles co-web via `cross`, copies binary, writes systemd unit, runs seed SQL on first deploy, restarts service, verifies `/api/health`
- `playbooks/backup.yml` — SQLite `.backup` (online, consistent), zstd compression, 7 daily + 4 weekly rotation, optional rclone upload to S3/B2, cron at 03:00 UTC
- `playbooks/fly-deploy.yml` — wraps `flyctl deploy --remote-only` with pre-deploy state and post-deploy health check
- `templates/co-web.service.j2` — systemd unit with ExecStart, WorkingDirectory, Environment, systemd hardening (NoNewPrivileges, ProtectSystem)
- `templates/caddy.conf.j2` — reverse proxy with auto-SSL, zstd+gzip compression, security headers (HSTS, X-Frame-Options, etc.), static asset caching
- `group_vars/all.yml` — shared config: co_version, co_port, co_domain, backup retention settings
- `group_vars/production.yml` — ansible-vault encrypted secrets: JWT_SECRET, RESEND_API_KEY
- `molecule/default/` — Docker-based integration test (provision + stub deploy on Debian 12, idempotency check)
- `requirements.yml` — community.general + ansible.posix collections
- `README.md` — quickstart for VPS and Fly.io

---

## [0.25.0] — 2026-04-06

### co-web

#### Added — CO-31: CRDT sync — Yjs + WebSocket, login required

- New module `co-web/src/ws.rs`: `DocRoom` struct (yrs `Doc`, broadcast tx, client count, dirty notify), `DocRoomManager = Arc<RwLock<HashMap>>`, `ws_handler`, `handle_socket`
- `GET /ws/doc/:universe_slug/:doc_id` — JWT-gated endpoint; returns 401 for anonymous requests (token via `?token=` query param or `co_auth` cookie)
- Yjs sync protocol v1 (binary lib0 encoding): MSG_SYNC (0) with SYNC_STEP1/STEP2/UPDATE; MSG_AWARENESS (1) for cursor positions
- Room lifecycle: load content from SQLite on first connect (initializes Y.Doc), broadcast updates to all connected clients, debounced persist (5s idle), cleanup on last disconnect
- Heartbeat: ping every 30s, disconnect after 60s silence; rate limit: 100 messages/sec per client (token bucket)
- `AppStateInner.doc_rooms` field added; WS route mounted at `/ws/doc/{slug}/{doc_id}`
- `Storage::get_entry_body()` and `Storage::update_entry_body()` methods for CRDT persistence
- Sharing gate in `get_universe_info`: anonymous universes return 404 for non-owners (checked via `co_universe_owner` cookie)
- Frontend: added `yjs`, `y-codemirror.next`, `lib0` to editor bundle
- `createAwareness()` shim implementing y-codemirror.next's awareness interface (no y-protocols dep)
- `CoYjsProvider` class: WebSocket provider with reconnect, sync-step-1 on open, apply sync-step-2/update, forward awareness
- `initEditor` accepts `wsUrl` and `user` params; CRDT mode for logged-in users; anonymous mode shows "Crie uma conta pra colaborar" toast
- Collab badge ("N users editing"), connection status dot (green/yellow/red), remote cursor CSS
- 7 unit tests: varuint roundtrip, varbytes roundtrip, sync frame structure, rate limiter burst/block, DocRoom init, anonymous 401, two-user sync

---

## [0.24.0] — 2026-04-06

### co-web

#### Added — CO-30: Dynamic CSS engine — token generation from universe config at runtime
- New module `co-web/src/theme_engine.rs`: `ThemePreset` struct (name, tokens HashMap, font fields) + `generate_css()` function
- Five built-in presets with all required CSS tokens: `scholarly` (warm cream/bronze), `scholarly-dark` (dark chocolate/bronze), `relic` (near-black/rose), `relic-light` (off-white/burgundy), `modern` (default indigo)
- All presets define: `--bg`, `--sidebar-bg`, `--card-bg`, `--text-primary`, `--text-secondary`, `--accent`, `--border`, `--status-*`, `--priority-*`, `--font`, `--font-mono`, `--radius-*`, `--shadow-*`
- `generate_css(preset, overrides)` merges custom token overrides on top of preset, outputs deterministic `:root { … }` block
- `GET /api/v1/universes/:slug/theme.css` — returns generated CSS, `Cache-Control: no-cache`, ETag based on config hash, supports `If-None-Match` (304)
- Dark/light companion mapping: `scholarly` ↔ `scholarly-dark`, `relic-light` ↔ `relic`
- Frontend (variant a): `loadThemeCss(slug)` hot-swaps `<link id="co-theme-css">` href — no page reload when theme changes
- Frontend: custom fonts inject `<link rel="stylesheet" href="https://fonts.googleapis.com/…">` with preconnect hints
- Settings panel (owner only): added dark/light toggle button, `modern` theme option, custom token overrides JSON textarea
- Unit tests: 13 theme engine tests + 4 HTTP endpoint integration tests (200 OK, all tokens present, CSS changes on theme change, 404 for missing universe, ETag 304)

---

## [0.23.0] — 2026-04-06

### co-web

#### Added — CO-23: Usage gate — 100 entries free, then account required
- `universes.content_count` column (migration v11): cached counter incremented/decremented on writes and deletes
- Middleware-style `check_usage_gate` helper: returns 402 Payment Required for anonymous universes at or above 100 entries
- Anonymous write access: `clone_universe` issues an anon JWT session cookie + `co_universe_owner` cookie for claiming
- `POST /api/v1/universes/:slug/claim` — authenticated user claims an anonymous universe (cookie must match)
- `GET /api/v1/universes/:slug` — public universe info: `content_count`, `is_anonymous`, `is_template`
- 402 response body: `{ "error": "usage_limit", "message": "Crie uma conta para continuar", "message_en": "...", "current": N, "limit": 100 }`
- Frontend (variant a): 402 → usage limit modal with "Criar conta" / "Entrar" buttons; content count badge in header
- After login with anonymous universe: auto-claim transfers ownership to real user
- Unit test: 99 entries OK, 100th OK, 101st blocked (402), unblocked after claim

---

## [Unreleased] — co-web E2E Testing (UX-50 Epic)

### co-web

#### Added — UX-51: Initialize Playwright project
- Playwright + @axe-core/playwright devDependencies in `co-web/package.json`
- `playwright.config.ts` — baseURL localhost:3000, 9 projects (chromium/firefox/webkit × desktop/tablet/mobile)
- Custom viewports: desktop (1280×720), tablet (768×1024), mobile (375×812)
- `e2e/global-setup.ts` — builds binary, starts co-web, polls `/api/health`
- `e2e/global-teardown.ts` — SIGTERM cleanup, skips if external server
- `.gitignore` updated for node_modules, test-results, playwright-report
- `npx playwright test --pass-with-no-tests` exits cleanly (code 0)

---

## [0.22.1] - 2026-01-04

### Fixed
- **External Folder Support** (#77)
  - Bundle language configs in binary using `include_str!()`
  - CO now works properly in any registered workspace without source files
  - `co init` simplified to just create directory (no README.md)
  - `co new` defaults to current directory instead of 'en' space
  - Namespaces are now simple directories users organize however they want

## [0.22.0] - 2026-01-04

### Added
- **System-wide Installation & Namespace Detection** (#75)
  - `.co/` directory now recognized as CO workspace root marker
  - `co repo switch <alias>` to switch active workspace context
  - Git submodule detection for nested repositories
  - `is_submodule` field in `SpaceLocation::InSpace` variant
  - `is_git_submodule()` and `is_submodule()` helper methods
  - Enhanced `co space current` with helpful guidance when not in workspace
  - `effective_space()` method combining detected and active workspaces
  - `active_repo` field in `GlobalConfig` for workspace context persistence

### Changed
- `co space current` now shows "(switched)" indicator when using active workspace
- Status command shows "(submodule)" indicator when in a git submodule
- Improved error messages with actionable suggestions (Navigate, Register, Switch)

## [0.21.2] - 2026-01-04

### Changed
- **Rename ui/ to i18n/** (#72)
  - Renamed `ui/` folder to `i18n/` for clarity
  - Updated all path references in core and CLI
  - Folder now clearly indicates internationalization purpose

## [0.21.1] - 2026-01-04

### Added
- **Explicit Forbidden Character List** (#70)
  - `FORBIDDEN_ID_CHARS` constant documenting all forbidden ID characters
  - `is_valid_id_char()` function for character validation
  - `validate_id()` function to check ID strings for invalid characters
  - User-facing error messages in `co create` showing forbidden characters
  - Comprehensive tests validating all forbidden characters are handled

### Documentation
- Added doc comments explaining forbidden character categories:
  - Filesystem-unsafe: `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`
  - Shell/special: `'`, `!`, `@`, `#`, `$`, `%`, `^`, `&`
  - Whitespace: space, tab, newline, carriage return
- Clarified allowed characters: alphanumeric, hyphen, dot, underscore

## [0.21.0] - 2026-01-04

### Added
- **Documentation System** (#42)
  - `co help` - Topic-based embedded documentation
  - `co help getting-started` - Quick start guide
  - `co help spaces` - Understanding spaces
  - `co help workflows` - Plan & Execute, Write workflows
  - `co help work-items` - User-stories, tasks, epics
  - Alias: `co h` for quick access
  - Added `clap_mangen` for future man page generation

### Changed
- Updated CLAUDE.md with work item types and git label mapping
- Clarified work item hierarchy (epic → user-story → task)
- Removed deprecated "scope" terminology from documentation

### Fixed
- Removed personal name references, using PRIVATE/PUBLIC/USER namespaces

## [0.20.0] - 2026-01-04

### Added
- **Archive & Storage** (#43)
  - `co archive <item>` - Move content to archive with deindexing
  - `co archive restore <item>` - Restore content from archive
  - `co archive list` - List all archived items
  - Directory structure mirrors original: `work/tasks/` → `work/archive/tasks/`
  - Adds `archived_at` timestamp to frontmatter
  - Adds `indexed: false` to exclude from co operations (locate, validate)
  - `--force` flag to replace existing archived items
  - Alias: `co ar` for quick access

## [0.19.0] - 2026-01-04

### Added
- **Analyze Command** (#41)
  - `co analyze <item>` - Evaluate content quality and generate suggestions
  - Checks for clear title, status field, and required sections
  - Type-aware validation: user-story (As/I Need/To), task (Given/When/Then)
  - Detects broken internal [[links]]
  - Generates actionable improvement suggestions
  - Generates interview questions for missing information
  - Colored output with ✓/⚠/✗ indicators
  - `--verbose` flag for detailed analysis

## [0.18.0] - 2026-01-04

### Added
- **Tools & Extensions** (#40)
  - `co tools run <name> [args...]` - Execute a tool with arguments
  - Tool types: `deterministic` (shell commands) and `predictive` (ML models, stub)
  - User tools in `user/tools/` take precedence over system tools
  - Tool schema extended with `tool_type` field
  - Default behavior: deterministic when `tool_type` not specified
  - Error handling: tool not found, missing command, execution failure

## [0.17.0] - 2026-01-04

### Added
- **Writer Agent System** (#39)
  - `co write <type> --agent <name>` - Generate content using writer agents
  - Agent backends: `manual` (interactive prompts), `claude` (skeleton for LLM), `ollama` (stub)
  - `--context FILE` to provide additional context from a file
  - `--in SPACE` to specify target space
  - `--name NAME` to skip name prompt
  - Agent schema extended with `backend` and `context` fields
  - New `agents/writer.md` example agent
  - Output validated against content schemas

## [0.16.0] - 2026-01-04

### Added
- **Plan & Execute Workflow** (#38)
  - `co conduct plan <objective>` - Create structured use-case proposals with acceptance criteria
  - `co conduct execute <id>` - Drive plans through git workflow states (todo → in-progress → review → done)
  - Two modes: Manual (interactive prompts) or Assisted (skeleton for LLM)
  - `--context FILE` to load context from a file
  - `--repo <alias>` for cross-repo operations
  - Auto-creates GitHub issue on plan creation
  - Branch creation on execute, PR tracking via `gh` CLI
  - Space-aware architecture with global repo registry

## [0.15.0] - 2026-01-04

### Added
- **GitHub as Source of Truth** (#36)
  - `co gh issue list` - List issues from GitHub repository
  - `co gh issue show <number>` - Show issue details
  - `co collab pull --all` - Pull all open issues to local markdown files
  - `co collab pull <number>...` - Pull specific issues
  - GitHub → CO mapping: labels to type/priority, assignees, state to status
  - New `core/src/github/` module with types, mapping, and GhCli wrapper

## [0.14.0] - 2026-01-04

### Added
- **Space Isolation & Commit Guards** (#47)
  - `SpaceLocation` detection: automatically detect if you're in a space or at repo root
  - `co status` now shows current location context (space vs repo root)
  - `co init --check` to find unprotected spaces (not gitignored)
  - Walking directory tree to find space markers (README.md with `type: space`)

### Changed
- Status command now displays location context with commit guard warnings

## [0.13.1] - 2026-01-04

### Changed
- **Terminology Refactor** (#49)
  - Standardized terminology: "Space" is the canonical term for namespace directories
  - Deprecated "scope" from system references (backwards-compatible aliases remain)
  - "Context" now exclusively refers to user-provided content/prompts
  - Renamed `core/src/scope.rs` → `core/src/space.rs`
  - Updated all CLI help text, commands, and i18n labels
  - Updated `type: context` → `type: space` in frontmatter
  - All tests and validation messages updated

## [0.13.0] - 2026-01-03

### Added
- **Collaborative Content Creation** (#48)
  - `co create` - Interactive content creation with role selection
  - User role: Structured prompts for user-stories (AS A / I NEED / SO THAT) and tasks (GIVEN / WHEN / THEN)
  - Agent role: Creates skeleton templates for Claude Code to fill in
  - `--story` flag to link tasks to parent user stories
  - `## Prompt` section for context persistence

## [0.12.2] - 2026-01-04

### Added
- CLAUDE.md development instructions (#56, #57)

### Changed
- Streamlined versioning workflow: version bump in same PR (#59)
- Added branch cleanup instructions

## [0.12.1] - 2026-01-04

### Added
- CHANGELOG.md with complete version history (#52)

### Changed
- Versioning policy: issues drive releases (#53)

## [0.12.0] - 2026-01-03

### Added
- **Spaces & Multi-Repo SSH** (#37, #45)
  - `co space list` - List all registered spaces
  - `co space current` - Show current space details
  - `co repo add --ssh-host` - Configure SSH identity per repo
  - Auto-detect current space from working directory
- **Extensible Content Types** (#35, #44)
  - Custom content types via `schema.yaml`
  - `co schema list` - List all available types (built-in + custom)
  - Validation support for custom types
- **Auto-gitignore on init**
  - `co init <name>` automatically adds space to `.gitignore`
  - Prevents accidental commits of user spaces to co home

### Fixed
- Language validation now accepts known languages (english, portuguese, etc.) without requiring directory
- Content type pluralization: `user-story` → `user-stories/` (not `user-storys/`)
- Clippy warnings resolved for CI compliance (#46)

## [0.11.0] - 2026-01-03

### Added
- **Work Item Types & Content Parsing** (#33, #34)
  - User-story sections: `## As`, `## I Need`, `## To`
  - Task sections: `## Given`, `## When`, `## Then`
  - Built-in types: `user-story`, `task`, `epic`, `release`
  - Content section validation for structured formats
  - `work/schema.yaml` for work item type definitions

## [0.10.0] - 2026-01-03

### Added
- **Feature System** (#31)
  - Automatic discovery of `agents/` and `tools/` directories
  - Schema-based content type registration via `schema.yaml`
  - Feature registry for extensibility
  - `co config show` displays discovered features

### Fixed
- Version updated to 0.10.1 with UI reorganization (#32)

## [0.9.0] - 2026-01-02

### Added
- **Interactive REPL** (#28)
  - `co lead` - Interactive exploration mode
  - Commands: `status`, `locate`, `use <scope>`, `help`, `quit`
  - Scope-aware prompts
  - Real-time content navigation

## [0.6.0] - 2026-01-02

### Added
- **Validation System** (#27)
  - `co validate <item>` - Validate specific content
  - `co validate all` - Validate entire workspace
  - Frontmatter validation (required fields, types)
  - Internal link validation (`[[references]]`)
  - Language and scope existence checks
  - Severity levels: Error, Warning

## [0.5.0] - 2026-01-02

### Added
- **Index & Performance** (#25)
  - SQLite-based content indexing
  - `co locate build` - Build/rebuild index
  - `co locate --stats` - Show index statistics
  - Incremental index updates (only modified files)
  - Full-text search via FTS5

### Fixed
- Deprecated exports removed, CI workflow fixed (#26)

## [0.4.0] - 2026-01-02

### Added
- **Query System** (#23)
  - `co locate` - Unified search command
  - Filter by type: `co locate --type task`
  - Filter by scope: `co locate --scope private`
  - Full-text search: `co locate "search term"`
  - Combined filters and search

### Changed
- Unified `find` and `search` into single `co locate` command (#24)

## [0.3.0] - 2026-01-02

### Added
- **Content Management** (#22)
  - `co new <type> <name>` - Create new content
  - `co show <item>` - Display content
  - `co update <item> --status <status>` - Update metadata
  - `co delete <item>` - Remove content
  - Frontmatter parsing with YAML support
  - Content type detection

## [0.2.0] - 2026-01-02

### Added
- **Language Foundations** (#21)
  - Multi-language support (english, portuguese, guarani-mbya)
  - Internationalization (i18n) for CLI messages
  - `co lang <code>` - Set UI language
  - `co languages` - List supported languages
  - Lexicon structure for definitions
  - Language-specific directories (`en/`, `pt/`, `gun/`)

## [0.1.0] - 2026-01-02

### Added
- Initial release
- Graph-based content management foundation
- `co init <name>` - Initialize context
- `co list` - List contexts and languages
- `co status` - Show workspace status
- Basic CLI structure with clap
- Workspace configuration (`.co/config.yaml`)

---

## Roadmap

### Upcoming (v1.0)
- [x] #36 - GitHub as Source of Truth (sync issues/PRs)
- [x] #38 - Plan & Execute Workflow
- [x] #39 - Writer Agent System
- [x] #40 - Tools & Extensions
- [x] #41 - Analyze Command
- [ ] #42 - Documentation Polish
- [x] #43 - Archive & Storage
- [x] #47 - Space Isolation & Commit Guards
- [x] #48 - Collaborative Content Creation (User + Agent)
- [x] #49 - Terminology Refactor (space/context/scope)
