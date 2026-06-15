# CO — Operations Guide

Runbook for deploying, verifying, and recovering the CO platform.

## Environments & Deploy

> **This section is the single source of truth** for environments and the deploy
> flow. CLAUDE.md, `docs/ci-cd.md`, `docs/delivery-pipeline.md`, and
> `docs/release-checklist.md` reference it rather than restating it.

| Env | App | URL | Role |
|-----|-----|-----|------|
| **Production** | `co-artelonga` (Fly `gru`) | `https://co.artelonga.com.br` (`co-artelonga.fly.dev`) | The only **required** deploy target. Public-facing. |
| **Staging** (optional) | `co-artelonga-staging` | `https://co-artelonga-staging.fly.dev` | **Manual preview only**, deployed by hand via `flyctl deploy --config fly.staging.toml`. NOT a release gate. The `staging-deploy.yml` workflow is a deliberate no-op — `FLY_API_TOKEN` is intentionally not a repo secret. |

**There is no UAT environment** (decommissioned). `fly.uat.toml` is dead — do not
deploy it. Deploy is **prod-direct**: there is no "UAT-first" step. The release
gate is the read-only CO-421 prod-usability suite plus `scripts/smoke-prod.sh`,
not a separate environment. See "Deploy procedure" below.

---

## Smoke test post-deploy

After every prod deploy, run the smoke script to verify invariants.

```bash
# Production (run after flyctl deploy)
bash scripts/smoke-prod.sh
```

The script exits 0 on full pass, 1 on any failure. Output is grep-friendly:

```
Smoke test: https://co.artelonga.com.br

✓ [01] /api/health → 200 (version 3.15.0)
✓ [02] /api/health/deep → 200 (db=ok disk=ok)
✓ [03] /api/v1/universes/template name=CO
✓ [04] /api/v1/universes/tempo visibility=public-static
✓ [04] /api/v1/universes/tempo/entries?type=event tempo events = 21
...
--- ALL CHECKS PASSED ---
```

On failure:

```
✗ [04] /api/v1/universes/tempo/entries?type=event expected tempo events 21, got 19  ← FAIL

--- FAILED (1 check(s) failed) ---
```

### Checks performed

| # | Endpoint | What is verified |
|---|----------|-----------------|
| 01 | `GET /api/health` | Returns 200 with `status=ok`; version is printed |
| 02 | `GET /api/health/deep` | Returns 200 with `db=ok` and `disk=ok` |
| 03 | `GET /api/v1/universes/template` | Template universe present with `name=CO` |
| 04 | `GET /api/v1/universes/{tempo,humanity,universo}` | `visibility=public-static` for each |
| 04 | `GET /api/v1/universes/{key}/entries?type=event` | Event counts pinned at 21/26/28 |
| 05 | `GET /api/v1/themes/modern` | Returns `text/css` containing `--accent: #6366f1` |
| 06 | `GET /`, `/app.js`, `/shared/timeline.html` | Static assets reachable with correct content-types |
| 07 | `GET /sw.js` | Body contains the current `CACHE_NAME` (`co-v6-offline`) |
| 08 | `POST /api/v1/auth/password-login` (bogus) | Returns 401, not 5xx — proves auth is reachable |
| 09 | `GET /api/v1/universes/template/entries` | `total >= 14` template entries present |
| 10 | `GET /favicon.svg` | Returns 200 |

### Override BASE_URL

Both scripts accept `BASE_URL` from the environment:

```bash
BASE_URL=https://co-artelonga.fly.dev bash scripts/smoke-prod.sh
```

### Pinned counts

Event counts for the timeline trio are pinned at the top of each smoke script:

```bash
EXPECTED_TEMPO_EVENTS=21
EXPECTED_HUMANITY_EVENTS=26
EXPECTED_UNIVERSO_EVENTS=28
EXPECTED_SW_CACHE_NAME='co-v6-offline'
```

When the seed JSON is edited (events added/removed), update the smoke script **in the same commit**.

### Wave 2 regression gate (CO-138)

```bash
# Against localhost (default) or prod via BASE_URL — read-only suite
BASE_URL=https://co.artelonga.com.br npx playwright test e2e/wave-2/ --project=chromium-desktop
```

Covers: CO-98 sidebar tree nesting, CO-107 Mermaid SVG rendering, CO-99 onboarding banner lifecycle.

### Smoke check table (CO-142 additions)

| # | Endpoint | What is verified |
|---|----------|-----------------|
| 11 | `GET /api/v1/universes/{template,quilomboaraucaria,co,tempo,humanity,universo}` | Each public universe returns 200 to anonymous |
| 12 | `GET /api/v1/universes/template` | `content_count >= 6` (CO-142 Phase B recompute) |

---

## Startup invariants (CO-142)

On every boot, `co-web` runs the following cleanup/reconciliation steps before
accepting traffic:

1. **`delete_deprecated_universes()`** — hard-deletes `co-dev` and `co-experience`
   rows (and their memberships). Idempotent no-ops once the rows are gone.

2. **`delete_stale_quilombo_variants()`** — hard-deletes `quilombo-blog`,
   `quilombo-blog-2`, `quilombo-blog-3`, and `qa-dev`. See `docs/UNIVERSES.md`.

3. **`recompute_content_counts()`** — for every universe, counts rows in its
   per-universe `entries` DB and writes the result to `universes.content_count`.
   This corrects drift caused by seed paths that call `upsert_entry_row` without
   calling `increment_universe_content_count`.

4. **`copy_dir_all(/app/seed-co, data/co/)`** — refreshes the dev board's
   source files from the image-bundled `work/co/` snapshot. Keeps completed
   task statuses in sync without a write-back loop.

The dev board API (`/api/v1/admin/co-dev`) is admin-only and reads from
`data/co/` (refreshed above). The SPA route is `/co/co/telemetria` (renamed
from the old `/co/co-dev/telemetria` in CO-142 Phase A).

---

## Deploy procedure (prod-direct, canonical)

There is no UAT step. The release gate is the read-only CO-421 prod-usability
suite plus the disk gate, then prod deploy, then the prod smoke test.

```bash
# 1. Local checks
cargo test
cargo clippy -- -D warnings

# 2. CO-421 read-only Playwright prod-usability gate (anonymous; never mutates prod)
cd co-web && BASE_URL=https://co.artelonga.com.br \
  npx playwright test e2e/prod-usability.spec.ts --project=desktop-chromium --workers=2

# 3. Pre-deploy gate — CO-446 disk check + a fresh green local pipeline report
bash scripts/pipeline-deploy-gate.sh || { echo "deploy gate FAILED — abort"; exit 1; }

# 4. Deploy to production
flyctl deploy

# 5. Smoke-test production
bash scripts/smoke-prod.sh
```

Optional manual staging preview (not a gate): `flyctl deploy --config fly.staging.toml`.

> **CO-446 — always gate disk before a migration deploy.** A release that adds a
> migration writes a `schema_version` row at boot. On a near-full `/data` that
> write fails with `SQLITE_FULL` and the server crash-loops (2026-06-11 +
> 2026-06-13 outages). `pipeline-deploy-gate.sh` checks `df -P /data` on prod and
> **blocks at > 85% full** (`DISK_MAX_PCT`). Extend *before* deploying — see
> "Disk-full recovery" below. To skip the check (no flyctl, or already verified):
> `--no-disk`.

---

## Disk-full recovery (CO-446)

`/data` is a fixed-size Fly volume. When it fills, the **next boot that runs a
migration panics** (`record_migration … database or disk is full`) and the
machine crash-loops until it hits max-restart — the site goes dark. As of CO-446
the boot now degrades to a clear `FATAL (CO-446): migrations failed …` log line
and a clean exit instead of a cryptic SQLite backtrace, but it still cannot
serve until the volume has headroom. The fix is to **extend the volume**.

```bash
# 1. Inspect current usage (which volume, how full)
flyctl volumes list -a co-artelonga
flyctl ssh console -a co-artelonga -C "df -h /data"

# 2. Extend the volume (volumes only ever grow). Pick the next size up.
flyctl volumes extend <vol-id> -s <new-GB> -a co-artelonga

# 3. CRITICAL: stop/start the machine — a plain `restart` does NOT resize the fs.
flyctl machine list -a co-artelonga
flyctl machine stop  <machine-id> -a co-artelonga
flyctl machine start <machine-id> -a co-artelonga

# 4. Confirm the filesystem grew and the boot is clean
flyctl ssh console -a co-artelonga -C "df -h /data"
flyctl logs -a co-artelonga --no-tail | grep -iE "CO-446|migration|disk"
bash scripts/smoke-prod.sh
```

**Why stop/start, not restart:** Fly only re-reads the volume size and grows the
ext4 filesystem on a fresh machine start. `flyctl machine restart` re-runs the
process against the *old* filesystem size, so the disk stays full and the
crash-loop continues. This bit us on 2026-06-13 — the live fix was
`volumes extend 10→20GB` + **stop/start**.

**Tuning knobs:**

| Variable | Default | Effect |
|----------|---------|--------|
| `CO_MIGRATION_MIN_FREE_BYTES` | 200 MiB | Boot pre-flight: abort migrations with a clear ERROR if free `/data` is below this (vs. panicking mid-migration). |
| `DISK_MAX_PCT` (deploy gate) | 85 | `pipeline-deploy-gate.sh` blocks a prod deploy when `/data` is fuller than this. |
| `ALLOW_FULL_DISK=1` (deploy gate) | unset | Downgrade the gate block to a warning (use only alongside a planned extend). |

> The real endgame is **S3 cold-tier offload (CO-81)** — without it `/data` grows
> without bound and this recurs. CO-446 is the safety net so the next time is a
> clear error + a one-command extend, not an outage.

---

## Admin login in production (CO-85)

Admin users with a `password_hash` log in via `POST /api/v1/auth/password-login`
in any environment. There is no UAT login.

```bash
curl -sc cookies.txt -X POST https://co-artelonga.fly.dev/api/v1/auth/password-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"yuri@artelonga.com.br","password":"<your-password>"}'
# → 200, Set-Cookie: session=<JWT>
```

---

## Secrets

```bash
# Production
flyctl secrets set JWT_SECRET=$(openssl rand -base64 48) -a co-artelonga
```

---

## Logs and diagnostics

```bash
flyctl logs -a co-artelonga          # Production live
flyctl status -a co-artelonga        # Machine state
flyctl ssh console -a co-artelonga   # Shell access
```

---

## Backup & restore (CO-104 + CO-119)

Two scripts handle the full backup lifecycle:

| Script | Purpose |
|--------|---------|
| [`scripts/backup-prod.sh`](../scripts/backup-prod.sh) | Daily S3 snapshot of SQLite + `universes/` |
| [`scripts/restore.sh`](../scripts/restore.sh) | Restore from S3 date or local file into a Fly app |

### S3 key layout

```
s3://artelonga-co-backups/
  co.db/<YYYYMMDD-HHMMSS>.db
  universes/<YYYYMMDD-HHMMSS>.tar.gz
```

Lifecycle: transition to S3-IA after 30 days, delete after 365 days
(`infra/s3/lifecycle.json`).

### Taking an on-demand backup

```bash
APP=co-artelonga BUCKET=artelonga-co-backups ./scripts/backup-prod.sh
# → [backup] uploaded to s3://artelonga-co-backups — done (date=20260501-031700)
```

### Restoring a backup

```bash
# Restore from S3 into staging (safe — no prod guard triggered).
BUCKET=artelonga-co-backups ./scripts/restore.sh 20260501-031700 co-artelonga-staging

# Restore from S3 into PROD (requires explicit confirmation flag).
BUCKET=artelonga-co-backups ./scripts/restore.sh 20260501-031700 co-artelonga \
  --yes-i-want-to-overwrite-prod

# Restore a local .db file (used by restore-drill.sh).
./scripts/restore.sh backups/co-prod-20260501_031700.db co-artelonga-staging
```

The script refuses to target `co-artelonga` (production) without the explicit
`--yes-i-want-to-overwrite-prod` flag — protecting against accidental prod wipes.

### Cron automation

**Option A (primary) — Fly app `co-backup-cron`:**

See `infra/backup-cron/`. One-time setup:

```bash
flyctl apps create co-backup-cron --org personal
flyctl secrets set -a co-backup-cron \
  FLY_API_TOKEN=<token> \
  AWS_ACCESS_KEY_ID=<key> \
  AWS_SECRET_ACCESS_KEY=<secret>
flyctl deploy --config infra/backup-cron/fly.toml
```

The app runs `crond` in the foreground and fires `scripts/backup-prod.sh`
daily at 03:17 UTC. Logs stream via `flyctl logs -a co-backup-cron`.

**Option B (fallback) — GitHub Actions:**

See `.github/workflows/backup.yml`. Requires repository secrets:
`BACKUP_AWS_ACCESS_KEY_ID`, `BACKUP_AWS_SECRET_ACCESS_KEY`, `FLY_API_TOKEN`.
Trigger manually via `workflow_dispatch` or let the daily `cron:` schedule fire.

### Running a restore drill

```bash
./tools/restore-drill.sh              # uses yesterday's backup
./tools/restore-drill.sh 20260430     # specific date
DRY_RUN=1 ./tools/restore-drill.sh   # verify backup availability without provisioning
```

Results are appended to `tools/restore-drill.log`. A passing run looks like:

```
OK  drill=co-drill-20260501-120000  date=20260430  restore=1  health=ok  univ=5  template=template  time=142s
```

### Quarterly cadence

Run `./tools/restore-drill.sh` once per quarter (Jan / Apr / Jul / Oct).
A scheduled agent handles this — see the `schedule` entry in `.claude/settings.local.json`.

### First-run checklist

1. Run `infra/s3/setup.sh` to create the bucket and apply the lifecycle policy.
2. Generate a dedicated IAM user (PutObject + GetObject only); store creds in cron app secrets.
3. Run `scripts/backup-prod.sh` manually; verify both artifacts appear in S3.
4. Run `scripts/restore.sh <date> co-artelonga-staging`; smoke-test staging.
5. Deploy the cron app (Option A) or enable the GH Actions workflow (Option B).
6. After the first automated run, confirm the new objects appear in S3.

---

## Local-source backup — external HD (CO-108)

Operator-driven, point-in-time cold-storage of all four content universe sources.
Complements the S3 ops backup (CO-104) — protects the local source ground truth that
Yuri authors against, not just the deployed state.

| Script | Purpose |
|--------|---------|
| [`scripts/backup-to-disk.sh`](../scripts/backup-to-disk.sh) | Archive one or more universes to an external HD |
| [`scripts/restore-from-disk.sh`](../scripts/restore-from-disk.sh) | Restore a `.tar.zst` archive into a local Co data dir |

### Archive format

Each universe is a self-contained `.tar.zst` bundle:

```
<universe_key>/
├── manifest.json    — provenance: source path, commit, sha256, schema version
├── co.db            — SQLite: universe row (+ entries for prod mode)
├── seed.sql         — one-shot universe registration (local mode only)
├── entries/         — full markdown source tree
└── README.md        — human-readable provenance
```

Bundles on the external HD are organized by run date:

```
/Volumes/<drive>/co-archive/
├── README.md
├── 2026-04-30/
│   ├── manifest.json              — global manifest with sha256 of each bundle
│   ├── quilomboaraucaria.tar.zst
│   ├── qa.tar.zst
│   ├── artelonga.tar.zst
│   └── rfq.tar.zst
└── 2026-05-07/
    └── …
```

Compression: zstd level 19. Markdown-heavy content compresses ≈ 8-12×.

### Taking a local-source backup

Connect external HD, then:

```bash
# Archive all four universe sources to the HD (local mode — no server required)
bash scripts/backup-to-disk.sh /Volumes/Backup --from local

# Archive from deployed prod state (requires flyctl + prod access)
bash scripts/backup-to-disk.sh /Volumes/Backup --from prod
```

| Mode | What gets archived |
|------|-------------------|
| `--from local` | Local markdown files only; synthetic co.db; useful before first deploy |
| `--from prod` | Prod's deployed SQLite + markdown files via `flyctl ssh` |

### Restoring from an HD archive

```bash
# Restore archive into a target data directory
bash scripts/restore-from-disk.sh \
    /Volumes/Backup/co-archive/2026-04-30/qa.tar.zst \
    /tmp/restored-co

# Boot co-web against the restored data
DATA_DIR=/tmp/restored-co/data cargo run -p co-web
# → universe accessible at http://localhost:3000/co/qa
```

The restore script:
1. Verifies SHA256 against the sidecar manifest (bit-rot check)
2. Checks `co_db_schema_version` compatibility (fails loud if too new)
3. Places `co.db` and `seed.sql` in the target data dir
4. Copies `entries/` to `data/universes/<key>/`

On first boot after restore:
- co-web renames `co.db → meta.db` (CO-77)
- `seed.sql` registers the universe row, then is auto-deleted
- For local-mode archives: board is empty until re-uploaded via vault API

### Verifying archive integrity

```bash
# Re-hash all archives in a run and compare against stored sha256
bash scripts/backup-to-disk.sh /Volumes/Backup --verify 2026-04-30
# → [artelonga] OK — artelonga.tar.zst (41 KB)
# → [qa] OK — qa.tar.zst (311 KB)
# → Verify result: 4 OK, 0 FAILED
```

Exits non-zero on any hash mismatch — safe to run in a cron job.

### Weekly cron (optional)

Run at a cadence that matches your authoring frequency. Example launchd plist
(`~/Library/LaunchAgents/com.artelonga.co-backup.plist`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.artelonga.co-backup</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/bash</string>
    <string>/Users/artelonga/projects/co/scripts/backup-to-disk.sh</string>
    <string>/Volumes/Backup</string>
    <string>--from</string>
    <string>local</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Weekday</key><integer>7</integer>  <!-- Sunday -->
    <key>Hour</key><integer>11</integer>
    <key>Minute</key><integer>0</integer>
  </dict>
  <key>StandardOutPath</key><string>/tmp/co-backup.log</string>
  <key>StandardErrorPath</key><string>/tmp/co-backup.err</string>
</dict>
</plist>
```

Load: `launchctl load ~/Library/LaunchAgents/com.artelonga.co-backup.plist`

### Storage budget

| Universe | Raw .md | Compressed (zstd-19) |
|---|---|---|
| quilomboaraucaria | ~600 KB | ~50 KB |
| qa | ~1.2 MB | ~100 KB |
| artelonga | ~3 MB | ~250 KB |
| rfq | ~280 KB | ~40 KB |
| **Per run** | **~5 MB** | **~440 KB** |

50 weekly runs/year ≈ 22 MB/year. Comfortably fits on any external HD for decades.

---

## Edge / CDN (CO-117)

`co.artelonga.com.br` runs behind Cloudflare CDN (proxied DNS, cache rules per spec).

### Cache rule summary

| Path | Behavior | Edge TTL |
|------|----------|----------|
| `/api/*` | Bypass | — |
| `/_app/immutable/*` | Cache | 1 year |
| `*.css` | Cache | 1 hour |
| `*.png / .svg / .webp / .avif` | Cache | 1 day |
| HTML pages | Cache by status | 60 s |

Auth responses (`Set-Cookie: session=`) are never cached — Cloudflare's
"Bypass cache on Cookie" rule + the origin's `Cache-Control: private, no-store`
provide defense in depth.

### Initial setup

```bash
cd infra/cloudflare
cp terraform.tfvars.example terraform.tfvars
# fill in cloudflare_api_token, zone_id, fly_ipv4
terraform init && terraform apply
```

### Verification

```bash
./tools/cf-verify.sh
```

### DNS migration steps (manual, one-time)

1. Log into Cloudflare dashboard → add `artelonga.com.br` zone
2. Note the Cloudflare nameservers and update at registrar
3. After propagation: `terraform apply` applies the A record + cache rules
4. Verify with `./tools/cf-verify.sh`

### Fly origin notes

No changes needed to the Fly app. The origin continues to receive requests
from Cloudflare IPs. Ensure `Cache-Control: private, no-store` is set on
all auth responses (already the case in co-web auth handlers).

---

## ClickHouse analytics (CO-123)

Single-node ClickHouse on Fly for ad-hoc analytics.  Bridges the gap between
WAE (cheap, real-time, low-cardinality) and the Phase 3 Iceberg lake.

### Architecture

```
Cloudflare WAE (co_telemetry dataset)
    │
    │  daily export at 04:07 UTC
    │  (co-clickhouse-export cron app)
    ▼
co_analytics.wae_events   ← MergeTree, partitioned by day, TTL 90 days
    │
    │  ad-hoc queries (admin only via flyctl proxy)
    ▼
clickhouse-client / HTTP API

Phase 3 (future): icebergS3('r2://co-lakehouse/...') reads directly
```

### Fly apps

| App | Purpose |
|-----|---------|
| `co-clickhouse` | ClickHouse server — 4 vCPU / 8 GB / 50 GB Volume |
| `co-clickhouse-export` | Daily WAE → ClickHouse export cron |

### One-time setup

```bash
# 1. Create apps and volume
flyctl apps create co-clickhouse --org personal
flyctl volumes create co_clickhouse_data --size 50 --region gru -a co-clickhouse

flyctl apps create co-clickhouse-export --org personal

# 2. Set secrets
flyctl secrets set CH_ADMIN_PASSWORD=$(openssl rand -base64 32) -a co-clickhouse

flyctl secrets set -a co-clickhouse-export \
  CF_ACCOUNT_ID=<cloudflare-account-id> \
  CF_API_TOKEN=<cf-analytics-read-token> \
  CH_PASSWORD=<same-password-as-above>

# 3. Deploy
flyctl deploy --config infra/clickhouse/fly.toml
flyctl deploy --config infra/clickhouse-export-cron/fly.toml
```

### Running queries

Open a proxy tunnel to ClickHouse (keep this terminal open):

```bash
flyctl proxy 8123:8123 -a co-clickhouse
```

Then query via `clickhouse-client` or plain HTTP:

```bash
# clickhouse-client (native protocol — open a second proxy on port 9000)
flyctl proxy 9000:9000 -a co-clickhouse
clickhouse-client --host 127.0.0.1 --port 9000 --user co_admin --password <pass>

# HTTP interface
curl -u co_admin:<pass> 'http://127.0.0.1:8123/?query=SELECT+count()+FROM+co_analytics.wae_events'
```

See `docs/analytics/sample-queries.sql` for ready-to-run queries:

- Top-10 universes by activity (last 24 h)
- Error rate by deploy / status code (last 7 days)
- 7-day and 30-day visitor retention

### Reading results

Query results stream to stdout as TSV by default.  For JSON:

```bash
curl -u co_admin:<pass> \
  'http://127.0.0.1:8123/?query=SELECT+...&default_format=JSONEachRow'
```

### WAE export job

The `co-clickhouse-export` app runs `scripts/wae-to-clickhouse.sh` daily at
04:07 UTC.  Check logs:

```bash
flyctl logs -a co-clickhouse-export --no-tail
```

Run manually for a specific date:

```bash
flyctl ssh console -a co-clickhouse-export \
  -C "EXPORT_DATE=2026-05-01 /scripts/wae-to-clickhouse.sh"
```

### Volume backup

ClickHouse's `BACKUP` command streams a consistent snapshot to any S3-
compatible target.  Run from inside the proxy session:

```bash
# Back up to Cloudflare R2
curl -u co_admin:<pass> 'http://127.0.0.1:8123/' --data \
  "BACKUP DATABASE co_analytics TO S3(
    'https://<ACCOUNT_ID>.r2.cloudflarestorage.com/co-clickhouse-backups/$(date +%Y%m%d)/',
    '<R2_ACCESS_KEY>', '<R2_SECRET_KEY>'
  )"

# Restore
curl -u co_admin:<pass> 'http://127.0.0.1:8123/' --data \
  "RESTORE DATABASE co_analytics FROM S3(
    'https://<ACCOUNT_ID>.r2.cloudflarestorage.com/co-clickhouse-backups/20260501/',
    '<R2_ACCESS_KEY>', '<R2_SECRET_KEY>'
  )"
```

For a full volume snapshot (all databases), replace `DATABASE co_analytics`
with `ALL`.

### Iceberg table function — Phase 3 smoke test

The `icebergS3()` table function is enabled in ClickHouse 24.x.  Run the
automated smoke test to verify end-to-end connectivity to R2:

```bash
# Step 1: create an empty Iceberg table on R2 (once)
S3_ENDPOINT=https://<ACCOUNT_ID>.r2.cloudflarestorage.com \
S3_BUCKET=co-lakehouse \
AWS_ACCESS_KEY_ID=<R2_ACCESS_KEY> \
AWS_SECRET_ACCESS_KEY=<R2_SECRET_KEY> \
python3 scripts/co-export.py

# Step 2: open proxy (separate terminal)
flyctl proxy 8123:8123 -a co-clickhouse

# Step 3: run smoke test
CH_PASSWORD=<pass> \
R2_ACCOUNT_ID=<id> \
R2_ACCESS_KEY=<key> \
R2_SECRET_KEY=<secret> \
bash infra/clickhouse/iceberg-smoke-test.sh
# → [iceberg-smoke] PASS — icebergS3() function is operational (Phase 3 ready)
```

### Logs and diagnostics

```bash
flyctl logs -a co-clickhouse              # ClickHouse live logs
flyctl logs -a co-clickhouse-export       # Export cron live logs
flyctl status -a co-clickhouse            # Machine state
flyctl ssh console -a co-clickhouse       # Shell access
```
