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
