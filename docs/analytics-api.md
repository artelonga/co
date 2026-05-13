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
