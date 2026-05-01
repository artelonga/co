# CO Agent — Cloudflare Workers Tail

`co-tail` is a Cloudflare **tail Worker** that subscribes to a target Worker's log stream,
converts each log event to a CO `TelemetryEvent`, and ships batches to the CO ingest endpoint.

This is the Cloudflare-native alternative to the [Fly Machine sidecar](./fly-sidecar.md) for
deployments where a sidecar container is not available.

## Architecture

```
┌───────────────────────────────────────────────┐
│ Cloudflare Edge                               │
│                                               │
│  ┌────────────────┐   log events   ┌────────┐ │
│  │  Your Worker   │ ─────────────▶│co-tail │ │
│  │  (any Worker)  │               │        │ │
│  └────────────────┘               └───┬────┘ │
│                                       │      │
└───────────────────────────────────────│──────┘
                                        │ HTTPS POST (gzip + HMAC-SHA256)
                                        ▼
                           co.artelonga.com.br/v1/ingest/events
```

`co-tail` uses the [Cloudflare Tail Workers](https://developers.cloudflare.com/workers/observability/logs/tail-workers/)
API — no code changes to your target Worker are required.

## Wire Format

Each POST to the CO ingest endpoint carries:

| Header | Value |
|--------|-------|
| `Content-Type` | `application/x-ndjson` |
| `Content-Encoding` | `gzip` (CF Workers native; Fly sidecar uses `zstd`) |
| `X-Co-Timestamp` | Unix timestamp (seconds) |
| `X-Co-Universe` | Universe ID |
| `X-Co-Signature` | HMAC-SHA256 hex signature |

### HMAC Signature

Identical to the Fly sidecar contract:

```
message   = "<unix_timestamp>|<universe_id>|<hex(SHA256(compressed_body))>"
signature = HMAC-SHA256(CO_HMAC_KEY, message)
```

The ingest endpoint verifies the signature and rejects requests where the timestamp
is outside a ±5 minute window.

### Event Schema

Each line in the decompressed body:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": "2026-05-01T12:34:56.789Z",
  "universe_id": "your-universe-slug",
  "kind": "log",
  "payload": {
    "message": "your log line here",
    "level": "log",
    "script_name": "your-worker"
  }
}
```

## Setup

### 1. Generate an HMAC Key

```bash
openssl rand -hex 32
```

Store the output — you will need it in step 3.

### 2. Deploy co-tail

```bash
cd workers/co-tail

# Store the HMAC key as a Wrangler secret (never commit it)
wrangler secret put CO_HMAC_KEY
# Paste the hex key when prompted

# Edit wrangler.toml — set CO_INGEST_URL and CO_UNIVERSE_ID for your deployment.
# Then deploy:
wrangler deploy
```

### 3. Attach co-tail to your target Worker

In your **target Worker's** `wrangler.toml`, add:

```toml
[[tail_consumers]]
service = "co-tail"
```

Redeploy your target Worker:

```bash
wrangler deploy
```

From this point, every log event emitted by your target Worker is forwarded to co-tail
and shipped to the CO ingest endpoint.

### 4. Register the HMAC key in CO

In your universe settings, set `CO_HMAC_KEY` to the same hex key you stored as a
Wrangler secret. The ingest endpoint uses this key to verify each incoming batch.

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `CO_INGEST_URL` | ❌ | `https://co.artelonga.com.br/v1/ingest/events` | CO ingest endpoint |
| `CO_UNIVERSE_ID` | ✅ | — | Universe slug this tail Worker reports for |
| `CO_HMAC_KEY` | ✅ | — | HMAC-SHA256 key, hex-encoded (set via `wrangler secret put`) |

## Cloudflare Tail Worker Quota

Cloudflare limits tail consumers per zone. Check the
[Cloudflare limits documentation](https://developers.cloudflare.com/workers/platform/limits/)
for current quotas. If you hit the limit, you can route multiple Workers to a single
co-tail instance by deploying co-tail once and adding `[[tail_consumers]]` in each
target Worker's `wrangler.toml`.

## Troubleshooting

**No events arriving at the ingest endpoint**

1. Check that `CO_HMAC_KEY` matches the key registered in your universe settings.
2. Verify the co-tail Worker is listed as a tail consumer for your target Worker.
3. Check Wrangler logs: `wrangler tail co-tail`.

**Authentication errors (401 from ingest)**

- Regenerate the HMAC key, update both the Wrangler secret and your universe settings.

**Tail Worker not receiving events**

- Ensure `wrangler deploy` was run on the target Worker after adding `[[tail_consumers]]`.
- Check the Cloudflare dashboard → Workers → your target Worker → Logs.
