# Load Tests — CO Platform

k6 scenario scripts for performance baseline testing against UAT.

**Never run against production.** Each script guards against prod URLs and aborts if `BASE_URL` looks like `co-artelonga.fly.dev` (without `-uat`) or `co.artelonga.com.br`.

---

## Prerequisites

```bash
brew install k6   # macOS
# or: https://k6.io/docs/get-started/installation/
k6 version        # v1.7+
```

---

## Scenarios

| Script | Flow | Auth |
|--------|------|------|
| `01-anon-browse.js` | Static assets + board API + timeline | None |
| `02-logged-in-board.js` | Universe browse + task create/update/delete | Session cookie (provisioned in setup) |
| `03-vault-write.js` | Vault list + PUT + DELETE | Bearer token (provisioned in setup) |

---

## Running a Scenario

```bash
# Default (50 VUs, 1m) against UAT
k6 run tests/load/01-anon-browse.js

# Custom VUs and duration
k6 run --vus 100 --duration 1m tests/load/01-anon-browse.js

# Custom BASE_URL (UAT only — prod is blocked by guard)
BASE_URL=https://co-artelonga-uat.fly.dev k6 run --vus 50 --duration 1m tests/load/01-anon-browse.js

# Export summary JSON for baseline recording
k6 run --vus 50 --duration 1m --summary-export /tmp/k6-summary.json tests/load/01-anon-browse.js
```

---

## Reading the Output

k6 prints a summary at the end of each run. Key fields:

```
http_req_duration: avg=160ms  p(90)=380ms  p(95)=620ms
http_req_failed..: 0.00%
iterations.......: 234  3.0/s
```

- **p(95)**: 95th-percentile response time. The threshold is `< 1500ms`.
- **http_req_failed**: Rate of HTTP errors (4xx/5xx). Threshold is `< 5%`.
- **iterations**: How many complete user flows ran and the rate.

Custom metrics per scenario:
- `board_load_ms` (01): total time for board landing (8 requests)
- `timeline_load_ms` (01): total time for timeline (4 requests)
- `board_write_ms` (02): time for one create→update→delete cycle
- `vault_write_ms` (03): time for one vault PUT

**THRESHOLD lines:**
- `✓` = threshold passed (healthy)
- `✗` = threshold crossed (investigate)

---

## Running the Full Baseline Suite

Run each scenario at 50, 100, and 500 VUs and record results in `baselines/`.

```bash
# Scenario 01 — anon browse
k6 run --vus 50  --duration 1m  --summary-export /tmp/01-50.json  tests/load/01-anon-browse.js
k6 run --vus 100 --duration 1m  --summary-export /tmp/01-100.json tests/load/01-anon-browse.js
k6 run --vus 500 --duration 30s --summary-export /tmp/01-500.json tests/load/01-anon-browse.js

# Scenario 02 — logged-in board (login provisioned once in setup)
k6 run --vus 50  --duration 1m  --summary-export /tmp/02-50.json  tests/load/02-logged-in-board.js
k6 run --vus 100 --duration 1m  --summary-export /tmp/02-100.json tests/load/02-logged-in-board.js
k6 run --vus 500 --duration 30s --summary-export /tmp/02-500.json tests/load/02-logged-in-board.js

# Scenario 03 — vault write (API token provisioned once in setup)
k6 run --vus 50  --duration 1m  --summary-export /tmp/03-50.json  tests/load/03-vault-write.js
k6 run --vus 100 --duration 1m  --summary-export /tmp/03-100.json tests/load/03-vault-write.js
k6 run --vus 500 --duration 30s --summary-export /tmp/03-500.json tests/load/03-vault-write.js
```

Wait 30s between scenario runs to let the UAT machine recover.

---

## Updating the Baseline

When re-running to compare against a new deploy or hardware change:

1. Run the scenarios above and capture `--summary-export` JSON files.
2. Copy `baselines/2026-04-29-uat.md` to a new file (e.g., `baselines/YYYY-MM-DD-uat.md`).
3. Update the tables with the new p50/p95/p99/error values from the terminal output.
4. Add a "Changes since previous baseline" section noting what changed.
5. Update the "Recommendation" section if the hardware/capacity picture changed.

**p50/p95** come from the terminal summary or the exported JSON (`http_req_duration.med` and `http_req_duration.p(95)`).  
**p99** is only shown in the terminal output (not in the JSON summary export); read it from the k6 terminal directly.  
**Error rate** comes from `http_req_failed.value` in the JSON (decimal, e.g., `0.01` = 1%).

---

## UAT Setup Notes

- **Scenario 02** requires the `LDTEST` board project to exist. It is created automatically in `setup()` if missing.
- **Scenario 03** requires the `vaulttest` universe to exist. It is created automatically in `setup()` if missing.
- If UAT is reset (`flyctl machine restart`), run scenario 02 once to recreate `LDTEST`.
- The vault token provisioned in `setup()` is valid for 90 days. Rotate by re-running.

---

## Troubleshooting

| Problem | Likely cause | Fix |
|---------|-------------|-----|
| `ABORT: refusing to run against production` | `BASE_URL` is prod | Use `https://co-artelonga-uat.fly.dev` |
| `UAT login failed in setup: 503` | UAT machine starting up | Wait 30s, retry |
| `attempt to write a readonly database` on UAT | LiteFS lost primary lease | `flyctl machine restart <id> -a co-artelonga-uat` |
| `429 Too Many Requests` on vault writes | Server rate limiter | Expected at 50+ VUs. Use fewer VUs or add `sleep()` |
| Board write creates piling up (usage gate 429) | Content count > 100 | Script uses create+delete; check LDTEST project |
