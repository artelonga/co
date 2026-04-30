---
title: "Deployment-order review of the platform tickets (CO-111 through CO-136)"
status: review
priority: high
created_at: 2026-04-30T00:00:00Z
reviewer: yuri (with Claude as drafting peer)
context_docs:
  - work/co/ROADMAP-V2-PLATFORM-REVIEW.md
  - work/co/SPRINT-V1-LAUNCH.md
  - work/co/ROADMAP-V1-LAUNCH.md
---

# Deployment-order review

The phase grouping in `ROADMAP-V2-PLATFORM-REVIEW.md` §D is right; the **within-phase ordering** and **cross-roadmap dependencies** were under-specified when I drafted CO-111 through CO-136. This review identifies the issues and locks the order down.

## TL;DR — 12 issues found

1. **CO-119** (restore drill) depends on **CO-104** (backup automation), which is in the *current sprint*, not Phase 0. Don't start CO-119 until CO-104 ships.
2. **CO-117** (CF CDN) should land before **CO-118** (WAE). The Worker proxy for WAE is clean only when the zone is on Cloudflare.
3. **CO-121** (A/B primitives) requires **CO-118** for exposure logging. Acceptance criteria call for WAE; can't be parallel with CO-118.
4. **CO-126** (Lakekeeper REST catalog) is a hard prerequisite for **CO-125** (Redpanda Iceberg Topics). I implied parallel; correct order is CO-126 → CO-125.
5. **CO-127** (Flink session-stitch) depends on CO-125 + CO-126 *and* on **CO-123** (ClickHouse sink) from Phase 2. Cross-phase dependency that needs an explicit gate.
6. **Phase 4** (CO-115) has hidden prereqs from the existing Tier 4 roadmap: **CO-86** (`.co` envelope), **CO-87** (composable protocol stack), **CO-93** (universe types unified). No `K_u` exists until those land — the privileged zone has nothing to unwrap.
7. Within Phase 4, **CO-130** (zone) must precede CO-131 + CO-132. CO-131 and CO-132 are parallel with each other but both gated on CO-130.
8. **CO-128 / CO-129** (conflict UI + jujutsu changelog) depend on existing **CO-61 / CO-54 / CO-95** (sync protocol + branching), not on the Phase 3 streaming infrastructure. They form an **independent track within Phase 3**.
9. **CO-122** (quota spec) is doc-only with no code dependencies. Ship it anywhere — even Phase 0 — instead of holding it in Phase 1.
10. **CO-136** (Pinot eval) is gate-conditional, not scheduled. Mark explicitly "blocked until evidence" rather than slot it in Phase 5.
11. **Sprint contention.** The current sprint (`SPRINT-V1-LAUNCH.md` Wave 1-5) touches shared files — especially `app.js`. Don't start Phase 0 platform work until Wave 1 is committed and pushed (or you'll hit merge conflicts on the same surface).
12. **Phase 3 is the heaviest phase.** Subdivide into 3a (UX track) and 3b (streaming track) — they're independent and can ship in either order.

---

## Cross-roadmap dependencies (existing tickets that gate platform work)

| Platform ticket | Gated by existing ticket(s) | Why |
|-----------------|------------------------------|-----|
| CO-119 | CO-104 (Tier 0) | Restore needs `restore.sh` from CO-104 |
| CO-123 | CO-77 (Tier 2) | LiteFS replicas should be live before adding ClickHouse sibling |
| CO-128, CO-129 | CO-61, CO-54, CO-95 (Tier 3) | Conflict UI + DAG render need a real op-log |
| CO-130 | CO-86, CO-87, CO-93 (Tier 4) | Privileged zone unwraps `K_u`; doesn't exist before envelope |

These four bridges decide the actual rollout cadence. Platform Phase _N_ cannot start until the bridge from Tier _N_ of the existing roadmap is across.

---

## Corrected per-phase DAG

### Phase 0 (CO-111)

```
   ┌─────────────────────────┐
   │ existing CO-104         │  (Tier 0, in current sprint)
   └─────────┬───────────────┘
             ▼
        ┌────────┐
        │ CO-119 │  restore drill
        └────────┘
             ┃   (parallel to)
   ┌────────┐    ┌────────┐
   │ CO-117 │ ─► │ CO-118 │  CDN → WAE
   └────────┘    └────────┘
```

**Parallel-safe set:** `{CO-119}` ‖ `{CO-117 → CO-118}`. Three tickets, two worktrees.

### Phase 1 (CO-112)

```
   CO-118 (Phase 0)
      │
      ▼
   ┌────────┐    ┌────────┐    ┌────────┐
   │ CO-121 │    │ CO-120 │    │ CO-122 │  doc-only — ship anytime
   └────────┘    └────────┘    └────────┘
```

**Parallel-safe set:** `{CO-120}` ‖ `{CO-121 (after CO-118)}` ‖ `{CO-122}`. Three independent tracks.

### Phase 2 (CO-113)

```
   CO-118 (Phase 0)              CO-120 (Phase 1)
       │                              │
       ▼                              ▼
   ┌────────┐                     ┌────────┐
   │ CO-123 │                     │ CO-124 │
   └────────┘                     └────────┘
       │
       ▼
   (gates Phase 3b CO-127)
```

**Parallel-safe set:** `{CO-123}` ‖ `{CO-124}`.

### Phase 3 (CO-114) — split into 3a + 3b

**3a — UX track** (depends on existing Tier 3 sync work):

```
   existing CO-61, CO-54, CO-95 (Tier 3 sync)
       │
       ▼
   ┌────────┐    ┌────────┐
   │ CO-128 │    │ CO-129 │  conflict UI ‖ jujutsu changelog
   └────────┘    └────────┘
```

**3b — Streaming track** (depends on Phase 2):

```
   CO-123 (Phase 2)
       │
       ▼
   ┌────────┐
   │ CO-126 │  Lakekeeper REST catalog
   └───┬────┘
       ▼
   ┌────────┐
   │ CO-125 │  Redpanda + Iceberg Topics (needs catalog)
   └───┬────┘
       ▼
   ┌────────┐
   │ CO-127 │  Flink session-stitch
   └────────┘
```

**Parallel-safe across 3a + 3b:** the two tracks have no shared dependencies and can ship in either order. Within 3b, the chain is **strictly serial**.

### Phase 4 (CO-115)

```
   existing CO-86, CO-87, CO-93 (Tier 4)
       │
       ▼
   ┌────────┐
   │ CO-130 │  privileged zone
   └───┬────┘
       │
       ├──────────────┬──────────────┐
       ▼              ▼              ▼
   ┌────────┐    ┌────────┐
   │ CO-131 │    │ CO-132 │   allow-list+DLP ‖ audit log
   └────────┘    └────────┘
```

**Parallel-safe set after CO-130:** `{CO-131}` ‖ `{CO-132}`.

### Phase 5 (CO-116)

```
   ┌────────┐
   │ CO-133 │  deploy.yaml schema
   └───┬────┘
       │
       ├──────────────┐
       ▼              ▼
   ┌────────┐    ┌────────┐
   │ CO-134 │    │ CO-135 │  static-on-R2 ‖ CF Pages
   └────────┘    └────────┘

   ┌────────┐
   │ CO-136 │  ⏸ blocked-on-evidence; not scheduled
   └────────┘
```

**Parallel-safe set after CO-133:** `{CO-134}` ‖ `{CO-135}`. CO-136 stays parked.

---

## Critical path through the whole plan

The longest must-be-serial chain (worst-case):

```
current sprint Wave 1-5 close
  → CO-117 → CO-118 → CO-121
  → existing CO-77
  → CO-123
  → CO-126 → CO-125 → CO-127
  → existing CO-86 + CO-87
  → CO-130 → CO-131 (or CO-132)
  → CO-133 → CO-134 (or CO-135)
```

That's **13 nodes** on the critical path. Everything else hangs in parallel off these.

---

## Parallel-safe execution groups (one per "cohort week")

If you batch work into cohorts that ship together, the natural cohorts are:

| Cohort | Tickets | Notes |
|--------|---------|-------|
| **C1** (after current sprint commits) | CO-117, CO-119, CO-122 | All independent; CO-122 is doc-only |
| **C2** | CO-118, CO-120 | After C1 (CO-117); CO-120 independent |
| **C3** | CO-121 | Depends on C2 (CO-118) |
| **C4** (after Tier 2 CO-77 lands) | CO-123, CO-124 | Phase 2; both parallel |
| **C5a** (after Tier 3 sync lands) | CO-128, CO-129 | UX track |
| **C5b** | CO-126 → CO-125 → CO-127 | Streaming track; serial within the cohort |
| **C6** (after Tier 4 CO-86/87 lands) | CO-130 | Solo |
| **C7** | CO-131, CO-132 | Parallel after C6 |
| **C8** | CO-133 | Solo |
| **C9** | CO-134, CO-135 | Parallel |
| **(parked)** | CO-136 | Wait for evidence |

C5a and C5b are independent — can ship in either order, or interleaved.

---

## Risk concentrations to flag

| Phase | Risk shape | Mitigation |
|-------|------------|------------|
| Phase 0 | Low — only edge config + a script | Ship fast; don't gold-plate |
| Phase 1 | Medium — A/B logic correctness | Stable-assignment test with 1k iterations is the headline acceptance |
| Phase 2 | Medium — first non-OLTP query surface | Keep WAE as parallel write so ClickHouse can be backed out without data loss |
| Phase 3a | Medium — UX + sync edge cases | Playwright e2e is mandatory, not optional |
| Phase 3b | **High — most novel infrastructure** | Stage Redpanda + Lakekeeper + Iceberg in UAT for ≥2 weeks before prod |
| Phase 4 | **Highest — security guarantees** | Independent security review before declaring privileged-zone GA; consider engaging external |
| Phase 5 | Low-medium — deployer adapters | Static-on-R2 first because the failure modes are obvious; CF Pages second |

---

## Sprint compatibility — read before kicking off

The current sprint touches `app.js`, `co-web` migrations, and version bumps. Phase 0 platform tickets that touch the same surfaces:

- **CO-117** modifies DNS + Cloudflare config — no shared files. **Safe to start now.**
- **CO-118** touches `co-web` (new telemetry emitter). **Defer until Wave 1 is committed and pushed** to avoid `app.js`/route-conflict thrash.
- **CO-119** is scripts-only. **Safe anytime.**
- **CO-122** is docs-only. **Safe anytime.**
- **CO-120** is `co-core` Rust + new `co-agent` binary. **Defer until Wave 1 is in.** No `co-web` overlap, but version bump on `Cargo.toml` collides.

Once Wave 1 is in `main`, the platform Phase 0 tickets (C1 + C2) can run in parallel worktrees with the rest of the sprint (Wave 2-5).

---

## Bottom line — what to do in the next two weeks

1. **Finish Wave 1** of the current sprint (commit + push of `1.20.2 → 1.21.1` diff). Do not start any platform ticket before that.
2. **Start Cohort C1**: CO-117, CO-119, CO-122 in parallel worktrees. None of them blocks the rest of the sprint.
3. **Ramp Cohort C2**: CO-118 + CO-120 once Wave 1 is in.
4. **Hold everything Phase 2+ behind the existing Tier 2 / Tier 3 / Tier 4 dependencies.** Don't try to shortcut the bridges; they are the bridges.

---

## Suggested follow-up

If this review is endorsed, update each platform-ticket frontmatter with explicit `depends_on:` (or equivalent link property) so the order is machine-readable, not just human-readable. The `work/schema.yaml` already has `epic` and `story` link properties — extending with a `depends_on` link property is a 5-minute schema addition that pays off through every subsequent sprint planning round.
