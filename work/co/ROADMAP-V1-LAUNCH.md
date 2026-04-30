---
title: "Release roadmap — current state to final v1 launch (SUPERSEDED)"
status: superseded
superseded_by: ROADMAP-V2-PLATFORM-REVIEW.md
priority: high
created_at: 2026-04-29T00:00:00Z
updated_at: 2026-04-30T08:49:22Z
---

# Release roadmap (V1 — superseded 2026-04-30)

> **This doc is superseded by [`ROADMAP-V2-PLATFORM-REVIEW.md`](ROADMAP-V2-PLATFORM-REVIEW.md).**
>
> The V2 doc keeps the **capability gates** of the V1 Tier 0–5 model but reframes them as **Phase 0–5** with explicit infrastructure deltas (Cloudflare CDN, Workers Analytics Engine, ClickHouse, Redpanda, Iceberg-on-R2, Flink, deployer adapters) drawn from the SR engineer's plan and `docs/platform-evaluation.md`. The V2 framing also adds four user-named v1 requirements that V1 missed:
>
> 1. Apple-style 4-way conflict UX (CO-128)
> 2. Jujutsu-shaped changelog renderer (CO-129)
> 3. Restore-drill cadence, not just snapshots (CO-119)
> 4. Multi-target deployer abstraction (CO-133/134/135)
>
> The Tier 0–5 → Phase 0–5 mapping table is in `SPRINT-V1-LAUNCH.md`. The V1 doc is kept in tree as a historical record of how the original framing read; **do not consult it for current planning**.

---

# Release roadmap (V1 archived content below)

A staged plan from **today (1.21.x — public test on `co.artelonga.com.br`)** to **stable v1 launch**. Tiers are gated by what real users can plausibly do without help; each gate adds a class of capability.

The roadmap is opinionated about ordering. Two principles:

1. **Storytelling first, scale second.** The platform must be *demonstrable* — anonymous visitor lands, sees something interesting, can self-create. Only then does it earn the right to load-balancing and op-logs.
2. **Trust before features.** Every tier closes a category of "we can't honestly say this works." Avoid stacking new capabilities on top of un-soaked ones.

## Where we are (snapshot 2026-04-29)

| | |
|---|---|
| Versions | `co-cli 0.29.x`, `co-web 1.21.x` |
| Live URLs | `co.artelonga.com.br` (prod), `co-artelonga-uat.fly.dev` (UAT) |
| Universes seeded | template, quilomboaraucaria, yggdrasil, **tempo**, **humanity**, **universo**, plus admin's: artelonga, rfq, qa-dev |
| Auth | password-login on prod (CO-85, in_progress → mostly done), API tokens via keychain |
| Visualizations | Kanban, Tabela, Conteúdo, Painel, **Linha do tempo (multi-universe overlay)** |
| Persistence | SQLite on Fly volume, content_count tracking, idempotent re-seeds |
| Recently shipped | universe-as-repo (CO-50), deterministic access (CO-49), atomic universe switching, modern theme override, frontmatter preview, orphan rescue, anon clutter cleanup, timeline trio, network-first SW |

## What's NOT done that could embarrass us

These are the gaps I'd flag if a friend asked "is Co really ready?":

1. **No universe CRUD UI** — universes are created via API/script. Real users can't make their own from the SPA. Blocking for v1.1.
2. **No backup story** — Fly volume is the only copy. Single point of failure.
3. **No load testing** — never proven the box can serve >1 user concurrently.
4. **Marketing site → Co telemetry** — endpoint shipped, marketing flip pending.
5. **Encryption at rest is roadmap, not reality** — privacy policy is honest about this, but it's a real limit.
6. **Sync from desktop (Obsidian, CLI, mobile) is half-built** — vault API works; CRDT path works for one universe; full sync UX (`co sync`) is open.
7. **`Meu Co` clutter** — fixed in 1.21.x via cleanup but indicates rough edges around anon flows.

The roadmap below answers each in priority order.

---

## Tier 0 — Stop the bleeding (this week, ~5 days work)

**Goal:** the prod surface is unembarrassing for a small alpha cohort. No new code, mostly verification + ops.

| | What | Owner | Done when |
|---|---|---|---|
| 0.1 | **Commit + push** the 1.20.2 → 1.21.1 working-tree diff (~12 files). Single coherent commit per logical chunk if needed. | yuri | `git status` clean, pushed to `origin/main` |
| 0.2 | **Verification pass** — anonymous flow + logged-in flow, all six themes, three timeline universes, conteudo view with frontmatter preview. Use `docs/feedback-checklist.md`. | yuri | All boxes checked, screenshot of timeline overlay |
| 0.3 | **Marketing site `ENDPOINT` flip** to `https://co.artelonga.com.br/api/v1/telemetry/events`, bump cache buster. | yuri | First row lands in `telemetry_events` with `site=artelonga` |
| 0.4 | **Backup automation** — daily cron snapshots Fly volume to S3-compatible storage. Keep last 30 + monthly. | yuri | First snapshot in S3, restore tested in scratch app |
| 0.5 | **Health check expansion** — `/api/health` already returns version. Add `/api/health/deep` that probes DB read, write, and disk. UptimeRobot / similar pinging it every 5 min. | yuri | Alert fires within 5 min of synthetic failure |
| 0.6 | **Lock down secrets** — rotate `JWT_SECRET` once. Put `CO_SEED_ADMIN_PASSWORD_HASH` in Fly secrets (already done?), verify `flyctl secrets list -a co-artelonga`. | yuri | Secrets verified, prod restart with new JWT_SECRET (forces re-login — communicate window) |

**Exit criteria:** 5 friends can use Co for 24 hours without yuri intervening.

---

## Tier 1 — Demoable v1.1 (1-2 weeks)

**Goal:** a stranger can self-create a universe from the SPA, document something in it, share a link, and have it persist.

### Tickets

- **CO-96** — Universe CRUD UI in the SPA (sidebar `+ New universe`, rename, change visibility, duplicate, delete with 30-day soft-delete). Phase 1 = create only. Phase 2 = rename + visibility. Phase 3 = soft-delete + restore.
- **NEW: CO-98** — Universe categories. Add a `category` field to universes; surface "Linhas do tempo" group prominently on template's home (currently a content page, should be structural). Phase 1 = data model + seed. Phase 2 = SPA discovery view.
- **NEW: CO-99** — Onboarding flow. First-time visitor walkthrough: 1) you're on template, 2) here are the views, 3) try the timeline, 4) make your own universe. Skippable, tracks completion in cookie.
- **NEW: CO-100** — Documentation pass. `docs/ARCHITECTURE.md`, `docs/OPERATIONS.md`, `docs/ONBOARDING.md`, `docs/CONTRIBUTING.md`. Reflect actual current state, not aspirational.
- **CO-83** — Mermaid: already shipped. Wire it into the new home page so universe descriptions can include diagrams.

### Stress test foundation (parallel track)

- **NEW: CO-101** — Load test script. `tests/load/` with k6 / vegeta scenarios for the top 10 endpoints. Baseline: 50 concurrent users, p95 < 500ms on `/entries`. Document the *exact* failure mode at 100, 500, 1000 (we expect SQLite contention before 1000 — that's fine, we just want to know).
- **NEW: CO-102** — Profile + capture. One scenario at a time, 10-min run, capture flamegraph + slow-query log. Commit results to `tests/load/baselines/<date>/`.

### Stack health

- **CO-64** — Post-GitHub cleanup. Remove dead `git_sync.rs` paths (we're keeping universe-as-repo for read-only sync but removing GitHub-specific assumptions). Document the universe-as-repo invariants.
- **NEW: CO-103** — Per-deploy regression test. The three timeline universes should always have N events; their presence is a smoke-test sentinel post-deploy.

**Exit criteria:**
- Stranger creates a universe from the SPA, makes 3 entries, shares a public link.
- Load test report committed showing measured p50/p95/p99 at 50 / 100 / 500 RPS.
- `docs/ARCHITECTURE.md` accurately describes the system as of 1.1.

---

## Tier 2 — Sustained public test v1.5 (~1 month)

**Goal:** Co handles a school class / team / community sustained over a week without operator intervention. Multi-device sync starts to feel real.

### Sync surface (the big one)

- **CO-91** — `co sync` canonical content-author UX. jj-tracked delta + automated changelog + co-token auth. Replaces `seed-prod-universes.sh` for end users.
- **CO-68** — Obsidian plugin deep-sync. Auto-sync, pull-on-open, conflict UI (INFRA-3). Vault API already works; this is the polish layer.
- **CO-69** — PWA offline. IndexedDB cache + Background Sync (INFRA-4). The new SW is network-first, but offline path through SW + IndexedDB is its own work.
- **CO-51** — CLI sync command. `co sync pull/push/watch`. Conflict resolution UX (last-write-wins by default, `--strategy ours/theirs/merge` flag).

### Operational

- **CO-77** — Per-universe SQLite + global metadata DB + LiteFS read replicas. Big win for isolation (one user's slow query doesn't block another's reads), and a prerequisite for Tier 3 multi-region.
- **CO-79** — Caching layer. Manifest, `theme.css`, hot queries. CDN strategy (Fly's built-in for now; Cloudflare in front later).
- **CO-80** — Per-tier rate limiting + quota. Token bucket per user/tier/operation. Currently zero throttling — first abusive client takes everyone down.
- **NEW: CO-104** — Admin telemetry dashboard. The visitor token / endpoint flip data should land somewhere yuri can read. Simple Grafana or a custom `/api/v1/admin/dashboard` returning aggregates.

### Identity

- **CO-97** — Unify visitor token (`visitante_id` ↔ `al_vid`). Already specced. Action depends on the May 13 telemetry-flip check having data to evaluate the three options.
- **CO-90** — Drop global admin tier. Tier becomes billing-only. Authority is per-universe via `owner_id` + `universe_members.role`. Privacy boundary.

**Exit criteria:**
- 50 simultaneous active users, no degradation.
- One Obsidian user has been syncing for a week without losing data.
- yuri has a dashboard showing daily traffic + retention.

---

## Tier 3 — Real collaboration v2.0 (~2-3 months)

**Goal:** Co becomes a *collaboration* platform. Two people can edit the same universe from different devices and the result is correct.

### Sync Protocol v1

- **CO-61** — Sync Protocol v1: op log + content-addressed blobs + 3-way merge + recursive resolution. The flagship architectural piece. CO-95 Phase 1 (universe duplicate) is the snapshot variant; this is the streaming variant.
- **CO-54** — Idempotency + conflict resolution. Concurrent edits across sync + web + co-auto. Op IDs + Lamport clocks + commutative ops where possible.
- **CO-95 Phase 2-4** — Universe branching. Phase 1 (snapshot duplicate) shipped. Phase 2 = op log foundation. Phase 3 = deterministic replay onto duplicate. Phase 4 = merge back to source.
- **CO-62** — Quilombo-blog sync adapter. UAT ↔ prod 3-way merge for photos. Proves the protocol on real data before we open it to others.

### Desktop / mobile

- **CO-58** — Desktop tray sync app + PWA offline (Phase 2-4 of sync roadmap). Tray icon shows sync status; click for log; settings for which universes auto-sync.
- **NEW: CO-105** — Capacitor mobile wrapper. iOS + Android, reuses the SPA. First version is online-only; offline ships with PWA work.

### Stress

- **CO-78** — Job queue + worker pool. Doc generation, sync, indexing, changelog all become async jobs.
- **CO-81** — Object storage for blobs. Filesystem sharding. Currently everything is in `/data/universes/...` — fine for thousands, breaks at millions.
- **CO-76** — Scalability infrastructure. Capstone ticket; covers anything left after CO-77/78/79/80/81.

**Exit criteria:**
- Two devices editing the same universe show consistent state within 5 seconds of last edit.
- Op log can replay any universe to any point in time.
- 500 simultaneous users tested.

---

## Tier 4 — Encrypted / archive-grade v3.0 (~4-6 months)

**Goal:** the privacy claims in `data/universes/template/content/privacidade.md` become true. Operators cannot read user content.

- **CO-86** — `.co` file format. Protobuf-wrapped markdown for transport-optimized, encrypted, self-describing content. Includes envelope encryption (ChaCha20-Poly1305) with user-derived keys.
- **CO-87** — Composable protocol stack. Hardware → cache → storage → network → privacy → security as `Layer` traits. Encryption-at-rest plugs in here as the "privacy" layer.
- **CO-93** — Universe types unified: public-static / private-static / private-dynamic. Each has a different sync + deploy story. Architecture document has it; needs to be true in code.
- **NEW: CO-106** — Recovery story for encrypted universes. If user forgets password, what happens? Write the recovery + escrow flow before encryption ships.

**Exit criteria:**
- yuri-as-operator cannot read another user's content even with full server access.
- Privacy policy updates to remove the "operator can technically read" disclaimer.
- Independent security review (even self-conducted) of the envelope.

---

## Tier 5 — Programmable platform v4.0+ (later)

**Goal:** Co is a substrate for arbitrary structured-content products. Other people build things on it.

- **CO-63** — Universe manifest + content-type plugin system. Per-universe schemas, doc generators, temporal+relational queries.
- **CO-70** — Manifest format spec. `_universe.yaml` at universe root.
- **CO-71** — Per-universe schema validator + generic JSON entry storage.
- **CO-72** — Doc-generator hooks. Scaladoc, Sphinx, MkDocs, ReDoc, Rustdoc, JSDoc.
- **CO-73** — Temporal model. First-class semantic dates (event_at, due_at, scheduled_at, …). Generalizes the timeline.
- **CO-74** — Relationship graph. Typed FK references + query DSL + wikilink promotion.
- **CO-75** — Version reconstruction. Replay op log to any timestamp; auto-changelog.
- **CO-89** — Git-backed universes (post-GitHub). Every repo-backed universe gets commits, profiles, events, analytics, Mermaid views.
- **CO-88** — End-to-end pipeline UAT. Localhost ↔ API ↔ web with per-universe stats (file size, transfer, telemetry).

**Exit criteria (long-horizon):**
- A third party publishes a content type to the registry.
- A community runs Co as their primary tool, not as ours.

---

## Cross-cutting tracks (run in parallel through all tiers)

### Stress testing

- Tier 1: baseline (k6, 50 users)
- Tier 2: sustained (1 hour, 200 users, real-data scenarios)
- Tier 3: chaos (random failures, network partitions)
- Tier 4: encrypted-throughput regression (the new format mustn't 10× the cost)

### Documentation

- Tier 1: ARCHITECTURE, OPERATIONS, ONBOARDING in PT and EN
- Tier 2: API reference auto-generated from route definitions
- Tier 3: client-library docs (Obsidian plugin, CLI, future SDKs)
- Tier 4: cryptography review notes
- Tier 5: plugin-author guide

### Repo population (yuri's content)

- artelonga (private) — production data
- quilomboaraucaria (public) — community blog
- qa-dev — working notes
- rfq — internal
- ArteLonga marketing site — telemetry consumer

Each gets: regular content updates, monitored metrics, soak time.

---

## What I recommend you do *next* (concrete order)

1. **Right now** — verify 1.21.1 works as advertised. Open `https://co.artelonga.com.br/shared/timeline.html?u=tempo,universo,humanity`, click `‹ ›`, toggle universes. If anything's broken I want to know in this session.
2. **Today** — commit + push. The working tree has a meaningful chunk of fixes; sitting uncommitted is a tail risk if I (or you) mess something up.
3. **Tomorrow** — Marketing endpoint flip + backup automation (Tier 0.3 + 0.4). Both unblock real-world signal.
4. **This week** — start CO-96 Phase 1 (universe create modal). Highest user-visible gap. Once that ships, the platform stops feeling like a developer artifact.

Items beyond that should be reviewed weekly against the tier exit criteria; they're not commitments yet.
