---
fire_at: 2026-05-13T09:17:00-03:00
created_at: 2026-04-29
status: pending
related: CO-97
---

# 2026-05-13: verify marketing → Co telemetry endpoint flip

**Context:** On 2026-04-29 we shipped the Co telemetry endpoint at
`https://co.artelonga.com.br/api/v1/telemetry/events`. Marketing site
(`assets/analytics.js`) needed to flip `ENDPOINT` to that URL and bump the
cache-buster.

**This file is a backup for the in-session cron** (`CronCreate` job
`70e9f835`, scheduled at this same datetime, marked durable but the runtime
warned it's session-only — so when Claude exits this session, the cron is gone).
If you start a new Claude session before May 13, ask it to re-arm the cron OR
paste this file's content as a /loop or manual prompt on May 13.

## Verification checklist

1. **Endpoint flipped** — `assets/analytics.js` in the marketing site repo
   (`artelonga/ArteLonga` or wherever the static site lives):
   ```js
   const ENDPOINT = "https://co.artelonga.com.br/api/v1/telemetry/events";
   ```
   Should NOT be a `fly.dev` URL or staging endpoint.

2. **Cache-buster bumped** — query param on the analytics.js script tag
   (e.g. `analytics.js?v=2026-04-29`) should be newer than 2026-04-29 if the
   marketing site re-deployed since.

3. **Endpoint live** — sample POST returns 2xx:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" -X POST \
     -H 'Content-Type: application/json' \
     -d '{"site":"artelonga","event":"test","ts":1715000000}' \
     https://co.artelonga.com.br/api/v1/telemetry/events
   # → 200 or 204
   ```

4. **Rows landing** — SSH into prod and query (requires user authorization):
   ```bash
   flyctl ssh console -a co-artelonga -C \
     "sqlite3 /data/co.db 'SELECT site, COUNT(*) FROM telemetry_events \
      WHERE created_at > datetime(\"now\", \"-24 hours\") GROUP BY site;'"
   ```
   Should return at least one row with `site=artelonga` if the flip is live
   AND someone visited the marketing site in the past 24h.

## If any step fails

Report the gap precisely (which step failed, exact output) so the user can act.
Don't make code changes — diagnostic only.

## Related work

- CO-97 (`work/co/CO-97.md`) — visitor token unification, depends on this flip
  being live before tackling the cross-domain cookie stitching.
- Marketing repo `assets/analytics.js`.
- Privacy disclosure: `data/universes/template/content/dados-rastreados.md`
  references the telemetry endpoint.
