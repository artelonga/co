# CO — Operations Guide

Runbook for deploying, verifying, and recovering the CO platform.

## Environments

| Env | App | URL |
|-----|-----|-----|
| **Production** | `co-artelonga` | `https://co.artelonga.com.br` |
| **UAT** | `co-artelonga-uat` | `https://co-artelonga-uat.fly.dev` |

**Deploy order: always UAT first.** Never push to production without a passing UAT smoke test.

---

## Smoke test post-deploy

After every deploy, run the smoke script to verify invariants.

```bash
# UAT (run after flyctl deploy --config fly.uat.toml)
bash scripts/smoke-uat.sh

# Production (run after flyctl deploy)
bash scripts/smoke-prod.sh
```

Both scripts exit 0 on full pass, 1 on any failure. Output is grep-friendly:

```
Smoke test: https://co.artelonga.com.br

✓ [01] /api/health → 200 (version 1.21.2)
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
| 07 | `GET /sw.js` | Body contains the current `CACHE_NAME` (`co-v3-network-first`) |
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
EXPECTED_SW_CACHE_NAME='co-v3-network-first'
```

When the seed JSON is edited (events added/removed), update the smoke script **in the same commit**.

### Wave 2 regression gate (CO-138)

```bash
BASE_URL=https://co-artelonga-uat.fly.dev npx playwright test e2e/wave-2/ --project=chromium-desktop
```

Covers: CO-98 sidebar tree nesting, CO-107 Mermaid SVG rendering, CO-99 onboarding banner lifecycle.

---

## Deploy procedure

```bash
# 1. Run local tests
cargo test -p co-web
cargo clippy -p co-web -- -D warnings

# 2. Deploy to UAT
flyctl deploy --config fly.uat.toml

# 3. Smoke-test UAT — gate on exit 0
bash scripts/smoke-uat.sh || { echo "UAT smoke FAILED — abort"; exit 1; }

# 4. Deploy to production
flyctl deploy

# 5. Smoke-test production
bash scripts/smoke-prod.sh
```

---

## UAT credentials

| Field | Value |
|-------|-------|
| Email | `yuri@uat.local` |
| Password | `uat` |
| Endpoint | `POST /api/v1/auth/uat-login` |

The `uat-login` endpoint returns 404 in production.

---

## UAT database reset

```bash
flyctl ssh console -a co-artelonga-uat -C "touch /data/uat-reset.flag"
flyctl machine restart -a co-artelonga-uat
flyctl logs -a co-artelonga-uat --no-tail | grep "UAT: reset"
```

---

## Secrets

```bash
# Production
flyctl secrets set JWT_SECRET=$(openssl rand -base64 48) -a co-artelonga

# UAT
flyctl secrets set JWT_SECRET=$(openssl rand -base64 48) -a co-artelonga-uat
```

---

## Logs and diagnostics

```bash
flyctl logs -a co-artelonga          # Production live
flyctl logs -a co-artelonga-uat      # UAT live
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
# Restore from S3 into UAT (safe — no prod guard triggered).
BUCKET=artelonga-co-backups ./scripts/restore.sh 20260501-031700 co-artelonga-uat

# Restore from S3 into PROD (requires explicit confirmation flag).
BUCKET=artelonga-co-backups ./scripts/restore.sh 20260501-031700 co-artelonga \
  --yes-i-want-to-overwrite-prod

# Restore a local .db file (used by restore-drill.sh).
./scripts/restore.sh backups/co-prod-20260501_031700.db co-artelonga-uat
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
4. Run `scripts/restore.sh <date> co-artelonga-uat`; smoke-test UAT.
5. Deploy the cron app (Option A) or enable the GH Actions workflow (Option B).
6. After the first automated run, confirm the new objects appear in S3.

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
