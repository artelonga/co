# Co — Release Roadmap

> One source of truth for what ships in which release. Updated as decisions land.

## Past releases (this session)

| Version | Date       | Bundles                                  | Status |
|---------|------------|------------------------------------------|--------|
| 1.15.0  | 2026-04-26 | CO-65 visibility-on-PUT                  | UAT + prod |
| 1.15.1  | 2026-04-26 | CO-66 API hygiene (500→409, seed idempotent, no auto-stop UAT) | UAT + prod |
| 1.16.0  | 2026-04-26 | CO-82 UAT mirror (dormant — env vars unset) | UAT + prod |
| 1.17.0  | 2026-04-27 | CO-83 Mermaid.js diagram rendering       | UAT (deploying) |

Other in-flight work that doesn't bump the scaffold version:
- **CO-84** — `co-auto` extracted into `dev/co-auto` (own version 0.1.0, NOT in scaffold workspace default-members)
- **Repo consolidation** — specs imported from `artelonga/co-dev` to `co/work/co/`; `artelonga/co-dev` ready to archive

## Upcoming releases (the plan)

### "1.18 era" — consolidation (NOT a scaffold version bump)

Target: ongoing housekeeping. None of these items modify the deployed binary, so no `Cargo.toml` bump is warranted.

| Item | State (2026-04-27) |
|------|---------------------|
| Local merged-branch cleanup (65→30) | ✓ done |
| `co-dev` archived on GitHub + tag pushed | ✓ done |
| **CO-67** prod seed (artelonga + rfq + content) | gated on email-code login |
| **CO-82 ops** — generate prod API token, set Fly secrets, verify mirror | gated on email-code login |
| **`dev/co-auto` polish (CO-84 step 2)** — split `auto.rs` into module files, migrate `run()` to `Pipeline` | versioned independently as `co-auto 0.2.0` when shipped |

When CO-67 + CO-82 ops run, they change prod data shape but not the deployed binary. Note them in `CHANGELOG.md` under the next scaffold release that ships after.

### 1.19.0 — "post-GitHub cleanup" (small schema change)

Target: ~1 week after 1.18.0. Removes dead code; consolidates docs.

- **CO-64** — delete `co-web/src/git_sync.rs`, drop the `git_*` columns from `universes` (online migration), remove `PUT /:slug/git`, `POST /:slug/sync`, `POST /:slug/webhook` routes
- Mark CO-50 + CO-55 as `status: deprecated` in their task files
- Write `co/docs/ARCHITECTURE.md` consolidating the post-GitHub data model (previously promised in `ROADMAP-SYNC.md`)

Risk: schema migration on live prod DB. Mitigation: online migration via `ALTER TABLE … DROP COLUMN`, validate post-migration via UAT first.

### 1.20.0 (or 1.x sweep) — "small features pile"

Floating release for whichever of these land first:
- **CO-83 polish** — wire `renderMermaidBlocks` into other render paths (board cards, content page, template universe). Currently only the entry zoom view triggers it.
- **CO-78 (job queue, lite)** — minimal SQLite-backed queue for non-blocking ops; precedes CO-72 doc generators
- **CO-79 (caching, lite)** — manifest LRU + theme.css ETag (the no-Redis subset, valid before CO-77)
- **CO-80 (rate limit, lite)** — token bucket per (user_id, op_class), in-process for v1

These can ship independently in any order; each is a minor bump.

### 2.0.0 — "scale" (BREAKING — schema reorganization)

Target: 6–10 weeks out. The load-bearing release.

**Headline**: storage shards from one `co.db` to `meta.db` + per-universe `data.db` files. CO-77 detailed plan: `work/co/CO-77-PLAN.md`.

In scope:
- **CO-77** per-universe SQLite + meta.db + LiteFS read replicas
- **CO-71** per-universe schema validator + generic JSON entry storage (lands AFTER 77 because it depends on per-universe DB to scale)
- **CO-70** manifest format spec (`_universe.yaml`)

Out of scope (defer to 2.1+):
- CO-72 doc-generator hooks (needs CO-78 job queue stable first)
- CO-73 temporal model
- CO-74 relationship graph
- CO-75 version reconstruction (needs CO-61 op log)

**Why 2.0**: every storage method changes internally. The Storage trait surface stays compatible, but the migration is non-reversible without restoring backups. SemVer's "you can break compat once" budget gets spent on this release.

### 2.1+ — "manifest" (additive on top of 2.0)

- CO-72 doc-generator hooks (scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc) — needs CO-78 stable
- CO-73 temporal model (event_at, due_at, scheduled_at, …)
- CO-74 relationship graph + query DSL + typed wikilink promotion
- CO-78 (full) job queue + worker pool — Redis-backed if SQLite contention shows
- CO-79 (full) caching layer with Redis L2

### 2.2+ — "history"

- CO-61 sync protocol v1 (op log, HLC, content-addressed blobs, 3-way merge)
- CO-75 version reconstruction (replay op log to any timestamp; auto-changelog)
- CO-62 quilombo-blog sync adapter

### 2.3+ — "platform"

- CO-80 (full) per-tier rate limiting + quota
- CO-81 object storage for blobs + filesystem sharding (when Fly volumes hit cost ceiling)
- CO-51 `co sync` CLI (INFRA-1) — was deferred from v1 roadmap; once op log is live it becomes meaningful
- CO-58 desktop tray sync app (INFRA-2)
- CO-69 PWA offline (INFRA-4)

## What's NOT going to ship as a release

- **CO-83 the deferred seed diagrams** — 8 of the 9 spec'd diagrams (safety/privacy, universe relationships, content-vs-form, editing flow, login flow, UAT→prod promotion, co.db ERD, quilomboaraucaria ERD). Author them as content when the feature ships, not as part of any release.
- **CO-55 GitHub SSH auth** — superseded by post-GitHub direction. Will be marked deprecated in 1.19.0.
- **CO-50 universe-as-repo** — same. The `git_sync.rs` code gets removed in 1.19.0.

## Decision points / open questions

1. **When does CO-67 prod seed run?** — Operationally gated on email-code login. Recommend bundling with 1.18.0 release work.
2. **Does CO-77 land before or after 1.19.0?** — After. Ship the small schema-change first (CO-64) so the migration tooling is exercised on something low-stakes before the big sharding migration.
3. **Postgres or SQLite-LiteFS forever?** — Decided: SQLite-LiteFS through 2.x. Re-evaluate at 3.0 if cross-tenant transactions become a real need.
4. **Is `dev/co-auto` v1.0.0 a separate release event?** — Versioned independently of scaffold. Ship `co-auto 1.0.0` once CO-84 polish lands (split into modules + Pipeline migration). Until then, it stays at `0.1.0` in `dev/co-auto/Cargo.toml`.

## Release process (going forward)

| Step | What | Who/automated |
|------|------|---------------|
| 1 | Open issue or use existing `work/co/CO-*.md` task | Yuri |
| 2 | Branch `feat/CO-N-…` (or run `co-auto --task CO-N`) | dev |
| 3 | Implement; bump version in `Cargo.toml` workspace; CHANGELOG entry | dev |
| 4 | UAT deploy: `flyctl deploy --config fly.uat.toml` | dev |
| 5 | Run UAT validation checklist (see `co/CLAUDE.md` UAT Verification Spec) | dev |
| 6 | Prod deploy: `flyctl deploy` | dev |
| 7 | Smoke test prod (health, key endpoints) | dev |
| 8 | Mark task `status: done` in `work/co/CO-N.md` | dev (or co-auto's StatusUpdateFinalizer) |

Branches stay short-lived; merge to main and push directly. No long-lived release branches needed for a single-developer project.
