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
