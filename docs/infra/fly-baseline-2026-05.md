# Fly.io Deployment Baseline — 2026-05 (pre-CO-281)

Captured: 2026-05-23
Captured by: Claude (CO-281 Phase 0)
Purpose: baseline for CO-281 Phase 1-4 cost reduction measurement.

## Data sources

- **`fly.toml` files** in each repo (authoritative for declared sizing)
- **flyctl probes and live `/api/health` calls were NOT available** in this
  capture environment (sandbox blocks `flyctl` and outbound HTTP)
- Costs are **estimated** from declared machine size × always-on assumption,
  using Fly's published 2026 pricing (see formula reference)
- Per-machine *actual* count, last-deploy date, and live version are placeholders
  marked `TBD (flyctl)` — to be filled in next time we run from a shell with
  Fly auth

## Summary table

| App (fly app) | Repo / fly.toml | Machine | RAM | auto_stop | min_machines | Region | Last deploy | Live version |
|---|---|---|---|---|---|---|---|---|
| **co-artelonga** | `co/fly.toml` | shared-cpu-1x | 512mb | `"stop"` | 0 | gru | TBD (flyctl) | TBD (flyctl) |
| **co-artelonga-uat** | `co/fly.uat.toml` | shared-cpu-1x | 256mb | `false` (off) | 0 | gru | TBD (flyctl) | TBD (flyctl) |
| **quilombo-araucaria** | `quilomboaraucaria/fly.toml` | shared-cpu-1x | **2048mb** | `"stop"` | **1** (always-on) | gru | TBD (flyctl) | TBD (flyctl) |
| **yggdrasil-artelonga** | `yggdrasil/fly.toml` | shared-cpu-1x | 512mb | `"stop"` | 0 | gru | TBD (flyctl) | TBD (flyctl) |
| **artelonga-rfq-gateway** | `rfq-gateway/fly.toml` | shared-cpu-1x (default) | 256mb (default) | `"stop"` | **1** (always-on) | gru | TBD (flyctl) | TBD (flyctl) |
| _artelonga-dev_ (unclear status) | `ArteLonga/fly.toml` | shared-cpu-1x | 256mb | `"stop"` | 0 | gru | TBD (flyctl) | _public site is on GH Pages_ |

`comunicacao/` has **no `fly.toml`** — confirmed it lives as a content universe
inside `co-artelonga`, not as a separate deployable.

## Per-app detail

### co-artelonga (prod)

- **fly.toml**: `/Users/artelonga/projects/co/fly.toml`
- **App**: `co-artelonga` in region `gru`
- **Build**: `co-web/Dockerfile` (Rust + LiteFS)
- **VM**: `shared-cpu-1x`, 1 vCPU, **512 MB RAM** (NB: spec CO-281 §Context
  guessed 1GB — actual declared is already 512MB)
- **HTTP**: internal_port 3000, force_https, `auto_stop_machines = "stop"`,
  `auto_start_machines = true`, `min_machines_running = 0`
- **Concurrency**: hard_limit=100 connections, soft_limit=80
- **Mount**: volume `co_data` → `/data` (LiteFS FUSE root; SQLite primary)
- **Env**: `CO_WEB_DATA=/data`, `CO_WEB_PORT=3000`, `LITEFS_DIR=/data`,
  `LITEFS_URL=http://localhost:20202`
- **Health check**: `GET /api/health` every 30s, 10s timeout, 90s grace

> Note: `min_machines_running = 0` is already set, but `auto_stop_machines =
> "stop"` (cold-boot, ~5-10s) rather than `"suspend"` (~250ms). The cold-boot
> is acceptable for the first-load tax today but is part of what Phase 1 would
> flip to `"suspend"` if we proved it didn't break the LiteFS-primary lease.

### co-artelonga-uat

- **fly.toml**: `/Users/artelonga/projects/co/fly.uat.toml`
- **App**: `co-artelonga-uat` in region `gru`
- **VM**: shared-cpu-1x, 1 vCPU, **256 MB RAM**
- **HTTP**: `auto_stop_machines = false` (literal off — machine never
  auto-stops), `min_machines_running = 0`
- **Mount**: `co_data` → `/data`
- **Env**: same as prod + `CO_ENV=uat`
- **Note**: memory `feedback_no_uat.md` says yuri has **decommissioned the UAT
  workflow**; this app may already be stopped at the Fly machine level even
  though the toml is checked in. Worth confirming via `flyctl status -a
  co-artelonga-uat` and dropping it from rotation if so.

### quilombo-araucaria

- **fly.toml**: `/Users/artelonga/projects/quilomboaraucaria/fly.toml`
- **App**: `quilombo-araucaria` in region `gru`
- **Build**: top-level `Dockerfile` (SvelteKit + Node)
- **VM**: `shared-cpu-1x`, **2048 MB RAM** (single biggest line item)
- **HTTP**: `auto_stop_machines = "stop"`, `min_machines_running = 1`
  (**always-on**, keeps one warm 24/7 — explicit comment: "kills the ~10s
  cold-start on first visit after idle")
- **Concurrency**: hard_limit=50, soft_limit=25
- **Body size limit**: 1 GiB (justified — 412 MB+ legitimate video uploads;
  RAM was sized for the double-buffered parse, comment: "1 GB OOM'd at 866 MB
  rss")
- **Mount**: `quilombo_data` → `/app/data` (SQLite + uploads + content dirs)
- **Health check**: `GET /api/v1/quilombo/versao` every 30s
- **Phase-1 candidate?**: partial. Cannot drop the 2GB tier (video upload
  workload is real), but `min_machines_running = 1` → `0` + `auto_stop =
  "suspend"` would still cut idle cost ~80%. Cold-start risk is the explicit
  10s the operator already named.

### yggdrasil-artelonga

- **fly.toml**: `/Users/artelonga/projects/yggdrasil/fly.toml`
- **App**: `yggdrasil-artelonga` in region `gru`
- **Build**: `yggdrasil-web/Dockerfile` (Rust + axum)
- **VM**: `shared-cpu-1x`, 1 vCPU, **512 MB RAM**
- **HTTP**: internal_port 3030, `auto_stop_machines = "stop"`,
  `min_machines_running = 0`
- **Concurrency**: hard_limit=100, soft_limit=80
- **Mount**: `yggdrasil_data` → `/data` (two SQLite DBs: yggdrasil + sementes)
- **Env**: SMTP placeholders + `YGGDRASIL_DB`, `YGGDRASIL_SEMENTES_DB`
- **Health check**: `GET /health` every 30s
- **Phase-1 candidate?**: yes — flip to `auto_stop = "suspend"` is a clean win;
  already on `min_machines_running = 0`.

### artelonga-rfq-gateway

- **fly.toml**: `/Users/artelonga/projects/rfq-gateway/fly.toml`
- **App**: `artelonga-rfq-gateway` in region `gru`
- **Build**: no `[build]` section beyond the header → Fly auto-detects (likely
  a Rust workspace via top-level Dockerfile if present, or uses
  `artelonga-rfq-gateway`'s buildpack)
- **VM**: **no `[[vm]]` block declared** → Fly defaults apply: `shared-cpu-1x`
  with 256mb RAM (Fly's default since 2023)
- **HTTP**: internal_port 8080, `auto_stop_machines = "stop"`,
  `min_machines_running = 1` (**always-on**)
- **Mount**: `rfq_artifacts` → `/app/artifacts` (observability ring JSONL files
  — per-machine volume, must exist in gru)
- **Health check**: `GET /health` every 30s
- **Phase-1 candidate?**: yes — `min_machines_running = 1 → 0` + `auto_stop =
  "suspend"` is the highest-yield single edit, since this app is rarely hit
  (per CO-281 sizing table: "internal-ish, suspend acceptable").

### ArteLonga (artelonga-dev) — status unclear

- **fly.toml**: `/Users/artelonga/projects/ArteLonga/fly.toml`
- **App declared**: `artelonga-dev` in region `gru` (NB: not `artelonga` —
  appears to be a dev variant)
- **Public site**: confirmed served from **GitHub Pages** at
  `artelonga.com.br` (README + CLAUDE.md both state "site estático, GitHub
  Pages serves direct from `main`")
- **VM**: shared-cpu-1x, 256mb
- **HTTP**: `auto_stop_machines = "stop"`, `min_machines_running = 0`
- **Mount**: `artelonga_dev_data` → `/app/data`
- **Action**: confirm with operator whether `artelonga-dev` Fly app is still
  running. If it is, it's an untracked cost; if it isn't, the fly.toml should
  be deleted to remove confusion. CO-281 should explicitly resolve this before
  Phase 1.

## Estimated current monthly cost

| App | Tier | RAM | Always-on? | Est. $/mo |
|---|---|---|---|---|
| co-artelonga | shared-1x | 512 MB | yes (min=0 but `stop` mode + LiteFS primary lease keeps it warm in practice) | $3.85 |
| co-artelonga-uat | shared-1x | 256 MB | `auto_stop_machines=false` (literally off) | $1.94 |
| quilombo-araucaria | shared-1x | 2048 MB | yes (`min_machines_running=1`) | ~$15.40 |
| yggdrasil-artelonga | shared-1x | 512 MB | no (`min=0`, suspends after idle) | ~$1-2 |
| artelonga-rfq-gateway | shared-1x | 256 MB | yes (`min_machines_running=1`) | $1.94 |
| artelonga-dev (unconfirmed) | shared-1x | 256 MB | no (`min=0`) | $0-1.94 |
| **Subtotal (machines)** | | | | **~$24-26/mo** |
| Volumes (~6 × 3GB @ $0.15/GB·mo) | | | | ~$2.70 |
| LiteFS / Consul (co-artelonga) | | | | included |
| Bandwidth (Sao Paulo egress) | | | | ~$1-3 |
| **Estimated total** | | | | **~$28-32/mo** |

### Comparison to CO-281 spec's pre-flight estimate

| Source | Estimated monthly machines |
|---|---|
| CO-281 spec §Context "Estimated monthly today" | ~$13-15/mo |
| This baseline (from actual fly.tomls) | ~$24-26/mo |

The delta is dominated by **quilombo-araucaria at 2GB always-on** — the spec
assumed 256 MB. The 2GB tier is justified by the video-upload workload comment
in its fly.toml, so this is a real (and known-good) cost line, not a sizing
mistake. The CO-281 plan as written would need to revise the
quilombo-araucaria target accordingly: 2GB stays, but flip `min=1 → 0` +
`auto_stop = "suspend"` for the idle-hours savings.

### Phase-1 candidates ranked by ROI

1. **artelonga-rfq-gateway**: flip `min=1 → 0` + `auto_stop = "suspend"`
   → saves ~$1.50/mo, easy win, no user-visible regression risk for an
   internal-ish app.
2. **quilombo-araucaria**: flip `min=1 → 0` + `auto_stop = "suspend"` (keep
   2GB) → saves ~$10-12/mo if idle ratio is 80%, with the explicit cold-start
   tax the operator already accepted in the toml comment.
3. **yggdrasil-artelonga**: flip `auto_stop = "stop" → "suspend"` →
   saves ~$0.50/mo; small but free.
4. **co-artelonga-uat**: confirm decommissioned per `feedback_no_uat.md`; if
   so, remove the app from Fly and delete `fly.uat.toml`. Potential saving:
   ~$1.94/mo + the operational cost of keeping it tested.

Combined Phase-1 savings projection: **~$12-15/mo**, i.e. ~45-50% of the
current machines bill — already approaching the CO-281 target band before
Phase 2 (embedding sidecar) and Phase 3 (right-size co-artelonga).

## Cost formula reference

Fly.io pricing (as of 2026, USD, monthly always-on):

- `shared-cpu-1x` 256 MB = $1.94/mo
- `shared-cpu-1x` 512 MB = $3.85/mo
- `shared-cpu-1x` 1 GB = $5.70/mo
- `shared-cpu-1x` 2 GB = ~$15.40/mo (linear-ish above 1 GB)
- `dedicated-cpu-1x` 2 GB = $31.43/mo

Volume storage: $0.15/GB·month (default 3 GB per volume = $0.45/mo each).

**Auto-suspend savings model**: a suspended machine costs ~0 for RAM/CPU
during the suspended window. If real traffic patterns leave the machine idle
80% of the time (typical for low-traffic internal services), expected savings
is ~80% of always-on cost. For a 95% idle ratio (rarely-hit internal apps
like rfq), savings approach 95%.

## Open items before Phase 1

- [ ] Run `flyctl status -a <app>` for each of the 5 apps from an
      authed shell; fill in `Last deploy` and confirm `Live version` matches
      `/api/health`
- [ ] Run `flyctl machine list -a <app> --json` and confirm declared
      `[[vm]]` sizes match what's actually provisioned (some apps may have
      been manually scaled outside the toml)
- [ ] Confirm `artelonga-dev` Fly app status (running, stopped, deleted) and
      either fold it into the baseline or remove its `fly.toml`
- [ ] Confirm `co-artelonga-uat` status (`feedback_no_uat.md` says UAT is
      decommissioned)
- [ ] Pull the Fly billing portal monthly total to anchor the estimate
      against ground truth — this baseline's $28-32/mo is derived, not
      observed

## References

- Spec: `work/co/CO-281.md`
- Memory: `feedback_no_uat.md` (UAT decommissioned, direct-to-prod model)
- Spec context table: CO-281 §Context "Current sizing vs target sizing"
- Fly pricing: https://fly.io/docs/about/pricing/
