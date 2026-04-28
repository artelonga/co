# Co — Release Roadmap

> One source of truth for what ships in which release. Updated as decisions land.

## Past releases (this session)

| Version | Date       | Bundles                                  | Status |
|---------|------------|------------------------------------------|--------|
| 1.15.0  | 2026-04-26 | CO-65 visibility-on-PUT                  | UAT + prod |
| 1.15.1  | 2026-04-26 | CO-66 API hygiene (500→409, seed idempotent, no auto-stop UAT) | UAT + prod |
| 1.16.0  | 2026-04-26 | CO-82 UAT mirror (dormant — env vars unset) | UAT + prod |
| 1.17.0  | 2026-04-27 | CO-83 Mermaid.js diagram rendering       | UAT + prod |
| 1.18.0  | 2026-04-27 | CO-85 password-login on prod             | UAT + prod |
| 1.18.1  | 2026-04-27 | CO-90 (preview): seed uses `tier='user'` (no global admin) | UAT + prod |
| 1.18.2  | 2026-04-27 | CO-82 mirror end-to-end (configured universe list, no /api/v1/universes auth refactor) | UAT + prod |
| 1.18.3  | 2026-04-27 | CO-82 mirror throttle (1s/entry under prod's 60 req/min) — quilombo 70 entries fully replicated to UAT | UAT + prod |

Other in-flight work that doesn't bump the scaffold version:
- **CO-84** — `co-auto` extracted into `dev/co-auto` (own version 0.1.0, NOT in scaffold workspace default-members)
- **Repo consolidation** — specs imported from `artelonga/co-dev` to `co/work/co/`; `artelonga/co-dev` archived on GitHub
- **Branch hygiene** — local branches 65 → 30; orphan refs from dead `institutional-pointset/co` remote dropped

## The big picture (vision through 3.0)

```
1.x  ── small features + cleanup, monolithic SQLite, markdown-on-the-wire
   │
2.0 ───── BREAKING: per-universe SQLite + LiteFS replicas
   │      manifest format ships; generic JSON entry storage
   │
2.x  ── manifest features (doc-gen, temporal, relations, sync protocol, version history)
   │
3.0 ───── BREAKING: `.co` protobuf format becomes default wire format
   │      composable protocol stack (hardware → cache → storage → network → privacy → security)
   │      `co` is itself defined as a protobuf data type — the platform's logical core
   │      end-to-end pipeline UAT with per-universe stats
   │
3.x  ── full ecosystem (CLI sync, desktop tray, PWA offline, mobile)
```

The 3.0 jump is driven by CO-86 (file format), CO-87 (layer traits), CO-88 (pipeline UAT), CO-89 (co-dev as content). Together they make `co` a *protocol-defined* platform, not just a CMS over markdown files.

## Upcoming releases

### "1.18 era" — consolidation (mostly done as of 2026-04-27)

| Item | State |
|------|-------|
| Local merged-branch cleanup (65→30) | ✓ done |
| `co-dev` archived on GitHub + tag pushed | ✓ done |
| **CO-85** password-login on prod | ✓ shipped 1.18.0 |
| **CO-90 (preview)** — seed uses `tier='user'` | ✓ shipped 1.18.1 |
| **CO-82 ops** — token + Fly secrets + reset; **mirror works end-to-end** (1.18.3) | ✓ done |
| **CO-67** prod seed (artelonga + rfq + content) | runnable: `bash scripts/seed-prod-universes.sh PASSWORD` |
| **`dev/co-auto` polish (CO-84 step 2)** — split `auto.rs`, migrate `run()` to `Pipeline` | versioned independently as `co-auto 0.2.0` when shipped |

### 1.18.0 — "password auth on prod" (small, unblocks operations)

Target: ~1 day of work.

- **CO-85** — `POST /api/v1/auth/password-login` (no env gate); env-driven admin seed (`CO_SEED_ADMIN_EMAIL` + `CO_SEED_ADMIN_PASSWORD_HASH`); replaces email-code friction with Argon2id login. Unblocks autonomous prod write paths.

After 1.18.0 ships, CO-67 + CO-82 ops become runnable without log-fishing.

### 1.19.0 — "post-GitHub cleanup" (small schema change)

- **CO-64** — delete `co-web/src/git_sync.rs`, drop the `git_*` columns from `universes` (online migration), remove `PUT /:slug/git`, `POST /:slug/sync`, `POST /:slug/webhook` routes
- Mark CO-50 + CO-55 as `status: deprecated` in their task files
- Write `co/docs/ARCHITECTURE.md` consolidating the post-GitHub data model

Risk: schema migration on live prod DB. Mitigation: online migration via `ALTER TABLE … DROP COLUMN`, validate post-migration via UAT first.

### 1.20.0 — "drop global admin tier" (multi-user readiness)

- **CO-90** — drop `tier='admin'` as a global authority signal. Audit and remove all `tier=='admin'` bypasses (`dev_board.rs:31`, `universe_routes.rs:765`). Define `tier` as billing-only (`anonymous`/`user`/`pro`). Migration converts existing `tier='admin'` rows to `'user'`. Every privileged action becomes per-universe (CO-49 enforces this). Spec: `work/co/CO-90.md`.

Why before 2.0: the `tier` cleanup is multi-user-readiness. Shipping CO-77 sharding without it risks a second user accidentally getting global authority via a misconfigured tier write.

### 1.21.0 (or 1.x sweep) — "small features pile"

Floating release for whichever of these land first:
- **CO-91 Phase 1** — `co sync push` subcommand replaces `scripts/seed-prod-universes.sh`. Same behavior, first-class CLI surface. Spec: `work/co/CO-91.md`.
- **CO-83 polish** — wire `renderMermaidBlocks` into other render paths (board cards, content page, template universe). Currently only the entry zoom view triggers it.
- **CO-78 (lite)** — minimal SQLite-backed job queue for non-blocking ops; precedes CO-72 doc generators
- **CO-79 (lite)** — manifest LRU + theme.css ETag (the no-Redis subset, valid before CO-77)
- **CO-80 (lite)** — token bucket per `(user_id, op_class)`, in-process for v1

Each is a minor bump.

### 2.0.0 — "scale" (BREAKING — schema reorganization)

**Headline**: storage shards from one `co.db` to `meta.db` + per-universe `data.db` files. Detailed plan: `work/co/CO-77-PLAN.md`.

In scope:
- **CO-77** per-universe SQLite + meta.db + LiteFS read replicas
- **CO-71** per-universe schema validator + generic JSON entry storage (lands AFTER 77 because it depends on per-universe DB to scale)
- **CO-70** manifest format spec (`_universe.yaml`)

Out of scope (defer to 2.1+):
- CO-72 doc-generator hooks (needs CO-78 job queue stable first)
- CO-73 temporal model
- CO-74 relationship graph
- CO-75 version reconstruction (needs CO-61 op log)

**Why 2.0**: every storage method changes internally. The Storage trait surface stays compatible, but the migration is non-reversible without restoring backups.

### 2.1+ — "manifest + git-backed universes" (additive on top of 2.0)

The biggest user-visible win in 2.x. Every repo-backed universe gets a uniform git-changelog view, contributor profiles, event calendar, and live analytics dashboards.

- **CO-89** (priority: critical) — git-backed universes: any universe with `git_source` set ingests commits/profiles/events as content; per-universe analytics + Mermaid Gantt views; generalizes the `co-dev` pattern to `artelonga`, `quilomboaraucaria`, `rfq`, and any future user's repo-backed universe
- **CO-72** doc-generator hooks (scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc) — needs CO-78 stable
- **CO-73** temporal model (event_at, due_at, scheduled_at, …) — first real test is CO-89's commit timeline + Gantt
- **CO-74** relationship graph + query DSL + typed wikilink promotion — `commit → task` resolution from CO-89's parsed commit messages is the canonical case
- **CO-78 (full)** job queue + worker pool — Redis-backed if SQLite contention shows
- **CO-79 (full)** caching layer with Redis L2

Sequencing inside 2.x:
```
CO-77 (per-universe SQLite) ✓ ships first → CO-70 (manifest) → CO-71 (validator+JSON storage)
                                                ↓
                                         CO-78 lite job queue
                                                ↓
                                         CO-89 git-backed universes ← biggest user win
                                                ↓
                                         CO-72 (doc gen) + CO-73/74 (temporal + relations) — validated by CO-89's data
```

### 2.2+ — "history"

- **CO-61** sync protocol v1 (op log, HLC, content-addressed blobs, 3-way merge)
- **CO-75** version reconstruction (replay op log to any timestamp; auto-changelog)
- **CO-62** quilombo-blog sync adapter

### 3.0.0 — "protocol" (BREAKING — `.co` becomes the wire format)

The platform stops being a markdown-over-HTTP CMS and becomes a protobuf-defined protocol with composable layers. Most of the scaffolding is already in place; this release rewires the wire format and the layered I/O.

In scope:
- **CO-86** `.co` file format — protobuf-encoded markdown wrapper with typed frontmatter, attachments graph, encryption envelope, signature, telemetry
- **CO-87** composable protocol stack — `Layer` trait + `Stack<B, T>` composer; concrete layers for filesystem, cache, SQLite, S3, HTTP, encryption (chacha20), signing (Ed25519), compression (zstd/brotli)
- **CO-88** end-to-end pipeline UAT — 5 layer-combo × 3 universe × 4 path matrix; per-universe stats (file count, raw bytes, encoded bytes, compression ratio, transfer ms); admin dashboard at `/co/co-dev/pipeline`; CI deploy gate

Out of scope (3.x+):
- Streaming decode of large `.co` files (whole-file decode is fine for v1)
- Field-level encryption within a single CoFile
- Cross-language SDK (Scala / TS) — port the model, not the impl

**Why 3.0**: the wire format changes. Old clients see new bytes. Auto-wrap on read keeps backward compat for plain markdown, but anything that expects markdown-on-the-wire as the canonical form needs to update.

### 3.1+ — "ecosystem" (additive on top of 3.0)

- **CO-91 Phases 2-4** — `co sync` multi-deployment + `co sync watch` + content-negotiated `.co` wire format. CO-91 supersedes the original CO-51 "co sync" idea; the post-pivot architecture (jj-tracked, layer-composed, token-authed) is the correct shape
- **CO-58** desktop tray sync app (INFRA-2)
- **CO-69** PWA offline (INFRA-4)
- **CO-80 (full)** per-tier rate limiting + quota
- **CO-81** object storage for blobs + filesystem sharding (when Fly volumes hit cost ceiling)

### 3.2+ — "mobile + multi-region"

- Capacitor mobile (Rust FFI or pure JS sync client; INFRA-5)
- Multi-region LiteFS replicas with read-routing
- P2P transport (`SyncProtocolTransport` over WebRTC, prototype only)

## Release-spanning workstreams

These don't slot into a single release; each evolves across multiple:

### Workstream: composable security/privacy

Threads through 3.0 (encryption + signing layers shipped) and 3.x (recipient-set management, key rotation, hardware-key signers). Driven by CO-86 + CO-87.

### Workstream: telemetry + observability

CO-46 (telemetry) → CO-88 (pipeline stats per universe) → CO-89 (co-dev as observability surface itself). Each layer adds a dimension; the same admin dashboard pivots all of them.

### Workstream: developer tooling (`dev/`)

`dev/co-auto` (CO-84) → `dev/co-pipeline` (CO-88's matrix runner, possibly its own crate) → future dev-only binaries. Versioned independently of scaffold; never in `default-members`.

## What's explicitly NOT going to ship as a release

- **CO-83 the deferred seed diagrams** — 8 of the 9 spec'd diagrams (safety/privacy, universe relationships, content-vs-form, editing flow, login flow, UAT→prod promotion, co.db ERD, quilomboaraucaria ERD). Author them as content (in `co-dev` post CO-89) when the feature ships.
- **CO-55 GitHub SSH auth** — superseded by post-GitHub direction. Marked deprecated in 1.19.0.
- **CO-50 universe-as-repo** — same. The `git_sync.rs` code gets removed in 1.19.0.

## Decision points / open questions

1. **When does CO-67 prod seed run?** — Once CO-85 (1.18.0) is on prod and the password-login flow works.
2. **Does CO-77 land before or after 1.19.0?** — After 1.19.0. Ship the small schema-change first (CO-64) so migration tooling is exercised before the big sharding migration.
3. **Postgres or SQLite-LiteFS forever?** — Decided: SQLite-LiteFS through 2.x. Re-evaluate at 4.0 if cross-tenant transactions become a real need.
4. **Is `dev/co-auto` v1.0.0 a separate release event?** — Versioned independently of scaffold. Ship `co-auto 1.0.0` once CO-84 polish lands (split into modules + Pipeline migration).
5. **CO-86 protobuf vs flatbuffers?** — Decided: protobuf (already in dep tree via `prost`; team mental model is protobuf-native). Reconsider only if zero-copy reads become a hot-path requirement.
6. **CO-87 layer trait async vs sync?** — Sync for v1. Switch to async if HTTP transport becomes the bottleneck under load (likely at 3.x scale, not before).
7. **CO-89 git ingestion source: `main` only or all branches?** — `main` only for v1. Feature branches are conversational, not historical record.

## Release process (going forward)

| Step | What | Who/automated |
|------|------|---------------|
| 1 | Open task or use existing `work/co/CO-*.md` | Yuri |
| 2 | Branch `feat/CO-N-…` (or run `co-auto --task CO-N`) | dev |
| 3 | Implement; bump version in `Cargo.toml` workspace; CHANGELOG entry | dev |
| 4 | UAT deploy: `flyctl deploy --config fly.uat.toml` | dev |
| 5 | Run UAT validation checklist (see `co/CLAUDE.md` UAT Verification Spec) | dev |
| 6 | Prod deploy: `flyctl deploy` | dev |
| 7 | Smoke test prod (health, key endpoints) | dev |
| 8 | Mark task `status: done` in `work/co/CO-N.md` | dev (or co-auto's StatusUpdateFinalizer) |
| 9 | (3.x+) CO-88 pipeline matrix passes as a deploy gate | CI |

Branches stay short-lived; merge to main and push directly. No long-lived release branches.

## Spec index for this session's additions

| ID | Spec file | Status | Releases |
|----|-----------|--------|----------|
| CO-85 | `work/co/CO-85.md` | todo | 1.18.0 |
| CO-86 | `work/co/CO-86.md` | todo | 3.0.0 |
| CO-87 | `work/co/CO-87.md` | todo | 3.0.0 |
| CO-88 | `work/co/CO-88.md` | todo | 3.0.0 (CI gate from then on) |
| CO-89 | `work/co/CO-89.md` | **scope expanded** — multi-universe git ingestion, not just co-dev | 2.1+ (priority: critical within 2.x) |
| CO-90 | `work/co/CO-90.md` | todo (preview shipped 1.18.1) | 1.20.0 |
| CO-91 | `work/co/CO-91.md` | todo (script prototype shipped) | 1.21.0 (Phase 1) → 2.0+ (Phases 2-4) |
| CO-77-PLAN | `work/co/CO-77-PLAN.md` | planning | 2.0.0 |
