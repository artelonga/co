# Infrastructure — How Each Deployable Is Deployed

> Last reviewed: 2026-06-01
> Reviewer: yuri@artelonga.com.br

The platform consists of **6 production services** + **1 static site**, all hosted on Fly.io in `gru` (São Paulo) region except `artelonga.com.br` (GitHub Pages). This document describes how each is deployed, its tech stack, persistence, and where its source code + domain live.

## Summary table

| Service | Fly app | Domain | Stack | Source repo | Persistence |
|---|---|---|---|---|---|
| **CO platform** | `co-artelonga` | `co.artelonga.com.br` + `co-artelonga.fly.dev` | Rust + Axum | `artelonga/co` | volume `co_data` at `/data` |
| **Yggdrasil** | `yggdrasil-artelonga` | `yggdrasil.artelonga.com.br` | Rust + Axum | `artelonga/yggdrasil` | volume `yggdrasil_data` at `/data` |
| **ArteLonga (web)** | _GitHub Pages_ | `artelonga.com.br` | static (Jekyll/?) | `artelonga/ArteLonga` | n/a — static |
| **Quilombo Araucaria** | `quilombo-araucaria` | `quilomboaraucaria.org` + `www.` | Node 22 (SvelteKit) | `artelonga/quilomboaraucaria` | volume `quilombo_data` at `/app/data` |
| **RFQ Gateway** | `rfq` (active), `artelonga-rfq-gateway` (legacy) | `rfq.artelonga.com.br` | Rust | `artelonga/rfq-gateway` | per-machine volume `rfq_artifacts` |
| **Artelonga Neuro** | `artelonga-neuro` | `neuro.artelonga.com.br` | _unverified_ | _?_ | _?_ |
| **CO UAT (stale)** | `co-artelonga-uat` | n/a | Rust + Axum | `artelonga/co` | volume `co_uat_data` |

## Per-service detail

### CO platform — `co-artelonga`

The graph content management system. Hosts the multi-universe board, vault API, sync, and (post-CO-323) subdomain routing for `yuri.artelonga.com.br`.

- **Domain:** `co.artelonga.com.br` (issued); raw Fly URL `co-artelonga.fly.dev` also works
- **Build:** `co-web/Dockerfile` — `rust:1.94-slim-trixie` builder → `debian:trixie-slim` runtime, ~70 MB final image
- **Internal port:** `3000`
- **Volume:** `co_data` (mount `/data`) holds SQLite (meta.db + per-universe DBs), entry files, game.db (CO-307)
- **Env:** `CO_WEB_DATA=/data`, `CO_WEB_PORT=3000`, `RUST_LOG=co_web=info,tower_http=info`
- **Auto-stop:** `stop` (machine suspends when idle, restarts on request)
- **Min machines:** 0 (cold-start tolerated)
- **Deploy:** `flyctl deploy` from repo root
- **Health:** `GET /api/health` → `{ "status": "ok", "version": "X.Y.Z" }` — **the gold-standard pattern** other sisters should match
- **Migration:** schema versioning via `co-web/src/storage/migrations.rs` (currently v51)

### Yggdrasil — `yggdrasil-artelonga`

Universe of universes — minigames hub + comunicacao (chat rooms) + universos catalog.

- **Domain:** `yggdrasil.artelonga.com.br` (issued)
- **Build:** `yggdrasil-web/Dockerfile` — same Rust 1.94 base as CO
- **Volume:** `yggdrasil_data` (mount `/data`) holds yggdrasil.db, sementes.db, instances, comunicacao rooms, lexicon
- **Env:** multiple paths for DBs/instances (see fly.toml); SMTP secrets set via `flyctl secrets`
- **Health:** **none currently** — `/api/health` returns 404. **Action item:** `/version` endpoint coming as 2.0.1 (per recent review)
- **Notable:** uses non-root user `ygg` — instances/blobs/comunicacao all under `/data` (writable)

### ArteLonga (web) — _static, GitHub Pages_

Public marketing/portfolio site. Lives outside Fly.

- **Domain:** `artelonga.com.br` → GitHub Pages (`server: GitHub.com` header confirms)
- **Source:** `artelonga/ArteLonga` repo (the same one that has the `Dockerfile` + `fly.toml`)
- **Anomaly:** `ArteLonga/fly.toml` declares `app = "artelonga-dev"` — that Fly app **does not exist** (404 on `flyctl certs list -a artelonga-dev`). The fly config is dormant. ArteLonga is currently a static site, not a Fly deployment.
- **What ships:** SvelteKit build output likely deployed via GitHub Actions to Pages branch
- **CO-334 changelog aggregation:** still picks up `ArteLonga/CHANGELOG.md` from the repo on disk

### Quilombo Araucaria — `quilombo-araucaria`

Community/content site (publicacoes, eventos, missions). Largest media payload (1 GiB upload cap for videos).

- **Domain:** `quilomboaraucaria.org` + `www.quilomboaraucaria.org` (both issued)
- **Build:** Node 22 alpine multi-stage; SvelteKit-based
- **Volume:** `quilombo_data` (mount `/app/data`) for SQLite + uploads + content
- **Auto-stop:** `suspend` (cheaper than `stop` — pre-warmed)
- **Env:** `BODY_SIZE_LIMIT=1073741824` (1 GiB), `TZ=America/Sao_Paulo`
- **Health:** `/health` returns `302` (redirects); endpoint exists but doesn't follow the JSON convention. Worth standardizing to `/api/health` or `/version`.

### RFQ Gateway — `rfq` (current); `artelonga-rfq-gateway` (legacy)

Quote/trade routing service.

- **Domain:** `rfq.artelonga.com.br` (issued on `rfq` app)
- **Build:** `Dockerfile` with `rust:1.91-slim-bookworm` (slightly older Rust than CO/Yggdrasil)
- **Internal port:** `8080` (different from the `3000` convention used by CO/Yggdrasil/Quilombo)
- **Volume:** `rfq_artifacts` for observability ring buffers (`inbound-/rejections-/fills-YYYY-MM-DD.jsonl`) — per-machine
- **Health:** `/health` endpoint (referenced in `[[http_service.checks]]`)
- **Two apps:** `rfq` (current, May 20) is the live one; `artelonga-rfq-gateway` (May 17) appears to be an older deploy from before the rename. **Action:** confirm + delete the stale one if `rfq` is canonical.

### Artelonga Neuro — `artelonga-neuro`

Deployed 6h ago — likely related to the yuri.artelonga.com.br LLM/AI vision (CO-328+).

- **Domain:** `neuro.artelonga.com.br` (issued)
- **Build:** _not yet surveyed_; need to find which repo. Possibly a separate Neuro repo, possibly inside ArteLonga.
- **Health:** `/health` → 404 (no health endpoint yet — same gap as Yggdrasil)
- **Action item:** document its source repo + tech stack + intended role

### CO UAT — `co-artelonga-uat` (stale)

Per `feedback_no_uat.md` (memory), the UAT environment was deliberately dropped in favor of direct-to-prod + smoke-test workflow. Last deploy May 2 (a month stale).

- **Recommendation:** delete this app to save Fly capacity. `flyctl apps destroy co-artelonga-uat` (irreversible).

## Cross-cutting infrastructure conventions

### Region

All apps run in **`gru`** (São Paulo). Single-region for simplicity + latency to South American users. Multi-region not currently used.

### TLS

Fly issues + manages all certs. All custom domains are `Issued` per `flyctl certs list`. Wildcard not used — each subdomain has its own cert.

### Auto-suspend (CO-285)

Most apps use `auto_stop_machines = "stop"` or `"suspend"` with `min_machines_running = 0` to drop to zero when idle, restart on request. Tradeoff: 5-15s cold-start latency for the first hit, near-zero idle cost.

### Persistent storage

All stateful apps use Fly volumes (per-machine, not shared). Backups: none automated yet — Fly snapshots are manual. **Risk:** machine loss = data loss for that universe.

### Health/version endpoints (parity gap)

| Service | `/api/health` | `/version` |
|---|---|---|
| CO | ✅ returns version | (implicit in `/api/health`) |
| Yggdrasil | ❌ 404 | coming as 2.0.1 |
| Quilombo | ⚠️ `/health` redirects | ❌ |
| RFQ | ⚠️ `/health` (no JSON shape published) | ❌ |
| Neuro | ❌ 404 | ❌ |

**Recommendation:** standardize on CO's pattern — `GET /api/health` returns `{ "status": "ok", "version": "X.Y.Z" }`. Required for CO-334's "deployed_version vs latest_changelog_version" comparison to work across the platform. ~1 hour per sister to add.

### Build/deploy cadence

| Service | Last deploy | Recent activity |
|---|---|---|
| CO | 36 min ago (2.36.0) | active — multiple releases/day during this session |
| Yggdrasil | 25 min ago (~2.0.x) | active |
| Neuro | 6h ago | recent (deployed yesterday during yuri.artelonga.com.br discussion) |
| Quilombo | May 20 | stale (12+ days) |
| RFQ | May 20 | stale |
| ArteLonga (static) | ongoing via GitHub Pages | active |
| CO UAT | May 2 | dead (recommend delete) |

### Deploy commands (operational)

| Service | Command |
|---|---|
| CO | `cd ~/projects/co && flyctl deploy` |
| Yggdrasil | `cd ~/projects/yggdrasil && flyctl deploy` |
| RFQ | `cd ~/projects/rfq-gateway && flyctl deploy` |
| Quilombo | `cd ~/projects/quilomboaraucaria && flyctl deploy` |
| ArteLonga | git push (GitHub Pages builds + publishes) |
| Neuro | `flyctl deploy` from wherever its source lives — needs documenting |

### Secrets

All managed via `flyctl secrets set -a <app>`. JWT secrets, OAuth keys, SMTP credentials. **Action item:** rotate ~yearly; nothing automatic.

## Action items surfaced by this review

| # | Item | Effort | Owner |
|---|---|---|---|
| 1 | Add `/api/health` (CO shape) to Yggdrasil, Quilombo, RFQ, Neuro | ~1h each | per-repo |
| 2 | Document Neuro's source repo + role | 30 min | yuri |
| 3 | Delete stale `co-artelonga-uat` app | 5 min | yuri |
| 4 | Delete stale `artelonga-rfq-gateway` if `rfq` is canonical | 5 min | yuri |
| 5 | Clean up ArteLonga repo's `fly.toml` (dormant `app="artelonga-dev"`) | 10 min | yuri |
| 6 | Automate Fly volume snapshots (or document manual cadence) | half day | platform |
| 7 | CO-334 (changelog aggregator) — extend to also poll `/version` per sister | small follow-up | tracked in CO-334 |

## Related

- **CO-285** — auto-suspend Fly machine on idle (shipped to all sister apps)
- **CO-282** — `co serve` localhost distribution
- **CO-323** — yuri.artelonga.com.br subdomain routing
- **CO-330** — runtime universe→repo bindings (deploy-free content)
- **CO-334** — cross-repo changelog aggregation (in flight)
- **feedback_no_uat.md** — rationale for dropping UAT environment
