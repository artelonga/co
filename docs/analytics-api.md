# Analytics Ingestion API — CO-177

Endpoint for receiving batched client-side events from external sites.

## Canonical URL

```
POST https://co.artelonga.com.br/api/v1/telemetry/events
```

## CORS Allowlist

The following origins are permitted for cross-origin requests:

| Origin | Methods | Credentials |
|--------|---------|-------------|
| `https://artelonga.com.br` | `POST`, `OPTIONS` | `false` |
| `https://www.artelonga.com.br` | `POST`, `OPTIONS` | `false` |

CORS is handled by the global `tower_http::cors::CorsLayer` configured with
`AllowOrigin::mirror_request()` — the caller's `Origin` is echoed back in
`Access-Control-Allow-Origin`. Clients **must** use `credentials: "omit"` (the
default for cross-site fetches without explicit credentials).

Admin telemetry endpoints (`/api/v1/admin/telemetry/*`) are not opened further —
they remain protected by GitHub admin authentication regardless of origin.

## Request Shape

```json
{
  "schema": 1,
  "batch": [
    {
      "s": 1,
      "site": "artelonga",
      "name": "page_view",
      "sid": "<session-uuid>",
      "vid": "<visitor-uuid>",
      "ts": 1715600000000,
      "tz": "America/Sao_Paulo",
      "path": "/blog/post",
      "query": "?ref=twitter",
      "ref": "https://twitter.com",
      "vw": 1440,
      "vh": 900,
      "lang": "pt-BR",
      "ua_brand": "Chrome",
      "utm": { "source": "newsletter", "medium": "email" },
      "experiments": {},
      "props": {}
    }
  ]
}
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `schema` | integer | yes | Must be `1` |
| `batch` | array | yes | 1–200 events |
| `site` | string | yes | Universe key (e.g. `"artelonga"`). Stored as `universe_key` for scoped queries. Max 64 chars. |
| `name` | string | yes | Event name (e.g. `"page_view"`). Max 64 chars. |
| `sid` | string | yes | Session ID (UUID) |
| `vid` | string | yes | Visitor ID (UUID, stored in localStorage) |
| `ts` | integer | no | Unix timestamp in milliseconds. Defaults to server time. |
| `path` | string | no | URL path |

## Response

| Status | Meaning |
|--------|---------|
| `204 No Content` | Events accepted (or batch was all bots/invalid — silent drop) |
| `400 Bad Request` | Schema ≠ 1, or batch empty / > 200 events |

## Database

Events land in `telemetry_events`:

| Column | Source |
|--------|--------|
| `universe_key` | `site` field (sanitised: non-empty, ≤ 64 chars) |
| `event_name` | `name` field |
| `event_type` | Derived: `page_view` → `pageview`, `js_error` → `error`, `perf_*` → `performance`, else `interaction` |
| `ip_hash` | Daily-salted xxh3 hash — raw IP never stored |
| `visitor_token` | `vid` |
| `session_id` | `sid` |
| `properties` | JSON blob: utm, experiments, vw, vh, lang, tz, query, ref, site |

Index `idx_telemetry_universe_time ON telemetry_events(universe_key, timestamp)`
makes universe-scoped time-range queries (CO-179 dashboard, CO-180 popularity) O(events_for_universe).

## Client Integration

In `artelonga/ArteLonga docs/analytics.js`, set:

```js
const ENDPOINT = "https://co.artelonga.com.br/api/v1/telemetry/events";
```

Use `credentials: "omit"` (already the default for cross-site fetches without cookies).

---

# Analytics API — CO Telemetry

This document describes the telemetry data CO collects, how it is processed,
and the privacy guarantees that apply.

## Overview

CO records anonymised events to `telemetry_events` (SQLite). No PII is stored.
The analytics dashboard at `/analytics` (admin-only) aggregates these events.

## Event Lifecycle

```
Request received
  │
  ├─ Bot detected?  →  discard
  │
  ├─ Extract raw IP from X-Forwarded-For
  │    │
  │    ├─ geo_lookup(raw_ip)  →  (country, city)   # in-process, < 1 ms
  │    │    Returns (None, None) for private/loopback IPs and when DB absent.
  │    │
  │    └─ hash_ip_daily(raw_ip)  →  ip_hash        # xxh3 + daily salt
  │         raw_ip falls out of scope — never logged, never stored
  │
  └─ INSERT INTO telemetry_events (... ip_hash, country, city ...)
```

### Privacy invariants

| Field | What is stored | What is NOT stored |
|-------|----------------|--------------------|
| `ip_hash` | xxh3(date + raw_ip) — rotates daily | Raw IP address |
| `country` | ISO 3166-1 α-2 code (e.g. `"BR"`) | Full address |
| `city` | City name (e.g. `"São Paulo"`) | Street / district |
| `visitor_token` | Random nanoid — no PII | Email, name |
| `user_id` | Internal UUID | Email, password |

Raw IPs are never written to logs or database rows.

## Schema — telemetry_events

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER | Auto-increment primary key |
| `timestamp` | TEXT | RFC 3339 UTC timestamp |
| `visitor_token` | TEXT | Anonymous visitor ID (cookie `al_vid` / `visitante_id`) |
| `user_id` | TEXT | Authenticated user ID if logged in |
| `session_id` | TEXT | Derived session hash (no raw JWT) |
| `event_type` | TEXT | `pageview`, `interaction`, `error`, `performance`, `crud` |
| `event_name` | TEXT | e.g. `page.view`, `task.create`, `theme.change` |
| `universe_key` | TEXT | Universe slug |
| `path` | TEXT | URL path |
| `properties` | TEXT | JSON — event-specific data |
| `duration_ms` | INTEGER | Server or client duration |
| `ip_hash` | TEXT | Daily-salted xxh3 hash of the request IP |
| `ua_device` | TEXT | `desktop` / `mobile` / `tablet` |
| `ua_browser` | TEXT | `chrome` / `firefox` / `safari` / `edge` / `other` |
| `ua_os` | TEXT | `windows` / `mac` / `linux` / `android` / `ios` / `other` |
| `country` | TEXT | ISO 3166-1 α-2 country code (CO-178) — NULL for private IPs |
| `city` | TEXT | City name in English (CO-178) — NULL for private IPs |

Retention: rows older than 90 days are deleted automatically.

## Geo Enrichment (CO-178)

### Data source

**MaxMind GeoLite2 City** — free for use with attribution.

> This product includes GeoLite2 data created by MaxMind, available from
> <https://www.maxmind.com>.

### Configuration

The database file path is configurable via the `GEOIP_DB_PATH` environment
variable (default: `/data/GeoLite2-City.mmdb`).

If the file is absent the server starts normally; all `country` / `city` values
remain NULL. No network calls are made at runtime.

### Deployment

Include `GeoLite2-City.mmdb` in the Docker image or mount it on the data volume:

```dockerfile
COPY GeoLite2-City.mmdb /data/GeoLite2-City.mmdb
```

Or download on first boot from a trusted source and place at the path above.

### Behaviour

| IP type | `country` | `city` |
|---------|-----------|--------|
| Public, DB loaded | ISO code (e.g. `"US"`) | City name (e.g. `"Mountain View"`) |
| Private / loopback (10.x, 172.16.x, 192.168.x, 127.x) | NULL | NULL |
| Public, DB absent | NULL | NULL |
| Unparseable input | NULL | NULL |

City granularity is the municipality (e.g. "São Paulo"), not street level.

## Admin endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/admin/telemetry/summary` | Aggregated dashboard stats |
| `GET` | `/api/v1/admin/telemetry/export` | Last 10 000 events as CSV |
| `GET` | `/co/co-dev/telemetria` | HTML analytics dashboard |

Both endpoints require GitHub admin authentication (`GESTAO_GITHUB_ADMINS`).

## Client-side endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/v1/telemetry/event` | Single client-side event |
| `POST` | `/api/v1/telemetry/events` | Batched marketing-schema events |

See `co-web/src/telemetry.rs` for accepted JSON shapes.

---

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
