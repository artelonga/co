# Wave 4 Retrospective — v3.0.0 "public launch — brain on any device" (2026-06-10)

**Scope shipped**: all 21 Wave-4 items (see `docs/roadmap.md` as-built ledger).
Themes: unified Sala surface, mobile shell (touch DnD, PWA, IA reflow),
verification stack (staging suite, contract probe, migration check, DoD CI),
edge protection (rate limits, robots/sitemap, analytics privacy).

## What went well
- One-surface/fractal-scope decision (sala-surface.md) resolved a three-way
  implementation divergence before it shipped; the constraint held through
  CO-354/355 and is codified for future waves.
- Parallel co-auto wave (5 agents) delivered 5 PRs in one night; the serial
  merge chain with per-PR quality passes caught every defect before main.
- The launch ritual earned its keep twice: staging deploy caught a fleet-wide
  boot crash (base-schema index on a migration-added column); the contract
  probe caught three metadata drifts.

## What hurt
- Stale-base branches: three PRs double-claimed migration v64; co-auto reused
  a v2.41-era worktree for CO-354. Fixed at root: co-auto now resets reused
  worktrees onto origin/main.
- Conflicting PRs get NO pull_request CI runs (GitHub cannot build the merge
  ref) — "no checks reported" looked like an Actions outage.
- Sibling features fought: CO-358's single-column board silently removed
  CO-356's cross-column drag (caught by CO-356's own tests; resolved with
  segment-button drop targets).
- Latent flakes surfaced under new matrix load: env-var race in lead_routes
  tests, manifest MIME expectation, CSS cascade ordering.

## Carry-forward (Wave 5)
- Seed staging fixtures + CO_STAGING_ADMIN_TOKEN so the staging suite runs
  its deep scenarios (28 passed / 3949 precondition-skips this run).
- sprint-review.ts epoch anchor produces "Sprint -1" before 2026-06-11 and
  harvests the wrong window — fix before Thursday cadence relies on it.
- DoD verifier phase 2: execute matched tests so the gate can block honestly.
- CO-104/119 S3 backup + restore drill remains the top ops item.
