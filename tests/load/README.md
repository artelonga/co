# Load Tests

k6 scenarios for CO load testing against UAT.

## Prerequisites

```bash
brew install k6
```

## Running a scenario

```bash
# Against UAT (default)
BASE_URL=https://co-artelonga-uat.fly.dev k6 run tests/load/06-mbya-browse.js

# Custom VU count and duration
BASE_URL=https://co-artelonga-uat.fly.dev k6 run --vus 100 --duration 1m tests/load/06-mbya-browse.js

# Save summary for baseline commits
BASE_URL=https://co-artelonga-uat.fly.dev k6 run --summary-export /tmp/run.json tests/load/06-mbya-browse.js
```

**Never run against prod.** Each script has a guard that aborts if `BASE_URL` looks like production.

## Reading the output

k6 prints a summary at the end of each run. Key metrics:

| Metric | What it means |
|--------|--------------|
| `http_req_duration p(95)` | 95th percentile latency — the "slow user" number |
| `http_req_failed` | Rate of unexpected failures (5xx, network errors; 4xx expected by the scenario are excluded) |
| `checks_succeeded` | Percentage of in-script assertions that passed |
| `iterations` | Complete scenario loops executed |
| Custom `mbya_entry_list_duration` | p50/p95/p99 specifically for the entries list endpoint |

## Updating the baseline

After a significant change (new index, hardware upgrade, algorithm change):

1. Run the scenario at 50 VU, 100 VU, and 500 VU (30s).
2. Copy the output to `tests/load/baselines/<YYYY-MM-DD>-<scenario>-<env>.md`.
3. Note what changed (the reason p95 moved) in the baseline file.
4. Commit the new baseline.

## Scenarios

| File | Universe | Traffic shape |
|------|----------|--------------|
| `06-mbya-browse.js` | `mbya` (4,608 lexemes) | Anonymous dictionary browse: info, entry list, tags, letter browse |

## Baselines

| File | Date | VUs tested | Finding |
|------|------|-----------|---------|
| `baselines/2026-05-01-mbya-uat.md` | 2026-05-01 | 50/100/500 | 500 VU collapse: O(N) scan on `entry_type`, p95=50s; 50/100 VU clean |
