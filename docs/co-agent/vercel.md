# CO Agent — Vercel Log Drain

CO provides a **Log Drain receiver** at `POST /v1/log-drains/vercel/{universe_id}` that
accepts Vercel's NDJSON log drain format, validates the Vercel HMAC-SHA1 signature, and
stores the events in CO's telemetry store with idempotent deduplication.

This is the Vercel-native alternative to the [Fly Machine sidecar](./fly-sidecar.md) for
deployments where a sidecar container is not available.

## Architecture

```
┌──────────────────────────────────────┐
│ Vercel                               │
│                                      │
│  ┌──────────────┐   NDJSON stream    │
│  │  Your App    │ ──────────────┐    │
│  │  (any Vercel │               │    │
│  │   project)   │               ▼    │
│  └──────────────┘  Log Drain webhook │
└──────────────────────────────────────┘
                           │ HTTPS POST (HMAC-SHA1 signed)
                           ▼
          co.artelonga.com.br/v1/log-drains/vercel/{universe_id}
                           │
                           ▼ validated + stored
                     log_drain_events (SQLite)
```

## Wire Protocol

Vercel sends NDJSON log entries as a POST to the configured drain URL and signs the
request with `HMAC-SHA1(secret, body)`, provided as the `x-vercel-signature` header
in the form `sha1=<hex>`.

CO validates the signature using the per-universe drain secret before processing any events.

### Vercel Log Entry Format

```json
{
  "id": "log_123abc",
  "message": "Function started",
  "source": "lambda",
  "level": "info",
  "host": "app.vercel.app",
  "path": "/api/hello",
  "timestamp": 1746100000000
}
```

Fields `source`, `level`, `host`, and `path` default to empty strings when absent.

## Setup

### 1. Configure the Drain Secret in CO

Set a per-universe drain secret via the CO admin API (or directly in your universe settings):

```bash
# Generate a secret
DRAIN_SECRET=$(openssl rand -hex 32)
echo $DRAIN_SECRET

# Set via the CO API (replace with your universe slug and admin token)
curl -X PUT https://co.artelonga.com.br/api/v1/universes/{universe_id}/log-drain-secret \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"secret\": \"$DRAIN_SECRET\"}"
```

### 2. Add a Log Drain in Vercel

In the Vercel dashboard:

1. Go to **Settings** → **Log Drains** → **Add Log Drain**
2. Set the **Endpoint URL** to:
   ```
   https://co.artelonga.com.br/v1/log-drains/vercel/{your-universe-slug}
   ```
3. Set the **Secret** to the value generated in step 1.
4. Select the log sources you want to forward (build, edge, lambda, static).
5. Save.

From this point, Vercel sends each log batch to the CO drain endpoint.

## Deduplication

Vercel delivers logs **at least once**. CO deduplicates events by `event_id` (Vercel's
`id` field) using `INSERT OR IGNORE` — duplicate deliveries are silently discarded.
Events without an `id` field receive a server-generated UUID.

## Endpoint Reference

```
POST /v1/log-drains/vercel/{universe_id}
```

| Header | Description |
|--------|-------------|
| `x-vercel-signature` | `sha1=<HMAC-SHA1 hex>` — required |
| `Content-Type` | `application/x-ndjson` |

**Response codes:**

| Code | Meaning |
|------|---------|
| `200 OK` | Events accepted (or body was empty) |
| `401 Unauthorized` | Signature validation failed |
| `404 Not Found` | Universe not found or drain secret not configured |
| `500 Internal Server Error` | Storage error |

## Troubleshooting

**401 Unauthorized**

- Verify the drain secret in Vercel matches the secret stored in CO for the universe.
- Vercel computes `HMAC-SHA1(secret, raw_body)` — ensure no middleware modifies the body
  before it reaches the CO endpoint.

**404 Not Found**

- Check that the universe slug in the URL exactly matches the universe key in CO.
- Ensure a drain secret has been configured for the universe (an empty secret returns 404).

**Events not appearing**

- Check Vercel's Log Drains dashboard for delivery status and any retry errors.
- Verify the CO endpoint is reachable from Vercel's servers.
- Enable debug logging on the CO server: `RUST_LOG=co_web=debug`.
