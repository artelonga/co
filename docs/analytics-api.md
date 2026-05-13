# Analytics Public API

Read-only endpoints that power artelonga.com.br analytics and ranking features.

No authentication required. Data is aggregated and stripped of all PII before
exposure — visitor tokens, IP hashes, and raw properties are never included.

## Base URL

```
https://co.artelonga.com.br/api/v1/analytics/public
```

## CORS

All endpoints accept cross-origin requests. The server mirrors the caller's
`Origin` header so `credentials: 'omit'` works without a hard-coded allowlist.

## Caching

Responses are cached in-process for **5 minutes** per unique query parameter
combination. A second identical request within the TTL is served from memory
with no DB hit. Cache entries within the TTL produce byte-identical `items`
arrays (stable order guaranteed by `ORDER BY views DESC, path ASC`).

---

## GET /popularity

Returns page-view counts for paths matching a given prefix, ordered by
popularity. Designed for the `bake-popularity` GH Action that commits
`assets/popularity.json` to artelonga/ArteLonga daily.

### Query parameters

| Param    | Type    | Default | Constraints                                        |
|----------|---------|---------|----------------------------------------------------|
| `prefix` | string  | —       | **Required.** Must start with `/`, no `..`, ≤ 64 chars |
| `days`   | integer | `30`    | Clamped to `[1, 365]`                              |

### Response shape

```jsonc
{
  "as_of": "2026-05-09T00:00:00Z",   // RFC-3339, UTC — time of cache fill
  "window_days": 30,                  // effective days after clamping
  "prefix": "/servicos/",            // echo of the requested prefix
  "items": [
    {
      "path": "/servicos/desenvolvimento-web/",
      "slug": "desenvolvimento-web",  // path stripped of prefix + trailing /
      "views": 1240,                  // total page_view events in window
      "visitors": 412                 // COUNT(DISTINCT visitor_token)
    },
    {
      "path": "/servicos/grafite/",
      "slug": "grafite",
      "views": 880,
      "visitors": 301
    }
  ]
}
```

**Empty result** (no events in window):

```jsonc
{ "as_of": "...", "window_days": 30, "prefix": "/servicos/", "items": [] }
```

### Ordering

`items` are ordered `views DESC, path ASC` (alphabetical tie-break on the full
path, which equals alphabetical on `slug` for a fixed prefix). This order is
deterministic and stable across re-runs with no new events.

### Limit

At most **200** items are returned — far more than the ~50 services currently
listed on artelonga.com.br.

### Error responses

| Status | Condition                                          |
|--------|----------------------------------------------------|
| `400`  | `prefix` missing, does not start with `/`, contains `..`, or exceeds 64 chars |

### SQL (reference)

```sql
SELECT path, COUNT(*) AS views, COUNT(DISTINCT visitor_token) AS visitors
FROM telemetry_events
WHERE universe_key = 'artelonga'
  AND event_name = 'page_view'
  AND path LIKE ?1 || '%'
  AND timestamp >= datetime('now', ?2)
GROUP BY path
ORDER BY views DESC, path ASC
LIMIT 200
```

Bot filter is applied upstream (CO-46) — bot UA strings are never inserted into
`telemetry_events`. Repeated views from the same user count — a curious visitor
reloading 3× signals stronger interest.

---

## GH Action — bake-popularity

The recommended integration pattern for artelonga.com.br is a scheduled
GitHub Action that bakes the response into a static JSON file committed to the
content repo. This avoids any runtime API dependency on the CO platform.

Place the following workflow in `.github/workflows/bake-popularity.yml` inside
the `artelonga/ArteLonga` repository:

```yaml
name: bake-popularity
on:
  schedule:
    - cron: '0 4 * * *'   # 04:00 UTC daily
  workflow_dispatch: {}
permissions:
  contents: write
jobs:
  bake:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Fetch popularity data
        run: |
          curl -fsSL \
            "https://co.artelonga.com.br/api/v1/analytics/public/popularity?prefix=/servicos/&days=30" \
            | jq -S '.' > assets/popularity.json
      - uses: stefanzweifel/git-auto-commit-action@v5
        with:
          commit_message: "chore: bake popularity.json"
          file_pattern: assets/popularity.json
```

The workflow lives in the `artelonga/ArteLonga` repo — `co` never needs a
cross-repo write token. `jq -S '.'` sorts JSON keys for a stable diff.

The `renderer.js` change that consumes `popularity.json` is a separate PR in
the `artelonga/ArteLonga` repo (out of scope for `co`).

---

## Strip (never exposed)

The following fields from `telemetry_events` are **never included** in public
responses:

- `visitor_token`
- `ip_hash`
- `properties` (raw UTM params, experiments, etc.)
- `user_id`
- `session_id`
