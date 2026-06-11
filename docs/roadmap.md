# CO Roadmap — Wave-based; v3.0.0 (public launch) and v3.1.0 (deploy/pipeline + KB) shipped — next: v3.2

Framed by the **IaaS** thesis (Intelligence as a Service — see [`ArteLonga/docs/intelligence-as-a-service.html`](https://artelonga.com.br/docs/intelligence-as-a-service.html), renamed from BaaS 2026-06-06).

**The bounded intelligence** — everything a schema or API contract can capture (deterministic, functional, verifiable) — **is the service**. **The brain** (biological, human) is deliberately excluded from the deterministic machinery and left free to roam toward creativity and free expression. The service liberates the brain; it doesn't commodify it.

**co é livre** (free software). **ñandé** (inclusive "we-with-you"), not **oré** (exclusive). The audience is part of "we" — never the object of "we own".

**`yuri`** is the reference brain. **`artelonga`** is the business. **CO** is the bounded service — identity · warehouse · payment · sync · event bus.

## Current state (2026-06-11)

- **Version**: v3.1.0 released 2026-06-11 (delivery pipeline + knowledge base); v3.0.0 public launch shipped 2026-06-10. `CHANGELOG-PENDING/` is empty post-release — CO-403 (onboarding/pipeline docs) opens the next cycle
- **Production**: `https://co-artelonga.fly.dev` — single Fly app, shared-cpu-1x, 512 MB
- **Staging** (post-CO-379): `https://staging.co.artelonga.com.br` — hand-deployed, auto-stops; PR-level contract probe is advisory for this reason (strict gate lives in release.yml)
- **Cadence**: bi-weekly Thursday 15:00 BRT releases (PR cutoff Wed 23:59 BRT); CO-382 release-gate cron live since #180
- **Release gate**: CO-382 DoD gate in `release-commit.sh` — blocks on `blocking_failures > 0` or missing `docs/scrum/dod/CO-N.json`; all current pending entries have reports at 0 blocking
- **Per-wave DoD**: `docs/release-checklist.md`

## Wave-aligned releases

Each wave = one git semver tag = a cohesive set of PRs tested + verified end-to-end.

| Wave | Tag | Theme | PR count |
|---|---|---|---|
| ⏪ done | v2.39.0 | Foundation (CO-211/280/291/301/337/338) | 6 |
| ⏪ done | **v2.40.0** | Substrate stable + OSS integrations | 10 |
| ⏪ done | v2.41.0 | Brain interlink + scrum + retro sim | 7 |
| ⏪ done | **v2.42.0** | Unified gestão + cross-env identity + live timeline (CO-360/377/…) | — |
| ⏪ done | **v2.43.0** | Federated event bus + sync + privacy (CO-376/378/383/384/385/389/391/394) | 8 |
| ⏪ done | **v3.0.0 — PUBLIC LAUNCH** (2026-06-10) | Workspace (Sala) + mobile + verification + edge protection — see as-built ledger below | 21 |
| ⏪ done | **v3.1.0** (2026-06-11) | Deploy/pipeline (CO-395/392/398) + KB/funnel/calendar (CO-367/371/372) + layered-arch spike (CO-390, informs CO-227/228). Time/forma (CO-387/396), conteúdo×forma (CO-393) and monetization (CO-366 — funnel steps 7–8 return 0 until billing_events lands) slipped to the next wave | 7 |
| ⚪ next | v3.2.0 | Carry-over: time/forma (CO-387/396) + conteúdo×forma (CO-393) + CO-366 monetization (supervised) + CO-388 security gate. Security epic (encrypted assets, .co format, fs-as-web) + docs wave (CO-402 design doc, CO-403 onboarding/pipeline docs) | ~11 |
| ⚪ planned | v3.3.0 | Sync + offline (op log, conflict UI) | 4 |
| ⚪ planned | v3.4.0 | Scale (job queue, cache, rate tiers, load tests) | 6 |
| ⚪ planned | v3.5.0 | Universe types (manifest plugins, git-backed) | 4 |
| ⚪ planned | v3.6.0 | Native shells (Capacitor) + advanced | 3 |

## Wave 4 — v3.0 (the big public release push)

### As-built ledger (reconciled 2026-06-10)

Everything below merged to `main`; `CHANGELOG-PENDING/` holds the entries that will
form the v3.0.0 changelog. Releases v2.42.0/v2.43.0 already shipped the early items.

| Task | Theme | PR | Merge commit | Shipped in |
|---|---|---|---|---|
| CO-380 universal event bus | 1 EDA spine | (pre-wave) | — | v2.41.0 |
| CO-381 live timeline /agora·/live | 1 | — | — | v2.42.0 |
| CO-384 federated bus bridge | 1 | #171-era | — | v2.43.0 |
| CO-383 Yggdrasil notes ingestion | 7 | #171 | `be80b9f` | v2.43.0 |
| CO-391 WS bridge integration test | 1 | #172 | `451fd46` | v2.43.0 |
| CO-385 UPSERT conflict tree (pulled fwd from v3.1) | — | — | — | v2.43.0 |
| CO-389 lexicon-sala live overlay (pulled fwd) | — | — | — | v2.43.0 |
| CO-394 seed relation extraction | — | #178 | `3cb790d` | v2.43.0 |
| CO-376 pre-prod migration validation | 6 | #173 | `6825eee` | v2.43.0 |
| CO-378 analytics privacy (noindex) | 2 | #166 | `7867729` | v2.43.0 |
| CO-365 storage backend trait | 3 | (batch A) | — | v2.42.0 |
| CO-360 unified /gestao | 3 | — | — | v2.42.0 |
| CO-377 cross-env identity | 6 | — | — | v2.42.0 |
| CO-379 staging app + DNS | 6 | — | — | v2.41.0 |
| CO-352 Sala primitive (one surface) | 4 | #164 | `a3d6062` | pending → v3.0.0 |
| CO-355 workspace template registry | 4 | #163 | `c3c3200` | pending → v3.0.0 |
| CO-354 suggest/review pipeline | 4 | #182 | `c58ddd5` | pending → v3.0.0 |
| CO-375 API contract probe | 6 | #179 | `f95ce22` | pending → v3.0.0 |
| CO-382 scrum DoD CI/CD | 7 | #180 | `423645e` | pending → v3.0.0 |
| CO-374 Playwright staging suite | 6 | #181 | `bea76f4` | pending → v3.0.0 |
| CO-397 rate limits + robots/sitemap (was CO-278-B) | 2 | #183 | `71ec4ad` | pending → v3.0.0 |
| CO-356 touch DnD (pointer events) | 5 | #185 | `7848975` | pending → v3.0.0 |
| CO-357 PWA shell | 5 | #186 | `052375f` | pending → v3.0.0 |
| CO-358 mobile IA reflow | 5 | #187 | in CI | pending → v3.0.0 |
| CO-359 mobile E2E CI matrix | 6 | #184 | parked: rebase after #187 | pending → v3.0.0 |

**Design decision (2026-06-09)**: the Sala is ONE surface with fractal scope —
`docs/architecture/sala-surface.md`. SPA tab = launcher; canvas lives only in
`shared/sala.html`. All-universes/subset scope + universe-as-node recursion are
"Open work" there and still need CO-N ids (CO-398 was taken by the delivery pipeline).

**v3.0.0 ships when**: #187 + #184 merge → `release-commit.sh 3.0.0 "public launch"`
(DoD gate green) → Thursday ritual: staging deploy → full Playwright suite →
release-gate → tag → retrospective.

**Theme**: Brain on any device, observable in real time, public.

**21 PRs** organized into **7 changelog themes** (CHANGELOG entries group by theme, not per-PR). Batched A/B/C/D for CI sanity. Each batch fires when prior batch's PRs merge.

### Changelog themes (v3.0)

| Theme | PRs | What ships |
|---|---|---|
| **1. Event-driven spine** | CO-380, CO-381, CO-384 | Universal event bus + live timeline at `/agora` (pt-BR) / `/live` (en) + federated bridge (CO ↔ Yggdrasil ↔ devices, no polling) |
| **2. Edge protection + privacy** | CO-278-B, CO-378 | Rate limits, abuse heuristics, noindex respect, robots.txt, sitemap.xml |
| **3. Substrate hardening** | CO-365, CO-360 | Pluggable backup backend (local default); unified /gestao/resumo dashboard |
| **4. Workspace primitive (Sala)** | CO-352, CO-354, CO-355 | Spatial canvas + suggest/review + template registry; absorbs Yggdrasil sala |
| **5. Mobile shell** | CO-356, CO-357, CO-358 | Touch DnD + PWA install + mobile IA reflow |
| **6. Staging + verification** | CO-379, CO-374, CO-375, CO-376, CO-377, CO-359 | Staging Fly app, Playwright suite, contract enforcement, migration validation, cross-env identity, mobile CI matrix |
| **7. Scrum CI/CD + Yggdrasil read** | CO-382, CO-383 | DoD-verifiable CI per task; read-only ingest of Yggdrasil notes |

### Batch A — substrate + edge gates (5 PRs, fire first)
| PR | Spec | Theme | What it ships |
|---|---|---|---|
| **CO-379** | Staging Fly app + DNS | 6 | `staging.co.artelonga.com.br` live |
| **CO-365** | Storage backend trait | 3 | LocalFsBackend default; S3/R2/Fly/GCS stubs |
| **CO-278-B** | Rate limits + abuse heuristics | 2 | 60/min anon; X-RateLimit-* headers |
| **CO-360** | Unified `/gestao/resumo` | 3 | One Svelte page, 4 tabs |
| **CO-380** | Universal event bus (EDA spine) | 1 | tokio broadcast + event_log + 6 subscribers |

### Batch B — Sala + identity + privacy (6 PRs)
| PR | Spec | Theme | What it ships |
|---|---|---|---|
| **CO-352** | Workspace primitive | 4 | `entry_type: workspace` + per-user state |
| **CO-354** | Suggest/review pipeline | 4 | draft → reviewed → published lifecycle |
| **CO-355** | Workspace template registry | 4 | `_workspace.yaml` per universe |
| **CO-377** | Cross-env identity (Phase 1) | 6 | yuri creds work on prod + staging |
| **CO-378** | Analytics privacy (noindex respect) | 2 | Private paths redacted in `/gestao/resumo` |
| **CO-381** | Live timeline `/agora` + `/live` | 1 | WebSocket-fed real-time observability |

### Batch C — mobile + Yggdrasil event-driven read (5 PRs)
| PR | Spec | Theme | What it ships |
|---|---|---|---|
| **CO-356** | Touch DnD on board | 5 | Pointer-events replaces HTML5 DnD |
| **CO-357** | PWA shell | 5 | Manifest, SW, install, offline cache |
| **CO-358** | Mobile IA pass | 5 | Drawer, breadcrumb collapse, board reflow |
| **CO-384** | Federated event bus bridge | 1 | Cross-deployment WS pub/sub (CO ↔ Yggdrasil ↔ devices). **No polling.** |
| **CO-383** | Yggdrasil notes event-driven ingest | 7 | CO subscribes to Yggdrasil's bus; notes flow in real-time via CO-384 |

(Note: CO-353 WebSocket lobby is **superseded by CO-380** — workspace presence becomes one consumer of the universal bus.)

### Batch D — Validation + DoD CI + tagging (4 PRs)
| PR | Spec | Theme | What it ships |
|---|---|---|---|
| **CO-374** | Playwright E2E suite for staging | 6 | 6 scenario files + acceptance generator |
| **CO-375** | API contract enforcement | 6 | Probe drift gates prod release |
| **CO-376** | Pre-prod migration validation | 6 | Snapshot+migrate+smoke per migration PR |
| **CO-359** | Mobile E2E CI matrix | 6 | Pixel 7 / iPhone 14 / iPad Pro projects |
| **CO-382** | Scrum-aligned CI/CD with DoD verification | 7 | Per-PR DoD gate, sprint review automation, release-gate.yml |

**v3.0 release ships** when Batch D closes + every PR has green DoD verification (CO-382) + `docs/release-checklist.md` Wave 4 section all green + retrospective passes.

## All open todo/in_progress work mapped

**Wave-5 outcome** (v3.1.0, 2026-06-11): CO-395/392/398 + CO-367/371/372 + CO-390
shipped (7 entries). The time/forma leg (CO-387 → CO-396+YG-123), CO-393, and
CO-366 monetization (still `todo` — funnel steps 7–8 await billing_events) did not
make the cut and carry over to v3.2, where CO-388 still gates any new public surface.


| ID | Title | Wave |
|---|---|---|
| CO-352…360, 365, 374…379, 397 | Workspace + mobile + verification + edge (see as-built ledger) | **v3.0 Wave 4 — merged** |
| CO-387 | Time-rendering primitive `<co-time-grid>` (high — prerequisite of CO-396/YG-123) | v3.2 carry-over — time/forma |
| CO-396 | Project timeline lens (gantt over `<co-time-grid>`, shared engine with YG-123) | v3.2 carry-over — time/forma |
| CO-393 | Composable universe UI — content lenses, schema-driven forms (spec CO-387/396 against this frame: one lens system) | v3.2 carry-over — conteúdo×forma |
| CO-395 | construir — markdown → Quartz public site (de-facto pattern of grcsamazonia/mse) | ✅ shipped v3.1.0 |
| CO-392 | co push — CLI → remote universe CRUD via Vault API | ✅ shipped v3.1.0 |
| CO-398 | Delivery pipeline no quadro — VC/deploy events drive status via CO-380 bus + GitHub webhooks (automation fills, never locks; client deploys keep stakeholder go) | ✅ shipped v3.1.0 |
| CO-388 | Security audit pipeline (Glasswing) — slot BEFORE any new public surface ships | v3.2 gate |
| CO-390 | SPIKE layered architecture — feeds implementation style of the rest | ✅ shipped v3.1.0 |
| Sala fractal scope (all-universes /sala, subset, universe-as-node) | needs CO-N ids — `sala-surface.md` Open work | v3.2 candidate |
| CO-367, 371, 372 | KB + funnel + calendar (cheap post-CO-382) | ✅ shipped v3.1.0 |
| CO-366 | Monetization — conversão/pagamento Hostinger (supervised, not headless; funnel steps 7–8 return 0 until its billing_events land) | v3.2 carry-over |
| CO-403 | Onboarding & pipeline docs — WELCOME.md (history-teller onboarding, two-doors universe in/out) + delivery-pipeline invariant (review localhost → approve → merge, git × jj) | next release — docs |
| **CO-104, 119** | **S3 backup + restore drill — not started; same S3 dep behind interim git backups and the failing CO-143 backup cron. Highest-leverage ops item on the board.** | **ops, schedule now** |
| CO-145 | Encrypted assets | v3.2 Wave 6 |
| CO-86, 87, 110 | `.co` format + protocol stack + fs-as-web | v3.2 Wave 6 |
| CO-61, 62, 128, 58 | Sync protocol + adapter + conflict UI + PWA offline | v3.3 Wave 7 |
| CO-76, 78, 79, 80, 101, 285, 286 | Scale infra + job queue + cache + rate tiers + load + Fly cost | v3.4 Wave 8 |
| CO-63, 70, 89, 93 | Universe types + manifest plugins + git-backed | v3.5 Wave 9 |
| CO-344, 211 v2, 264 | Native shells + advanced | v3.6 Wave 10 |
| CO-227, 228, 170, 144, 283, 284, 298 | Ongoing refactors (no wave) | Continuous |
| CO-94, 143, 146, 162, 277 | Already resolved (status flipped today) | — |

## The BaaS invariants (must hold across every wave)

1. **3 separated layers** — brain content / brain surface / CO platform
2. **Sovereign edge** — brain owner controls hardware + data; CO is consented async
3. **No third-party lock-in** — every external integration behind a trait
4. **Cache-first delivery** — local write renders immediately; sync eventual; ingest break ≠ unavailable

## The bi-weekly release cadence (cron-driven from Wave 4)

```
Monday-Wednesday: PR work
Wednesday 23:59 BRT: PR cutoff for current sprint
Thursday 06:00 BRT: main HEAD → staging auto-deploy
Thursday 06:00-12:00 BRT: Playwright full suite vs staging
Thursday 12:00 BRT: green = release candidate; red = abort
Thursday 12:00-14:30 BRT: human review window
Thursday 15:00 BRT: prod deploy via release-commit.sh + git tag
Thursday 15:05 BRT: retrospective begins
```

CO-372 calendar surfaces the cadence; CO-376 gates with migration validation; CO-374 gates with E2E suite.

## Cost addendum

- Current prod: ~$3-8/mo (shared-cpu-1x, 512 MB, 3 GB volume)
- Add staging (Wave 4): +$3/mo
- Total post-Wave 4: ~$6-11/mo
- Growth tier (50 universes): ~$8/mo prod + $3/mo staging
- Yggdrasil-scale (200 universes): ~$25-30/mo prod + $3/mo staging

## Conventions

- Branch: `feat|fix|refactor/CO-<n>-<short-desc>` (no `issue-` prefix)
- Commits: conventional, `Co-Authored-By: Claude <noreply@anthropic.com>`
- Spec format: `work/co/CO-<n>.md` with YAML frontmatter
- Forbidden in agent PRs: `Cargo.toml`, `co-cli/Cargo.toml`, `CHANGELOG.md` (release-commit owns)
- Changelog entries: `CHANGELOG-PENDING/CO-<n>.md`
- Merge: `scripts/safe-merge-pr.sh artelonga/co <pr>` (never bare `gh pr merge --delete-branch`)

## Reference repos

| Repo | Role | Surfaced as universe? |
|---|---|---|
| `artelonga/co` | the platform (this repo) | `co` |
| `artelonga/ArteLonga` | BaaS thesis + reference surfaces (`docs/brain-as-a-service.md` · `scrum/scrum.md`) | `artelonga` + child universes `yuri`, `neuro` |
| `artelonga/quilomboaraucaria` | **canonical reference architecture** (analytics, telemetry, atividades, schema_versoes, OpenAPI codegen — patterns CO adopts) | `quilomboaraucaria` |
| `artelonga/yggdrasil` | game runtime + sala (workspace absorbed into CO Wave 4) | `yggdrasil` |
| `artelonga/comunicacao` | mbya + yoruba lexicon source | `comunicacao` (parent of `mbya`, `yoruba`) |
| `artelonga/topologia` | concept plane source (CO-141) | `topologia` (parent of language family) |
| `artelonga/mbya` | Arandu — standalone Mbyá Guarani lexicon (Rust + Dioxus + SQLite) | `mbya` (promoted to first-class via CO-348) |
| `artelonga/retro-umarizal` | neighborhood memory site | `retro-umarizal` (CO-347) |
| `artelonga/rfq-gateway` | RFQ trade log | `rfq` |
| `pewdiepie-archdaemon/odysseus` | open-source AI workspace reference (read-only) | `odysseus` (CO-364) |
| `anthropics/claude-code` | Anthropic agentic CLI reference (read-only) | `claude-code` (CO-364) |

## Pre-flight gate (every release)

See `docs/release-checklist.md` for the full per-wave checklist.
