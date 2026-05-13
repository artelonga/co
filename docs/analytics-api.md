# Analytics Public API

Read-only endpoints that power the `artelonga.com.br/analytics` dashboard.

No authentication required. Data is aggregated and stripped of all PII before
exposure — see [Strip](#strip) below.

## Base URL

```
https://co.artelonga.com.br/api/v1/analytics/public
```

## CORS

Both endpoints accept cross-origin requests from `https://artelonga.com.br`.
The server mirrors the caller's `Origin` header so `credentials: 'omit'` works
without a hard-coded allowlist. Admin endpoints are not affected.

## Caching

Responses are cached in-process for **5 minutes** per unique query parameter
combination. A second identical request within the TTL is served from memory
with no DB hit.

---

## GET /summary

Returns aggregated metrics for the `artelonga` universe.

### Query parameters

| Param | Type | Default | Constraints |
|-------|------|---------|-------------|
| `days` | integer | `30` | clamped to `[1, 365]`; `0` → 400 |

### Response shape

```jsonc
{
  "as_of": "2026-05-09T00:00:00Z",   // RFC-3339, UTC
  "window_days": 30,
  "views": 12345,                     // pageview events in window
  "events_total": 28910,             // all events in window
  "visitors": 2341,                  // distinct visitor_tokens
  "returning": 412,                  // visitors seen on >= 2 distinct days
  "sessions": 3877,                  // distinct session_ids
  "session_avg_ms": 71200,           // avg duration_ms for events with duration > 0
  "countries": 18,                   // distinct countries (0 until CO-178 deployed)
  "cities": 67,                      // distinct cities (0 until CO-178 deployed)
  "timeseries": [
    { "bucket": "2026-04-10", "count": 142 },
    ...
  ],
  "top_pages": [
    { "path": "/", "views": 1240, "visitors": 412 },
    ...
  ],
  "geo": [
    { "country": "BR", "city": "São Paulo", "visitors": 410, "sessions": 622 },
    ...
  ]
}
```

`geo` is empty until CO-178 (geo enrichment) populates `country`/`city` columns
in `telemetry_events`.

### Example

```bash
curl "https://co.artelonga.com.br/api/v1/analytics/public/summary?days=7"
```

---

## GET /recent

Returns the most recent raw events for the `artelonga` universe, newest first.

### Query parameters

| Param | Type | Default | Constraints |
|-------|------|---------|-------------|
| `limit` | integer | `50` | clamped to `[1, 200]` |

### Response shape

```jsonc
{
  "events": [
    {
      "ts": 1746820000000,     // milliseconds since Unix epoch
      "name": "page_view",    // event_name
      "path": "/",            // nullable
      "country": "BR",        // nullable; null until CO-178 deployed
      "city": "São Paulo"     // nullable; null until CO-178 deployed
    },
    ...
  ]
}
```

### Example

```bash
curl "https://co.artelonga.com.br/api/v1/analytics/public/recent?limit=20"
```

---

## Strip

The following fields from `telemetry_events` are **never** exposed:

| Field | Reason |
|-------|--------|
| `visitor_token` | pseudonymous visitor ID |
| `ip_hash` | hashed IP address |
| `properties` | raw event properties (may include UTM params, experiment flags) |
| `user_id` | logged-in user identity |
| `session_id` | raw session token |

Geo data is derived server-side from the raw IP before it is hashed and
discarded. City granularity is "São Paulo", never a street address.

---

## Data source

All data comes from the `telemetry_events` SQLite table filtered to
`universe_key = 'artelonga'`. The composite index
`idx_telemetry_universe_time ON telemetry_events(universe_key, timestamp)`
(added in CO-179) makes window queries cheap.

Geo enrichment (CO-178) uses MaxMind GeoLite2 City in-process. Attribution:
This product includes GeoLite2 data created by MaxMind, available from
<https://www.maxmind.com>.

## Related

- CO-179 — implementation
- CO-177 — CORS + `universe_key` population
- CO-178 — geo enrichment (country/city)
- CO-180 — popularity rankings (sibling endpoint)
- CO-105 — admin telemetry dashboard (internal counterpart)
