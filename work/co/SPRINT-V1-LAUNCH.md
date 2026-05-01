---
title: "Sprint plan — current state to v1.0 launch (hours, not months)"
status: living
priority: critical
created_at: 2026-04-29T00:00:00Z
updated_at: 2026-04-30T08:49:22Z
supersedes_doc: ROADMAP-V1-LAUNCH.md
governed_by: ROADMAP-V2-PLATFORM-REVIEW.md
---

# Sprint plan — Wave / Phase reconciliation as of 2026-04-30

This is the **operator's day-to-day execution view**. The strategic doc is `work/co/ROADMAP-V2-PLATFORM-REVIEW.md` — that one introduced the **Phase 0–5** framing (epics CO-111…CO-116) which supersedes `ROADMAP-V1-LAUNCH.md`'s Tier 0–5. The Wave 1–5 sequencing here is the same content; the column on the right tells you which Phase each Wave-task lives under in the V2 framing.

**Working assumption:** each task is one `co-auto --task CO-XXX --space co` run, branching off `main`, completing with a passing build + tests + clippy + a deploy to UAT.

**Definition of done for each task:**
1. Branch + PR (or direct commit per project policy)
2. Tests added or updated (Rust + Playwright where applicable)
3. `cargo test -p co-web --lib` clean
4. `cargo clippy -p co-web --release -- -D warnings` clean
5. `cargo fmt` applied
6. Deployed to UAT, manual smoke check passes
7. CHANGELOG entry
8. Version bump in `Cargo.toml` + `co-cli/Cargo.toml`

---

## Wave → Phase mapping

| Wave (V1) | Phase (V2) | Theme | Epic |
|---|---|---|---|
| 1 — Foundation | **Phase 0** product side | Smoke, deep-health, backups | CO-111 |
| 2 — Demoable v1.1 | **Phase 1** product side | Universe CRUD, hierarchy, onboarding, mermaid, docs, dashboard | CO-112 |
| 3 — Operations | **Phase 1** ops + **Phase 2** ops | Backups, load tests, archive, mbya | CO-112, CO-113 |
| 4 — Polish | **Phase 2** product side | Soft-delete, PWA offline, visitor token | CO-113 |
| 5 — Hardening | **Phase 2** infra side | Rate limit, caching, v1.0 tag | CO-113 |

The **new V2 platform infrastructure** (Cloudflare CDN, WAE, ClickHouse, Redpanda, Iceberg, Flink, deployer adapters) lives in additional tickets (CO-117…CO-136) listed below in their respective Phases. Those are interleaved with Wave 2-5 items per V2 §G's guidance: **finish current product wave first**, then file the new platform tickets, then resume.

---

## Status — current sprint

### Phase 0 (Foundation — Wave 1, ~done)

| Task | Ticket | Status |
|---|---|---|
| A1 — commit + push baseline | (chore) | ✅ done — `318093d`, `d2522cf`, `3cc1f40` |
| A2 — smoke test scripts | CO-103 | ✅ done — `5002540`, `0e95e78`, `2e9ec7d` |
| A3 — `/api/health/deep` | CO-106 | ✅ done (folded into A2's commit) |
| C1 — S3 backup automation | CO-104 | ⬜ not started — Wave 3, Phase 0 ops |
| **NEW: PLAT-1** — Cloudflare in front | CO-117 | ⬜ blocked until product Wave 2 ships (per V2 §G) |
| **NEW: PLAT-2** — WAE telemetry binding | CO-118 | ⬜ depends on CO-117 |
| **NEW: PLAT-3** — Restore drill | CO-119 | ⬜ depends on CO-104 |

### Phase 1 (Demoable + telemetry — Wave 2, in flight)

| Task | Ticket | Status |
|---|---|---|
| B2 — hierarchical universes | **CO-98** | ⬜ **next up** — run before B1 |
| B1 — create-universe modal | CO-96 P1 | ⬜ after B2 (uses parent_key) |
| B3 — mermaid in universe home | CO-107 | ⬜ small, parallel-safe |
| B4 — onboarding 3-step coach | CO-99 | ⬜ parallel-safe with B3 |
| C3 — docs pass | CO-100 | ⬜ run after Wave 2 closes |
| C4 — admin dashboard | CO-105 | ⬜ |
| **NEW: PLAT-4** — co-agent adapter trait | CO-120 | ⬜ Phase 1 platform infra |
| **NEW: PLAT-5** — A/B primitives on OLTP | CO-121 | ⬜ Phase 1 platform infra |
| **NEW: PLAT-6** — Quota/tier model spec | CO-122 | ✅ done — [`docs/QUOTAS.md`](../../docs/QUOTAS.md) |

### Phase 2 (Sustained public test — Wave 3 + Wave 4 + Wave 5)

| Task | Ticket | Status |
|---|---|---|
| C2 — load tests (k6) | CO-101 | ⬜ |
| Universe archive + external-HD backup | CO-108 | ⬜ |
| Mbya stress-test corpus | CO-109 | ⬜ |
| D1 — rename/visibility | CO-96 P2 | ⬜ |
| D2 — soft-delete + 30-day trash | CO-96 P3 | ⬜ |
| D3 — visitor token unification | CO-97 | ⏸ blocked on May 13 telemetry-flip data |
| D4 — PWA offline Phase 1 | CO-69 P1 | ⬜ |
| E1 — rate limiting | CO-80 P1 | ⬜ |
| E2 — caching | CO-79 P1 | ⬜ |
| **NEW: PLAT-7** — ClickHouse single-node | CO-123 | ⬜ Phase 2 platform infra |
| **NEW: PLAT-8** — co-agent variants (CF Workers, Vercel) | CO-124 | ⬜ Phase 2 platform infra |

### Phase 3 (Real collaboration + streaming — post-v1.0)

| Task | Ticket | Status |
|---|---|---|
| Sync Protocol v1 | CO-61 | ⬜ |
| Idempotency + conflict resolution | CO-54 | ⬜ |
| CLI sync (folds into CO-91) | CO-51 | ⬜ |
| Universe branching Phase 2-4 | CO-95 | ⬜ |
| Desktop tray + PWA Phase 2-4 | CO-58 | ⬜ |
| Obsidian deep-sync | CO-68 | ⬜ |
| **NEW** Redpanda + Iceberg Topics | CO-125 | ⬜ |
| **NEW** Lakekeeper REST catalog + Iceberg-on-R2 | CO-126 | ⬜ |
| **NEW** Flink session stitching | CO-127 | ⬜ |
| **NEW** Apple-style 4-way conflict UI | CO-128 | ⬜ user-named v1 requirement |
| **NEW** Jujutsu-shaped changelog renderer | CO-129 | ⬜ user-named v1 requirement |

### Phase 4 (Encrypted + privileged compute zone — post-v1.0)

| Task | Ticket | Status |
|---|---|---|
| `.co` envelope format | CO-86 | ⬜ |
| Composable layer stack | CO-87 | ⬜ |
| Filesystem-as-Web (E2E remote editing) | CO-110 | ⬜ |
| **NEW** Privileged compute zone | CO-130 | ⬜ |
| **NEW** Aggregation allow-list + k-anon DLP | CO-131 | ⬜ |
| **NEW** Key-access audit log | CO-132 | ⬜ |

### Phase 5 (Programmable platform + multi-target deployer — long horizon)

Co becomes the only surface. `deploy.yaml` schema + per-universe manifest validation; deployer adapter trait; static-on-R2 + Cloudflare Pages adapters; Pinot evaluated against real traffic.

CO-63, CO-70, CO-71, CO-72, CO-73, CO-74, CO-75, CO-89, CO-88 (all programmable-platform tickets) plus CO-133, CO-134, CO-135, CO-136 (deployer adapters + Pinot eval).

---

## Coordination notes (read before kicking off agents)

- **`app.js` is the conflict surface.** B1, B2, B3, B4 all touch it. Run B2 (CO-98) first since it changes the data model that B1 consumes; then B1; then B3 + B4 in parallel worktrees.
- **Version bumps serialise.** Every code-touching task bumps `Cargo.toml` + `co-cli/Cargo.toml`. Two agents bumping the same file conflict on merge. If multiple in worktrees, last-merged rebases.
- **Per V2 §G:** finish Wave 2 (Phase 1 product side) before starting CO-117/118/119/120/121/122 (Phase 0+1 platform side). Don't interleave product CRUD work with Cloudflare account setup mid-sprint.
- **CO-104 still pending.** Backup automation is product-side ops, not platform infra. Can run parallel-to-Wave-2 since it touches `scripts/` only. Closes Phase 0.
- **CO-128 + CO-129 are user-named v1 requirements** that the original V1 sprint missed. They're in Phase 3 now. If you want them earlier, reframe as Wave 4 (Phase 2.5).

## Wave 1 — Foundation (≈1.5 h, sequential)

Stop the bleeding. Nothing new visible to users yet, but everything that follows is safer.

### A1 — Commit + push the 1.20.2 → 1.21.1 working-tree diff

| | |
|---|---|
| Type | `chore` |
| Estimate | 15 min |
| Branch | `chore/commit-1.20.2-to-1.21.1` (or direct to main per CLAUDE.md) |
| Files | working tree as-is |
| Acceptance | `git status` clean, `git push origin main` succeeds, no force-push, CHANGELOG already current |

**co-auto prompt:**
```
Read git status, git diff, git log -5. Group the uncommitted changes into 2-3
coherent commits with conventional messages: one for the legal pages refactor +
seed system (1.20.2-1.20.3), one for the orphan rescue + atomic switching
suite (1.20.4-1.20.10), one for the timeline trio + multi-overlay (1.21.x).
Stage files explicitly per commit (no git add -A). Push to origin/main. Do not
amend, no force push. Verify CHANGELOG already covers all the work; if not,
add the missing block to the *latest* version in CHANGELOG.md and roll into
the same commit.
```

### A2 — Per-deploy regression smoke test (CO-103)

| | |
|---|---|
| Type | `test` |
| Estimate | 30 min |
| Branch | `feat/co-103-deploy-smoke` |
| Files | `scripts/smoke-prod.sh`, `scripts/smoke-uat.sh`, GitHub Actions step (deferred — local script for now) |
| Acceptance | `bash scripts/smoke-prod.sh` exits 0 on healthy prod; exits non-zero with diagnostic output if any of: `/api/health` returns ≠ 200, template/tempo/humanity/universo missing, any of the three timeline universes have ≠ N events, login endpoint 5xx |

**co-auto prompt:**
```
Create scripts/smoke-prod.sh and scripts/smoke-uat.sh. Each takes BASE_URL
(default https://co.artelonga.com.br for prod, https://co-artelonga-uat.fly.dev
for uat). It exits 0 if all of these pass:
  - /api/health returns 200 with status=ok
  - /api/v1/universes/template, /tempo, /humanity, /universo all return 200
  - tempo has 21 events, humanity has 26, universo has 28 (read from
    /api/v1/universes/<key>/entries?type=event)
  - /shared/timeline.html returns 200
Each failure prints a clear line with the gap. Add a CHANGELOG entry. Bump
patch version. The script must be idempotent and runnable without auth.
```

### A3 — `/api/health/deep` endpoint (CO-106)

| | |
|---|---|
| Type | `feat` |
| Estimate | 30 min |
| Branch | `feat/co-106-deep-health` |
| Files | `co-web/src/server.rs` (or new `health.rs`), tests |
| Acceptance | `GET /api/health/deep` returns 200 with JSON when all probes pass; 503 with same JSON shape (with `ok: false` on the failing probe) on any failure |
| Spec | `work/co/CO-106.md` |

**co-auto prompt:**
```
co-auto --task CO-106 --space co
```
Spec at `work/co/CO-106.md` lists the three probes (db_read, db_write,
disk_writable), the response shape, the lazy-create `health_probes` table,
and the no-cache headers. Parallel-safe with CO-103 (no shared files).

---

## Wave 2 — Demoable v1.1 (≈6 h, mostly parallel)

The platform stops feeling like a developer artifact. Stranger can self-create.

### B1 — CO-96 Phase 1: Create universe modal

| | |
|---|---|
| Type | `feat` |
| Estimate | 2 h |
| Branch | `feat/co-96-p1-create` |
| Files | `co-web/static/variants/a/app.js`, `co-web/static/variants/a/index.html`, `co-web/static/variants/a/style.css`, Playwright e2e |
| Acceptance | Sidebar shows `+ New universe` button; click opens modal with fields (key, name, description, visibility radio); submit → POST `/api/v1/universes` → switch to new universe via `bootAppForUniverse` |
| Spec | `work/co/CO-96.md` |

**co-auto prompt:**
```
Implement Phase 1 of CO-96 (work/co/CO-96.md): the create-universe flow only.
- Sidebar header gets a `+ New universe` button (next to the user-universes
  group label). Visible only when logged in.
- Modal: fields for key (slug, with format hint), name, description (optional),
  visibility radio (private default / public). Submit POSTs to
  /api/v1/universes; on 201 switches to the new universe via
  bootAppForUniverse(key); on 409 shows "Slug already taken"; other errors show
  a toast.
- E2E test in co-web/e2e/: log in as yuri, click +, fill, submit, verify new
  universe appears in sidebar AND becomes active.
Defer rename/visibility/duplicate/delete to Phases 2 and 3.
```

### B2 — CO-98: Hierarchical universes (parent → children)

| | |
|---|---|
| Type | `feat` |
| Estimate | 1.5 h |
| Branch | `feat/co-98-hierarchy` |
| Files | migrations, `co-web/src/storage.rs`, `co-web/src/universe_routes.rs`, `co-web/static/variants/a/app.js` |
| Acceptance | universes table has `parent_key TEXT NULL` column; `seed_all_timeline_universes` sets `tempo`, `humanity`, `universo` to `parent_key='template'`; `/api/v1/universes` includes `parent_key` field; SPA sidebar groups child universes under their parent (collapsible); template's home page shows direct child links |
| Spec | this file (also create `work/co/CO-98.md`) |

**co-auto prompt:**
```
Implement CO-98 (write spec at work/co/CO-98.md first, then code).

Data model:
- Add `parent_key TEXT NULL` column to universes via a versioned migration in
  storage.rs migrations table. Default null. No FK constraint (universes
  can be deleted independently).
- Update Universe model + JSON serialization.

Seed:
- seed_timeline_universe accepts optional parent_key from manifest. Update
  the three timeline JSONs to set parent_key="template".

API:
- GET /api/v1/universes/:slug returns parent_key.
- GET /api/v1/universes returns flat list (existing behavior) — sidebar does
  the grouping client-side.

SPA:
- Sidebar groups universes by parent_key. Children render indented under
  their parent with a chevron. Saved-state in localStorage.
- Template's home page (renderUniverseHome) shows a "Explorar" section listing
  direct children (currently tempo/humanity/universo) as styled cards with name
  + description + link.

Tests:
- Storage test: insert parent + children, check round-trip.
- Playwright: load template, verify the timeline trio shows under it in the
  sidebar.
```

### B3 — Mermaid in universe home page (CO-107)

| | |
|---|---|
| Type | `feat` |
| Estimate | 30 min |
| Branch | `feat/co-107-mermaid-home` |
| Files | `co-web/static/variants/a/app.js` (renderUniverseHome), `co-web/seed/template/sobre.md` (sample diagram) |
| Acceptance | `index.md` (or any seeded template page) containing a `\`\`\`mermaid` block renders the diagram in the universe-home view; lazy loads the bundle; theme-aware via CSS vars |
| Spec | `work/co/CO-107.md` |

**co-auto prompt:**
```
co-auto --task CO-107 --space co
```
Spec at `work/co/CO-107.md`. Tiny ticket — wires the existing
`renderMermaidBlocks` helper (CO-83, shipped) into the new
`renderUniverseHome` surface (CO-Universe-Home, shipped 1.20.11) and adds a
sample diagram of the timeline trio to template's content.

### B4 — CO-99: First-time onboarding banner

| | |
|---|---|
| Type | `feat` |
| Estimate | 1 h |
| Branch | `feat/co-99-onboarding` |
| Files | new `work/co/CO-99.md`, `co-web/static/variants/a/app.js`, `co-web/static/variants/a/style.css` |
| Acceptance | First visit to template (no `co_onboarded` cookie) shows a 3-step coach mark: "1) views" → "2) timeline" → "3) make your own"; dismissable; persists dismissal in cookie |

**co-auto prompt:**
```
Write spec at work/co/CO-99.md, then implement.

A coach-mark banner that appears ONLY when:
- visitor is anonymous (api.me() returned null)
- on the template universe (state.isTemplate === true)
- co_onboarded cookie is not set

Three steps in sequence (not modal — a non-blocking banner with a Next button):
  1. "Olá! O Co tem várias visões — Quadro, Tabela, Conteúdo, Linha do tempo.
     Clique nas abas acima para alternar."
  2. "Tente a Linha do tempo. Aqui está [link]: tempo, universo, humanity
     sobrepostos."
  3. "Quando quiser criar seu próprio universo, clique em + Novo universo na
     barra lateral."

After step 3 or "Pular", set cookie co_onboarded=1; Path=/; Max-Age=31536000.
Position: floating bottom-right card, dismissable. Skip on mobile (no real
estate).
```

---

## Wave 3 — Operations (≈5 h, mostly parallel)

Foundations for sustained traffic. None visible to users; all visible to operators.

### C1 — CO-104: Backup automation

| | |
|---|---|
| Type | `feat` |
| Estimate | 1.5 h |
| Branch | `feat/co-104-backups` |
| Files | new `work/co/CO-104.md`, `scripts/backup-prod.sh`, `scripts/restore.sh`, fly cron config |
| Acceptance | Daily cron snapshots SQLite + universes/ dir to S3-compatible bucket; 30 daily + 12 monthly retained; restore.sh tested against a scratch app |

**co-auto prompt:**
```
Spec at work/co/CO-104.md, then implement.

backup-prod.sh:
- SSH into co-artelonga
- sqlite3 /data/co.db ".backup /tmp/co-$(date +%Y%m%d).db"
- tar czf /tmp/universes-$(date +%Y%m%d).tar.gz -C /data universes
- Upload both to ${S3_BUCKET} (default: artelonga-co-backups via flyctl
  storage); use AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY from env or flyctl
  secrets.
- Retention: keep last 30 daily, 12 monthly (yyyymm), prune older.
- Exit 0 on success.

restore.sh:
- Args: <backup_date> <target_app>
- Downloads from S3, scps into target Fly app, sqlite3 .restore.

Cron: separate Fly app `co-backup-cron` running every 24h (use fly machines
schedule), or a github action with cron schedule. Document trade-off in spec.
First run: kick off manually, verify S3 has the artifact.
```

### C2 — CO-101: Load test scaffolding

| | |
|---|---|
| Type | `test` |
| Estimate | 1.5 h |
| Branch | `feat/co-101-loadtest` |
| Files | new `work/co/CO-101.md`, `tests/load/*.js`, `tests/load/baselines/<date>.md` |
| Acceptance | k6 scripts cover the 8 hottest endpoints (health, list_universes, info, config, theme.css, entries, entries/tags, projects); baseline run committed against UAT showing p50/p95/p99 at 50 / 100 / 500 RPS; failure mode at higher RPS documented |

**co-auto prompt:**
```
Spec at work/co/CO-101.md.

Create tests/load/ with k6 scripts:
- 01-anon-browse.js: anonymous user pageviews (template home, timeline,
  conteudo)
- 02-logged-in-board.js: yuri-style power user (login, switch universes,
  create entry)
- 03-vault-write.js: API token holder uploading entries

Use BASE_URL env var (default UAT). Run with k6 run --vus N --duration 1m.

Capture baselines for 50 / 100 / 500 VUs against UAT (do NOT run against prod).
Commit results in tests/load/baselines/2026-04-29-uat.md as a table:
  scenario | VUs | duration | p50 | p95 | p99 | errors

Document the hardware profile (Fly machine size) at run time. Spec the
follow-up: what size needs to be tested before launch.
```

### C3 — CO-100: Documentation pass

| | |
|---|---|
| Type | `docs` |
| Estimate | 1.5 h |
| Branch | `docs/co-100-docs` |
| Files | new `work/co/CO-100.md`, `docs/ARCHITECTURE.md`, `docs/OPERATIONS.md`, `docs/ONBOARDING.md` (ensure `CONTRIBUTING.md` exists at repo root) |
| Acceptance | Each file describes the actual current state (1.21.x), not aspirational. ARCHITECTURE has component diagram (Mermaid) of: SPA → axum routes → storage → SQLite + Fly volume; OPERATIONS covers deploy / backup / restore / secret rotation; ONBOARDING is a 5-min "set up Co locally" |

**co-auto prompt:**
```
Spec at work/co/CO-100.md, then write three documents under docs/.

docs/ARCHITECTURE.md:
- Component diagram (Mermaid C4-context-style)
- Data flow: anonymous visitor, logged-in visitor, API token user
- Universe storage: rows in SQLite + filesystem in /data/universes/<key>/
- Theme system: ThemePreset → /api/v1/themes/<name> + per-universe overrides
- Deployment surface: Fly.io GRU, custom domain, certs, secrets
Reflects 1.21.x reality.

docs/OPERATIONS.md:
- flyctl deploy --config fly.uat.toml / fly.toml
- Logs: flyctl logs -a <app> --no-tail
- SSH: flyctl ssh console
- Backup: scripts/backup-prod.sh (link to CO-104)
- Restore: scripts/restore.sh
- Secret rotation: JWT_SECRET; document the side-effect (all sessions invalidated)
- UAT reset: touch /data/uat-reset.flag and machine restart
- Health: /api/health, /api/health/deep
- Smoke: scripts/smoke-prod.sh (link to CO-103)

docs/ONBOARDING.md:
- Clone, cargo build, cargo run -p co-web (10 lines)
- Open http://localhost:3000
- Login as the dev admin (CO_SEED_ADMIN_EMAIL / _PASSWORD_HASH)
- Create your first universe
- Where things live in the codebase

Cross-link all three. PT and EN both? Default PT (consistency with audience),
add a note that EN is welcome contribution.
```

### C4 — CO-105: Admin telemetry dashboard

| | |
|---|---|
| Type | `feat` |
| Estimate | 1 h |
| Branch | `feat/co-105-admin-dashboard` |
| Files | new `work/co/CO-105.md`, `co-web/src/admin_routes.rs`, `co-web/static/variants/a/admin.html` |
| Acceptance | `GET /api/v1/admin/dashboard` (admin-tier required) returns daily traffic, top universes, error rate, signup count; `/admin` page renders the data with simple HTML table + sparkline (no JS framework) |

**co-auto prompt:**
```
Spec at work/co/CO-105.md.

Endpoint /api/v1/admin/dashboard requires JWT with tier=admin (or seeded admin
email check — pick one consistent with existing admin gates). Returns:
{
  date_range: ...,
  daily: [{date, pageviews, uniques, signups, errors}],
  universes: [{key, name, entries, last_active}],
  totals: {users, universes_active_7d}
}

Aggregates from telemetry_events (already exists), users, universes,
universe_members tables. Cache 5 min in-memory.

/admin static page (no SPA framework): table of daily stats, table of top 10
universes by entries-touched in last 7 days. Sparkline via inline SVG.
```

---

## Wave 4 — Polish (≈4 h, parallel safe)

Filling out the universe lifecycle. Each unblocks a class of "I can't do X."

### D1 — CO-96 Phase 2: Rename + change visibility

| | |
|---|---|
| Type | `feat` |
| Estimate | 1.5 h |
| Branch | `feat/co-96-p2-rename` |
| Files | `co-web/src/universe_routes.rs`, `co-web/static/variants/a/app.js` |
| Acceptance | Right-click (or kebab menu) on universe in sidebar → Rename / Change visibility; PUT `/api/v1/universes/:slug` (already exists, may need extension); reflects in sidebar without reload |
| Spec | `work/co/CO-96.md` |

### D2 — CO-96 Phase 3: Soft-delete + 30-day trash

| | |
|---|---|
| Type | `feat` |
| Estimate | 2 h |
| Branch | `feat/co-96-p3-delete` |
| Files | migration, storage, routes, SPA |
| Acceptance | Right-click → Delete (with confirm); universe gets `deleted_at` timestamp; hidden from listings but recoverable via `/api/v1/universes/:slug/restore` for 30 days; cron purges after 30 days |
| Spec | `work/co/CO-96.md` |

### D3 — CO-97: Visitor token unification

| | |
|---|---|
| Type | `feat` |
| Estimate | 1 h |
| Branch | `feat/co-97-visitor-token` |
| Files | `co-web/src/quilombo_telemetria.rs`, marketing repo (separate PR there) |
| Spec | `work/co/CO-97.md` |
| Note | **Wait for May 13 telemetry-flip cron data before starting.** Without it the cookie strategy choice is theoretical. |

### D4 — CO-69 Phase 1: PWA offline minimum

| | |
|---|---|
| Type | `feat` |
| Estimate | 1.5 h |
| Branch | `feat/co-69-p1-pwa` |
| Files | service worker (already in place), `co-web/static/shared/sw.js`, `co-web/static/variants/a/app.js`, IndexedDB cache layer |
| Acceptance | Open the app, go offline, last-viewed universe content + last entries still browseable read-only; conteudo view shows offline banner; mutations queued and replayed on reconnect (Phase 2) |
| Spec | `work/co/CO-69.md` (already exists — verify) |

---

## Wave 5 — Hardening (≈4 h, sequential)

Last steps before tagging v1.0. Run after Wave 1-4 are merged.

### E1 — CO-80 Phase 1: Rate limiting (token bucket per IP/user)

| | |
|---|---|
| Type | `feat` |
| Estimate | 1.5 h |
| Files | `co-web/src/middleware/rate_limit.rs` |
| Acceptance | Anonymous IPs limited to 60 req/min; authed users 300 req/min; 429 with `Retry-After` header; bypass for healthcheck |
| Spec | `work/co/CO-80.md` |

### E2 — CO-79 Phase 1: Manifest + theme caching

| | |
|---|---|
| Type | `perf` |
| Estimate | 1 h |
| Files | route handlers |
| Acceptance | `Cache-Control: public, max-age=300, must-revalidate` + ETag on theme.css (already partial) and `/api/v1/universes/:slug/config`; in-memory ETag cache; cache hit logged |
| Spec | `work/co/CO-79.md` |

### E3 — Final v1.0 verification + tag

| | |
|---|---|
| Type | `chore` |
| Estimate | 1 h |
| Steps | run `scripts/smoke-prod.sh`; run k6 50 VU scenario against UAT; verify backups working; verify all checklist boxes in `docs/feedback-checklist.md`; tag `v1.0.0`; deploy with `--strategy bluegreen` |

**co-auto prompt:**
```
Run all of:
  scripts/smoke-prod.sh
  scripts/smoke-uat.sh
  k6 run --vus 50 --duration 5m tests/load/01-anon-browse.js
Capture results in docs/v1.0-launch-verification.md.
If everything green, bump versions to 1.22.0 / 0.30.0 (or whatever is current
+ minor), update CHANGELOG with v1.0.0 release notes summarizing 1.20.x and
1.21.x, git tag v1.0.0, push tag.
Deploy prod with --strategy bluegreen if available, else regular.
Open champagne (figuratively).
```

---

## Total time + ordering

| Wave | Tasks | Sequential / parallel | Estimated agent hours |
|---|---|---|---|
| 1 | A1, A2, A3 | sequential | 1.5 |
| 2 | B1, B2, B3, B4 | mostly parallel | 6 |
| 3 | C1, C2, C3, C4 | parallel | 5 |
| 4 | D1, D2, D3, D4 | parallel | 4 |
| 5 | E1, E2, E3 | sequential | 4 |
| | | **Total** | **~21 hours** |

With 2-3 worktrees running in parallel for Wave 2-4, calendar wall-clock could be **2-3 days** of mostly hands-off agent execution.

## Pre-flight checklist before kicking off

- [ ] Wave 1 commits the working tree first (so future agents start from a clean base).
- [ ] Each `co-auto` invocation gets the task spec via `--task CO-XXX --space co`.
- [ ] Operator (yuri) reviews each PR before merge — agents propose, human disposes.
- [ ] Marketing endpoint flip (separate repo) happens between Wave 1 and Wave 2 so telemetry data starts flowing for CO-97 evaluation in Wave 4.
- [ ] May 13 cron auto-fires the telemetry verification — block CO-97 (D3) until that report.

## What we are deliberately NOT doing in this sprint

- Sync Protocol v1 (CO-61) — too large, real engineering, not hours.
- Per-universe SQLite + LiteFS (CO-77) — too large, schema migration risk.
- `.co` protobuf format + encryption-at-rest (CO-86) — Tier 4 of prior roadmap, out of scope.
- Manifest + content-type plugin system (CO-63/70/71) — Tier 5.
- Mobile (Capacitor) — needs UI to be stable first.

These remain valid for post-v1 sprints; they are NOT v1.0 blockers.
