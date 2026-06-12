# CO — Release Roadmap (PM-sequenced)

> Canonical **ordering** authority for the CO backlog. Last sequenced: **2026-06-12**.
> (For the shipped v1.0 MVP plan, see Appendix A at the bottom.)
>
> **Read order from the Release column, not from `priority`.** `priority` now
> means *strategic weight*; the **Release** below means *when*. A `critical` item
> parked at v4.0 is strategically vital but **not** "do next" — the release slot
> governs sequence. Items in `status: backlog` are deferred behind a signal (see
> "Parked" at the bottom); they are not scheduled until that signal fires.

## Product north star

CO is a **GitHub-independent, post-git content platform** — universes (content
workspaces) that are markdown-canonical, served as sites, e2e-usable and
**monetizable** in prod. "co é livre"; bounded intelligence is the paid service.
The product must (1) take money, (2) deliver the "replace git" sync promise,
(3) stay cheap to run at current scale.

## Release sequence

### v3.6 — Fleet observability *(in flight)*
Internal: make co-auto spend visible so model routing is data-driven.
- CO-426 usage API + dashboard · CO-427 model routing · CO-437 usage metadata
  (tool/output/PR + model×universe) · CO-429 claude-code universe
- **Exit:** deploy → close CO-419 §E (nlp re-parent), close CO-414 thread.

### v3.7 — Mythos *(architecture; on Fable, serial, characterized)*
Internal enabler, **justified**: CO-431 (Universo→core) and CO-433 (storage
shard + EntryStore) are prerequisites for both scale and the v4.0 sync core.
- Epic CO-430 → CO-431, CO-432, CO-433, CO-434, CO-435 (+CO-297 rate-limit), CO-436
- Builds on the done CO-284 trait layer (enforces/extends it).

### v3.8 — Money & Activation ⭐ *(the neglected lever — pull forward)*
Product is live & usable (CO-421 ✓) but takes no payment and has weak first-run.
Highest user/revenue ROI per effort.
- **CO-366** conversion + payment (Hostinger checkout, provider-agnostic trait)
- **CO-99** onboarding banner (three-step coach-mark) *(bumped to high)*
- **CO-401** staging fixtures + `CO_STAGING_ADMIN_TOKEN` (de-risks CI; kills the
  recurring contract-probe red)
- **CO-281** Fly cost — ph1/ph2 (CO-285 auto-suspend, CO-286 extract co-embedding)

### v4.0 — Post-git Core: Sync 🎯 *(the differentiator; MAJOR, multi-iteration)*
The "replace git" promise. Breaking → MAJOR. Likely spans v4.0.x.
- Foundation: **CO-70/CO-63** manifest + content-type plugins → **CO-61** Sync
  Protocol v1 (op log + content-addressed blobs + 3-way) → **CO-75** version
  reconstruction
- UX: **CO-51/CO-91** `co sync` CLI · **CO-54/CO-128** idempotency + conflict UI ·
  **CO-96** universe CRUD UI · **CO-90** drop global admin tier
- Hardening: **CO-88** e2e pipeline UAT · **CO-64** post-GitHub cleanup ·
  **CO-78/79/80** job queue + caching + per-tier rate-limit · **CO-81** blob object-store
- Adjacent: **CO-89/CO-93** git-backed + types/sync · **CO-98** hierarchical universes ·
  **CO-86/CO-87** `.co` format + protocol stack · **CO-62** quilombo-blog sync adapter

### v4.x — Open platform
- **CO-278** public API remainder (agent dispatch + universe + telemetry; phase-1
  rate-limits already done)
- **CO-110** Filesystem-as-Web (needs the sync core first)
- **CO-281** ph3/ph4 (CO-287 right-size, CO-288 cost panel)

### Continuous / opportunistic *(fold into whichever release touches the area)*
- UX: CO-283 graph canvas · CO-353 lobby/presence · CO-338 surface keys ·
  CO-368 scrum artifacts · CO-396 timeline lens · CO-399/CO-400 sala · CO-413 bridge ·
  CO-144 dados panel · CO-212 cloud viewer
- Hygiene: CO-170 universe hygiene · CO-210 security/deps/license SPA · CO-241
  content-volume metrics · CO-97 visitor-token unify · CO-178 geo · CO-204 chat
  origin · CO-409 variant-i debt · CO-120 co-agent adapter · CO-231 docs epic

## Parked → `backlog` (deferred behind a signal)

**Theme F — data-lake / horizontal scale.** Hedix-grade analytics for kQPS
user-facing load. You have one operator + a few users. Per CO-284's own charter:
*don't pay the scale tax until measured need.* **Gate: paying users or measured
query load.**
- Phase epics: CO-111, CO-112, CO-113, CO-114, CO-115, CO-116
- CO-118 Workers Analytics · CO-121 A/B-on-OLTP · CO-123 ClickHouse · CO-124
  co-agent CF/Vercel variants · CO-125 Redpanda · CO-126 Iceberg catalog · CO-127
  Flink · CO-130 privileged compute zone · CO-131 DLP/k-anon · CO-132 key-access
  audit · CO-135 CF Pages deployer · CO-136 Pinot
- CO-101 load-test scaffolding (premature pre-PMF)

**Not now** (revisit when core/PMF justifies): CO-58 desktop tray · CO-68/CO-213
Obsidian deep-sync · CO-108 backup-to-HD · CO-109 mbya stress universe · CO-298/CO-299
local `--staging` (no-UAT decision).

## Standing PM guardrails
1. **WIP limit:** at most one internal-enabler epic in flight. After Mythos,
   pivot to user-facing (v3.8) — protect that pivot; resist a third internal epic.
2. **Money is P0 once usable** — and it's usable now. Don't let CO-366 slip past v3.8.
3. **`high` is not a roadmap.** ~30 items were `high`; that's noise. This file is
   the ranking; re-derive priority from the Release column.

---

# Appendix A — Historical: v1.0 MVP roadmap (shipped)

> Preserved for provenance. These phases shipped through 2025–early 2026 (board
> overhaul, public MVP at artelonga.com.br/co, Obsidian ecosystem, telemetry/UAT).

## Phase 1–2: Board (done)
1–7. CO-2..CO-8: Board API + UI overhaul ✅

## Phase 3: Public MVP — artelonga.com.br/co (Epic: CO-20, done)
- 3a Core arch: CO-21 universe CRUD ✅ · CO-36 entry abstraction · CO-24 content/form
- 3b Platform: CO-23 usage gate · CO-25 theme gating · CO-30 dynamic CSS
- 3c Editor/collab: CO-29 CodeMirror · CO-31 CRDT sync
- 3d Frontend/i18n: CO-26 i18n · CO-22 template ✅ · CO-27 landing
- 3e Deploy/quality: CO-32 Ansible · CO-33 E2E
- 3f Release: CO-28 OSS repo setup

## Phase 4: Obsidian Ecosystem (v1.1)
- CO-35 Vault REST API + Clipper · CO-34 Obsidian plugin

## Phase 5: Polish, Telemetry, UAT (post-v1.0)
- CO-39 markdown pipeline · CO-40 UI adequation · CO-41 quilomboaraucaria deploy ·
  CO-42 content redesign · CO-43 dev board · CO-44 UAT env · CO-45 UAT→dev ·
  CO-46 telemetry · CO-47 privacy update · CO-48 schema docs
