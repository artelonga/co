# CO Roadmap — Wave-based; v3.0.0 → v3.15.0 shipped (full history in `CHANGELOG.md`) — next: TBD (owner to sequence)

Framed by the **IaaS** thesis (Intelligence as a Service — see [`ArteLonga/docs/intelligence-as-a-service.html`](https://artelonga.com.br/docs/intelligence-as-a-service.html), renamed from BaaS 2026-06-06).

**The bounded intelligence** — everything a schema or API contract can capture (deterministic, functional, verifiable) — **is the service**. **The brain** (biological, human) is deliberately excluded from the deterministic machinery and left free to roam toward creativity and free expression. The service liberates the brain; it doesn't commodify it.

**co é livre** (free software). **ñandé** (inclusive "we-with-you"), not **oré** (exclusive). The audience is part of "we" — never the object of "we own".

**`yuri`** is the reference brain. **`artelonga`** is the business. **CO** is the bounded service — identity · warehouse · payment · sync · event bus.

## Current state (2026-06-15)

- **Version**: **prod = v3.15.0** (released 2026-06-14 — telemetry cold-tier archival to Parquet, job queue + worker pool, native-OTel usage capture, folder-as-sub-sala). **`CHANGELOG.md` is the source of truth for what shipped** — the per-wave table below was last reconciled at v3.4.0 and lags the changelog; treat the changelog as authoritative for 3.5.0 → 3.15.0. Shipped 3.5.0–3.15.0 (see CHANGELOG): security gate + ops resilience (3.5.0), fleet observability + model routing (3.6.0), composable architecture (3.7.0), payment/onboarding (3.8.0), post-git sync engine (3.9.0), resilience + chat/timeline (3.10.0), token scopes + scrum board (3.11.0), federation + public API (3.12.0), fractal sala + API envelope (3.13.0), StaaS storage foundation (3.14.0), telemetry/job-queue/OTel (3.15.0).
- **Production**: `https://co-artelonga.fly.dev` (Fly `gru`) — the only required deploy target.
- **Staging**: `co-artelonga-staging` exists but is a **manual/optional preview** (`flyctl deploy --config fly.staging.toml`), NOT a release gate. **UAT is decommissioned** (`co-artelonga-uat` destroyed 2026-06-01). Authoritative env/deploy description: [`docs/OPERATIONS.md` → "Environments & Deploy"](OPERATIONS.md).
- **Deploy**: prod-direct — local checks → CO-421 read-only prod-usability gate → `scripts/pipeline-deploy-gate.sh` (CO-446 disk gate + fresh green local pipeline report) → `flyctl deploy` → `scripts/smoke-prod.sh`.
- **Release gate**: CO-382 DoD gate in `release-commit.sh` — blocks on `blocking_failures > 0` or missing `docs/scrum/dod/CO-N.json` (override `--ignore-dod` when per-PR `dod-verify` already passed but the persisted JSON isn't on main; recent releases used this).
- **Per-wave DoD**: `docs/release-checklist.md`

> **Forward plan is stale and needs owner sequencing.** The wave table below predates 3.5.0–3.15.0; several "planned" items have since shipped — notably **Wave 8 (scale)**: CO-78 job queue (shipped 3.15.0) and the CO-449 telemetry cold-tier/storage work (3.14.0/3.15.0). Re-sequence against `work/co/ROADMAP.md` (canonical ordering) before relying on the table.

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
| ⏪ done | **v3.1.0** (2026-06-11) | Deploy/pipeline (CO-395/392/398) + KB/funnel/calendar (CO-367/371/372) + layered-arch spike (CO-390, informs CO-227/228) | 7 |
| ⏪ done | **v3.2.0** (2026-06-11) | Composable universe UI / content lenses (CO-393) + onboarding/pipeline docs (CO-403, WELCOME.md) + `co updates` CLI release notes (CO-404) | 3 |
| ⏪ done | **v3.3.0** (2026-06-11) | Sala grid landscape — type-on-square + working drag-and-drop + folders (CO-410) | 1 |
| ⏪ done | **v3.3.1** (2026-06-11) | Backup never blocks the boot — disk guard, debounce, count-based retention (CO-405) | 1 |
| ⏪ done | **v3.4.0** (2026-06-12) | `source: github` adapter (repo→universe, ipynb→md, CO-417) + `<co-time-grid>` time/calendar lens (CO-387) + render-review-publish traceback (CO-418) + CLI User-Agent fix & docs/CLI.md (CO-411) + tutorial parity (CO-412) | 5 |
| ⚪ **next** | **v3.5.0** | **Security gate + ops hardening + prod-e2e**: CO-388 security audit pipeline (Glasswing) + CO-406 graceful startup degradation + CO-407 uptime alerting + CO-408 ops small-batch + CO-420 Yggdrasil UX→epics/timeline. CO-421 prod-e2e usability gate is the candidate cap | ~6 |
| ⚪ planned | **Wave 6 — security epic** | `.co` format + filesystem-as-web + encryption envelope — design pinned in `architecture/wave6-security-epic.md` (CO-402). CO-86/87/110 + CO-145 epic. Encryption-at-rest precondition (CO-104 backup + CO-119 restore drill) now **done** | — |
| ⚪ planned | Wave 7 — sync + offline | Op-log sync protocol + conflict UI + PWA offline (CO-61/62/128/58) | 4 |
| ⚪ planned | Wave 8 — scale | Job queue, cache, rate tiers, load tests, Fly cost (CO-76/78/79/80/101/285/286 + CO-111…113) | 6 |
| ⚪ planned | Wave 9 — universe types | Manifest plugins, git-backed (CO-63/70/89/93 + CO-116) | 4 |
| ⚪ planned | Wave 10 — native shells | Capacitor + advanced (CO-344/211-v2/264) | 3 |

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

**Reconciled 2026-06-12.** Since launch, v3.1.0 → v3.4.0 all shipped. The
time/forma leg landed: **CO-387 `<co-time-grid>` shipped in v3.4.0** (CO-396 the
project-timeline lens over it is still `todo`). **CO-393 composable lenses shipped
in v3.2.0.** The CO-417/418 source→universe→publish journey (epic CO-414) shipped
in v3.4.0. CO-388 (security gate) is now done and pending in v3.5.0; the
encryption-at-rest precondition (CO-104 backup + CO-119 restore drill) is **done**,
so Wave 6 is unblocked. CO-366 monetization is still `todo` (funnel steps 7–8 await
`billing_events`).

> ⚠️ **Verify before claiming "shipped".** This table cross-checks each claim
> against `git tag`, `CHANGELOG.md`, and the `status:` field of `work/co/CO-N.md`.
> A prior pass wrongly claimed CO-366 shipped; CO-366 is **still `todo`**.

| ID | Title | Status / Wave |
|---|---|---|
| CO-352…360, 365, 374…379, 397 | Workspace + mobile + verification + edge (see as-built ledger) | ✅ shipped v3.0.0 |
| CO-367, 371, 372, 390, 392, 395, 398 | KB/funnel/calendar + layered spike + co push + construir + delivery pipeline | ✅ shipped v3.1.0 |
| CO-393, 403, 404 | Composable lenses + WELCOME.md onboarding + `co updates` CLI | ✅ shipped v3.2.0 |
| CO-410 | Sala grid landscape (type-on-square, DnD, folders) | ✅ shipped v3.3.0 |
| CO-405 | Backup never blocks boot (disk guard, debounce, retention) | ✅ shipped v3.3.1 |
| CO-387, 411, 412, 417, 418 | `<co-time-grid>` + CLI UA fix + tutorial parity + source:github + traceback | ✅ shipped v3.4.0 |
| CO-388 | Security audit pipeline (Glasswing) — gate before new public surface | ✅ done — **pending v3.5.0** |
| CO-406 | Graceful startup degradation — universe-pool pragma failure tolerance | `todo` — **v3.5.0** |
| CO-407 | Uptime alerting — prod health probe with push notification | ✅ done — **pending v3.5.0** |
| CO-408 | Ops small-batch — staging DNS, Google OAuth error body, misc | ✅ done — **pending v3.5.0** |
| CO-420 | Yggdrasil UX → epics/user-stories + timeline + median-TTC report | ✅ done — **pending v3.5.0** |
| CO-421 | Prod-e2e usability gate — anonymous read-only Playwright smoke | `todo` — v3.5.0 candidate cap |
| CO-396 | Project timeline lens (gantt over `<co-time-grid>`, shared engine with YG-123) | `todo` — v3.5.0+ |
| CO-414 | EPIC experiences-as-features (Miguel/Yuri/Yggdrasil) | `in_progress` — 417/418/419/420 shipped; **CO-415/416 pending** |
| CO-415 | Login com GitHub (OAuth) — Miguel journey | `todo` — v3.5.0+ |
| CO-416 | Auto pt-br ↔ en translation — Miguel journey | `todo` — v3.5.0+ |
| CO-413 | Bidirectional bridge universes — event-bus universos | `todo` — Wave 7 (sync-adjacent) |
| CO-419 | Universo yuri/nlp — SensorySpeech + real telemetry | ✅ shipped v3.4.0 |
| CO-366 | Monetization — conversão/pagamento (supervised; funnel steps 7–8 await `billing_events`) | `todo` — unscheduled |
| Sala fractal scope (all-universes /sala, subset, universe-as-node) | needs CO-N ids — `sala-surface.md` Open work | candidate |
| **CO-104, 119** | **S3 backup + restore drill — now `done`. The encryption-at-rest precondition for Wave 6 is satisfied. CO-143 backup-cron is `superseded` (daily snapshots now in-process).** | ✅ **done — Wave 6 unblocked** |
| CO-145 (+ CO-146…150) | Encrypted indexable lazy-loaded assets — children done; epic closeable | Wave 6 — close epic |
| CO-86, 87, 110 | `.co` format + protocol stack + fs-as-web (design: `architecture/wave6-security-epic.md`) | Wave 6 |
| CO-402 | Wave-6 security-epic design doc | ✅ done — this PR |
| CO-61, 62, 128, 58 | Sync protocol + adapter + conflict UI + PWA offline | Wave 7 |
| CO-76, 78, 79, 80, 101, 285, 286 | Scale infra + job queue + cache + rate tiers + load + Fly cost | Wave 8 |
| CO-63, 70, 89, 93 | Universe types + manifest plugins + git-backed | Wave 9 |
| CO-344, 211 v2, 264 | Native shells + advanced | Wave 10 |
| CO-227…231 | Server-decomposition refactor epics — all children done; **closeable at release** | Continuous |
| CO-170, 144, 283, 284, 298 | Ongoing refactors / infra (no wave) | Continuous |
| CO-94, 143, 146, 162, 277 | Resolved / superseded | — |

> Full open-epic index with child user-stories: `architecture/epics-backlog.md`.

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
