# Co — Path to Stable Release (v1.1.0)

> Current: v1.0.0 deployed (MVP). Target: v1.1.0 stable, fully tested, post-MVP polish + telemetry.

## Principles

1. **Foundation before features.** Tasks that other tasks depend on land first.
2. **Dev board is the single source of truth.** All work tracked in CO-* tasks visible to Yuri (CO-43).
3. **UAT is mandatory** for any task that touches data, UI, or auth. UAT runs all acceptance tests before prod.
4. **Incremental deploys.** Small batches (1-3 tasks) per UAT cycle. No "big bang" releases.
5. **Telemetry-first.** Once CO-46 lands, every subsequent task adds events to track adoption.
6. **Acceptance tests run on UAT, not local.** Local tests prove correctness; UAT tests prove the deployed system works.

---

## Phase A — Foundation (no user-facing changes)

These unblock everything else and have minimal risk. Land in this order.

### A1. CO-43: Hidden dev board for Yuri
**Why first:** Required by CO-44. Lets the dev workflow be visible.
**UAT requirements:**
- Yuri logs in → sees `co-dev` universe
- All CO-* tasks visible with full metadata
- Anonymous → 404
**Acceptance tests:**
- [ ] `e2e/dev-board.spec.ts`: admin auth → universe visible → task metadata correct
- [ ] Filter by status/priority/module works
- [ ] Non-admin user → 404
**Deploy gates:** UAT → manual verification → prod

### A2. CO-44: UAT environment setup
**Why second:** Depends on CO-43. Required by CO-45 and all subsequent UAT testing.
**UAT requirements:** This task IS the UAT setup. After landing:
- `CO_ENV=uat` set on co-artelonga-uat
- User `yuri` / `uat` exists on UAT only
- Reset flag works (`/data/uat-reset.flag`)
- Anonymous visitors get fresh state
**Acceptance tests:**
- [ ] `e2e/uat-flow.spec.ts`: yuri login → admin access → co-dev visible
- [ ] Reset flag → DB cleaned → re-seeded
- [ ] Anonymous: no leftover data between sessions
- [ ] Production unchanged (CO_ENV unset path tested)
**Deploy gates:** UAT-only initially. Production deploy only enables the env var change (no behavior shift).

### A3. CO-46: Telemetry system
**Why third:** Independent of A1/A2 but required by A4 (privacy update). Adds visibility for all subsequent work.
**UAT requirements:**
- Telemetry events flowing into `telemetry_events` table on UAT
- DNT respected
- Cookie consent gates tracking
**Acceptance tests:**
- [ ] `e2e/telemetry.spec.ts`: simulated user flow → events recorded with correct schema
- [ ] DNT header → no events
- [ ] No PII in any field (assert via property check)
- [ ] Performance impact < 5ms per request (load test)
**Deploy gates:** UAT verified → prod with consent banner already in place

### A4. CO-47: Privacy policy update
**Why fourth:** Depends on CO-46 (need the data list to be accurate).
**UAT requirements:**
- New privacy policy text in seed
- `dados-rastreados.md` page exists in template
- Cookie banner mentions telemetry
- DB clear on UAT to re-seed
**Acceptance tests:**
- [ ] `e2e/privacy.spec.ts`: privacy page loads → telemetry section present → link to dados-rastreados works
- [ ] dados-rastreados page lists every event from telemetry_events schema
- [ ] Cross-check: every event in code matches the policy doc
**Deploy gates:** UAT → manual review of legal text → prod

### A5. CO-48: Schema documentation MVP (data only)
**Why fifth:** Independent foundation. Doesn't touch users but ensures everything is documented.
**UAT requirements:** None (docs only, no UAT impact).
**Acceptance tests:**
- [ ] CI lint passes on all 5 YAML files
- [ ] Cross-check: every table in storage.rs migrations appears in tables.yaml
- [ ] Cross-check: every route in server.rs appears in endpoints.yaml
- [ ] Test in `co-web/tests/schema_docs_tests.rs`
**Deploy gates:** Direct to prod (no runtime impact)

---

## Phase B — UX critical path

These directly improve the user-facing experience. Highest impact for users.

### B1. CO-39: Markdown rendering pipeline
**Why first in phase:** Required by CO-42. Small focused task.
**UAT requirements:**
- All cards (kanban + content) show rendered markdown (no raw escapes)
- Viewer modal renders all GFM features
**Acceptance tests:**
- [ ] `e2e/markdown.spec.ts`: kanban card with `**bold**` shows bolded text
- [ ] Code blocks render in cards (without highlighting) and modal (with highlighting)
- [ ] Tables, lists, footnotes, task lists all render
- [ ] DOMPurify strips `<script>` and `onclick` attrs
- [ ] Bundle size assertion (< 30KB increase)
**Deploy gates:** UAT visual diff → prod

### B2. CO-42: Content page redesign **(critical)**
**Why second:** Depends on CO-39. The biggest UX win.
**UAT requirements:**
- Folder hierarchy renders correctly at all nesting levels
- Cards show rendered markdown
- Zoom viewer opens on click
- Double-click → edit mode
- View dados panel shows accurate stats
- Tasks/events sections collapsed by default
**Acceptance tests:**
- [ ] `e2e/content-folders.spec.ts`: 0 folders → flat list, 1 folder → header + items, nested → recursive
- [ ] `e2e/content-viewer.spec.ts`: click card → modal opens → ESC closes → click outside closes
- [ ] `e2e/content-edit.spec.ts`: double-click → editor → save → re-render
- [ ] `e2e/content-dados.spec.ts`: view dados → all metadata fields present → stats accurate
- [ ] `e2e/content-sections.spec.ts`: tasks collapsed by default, expand persists
- [ ] Mobile: zoom viewer + dados panel work at 375px width
**Deploy gates:** UAT → user testing → prod

### B3. CO-41: Quilomboaraucaria universe
**Why third:** Independent, but big content addition. Validates the import pipeline.
**UAT requirements:**
- Universe `quilomboaraucaria` accessible at `/co/quilomboaraucaria`
- Real content from quilombo-blog visible
- Stats endpoint returns real numbers
**Acceptance tests:**
- [ ] `e2e/quilombo-universe.spec.ts`: visit → content loads → theme applied → stats endpoint works
- [ ] Importer is idempotent (run twice → same data)
- [ ] Posts, events, missions render with correct frontmatter
- [ ] Number of users matches quilombo-blog
**Deploy gates:** UAT → spot-check content → prod

---

## Phase C — Big features

Deeper functionality, depends on stable foundation.

### C1. CO-38: Yggdrasil minigames hub
**Why first in phase:** Self-contained, doesn't affect existing universes.
**UAT requirements:**
- yuri@uat plays each game → score recorded → leaderboard updates
- Anonymous visit to `/co/yggdrasil` → login wall (not 404)
- Migration v14 (`requires_login` column) applied cleanly
**Acceptance tests:**
- [ ] `e2e/yggdrasil-access.spec.ts`: anonymous → login wall, logged in → hub
- [ ] `e2e/yggdrasil-game-tetris.spec.ts`: launch → play → game over → score submitted → personal best updated
- [ ] Repeat for snake, invaders, pointset, poker (5 specs)
- [ ] `e2e/yggdrasil-leaderboard.spec.ts`: global leaderboard accurate
- [ ] `e2e/yggdrasil-profile.spec.ts`: player profile page shows correct stats
- [ ] Mobile: touch controls work for at least Tetris and Snake
- [ ] Other universes (template, clones) still accessible to anonymous (regression test)
**Deploy gates:** UAT → 24h soak (telemetry watching for errors) → prod

### C2. CO-45: UAT → dev change promotion
**Why second in phase:** Depends on CO-44 (UAT env). Enables future iterative development.
**UAT requirements:**
- All write operations on UAT logged to `uat_mutations`
- Export endpoint produces valid tarball
- Tarball can be applied to dev codebase
**Acceptance tests:**
- [ ] `e2e/uat-promote.spec.ts`: make 3 changes on UAT → export → tarball contains expected files
- [ ] Apply tarball locally → next deploy includes changes
- [ ] Production: tracking disabled (CO_ENV != uat assertion)
- [ ] Snapshot version increments correctly
**Deploy gates:** UAT-only feature. Prod deploy only enables the env var check.

### C3. CO-40: UI adequation
**Why third:** Awaiting spec from user. Once spec arrives, this can land alongside other Phase C tasks.
**UAT requirements:** Per the spec when it arrives.
**Acceptance tests:** Visual regression with Playwright screenshots before/after.
**Deploy gates:** UAT visual review → prod

---

## Phase D — Stabilization & v1.1.0 release

### D1. Full E2E test suite consolidation
- [ ] All `e2e/*.spec.ts` from previous tasks collected
- [ ] Run on UAT against deployed code (not local)
- [ ] Run on prod against deployed code (smoke-only, no destructive ops)
- [ ] CI matrix: chromium-desktop, chromium-mobile, firefox-desktop
- [ ] Performance budget: page load < 2s, time-to-interactive < 3s

### D2. Security audit pass
- [ ] OWASP top 10 review (recent CSRF fix already covers one)
- [ ] Rate limiting verified per endpoint
- [ ] CSP headers on all HTML responses
- [ ] Dependency audit: `cargo audit`, `npm audit` (editor + plugin)
- [ ] Manual: try common attack vectors (SQL injection, XSS, JWT tampering)

### D3. Performance pass
- [ ] Telemetry data review (CO-46): identify slow endpoints
- [ ] DB query optimization (EXPLAIN on hot queries)
- [ ] Bundle size review (lazy-load Yggdrasil game JS, CodeMirror)
- [ ] Image optimization (avif/webp)

### D4. Documentation pass
- [ ] CLAUDE.md current and accurate
- [ ] CONTRIBUTING.md reflects new flows (UAT workflow)
- [ ] DEV-TESTING.md has E2E commands
- [ ] CHANGELOG.md complete from v1.0.0 → v1.1.0
- [ ] Schema docs (CO-48) cross-checked

### D5. Release: v1.1.0
- [ ] Bump `Cargo.toml` workspace version → `1.1.0`
- [ ] Update CHANGELOG with all CO-38 to CO-48 changes
- [ ] Tag `v1.1.0`, push to GitHub
- [ ] Create GitHub Release with binaries (macOS, Linux, Windows)
- [ ] Final UAT pass (all E2E green)
- [ ] Deploy to prod
- [ ] Smoke test prod
- [ ] Announce

---

## UAT Workflow (every task)

For each task in Phase A, B, C:

```
1. Local
   ├── Implement task
   ├── cargo test (unit + integration)
   ├── cargo clippy -- -D warnings
   └── Manual smoke test on localhost:8742

2. UAT deploy
   ├── flyctl deploy --config fly.uat.toml
   ├── Clear UAT DB if seed changed: ssh + rm + restart
   ├── Run E2E suite against UAT URL
   └── Manual visual inspection in 2 themes

3. Verification gate
   ├── All acceptance tests pass
   ├── No new errors in telemetry (after CO-46 lands)
   └── Spot-check on mobile

4. Production deploy
   ├── flyctl deploy
   ├── Clear prod DB only if seed changed (rare)
   ├── Smoke test prod
   └── Watch logs for 10min

5. Mark task done
   └── Update CO-N.md status: in_progress → done
```

---

## Execution Order Summary

```
PHASE A (Foundation, 1-2 weeks)
  CO-43 (dev board) ──┐
                       ├─→ CO-44 (UAT env) ──┐
  CO-46 (telemetry) ──┤                      │
       └─→ CO-47 (privacy update)             │
  CO-48 (schema docs) ─ independent           │
                                              │
PHASE B (UX critical, 2-3 weeks)              │
  CO-39 (markdown) ──→ CO-42 (content) ←──────┤
  CO-41 (quilombo) ─ independent              │
                                              │
PHASE C (Big features, 3-4 weeks)             │
  CO-38 (Yggdrasil) ─ independent             │
  CO-45 (UAT promote) ←───────────────────────┘
  CO-40 (UI adequation) ─ awaits spec
                                              
PHASE D (Stabilization, 1 week)
  E2E consolidation → security audit → perf pass → docs → v1.1.0 release
```

## Risk-Adjusted Order

If you want to **minimize risk**, prioritize:
1. CO-46 (telemetry) — gives visibility for everything else
2. CO-43 (dev board) — Yuri's daily driver
3. CO-44 (UAT env) — proper testing environment
4. CO-39 → CO-42 (markdown + content) — fixes the biggest UX gap
5. CO-47 (privacy) — keep legal compliant
6. CO-48 (schema docs) — foundation for future work
7. CO-41 (quilomboaraucaria) — content addition
8. CO-38 (Yggdrasil) — biggest new feature
9. CO-45 (UAT promote) — workflow improvement
10. CO-40 (UI adequation) — awaits spec
```

## Total: 10 tasks → ~6-10 weeks → v1.1.0
