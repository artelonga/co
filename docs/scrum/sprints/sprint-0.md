# Sprint 0 (2026-05-29 → 2026-06-11)

**Sprint Goal**: (retrospective — inferred from PBIs)
**Release**: v2.40.0
**Velocity**: 39 PBIs delivered

## Delivered PBIs

### CO-363 — Cross-universe wikilink resolver — `[[key::path]]` populates entry_relations.to_universe (#156)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] `extract_relations` parses `[[mbya::terms/jaxy-jatere]]` and writes a row with `to_universe="mbya"`, `to_entry="terms/jaxy-jatere"`.
- [ ] `extract_relations` parses `[[concepts::mother.md|mãe]]` and stores label `"mãe"` in `link_text`.
- [ ] Same-universe `[[terms/x]]` still resolves to `to_universe = from_universe` (no regression).
- [ ] `[[../sibling/x]]` emits a deprecation log line + row with `relation_type="wikilink_relative_deprecated"`.
- [ ] Frontmatter `concept: yoruba::terms/ogunte` populates `to_universe="yoruba"`.
- [ ] Migration backfills cross-universe relations for the existing 11,500 entries.
- [ ] Unit tests cover all 5 forms.
- [ ] `GET /api/v1/universes/mbya/graph?universes=mbya,yoruba` (after CO-345 ships) returns edges connecting the two.

### CO-350 — Catalog → OpenAPI codegen + CI drift check (#155)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] /Users/artelonga/projects/co/co-web/scripts/generate-openapi.ts exists and runs under `node --import tsx`.
- [ ] `npm run openapi:gen` regenerates co-web/openapi.yaml from the catalog without manual edits.
- [ ] `npm run openapi:check` exits 0 when in sync, exits 1 with a readable diff when drifted.
- [ ] CI workflow `openapi-check.yml` runs on PRs that touch any of: `*_routes.rs`, `api-catalog.md`, `openapi.yaml`, the generator script.
- [ ] Bootstrap commit aligns catalog + openapi.yaml so `--check` passes on `main` immediately after merge.
- [ ] A deliberate test PR that adds a new route without a catalog row fails CI.
- [ ] README or `docs/architecture/api-catalog.md` header gains a 3-line "How to add a route" note.

### CO-361 — Atividades audit log + schema_versoes admin surface (#154)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] Migration v18 creates `atividades` table with 3 indexes and CHECK constraints
- [ ] Migration v18 creates `schema_versoes` table and backfills rows for v1..v17
- [ ] `platform::atividade::log_atividade()` exists, deferred-write via `tokio::spawn`, never blocks response
- [ ] `redact()` strips SENSITIVE_KEYS at any JSON depth
- [ ] `sha256_short()` returns 16 hex chars
- [ ] All listed call sites (task CRUD, universe CRUD, login/logout) log atividades
- [ ] `record_migration!` macro used for any new migrations going forward (lint or docs)
- [ ] `GET /api/v1/gestao/atividades` returns paginated feed, admin-only
- [ ] `/gestao` header shows "DB schema vN / app vX.Y.Z" with drift indicator
- [ ] 180-day retention background job deletes old atividades nightly
- [ ] No password/token/secret string appears in any `atividades.conteudo` row (test asserts this on a login flow)
- [ ] Integration test: create task → atividades row appears within 100 ms with correct acao/entidade/after-diff
- [ ] Integration test: schema_versoes contains a row for every entry in schema_version after migration

### CO-345 — Cross-universe graph view + publishable saved views (#153)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] `GraphQuery.universes` accepts comma list; when set, both queries drop the `to_universe IS NULL` filter and union nodes/edges across the set.
- [ ] Node `id` is `universe::path` when multi-universe mode is active; single-universe mode keeps the bare `path` (back-compat).
- [ ] `graph_views` table + 5 CRUD endpoints; integration tests for each.
- [ ] Visibility enforced: `private` requires owner, `unlisted` requires the slug, `public` is open.
- [ ] UI chip strip lists all `public-subscribable` universes; toggling re-renders without page reload.
- [ ] `/graph-views/{slug}` SPA route hydrates filters from the saved view and renders the canvas.
- [ ] E2E test: create a view with `universes=mbya,yoruba`, save, open URL in incognito, assert nodes from both universes render.
- [ ] Existing single-universe behavior unchanged when `?universes` is absent.
- [ ] Sub-task (if needed): `extract_relations` emits `to_universe` for `[[key::path]]` wikilinks; unit test in relation_index.rs.

### CO-364 — Add open-source reference universes — odysseus + claude-code
_Merged: 2026-06-05_
_Release: v2.40.0_

- [x] `odysseus` row exists in `universes` with `remote_url = https://github.com/pewdiepie-archdaemon/odysseus`, `remote_ref = "dev"`
- [x] `claude-code` row exists with `remote_url = https://github.com/anthropics/claude-code`, `remote_ref = "main"`
- [x] `content_subdirs` limits the sync to docs + top-level READMEs/CHANGELOGs
- [x] Boot backfill is idempotent (`WHERE remote_url IS NULL`)
- [x] Both visible at `https://co-artelonga.fly.dev/api/v1/universes/{odysseus,claude-code}` after deploy
- [x] CO-337 background worker clones them within 15 min of first boot

### CO-340 — Per-universe analytics rollups + filterable summary + historical-path bridge (#152)
_Merged: 2026-06-05_
_Release: v2.40.0_

_(no acceptance criteria in spec)_

### CO-339 — Feedback validation — reject empty bodies + probe paths at the API (#151)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] `POST /api/v1/feedback {body: ""}` returns 400 `body_too_short`
- [ ] `POST /api/v1/feedback {body: "ok", entry_path: "/_probe"}` returns 400 `probe_path_blocked`
- [ ] `POST /api/v1/feedback {body: "Found a bug in mbya terms", entry_path: "/yuri"}` succeeds with 201
- [ ] 4th submission from same IP within 1 hour returns 429
- [ ] Migration marks the 16 existing probe rows `wont-fix`
- [ ] No regression in CO-333 widget submission flow

### CO-362 — Markdown render — rewrite http:// asset URLs to https:// to eliminate mixed-content warnings (#150)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] Markdown body containing `![badge](http://img.shields.io/x.svg)` renders as `<img src="https://...">`.
- [ ] Markdown body containing `[link](http://example.com)` keeps `href="http://..."` (links unchanged).
- [ ] Markdown body containing `<script src="http://...">` is dropped or commented out.
- [ ] `/artelonga` page loads with 0 mixed-content warnings in browser console.
- [ ] Unit test covers all 3 transforms.

### CO-279 — Every universe must seed a default project — fix private-universe + template-seed regression (#149)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] `test_template_has_sample_tasks` passes (9 tasks under project `CO`)
- [ ] `test_template_projects_public` passes (returns `key: "CO"`)
- [ ] `test_write_to_template_forbidden` passes (403, not 500)
- [ ] `test_update_template_task_forbidden` passes (403, not 500)
- [ ] New test: create authenticated user → visit private universe → assert default project exists
- [ ] Yuri logs in to UAT/prod → private universe shows a default project, not "no project found"
- [ ] `cargo test --workspace` is fully green
- [ ] CI on `main` returns success

### CO-346 — Fix SPA empty-board mystery — co universe shows no content despite 1227 entries on prod (#148)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] PR description includes the actual root cause (one of the three or a different finding) with browser-side state captured from prod.
- [ ] Anonymous visitor to `/co` on prod sees at least one entry/project/task on the board within 5s of load.
- [ ] Logged-in visitor with their own universes can navigate to `/co` (via URL or sidebar) and stay there — no auto-bounce to their own universe when the URL is explicit.
- [ ] Playwright E2E covers the anonymous-visitor case and passes against UAT.
- [ ] No regression: template universe still loads + redirects logged-in users to their primary universe when URL is `/` (existing behavior).
- [ ] `cargo test -p co-web` + `cargo clippy -- -D warnings` clean.

### CO-347 — Surface missing content universes on prod — yuri / retro-umarizal / yoruba / neuro (#146)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] Four new rows present in prod `universes` table after deploy: yuri, retro-umarizal, yoruba, neuro.
- [ ] Each row has `remote_url`, `remote_ref='main'`, appropriate `content_subdirs`, `anon_published_only` correctly set (1 for yuri, 0 for others).
- [ ] `yoruba.parent_key = 'comunicacao'`; `neuro.parent_key = 'artelonga'`.
- [ ] After first sync cycle (≤15 min), each universe has `remote_last_sync` populated and `content_count > 0`.
- [ ] `GET /api/v1/universes/yuri/entries` (anonymous) returns only entries with `published: true`.
- [ ] `GET /api/v1/universes/yoruba/entries` returns the terms from `~/projects/comunicacao/yoruba/terms/`.
- [ ] Sidebar on the prod SPA lists all four universes for an anonymous visitor.
- [ ] Idempotency: re-running the boot UPDATE does not change rows whose `remote_url` is already set (the `WHERE remote_url IS NULL` guard works).
- [ ] `cargo test -p co-web` clean.

### CO-348 — Mbya promote to first-class + merge yoruba term sources (#147)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] /Users/artelonga/projects/mbya/_universe.yaml exists with the CO-141 schema; mbya repo loads as a first-class CO universe on localhost (`co serve` picks it up via co-universes.yaml or direct subscription).
- [ ] `~/projects/comunicacao/mbya/` removed from comunicacao repo (or replaced with a pointer README); commit pushed.
- [ ] CO-347's seed update bumped: `mbya.remote_url = 'https://github.com/artelonga/mbya'`.
- [ ] Each of the 8 topologia/yoruba terms reviewed; divergences merged into comunicacao/yoruba/terms/ by hand; merge notes in PR description.
- [ ] Topologia/yoruba/terms/*.md removed; topologia/yoruba retains `_universe.yaml` + `index.md` as the shape exemplar (with a comment pointing to the canonical location).
- [ ] After this lands + CO-347 deploys, prod has: `mbya` row syncing from its own repo, `yoruba` row syncing from comunicacao/yoruba subfolder, **no duplicate yoruba content** anywhere.
- [ ] No regression to comunicacao's other content (sources/, docs/, content/).

### CO-349 — Yggdrasil RPG sub-universe scaffolding — 48+ folders, schemas later (#145)
_Merged: 2026-06-05_
_Release: v2.40.0_

- [ ] /Users/artelonga/projects/yggdrasil/content/ exists with 6 category roots × 4 universe folders (parent + 3 languages) = 30 `_universe.yaml` files.
- [ ] 60+ stub markdowns (15+15+10+10+5+5) live in the category roots.
- [ ] /Users/artelonga/projects/yggdrasil/scripts/scaffold-content.sh runs idempotently — re-running creates no new diffs.
- [ ] On localhost, `co serve` against yggdrasil shows the 30 sub-universes in the sidebar (nested under `yggdrasil`).
- [ ] On prod, after the yggdrasil sister-repo sync runs (≤15 min post-deploy), `GET /api/v1/universes?parent=yggdrasil` returns 30 rows.
- [ ] PR includes a screenshot of the prod sidebar showing the nested tree.
- [ ] No changes to /Users/artelonga/projects/yggdrasil/universes/ (runtime crates untouched).

### CO-280 — Universe vs sub-universe vs deployable-unit — visual + nav clarification across SPA (#142)
_Merged: 2026-06-04_
_Release: v2.40.0_

- [ ] First-time visitor opens `/co` — within 3 seconds, can articulate: "this is the CO platform, with sister-deployable platforms above; below is the current universe's content"
- [ ] Operator views the sidebar and immediately knows which buttons are dev/admin tools vs end-user actions
- [ ] Visiting `/yggdrasil/shandara` (when scaffolded) shows breadcrumb `Yggdrasil › Shandara` and the sidebar tree highlights Shandara under Yggdrasil
- [ ] `co_dev_ship` is either clearly labeled with purpose or removed
- [ ] No regressions on existing navigation (kanban, entry view, board switcher all still work)
- [ ] Playwright spec covers all three IA layers

### CO-211 — Universe Content API contract — formal v1 spec with OpenAPI so any client renders any universe (#144)
_Merged: 2026-06-03_
_Release: v2.39.0_

- [ ] `docs/api/openapi.yaml` exists, valid OpenAPI 3.1
- [ ] All listed endpoints documented with request schema + response
- [ ] Schemas use shared components for `Universe`, `Entry`, `Project`,
- [ ] `/api/openapi.json` serves the spec
- [ ] `/api/docs` renders Swagger UI (or alternative like Redoc)
- [ ] CO-N test suite: parse the spec and validate at least one
- [ ] CI gates: spec must be valid YAML + valid OpenAPI 3.1

### CO-291 — CO-284-B — Telemetry trait + OTLP exporter (feature-flagged) (#143)
_Merged: 2026-06-03_
_Release: v2.39.0_

- [ ] Without OTLP env var set, behavior is identical to today (stderr logs)
- [ ] With OTLP env var pointing at a local Jaeger, spans appear in the UI within 10s of an HTTP request
- [ ] Each HTTP request produces at least one trace with parent/child spans for DB queries
- [ ] No measurable latency regression on warm requests (telemetry adds < 5% overhead)

### CO-301 — Task archive — per-task worktree compression + queryable change-log link (#141)
_Merged: 2026-06-03_
_Release: v2.39.0_

- [ ] Every merged PR (post-CO-301) automatically gets a `docs/task-archive/<TASK-ID>.json` file
- [ ] Backfill produces archives for the existing ~50 merged tasks
- [ ] `co-task show CO-279` prints task metadata in &lt; 1 second from JSON
- [ ] `co-task diff CO-279` runs `git show 9e02c7e --stat` (the merge commit)
- [ ] Disk usage of `.worktrees/` and `.claude/worktrees/` drops below 5 GB after first prune run
- [ ] `safe-merge-pr.sh` end-to-end takes &lt; 5 seconds added overhead vs today
- [ ] CHANGELOG-PENDING/CO-301.md documents the new workflow

### CO-337 — Remote sister-repo sync — universes pull content from remote git on prod (#140)
_Merged: 2026-06-03_
_Release: v2.39.0_

- [ ] Schema migration adds 3 columns to `universes` (additive, nullable)
- [ ] `vcs::clone` + `vcs::pull` helpers added (or reused from CO-331)
- [ ] `run_remote_sister_repo_seeds` clones + walks each universe's `remote_url`
- [ ] Resolution order: local path wins when set + exists; else remote
- [ ] Background task re-syncs every 15 min (configurable)
- [ ] PATCH endpoint accepts new fields
- [ ] `comunicacao` prod universe successfully ingests the 13 source records after migration backfill
- [ ] No regression in localhost flow (local_repo_path still works as primary)
- [ ] `docs/sister-repo-sync.md` documents auth + cadence + the env vars

### CO-333 — Feedback system — Yggdrasil-compatible, per-universe + per-entry locus (#139)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `feedback` table exists with the schema above (additive, includes Yggdrasil's columns)
- [ ] `POST /api/v1/feedback` accepts Yggdrasil's payload shape (universe-wide)
- [ ] `POST /api/v1/feedback/<universe>/<entry_path>` attaches feedback to a specific entry
- [ ] Anonymous + authenticated submissions both work
- [ ] Owner-only `PATCH /feedback/<id> { status }` works (403 for non-owner)
- [ ] Entry view shows unread-feedback badge for owner
- [ ] Side panel renders + lists per-entry feedback, allows status changes
- [ ] `/<universe>/feedback` mural page works
- [ ] Federation forward (`CO_FEEDBACK_FORWARD_URL`) works when set
- [ ] CO-332 chat tool `submit_feedback` callable

### CO-336 — Feedback → PR/commit traceable (open-source issue-tracker semantics) (#137)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] Schema migration adds 4 columns (additive, no break)
- [ ] `PATCH /feedback/<id>` accepts linked_ref + summary + response + visibility
- [ ] Status state machine enforced (open → reviewed → addressed/wont-fix/duplicate)
- [ ] `addressed` auto-sets `public_visible=1`
- [ ] `wont-fix` requires `owner_response` (400 if missing)
- [ ] Public mural lists addressed + wont-fix items for anon visitors
- [ ] Owner-only mural lists all (including private)
- [ ] Per-entry feedback strip shows public history
- [ ] GitHub PR/commit URL → auto-fetch title (cached)
- [ ] CO-332 chat exposes `get_feedback_status` tool
- [ ] CO-334 cross-link: `linked_ref=commit:abc` appears in changelog feed for that commit

### CO-335 — Centralized graph rendering — one primitive, content in CO, UI customization deferred (#138)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `co_graph.render()` API documented + published
- [ ] `GET /api/v1/universes/<u>/graph` returns the standardized nodes+edges shape
- [ ] ArteLonga neuro pages migrated — visual identical, code shared
- [ ] Yggdrasil comunicacao migrated — visual identical, code shared
- [ ] `/universe/<key>/graph` page works for any CO universe
- [ ] No duplicate force-layout / physics / pan-zoom code in any sister repo

### CO-334 — Cross-repo changelog aggregation — sister-repo releases interleaved into CO's changelog view (#136)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `release_notes` table exists, indexed on date + repo
- [ ] `parse_keep_a_changelog` correctly parses ArteLonga's, Quilombo's, Yggdrasil's, RFQ's, and CO's own CHANGELOG.md files (5 fixtures)
- [ ] `run_release_notes_refresh` ingests all configured repos at boot; idempotent
- [ ] `GET /api/v1/changelog/feed` returns interleaved entries newest-first
- [ ] Repo filter works
- [ ] `/changelog` page renders the multi-repo view with filter dropdown
- [ ] New release committed to a sister repo's CHANGELOG.md appears in CO's view within 5 min (configurable polling)

### CO-332 — External assistant — non-Claude LLM with deterministic tool routing for yuri.artelonga.com.br (#134)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `POST /api/v1/chat/<universe>` exists, anon-accessible for opted-in universes
- [ ] Tool calls go through the existing `/entries` filter — anon never sees draft content
- [ ] Provider routing: `/chat` cannot be configured to use Claude (test asserts)
- [ ] Ollama integration works end-to-end (assumes Ollama installed)
- [ ] OpenAI fallback works when configured
- [ ] Chat UI renders, streams tokens, shows tool calls in flight
- [ ] Deployment-status tool returns curated `flyctl status` data
- [ ] Each chat query appears in CO-329 analytics

### CO-331 — Tools as git repos — npm-like install/version/conflict + jj-compatible (#133)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `tools` table exists with schema above
- [ ] `co tool add` clones (or registers a local path) + sets version pin
- [ ] `co tool list` shows status (installed, pinned version, follows-main)
- [ ] `co tool update <key>` re-fetches and checks out the new ref
- [ ] `co tool update --all` refreshes all `follow_main=1` tools
- [ ] `co tool remove` deletes the checkout + DB row
- [ ] jj-backed repos work end-to-end (detected via `.jj/`)
- [ ] Conflict warning surfaces when two tools both define competing entries (initial: name collisions, expand later)

### CO-328 — Local LLM (macOS) + Claude Code hook integration (#132)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `POST /api/v1/ai/query` with `{provider: "ollama", prompt: "..."}` returns response
- [ ] `POST /api/v1/ai/query` with `{provider: "claude", prompt: "..."}` spawns claude + streams back
- [ ] AI status endpoint reflects reality
- [ ] If neither is installed, query returns 503 with helpful install hint
- [ ] CO-327 desktop notification fires when claude session needs input or finishes

### CO-327 — macOS desktop notifications for CO events (#129)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] Running `co serve` + CO-326 message → desktop notification appears
- [ ] `CO_DESKTOP_NOTIFY=off co serve` → no notifications fired
- [ ] Notification click opens browser to `/notifications`
- [ ] No notification on Linux/Windows (graceful no-op)

### CO-329 — /analytics non-indexed real-time telemetry + background-process visibility (#131)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `GET /analytics` returns HTML page (auth-gated, noindex)
- [ ] `WS /api/v1/analytics/stream` pushes events in real-time
- [ ] Live request log scrolls with each request
- [ ] Background process table updates as workers start/finish
- [ ] Active AI sessions visible
- [ ] Error count + recent error list

### CO-325 — Reference type system + recursive composition + notas abstraction (#130)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] Schema files in `work/co/schema/` for each new type
- [ ] Category aggregation: querying `type:music` returns songs + albums
- [ ] Recursive `references` field stored as JSON column or sub-table; queryable
- [ ] `notas` type with frontmatter validation
- [ ] Query DSL supports `type:`, `author:`, `caderno_id:`, `before:`, `after:`

### CO-323 — yuri.artelonga.com.br — subdomain routing to a single-universe view (#128)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `curl -H "Host: yuri.artelonga.com.br" https://co-artelonga.fly.dev/` returns the yuri universe SPA
- [ ] In a browser at https://yuri.artelonga.com.br/ the page shows only yuri's entries; no sidebar of other universes
- [ ] Anonymous visitor sees only entries with `visibility: public` (depends on CO-324)
- [ ] yuri (logged in via .artelonga.com.br cookie domain) sees all entries
- [ ] `/2026-05-31` resolves to a daily note (depends on CO-324A)

### CO-326 — Direct messaging — send to yuri@artelonga.com.br (email + in-app) (#127)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] `POST /messages` accepts form data, returns 200
- [ ] Email arrives at yuri@artelonga.com.br (verify via log or actual delivery)
- [ ] In-app notification appears for yuri on next page load
- [ ] Rate limit kicks in at 6th message in an hour from same IP
- [ ] Form has honeypot field that drops spam silently

### CO-330 — Runtime universe→repo bindings + anon published-only filter (deploy-free) (#126)
_Merged: 2026-06-01_
_Release: v2.35.0_

- [ ] Migration v51 adds 3 columns + backfills 8 universe rows (7 existing + yuri)
- [ ] `seed_orchestrator::run_sister_repo_seeds` no longer has the hardcoded `mappings` array
- [ ] `PATCH /universes/<key>/source` works, owner-only, returns 403 for non-owners
- [ ] Anonymous `GET /universes/yuri/entries` returns only entries with `frontmatter.published == true`
- [ ] Authenticated owner sees all entries on the same endpoint
- [ ] `co launch` auto-binds the new universe to the launch directory
- [ ] `cargo test` + `cargo clippy -- -D warnings` clean
- [ ] No regression: existing universes still ingest their content after the refactor (backfill correctness)

### CO-322 — co launch — bootstrap a universe from the current repo (Fly-style) (#124)
_Merged: 2026-05-30_
_Release: v2.33.0_

- [ ] `co launch` in a clean dir creates a universe with the dir's name as key
- [ ] `co launch --key foo` overrides the derived key
- [ ] `co launch --public` makes the universe `public-subscribable`
- [ ] `co launch --now` opens browser on `http://localhost:<port>/<key>`
- [ ] Re-running `co launch` in the same dir is idempotent (upsert, not duplicate)
- [ ] Output prints clear summary: N pages + M tasks ingested
- [ ] `co launch --help` example block matches the reference UX above

### CO-321 — E2e — localhost trial flow (subscribe / unsubscribe / sister repos / themes) (#123)
_Merged: 2026-05-30_
_Release: v2.33.0_

- [ ] `co-web/e2e/localhost-trial.spec.ts` exists with the 7 test cases above
- [ ] All 7 pass against a freshly-built `co serve --port <ephemeral>`
- [ ] CI runs the spec (already auto-discovered by Playwright config)
- [ ] No new fixtures needed beyond `apiContext`, `seedProject`, `TestServer`

### CO-320 — (spec not found) (#122)
_Merged: 2026-05-30_
_Release: v2.33.0_

_(no acceptance criteria in spec)_

### CO-319 — (spec not found) (#121)
_Merged: 2026-05-30_
_Release: v2.33.0_

_(no acceptance criteria in spec)_

### CO-318 — (spec not found) (#120)
_Merged: 2026-05-29_
_Release: v2.32.0_

_(no acceptance criteria in spec)_

### CO-317 — (spec not found) (#119)
_Merged: 2026-05-29_
_Release: v2.32.0_

_(no acceptance criteria in spec)_

### CO-316 — (spec not found) (#118)
_Merged: 2026-05-29_
_Release: v2.32.0_

_(no acceptance criteria in spec)_

### CO-315 — (spec not found) (#117)
_Merged: 2026-05-29_
_Release: v2.32.0_

_(no acceptance criteria in spec)_

## Carried Over

- (none tracked — retrospective simulation uses merge commits only)
