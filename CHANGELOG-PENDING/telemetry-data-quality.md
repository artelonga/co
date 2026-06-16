## telemetry-data-quality — Per-site breakdown, geo on records, real dwell

Fixed three data-quality gaps in the public (unauthenticated) analytics read path
(`GET /api/v1/analytics/public/summary` and `/recent`), which is the apex
dashboard's data source. Read-only, no schema/ingest-contract changes.

### Per-site breakdown
- `?site=<name>` now scopes **all** summary/recent metrics to that site (the
  `site` value the marketing beacon stores in the `universe_key` column). It
  takes precedence over `?universe=` (same underlying column).
- The summary response gains a `sites: [{site, views, visitors, sessions}]`
  array — per-site engagement in a single call, grouped by `universe_key`.

### Geo on records
- `/recent` now populates `country`/`city` per event from the stored geo columns
  (resolved server-side at ingest, CO-178) instead of always returning `null`.
  No raw IPs are exposed.

### Dwell
- `session_avg_ms` is computed as the average per-session span
  (`max(timestamp) − min(timestamp)` per `session_id`, single-event sessions
  excluded). Previously it was `AVG(duration_ms)`, which is always 0 for
  marketing beacons (they don't carry a per-event `duration_ms`).

### Why
The apex dashboard could only see one global aggregate, recent events showed no
geography, and dwell was permanently 0 — so per-site engagement, visitor
geography, and session depth were all invisible despite the data being present.
