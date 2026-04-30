---
title: "Review of SR-engineer platform-infrastructure roadmap (quilomboaraucaria → co)"
status: review
priority: high
created_at: 2026-04-30T00:00:00Z
reviewer: yuri (with Claude as drafting peer)
reviewed_doc: SR-engineer plan §7 (data infra) / §8 (10-layer arch + co-agent + privacy) / §9 (4-phase rollout)
context_docs:
  - docs/platform-evaluation.md (Parts I-III, this session)
  - work/co/ROADMAP-V1-LAUNCH.md (Tier 0-5)
  - work/co/SPRINT-V1-LAUNCH.md (current wave status, 2026-04-30)
---

# Review — SR engineer's platform-infrastructure plan vs. CO reality

## TL;DR

**Endorse the architecture, reject the timeline framing, add the missing pieces.**

The §8 ten-layer model is structurally correct. The data-infra picks (Redpanda > Kafka, Flink > Spark, Iceberg-on-R2 as keystone) match the analysis in `docs/platform-evaluation.md` Part II. The §8.3 honesty about the encryption/analytics tradeoff is what a senior engineer should write — but it under-specifies the privileged compute zone, which is the hardest privacy-engineering problem in the design.

The §9 phasing — _"Phase 0: MVP on Cloudflare alone, no streaming"_ — **misreads CO's starting state**. CO is at v1.21.x in public test on Fly+LiteFS with native Rust; "MVP on Cloudflare alone" is a rewrite, not Phase 0. The phasing needs to interleave with the existing Tier 0–5 product roadmap, not replace it.

The plan is missing four things CO has explicitly named as requirements: (1) jujutsu-shaped conflict resolution, (2) multi-target deployer abstraction, (3) restore-drill cadence, (4) tier/quota model for multi-tenancy.

---

## §A — What the SR plan gets right (no changes needed)

| Area | SR plan position | Reviewer note |
|------|------------------|---------------|
| Redpanda > Kafka | "Kafka API, single binary, no ZK, ~6× lower p99" | ✅ Confirmed. Native Iceberg Topics (Parquet) collapse two components into one. ([docs.redpanda.com/current/manage/iceberg](https://docs.redpanda.com/current/manage/iceberg/about-iceberg-topics/)) |
| Flink > Spark for streaming | "exactly-once stateful, ms latency" | ✅ Confirmed. Spark for batch ML and backfills only. |
| Iceberg-on-R2 keystone | "open table format, free egress, snapshot=backup, cross-cloud portability is a config switch" | ✅ Confirmed. Pair with REST catalog spec (Polaris / Lakekeeper / Nessie) — see `platform-evaluation.md` §13. |
| Pinot ≈ ClickHouse | "tie on capability ... pick on operator taste" | ⚠️ Endorse, but **make a recommendation rather than punting**. See §B.5. |
| 10-layer model (L0→L9) | Surface → orchestration → OLTP → cache → bus → stream → OLAP → lake → batch → deploy targets | ✅ Clean layering. Add edge labels to clarify which layer owns which guarantee. |
| Co-agent as Rust sidecar | "tails logs, encrypts, batches+zstd, pushes to Redpanda, heartbeats" | ✅ Right idea. Underspecified for non-sidecar targets — see §C.1. |
| Encryption tradeoff honesty | "Pinot/ClickHouse can't query ciphertext directly" + privileged decryption zone | ✅ The honest answer. Underspecified hardening — see §C.2. |

---

## §B — What needs pushback

### B.1 — "Phase 0: MVP on Cloudflare alone" misreads the starting state

CO is **already past MVP**:
- Live on `co.artelonga.com.br` (prod) and `co-artelonga-uat.fly.dev` (UAT)
- 9 universes seeded, kanban + tabela + conteúdo + painel + linha-do-tempo views shipped
- Auth (password-login, API tokens via keychain), 6 themes, frontmatter preview, atomic universe switching
- v1.21.x with `co-cli 0.29.x`, Tier 0 nearly closed (per `SPRINT-V1-LAUNCH.md`)

A "Cloudflare-only MVP" implies rewriting `co-core` (Rust, native rusqlite, FFmpeg, Argon2id) to Workers Wasm. That's quarters of work for marginal latency gain and loses the existing investment. The recommendation in `platform-evaluation.md` §5 — **Fly compute + Cloudflare storage/edge** — stands.

**Revised Phase 0 framing:** "Cloudflare in front of Fly (CDN, Pages, R2), no streaming yet, no rewrite." This is achievable in weeks, not months.

### B.2 — Phase timing decoupled from sprint reality

SR plan: 2 mo / 5 mo / 9 mo / 14 mo. Existing roadmap (`ROADMAP-V1-LAUNCH.md`) phases by **capability gate**, not calendar:
- Tier 0 = stop the bleeding (5 days)
- Tier 1 = demoable v1.1 (1-2 weeks)
- Tier 2 = sustained public test v1.5 (~1 month)
- Tier 3 = real collaboration v2.0 (~2-3 months)
- Tier 4 = encrypted/archive-grade v3.0 (~4-6 months)
- Tier 5 = programmable platform v4.0+

Calendar-anchored phases promise dates the team can't keep. **Replace SR's calendar phases with capability gates that each include an infrastructure delta.**

### B.3 — "Tinybird makes it serverless ... pick on operator taste" is a punt

For CO specifically the recommendation is **ClickHouse self-hosted on Fly Machine** (Phase 2C) before any Tinybird/Pinot consideration:
- Native binary, runs in same Fly region as `co-web` (low ingest latency from `co-agent`)
- Iceberg read via `iceberg(...)` table function
- No vendor lock-in, no per-row pricing surprise
- Pinot only if user-facing analytics with thousands of QPS becomes a product surface — which Tier 5 might justify, Tier 2-3 will not.

### B.4 — Phase 0 "MVP, no streaming" skips the trivial-cost telemetry win

`Cloudflare Workers Analytics Engine` is free at CO's scale and gives real-time low-cardinality telemetry **today** without standing up Redpanda/Iceberg. Existing Tier 0.5 (deep health) and CO-104 (admin dashboard) already imply this work. SR plan should land WAE in Phase 0 as the cheap-tier telemetry, then graduate to Redpanda when WAE limits bite.

### B.5 — A/B testing parked at Phase 2 is wrong-by-default

A/B testing primitives (feature flag table + exposure events table + holdback assignment) are a **week of work on the existing OLTP**, not a quarter of work on a streaming pipeline. Build them in Phase 1. Iceberg/Pinot enter the picture only when A/B *analysis* outgrows ad-hoc DuckDB queries.

---

## §C — What's missing

### C.1 — Co-agent strategy varies by deployment target

A "Rust sidecar" works on Fly Machines and bare-metal containers. It does not work on:

| Target | Reality | Strategy |
|--------|---------|----------|
| Fly Machine | Real sidecar process | ✅ Co-agent as designed |
| Cloudflare Worker | No separate processes | **Tail Workers** (CF native log-streaming Worker that ships to Redpanda ingest) |
| Cloudflare Pages | Static, no runtime | **Browser SDK beacon** + edge-cache hit telemetry from CF Logpush |
| Vercel Function | No sidecar; outbound HTTP only | **Log Drains → CO ingest endpoint** + edge instrumentation in route handlers |
| Fargate task | Real sidecar process | ✅ Co-agent as designed |
| Static-on-R2 | No runtime | **Browser SDK only** |

Co-agent must therefore be **a strategy, not a process**. The architecture doc should name the variant per target. Adapter shape ≈ `trait CoAgent { fn ship(events: &[Event]) -> Result<()> }` with implementations per target.

### C.2 — Privileged compute zone for decrypted analytics needs hardening spec

§8.3's "privileged compute zone where Flink decrypts events" is a phrase, not a design. The hardening spec must include:

1. **Network isolation.** The zone runs in a Fly app/region disjoint from `co-web`; only inbound is from Redpanda topic `events.encrypted.*`; only outbound is to OLAP store `events.aggregated.*`. No public ingress.
2. **Key access audit log.** Every `K_u` unwrap is recorded with `(timestamp, universe_id, job_id, output_aggregation_id)`. Audit log lives outside the zone (write-only from inside).
3. **DLP at the boundary.** Output rows must pass a k-anonymity check (k ≥ 5 by default). Per-user or per-content rows cannot leave the zone.
4. **Job allow-list.** Aggregation jobs are pre-registered code, not arbitrary SQL. No `SELECT *`. No interactive notebooks against the zone.
5. **Time-bounded key access.** Job leases `K_u` for its duration, returns it. No long-lived in-memory key cache outside the zone's runtime.

Without these, "operator cannot read user content" (Tier 4 exit criterion) is a marketing claim, not an engineering guarantee.

### C.3 — Conflict resolution / jujutsu changelog is absent

User explicitly named Apple-style 4-way conflict UX (`Ignore / Replace / Keep both / Apply to all`) and jujutsu-shaped changelog rendering as v1 requirements. SR plan doesn't address these. They live at L1/L2 (orchestration + OLTP) of the 10-layer model and **don't depend on the streaming layers at all** — implementable in Tier 3 (CO-61 / CO-54 / CO-95) without waiting for Redpanda.

Catalog choice has a tie-in: **Apache Nessie**'s git-like data branching mirrors the jujutsu mental model. Worth a deeper look as the lake catalog when Phase 2 lands.

### C.4 — Multi-target deployer abstraction missing

`platform-evaluation.md` Part III §18 specifies a `deploy.yaml` manifest per universe and a deployer-adapter abstraction (`static-on-R2`, `cloudflare-pages`, `fly-app`, `vercel`, `fargate`). This is what makes "CO is the only surface" true. Absent from SR plan. Belongs in Tier 3 or early Tier 5.

### C.5 — Restore drills, not just snapshots

"Iceberg snapshots = backup" is necessary but not sufficient. Add a quarterly **restore drill**: pick a snapshot, restore to a scratch app, diff against original, document time-to-restore. Without it, the backup is unproven.

### C.6 — Tier/quota model for multi-tenant scaling

Anonymous = 100 entries (already enforced). Paid tiers raise the cap, but the model isn't specified. Needed before billing surfaces ship. Belongs in Tier 2 alongside CO-80 (rate limiting).

---

## §D — Reconciled roadmap (interleaved with existing tiers)

Phases below align to the existing Tier 0-5 capability gates. Each phase pairs **product capability** (existing tickets) with the **infrastructure delta** (new from SR plan + this review). Don't skip a phase to chase the next; each pays for itself.

### Phase 0 — Foundation hardening (current, ~5 days)

**Capability gate (existing Tier 0):** prod surface unembarrassing for small alpha cohort.

**Infrastructure delta (new):**
- ✅ Cloudflare in front of Fly (CDN cache for static assets, Pages for SPA build) — _no rewrite_.
- ✅ Workers Analytics Engine wired for `/api/*` exposure events — replaces SR plan's "no streaming yet" with cheapest possible telemetry path.
- ✅ R2 bucket created, encryption envelope spec'd (CO-86 prep), no migration yet.

**Tickets in flight (per SPRINT-V1-LAUNCH.md):**
- CO-103 (smoke test) — 🟡 in-progress
- CO-106 (deep health) — ⬜ ready
- CO-96 P1 / CO-98 / CO-99 / CO-100 / CO-104 / CO-107 — Wave 2-3
- CO-104 (backups) → **add: restore drill script** (§C.5)

**New tickets (file in this phase):**
- **CO-PLAT-1** — Cloudflare in front of `co.artelonga.com.br` (cache rules, preserve auth cookies)
- **CO-PLAT-2** — WAE binding from `co-web` for exposure + telemetry events
- **CO-PLAT-3** — Restore-drill script + quarterly cron + result log

### Phase 1 — Demoable + telemetry (Tier 1, 1-2 weeks)

**Capability gate:** stranger creates universe → makes 3 entries → shares public link.

**Infrastructure delta:**
- Co-agent **strategy** spec (§C.1) — adapter shape, not implementation. No streaming bus yet.
- A/B primitives on existing OLTP (§B.5): `feature_flags`, `ab_assignments`, `ab_exposures` tables; assignment via stable hash; exposure logging via WAE.

**Existing tickets (Tier 1):**
- CO-96 (universe CRUD UI), CO-98 (categories), CO-99 (onboarding), CO-100 (docs), CO-101 (load test), CO-102 (profile), CO-83 (Mermaid)

**New tickets:**
- **CO-PLAT-4** — Co-agent adapter trait + first impl (Fly sidecar variant)
- **CO-PLAT-5** — A/B primitives on OLTP (3 tables + assignment helper + exposure logger)
- **CO-PLAT-6** — Quota/tier model spec doc (no enforcement yet — see Phase 2)

### Phase 2 — Sustained public test (Tier 2, ~1 month)

**Capability gate:** 50 simultaneous active users, no degradation; one Obsidian user syncing for a week.

**Infrastructure delta:**
- Per-universe SQLite + global metadata DB + LiteFS read replicas (CO-77, already on plan)
- ClickHouse (single Fly Machine) for ad-hoc analytics queries over WAE export + co-agent events
- Tier/quota enforcement (CO-80 + CO-PLAT-6 outputs)

**Existing tickets (Tier 2):**
- CO-91 (`co sync` UX), CO-68 (Obsidian deep-sync), CO-69 (PWA offline), CO-51 (CLI sync), CO-77 (per-universe SQLite), CO-79 (caching), CO-80 (rate limiting), CO-104 (admin dashboard), CO-97 (visitor token unify), CO-90 (drop global admin tier)

**New tickets:**
- **CO-PLAT-7** — ClickHouse single-node on Fly + `iceberg(...)` table function ready (no Iceberg lake yet; query WAE export only)
- **CO-PLAT-8** — Co-agent CF Worker tail variant + Vercel Log Drain variant (so user deployments on those targets ship telemetry too)

### Phase 3 — Real collaboration + streaming bus (Tier 3, ~2-3 months)

**Capability gate:** two devices editing same universe → consistent state within 5s.

**Infrastructure delta:**
- **Redpanda** small cluster (1-3 brokers on Fly Machines, or Redpanda Cloud BYOC)
- **Iceberg lake on R2** with **REST catalog** (start with Lakekeeper; evaluate Nessie for jujutsu alignment)
- **Flink** first job (sessionization or A/B exposure → metric join)
- Conflict-resolution UI ships here (CO-61 / CO-54 / CO-95) — **Apple-style 4-way + Apply-to-all** + jujutsu changelog render

**Existing tickets (Tier 3):**
- CO-61 (sync protocol v1), CO-54 (idempotency + conflict), CO-95 (universe branching), CO-62 (quilombo-blog adapter), CO-58 (desktop tray), CO-105 (Capacitor), CO-78 (job queue), CO-81 (object storage)

**New tickets:**
- **CO-PLAT-9** — Redpanda cluster on Fly (or Cloud BYOC eval) + Iceberg Topics enabled + Schema Registry
- **CO-PLAT-10** — REST catalog (Lakekeeper) deploy + Iceberg-on-R2 first table (`events`)
- **CO-PLAT-11** — First Flink job: session stitching, output to ClickHouse `sessions_aggregated`
- **CO-PLAT-12** — Apple-style conflict UI (`Ignore / Replace / Keep both / Apply to all`)
- **CO-PLAT-13** — Jujutsu-shaped changelog renderer (read op log, render commit DAG)

### Phase 4 — Encrypted / privileged zone (Tier 4, ~4-6 months)

**Capability gate:** operator cannot read user content even with full server access.

**Infrastructure delta:**
- `.co` envelope format ships (CO-86)
- **Privileged compute zone** for analytics decryption with all five hardening controls (§C.2)
- Co-agent payloads encrypted under `K_u` before Redpanda
- Aggregation jobs are pre-registered, k-anonymity-enforced

**Existing tickets (Tier 4):**
- CO-86 (`.co` file format), CO-87 (composable protocol stack), CO-93 (universe types unified), CO-106 (recovery story)

**New tickets:**
- **CO-PLAT-14** — Privileged zone Fly app (separate org/region; locked-down ingress; audit log target)
- **CO-PLAT-15** — Aggregation job allow-list + k-anonymity DLP at zone egress
- **CO-PLAT-16** — Key-access audit log (write-only from zone; read-only from `co-web`)

### Phase 5 — Programmable platform + multi-deployment (Tier 5, later)

**Capability gate:** third party publishes a content type to the registry; user deploys their universe to their own domain.

**Infrastructure delta:**
- Multi-target **deployer abstraction** ships (§C.4): `deploy.yaml` schema + adapters for `static-on-R2`, `cloudflare-pages`, `fly-app` (then later: `vercel`, `fargate`)
- Cross-universe federated analytics (Pinot or ClickHouse cluster, per-universe partitions, central queries)
- Marketplace surface (Tier 5 capstone)

**Existing tickets (Tier 5):**
- CO-63, CO-70, CO-71, CO-72, CO-73, CO-74, CO-75, CO-89, CO-88

**New tickets:**
- **CO-PLAT-17** — `deploy.yaml` schema + universe-level deployment manifest validation
- **CO-PLAT-18** — Deployer adapter trait + first impl (`static-on-R2`)
- **CO-PLAT-19** — Second deployer adapter (`cloudflare-pages`)
- **CO-PLAT-20** — Pinot eval (only if user-facing analytics with kQPS becomes a product surface)

---

## §E — Critical UAT test steps (per phase)

UAT spec already lives in `CLAUDE.md` §"UAT Verification Spec" (1-10). Each phase below adds the deltas. Run these against `co-artelonga-uat.fly.dev` after every deploy in that phase. Hard-fail any check that regresses.

### Phase 0 UAT additions

| # | Check | How |
|---|-------|-----|
| 0-A | Cloudflare in front: cache HIT for `theme.css`, cache BYPASS for `/api/*` | `curl -I https://co.artelonga.com.br/theme.css` → expect `cf-cache-status: HIT` after warmup; `curl -I .../api/health` → `BYPASS` |
| 0-B | WAE event lands within 60s of exposure | Trigger 1 page-view, wait 60s, query WAE for the row |
| 0-C | Restore drill: snapshot S → scratch app → first-page render OK | Run `tools/restore-drill.sh <snapshot-id>` → expect green |

### Phase 1 UAT additions

| # | Check | How |
|---|-------|-----|
| 1-A | Universe CRUD: stranger creates universe, makes 3 entries, shares link | Manual; documented in `feedback-checklist.md` |
| 1-B | A/B primitive: assigning user `u_test` to flag `home_v2` is stable across 10 reads | `curl /api/v1/ab/assign?user=u_test&flag=home_v2` × 10 → all same value |
| 1-C | Co-agent (Fly variant) ships heartbeat every 60s, drops events under backpressure | Tail target's logs; confirm heartbeat cadence; saturate ingest, observe backoff |

### Phase 2 UAT additions

| # | Check | How |
|---|-------|-----|
| 2-A | LiteFS replica: write to primary, read from replica region within 2s | `flyctl ssh console` to two regions, write+read |
| 2-B | Per-universe SQLite isolation: slow query in universe A doesn't block universe B | Run synthetic 5s query in A while measuring B p99 |
| 2-C | ClickHouse query over WAE export: aggregate yesterday's events under 1s | `clickhouse-client -q "SELECT count() FROM events WHERE date=yesterday()"` |
| 2-D | Quota enforced: anonymous user blocked at entry 101 (already in spec) | Existing UAT step §2.7 — confirm |
| 2-E | Co-agent CF Worker tail variant: deploy a sample Worker, confirm events arrive | Sample worker emits 100 logs → assert ≥99 in CO ingest |

### Phase 3 UAT additions

| # | Check | How |
|---|-------|-----|
| 3-A | Redpanda Iceberg Topic: produce 1k events → land in Iceberg-on-R2 within 30s | Use `rpk topic produce`, then query the Iceberg table |
| 3-B | Flink session-stitch job: 5 events from same `session_id` → 1 row in `sessions_aggregated` | Produce 5 events, wait 30s, query ClickHouse |
| 3-C | Conflict UI 4-way: simulate concurrent edit → modal shows `Ignore / Replace / Keep both / Apply to all` | Playwright: open same entry on 2 sessions, edit both, save B then A |
| 3-D | Jujutsu changelog: branch → 3 commits → merge → render shows DAG with 3 nodes + merge node | Playwright + visual diff |
| 3-E | Two-device sync: edit on device A and B, both converge within 5s | Manual; record screencast |

### Phase 4 UAT additions

| # | Check | How |
|---|-------|-----|
| 4-A | Operator-cannot-read: `flyctl ssh console -a co-artelonga` → `cat /data/universes/<u>/content/*.co` shows ciphertext, not plaintext | Manual SSH check |
| 4-B | Privileged zone isolation: scan zone's outbound — no destinations except `events.aggregated.*` topic | `flyctl machine status` + network policy check |
| 4-C | k-anonymity DLP: aggregation job that would produce a row with count<5 → rejected at egress | Submit synthetic narrow query; expect rejection |
| 4-D | Key audit log: every decrypt produces an audit row; tampering attempts logged | Trigger 10 jobs, assert 10 audit rows; modify a row in-place, assert detection |
| 4-E | Recovery flow: user with lost password runs through escrow → regains access | Manual; documented in CO-106 |

### Phase 5 UAT additions

| # | Check | How |
|---|-------|-----|
| 5-A | `deploy.yaml` validation: malformed manifest rejected with specific error | 10 fixtures in `tests/fixtures/deploy/` |
| 5-B | Static-on-R2 deployer: universe `u_test` deploys → public URL serves first page | Trigger deploy, `curl` returns 200 on landing page |
| 5-C | Cloudflare Pages deployer: same universe, target=cloudflare-pages → deploys to `*.pages.dev` | Same shape, different adapter |
| 5-D | Cross-universe analytics: query against 3 universes returns federated result | ClickHouse/Pinot query with `WHERE universe_id IN (u1,u2,u3)` |

---

## §F — Open questions for the SR engineer (quilomboaraucaria team)

Before any of this gets blessed, surface answers to:

1. **Phase 0 framing** — do you accept the "no rewrite" constraint? CO is past MVP. Phase 0 is "Cloudflare in front of Fly," not "rewrite to Cloudflare-only." Confirm or push back.
2. **Co-agent on Vercel/Cloudflare/static targets** — your sidecar model doesn't apply. Do you accept the per-target adapter strategy in §C.1?
3. **Privileged compute zone hardening** — do you sign off on the five controls in §C.2 as the bar? Or propose stricter (Nitro Enclaves) / looser (single-control)?
4. **Pinot vs ClickHouse for CO** — your "operator taste" punt is fine for quilombo but not for CO. **ClickHouse self-hosted** is my recommendation; will you take it?
5. **Catalog choice** — Lakekeeper, Polaris, or Nessie? Nessie's data-branching aligns with CO's jujutsu mental model. Worth a 1-day spike before Phase 3.
6. **Kafka/Redpanda hosting** — Fly Machines self-host vs. Redpanda Cloud BYOC. What ops budget are we willing to spend here?
7. **Conflict-UX design ownership** — who writes the spec for the Apple-style 4-way modal + Apply-to-all? Frontend or core?

---

## §G — Recommended next 3 actions

1. **This week** — close the existing Tier 0 sprint (CO-103 / CO-106 / CO-96 P1 / etc.). Don't start new Phase 0 platform tickets until current wave is shipped. Inventory check, not new work.
2. **Next week** — file CO-PLAT-1, CO-PLAT-2, CO-PLAT-3 as new sprint tickets; co-auto them in parallel where safe (PLAT-1 + PLAT-3 are parallel; PLAT-2 depends on PLAT-1 for cache routing).
3. **Within 2 weeks** — schedule a 60-min sync with the SR engineer to walk through §F open questions and align on Phase 1+ scope before any infrastructure ticket lands.

---

**Reviewer's bottom line:** The SR engineer's plan is structurally right and timeline-wise wrong. Endorse the architecture, replace the calendar phasing with the existing tier-gated phasing, harden the privacy zone spec, add the four missing requirements (conflict UX, deployer adapters, restore drills, quotas). Then it ships.
