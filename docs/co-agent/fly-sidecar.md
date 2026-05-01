# CO Agent — Fly Machine Sidecar

`co-agent` is a tiny Rust sidecar that tails your application's log stream, batches events into zstd-compressed JSON-Lines payloads, signs each batch with HMAC-SHA256, and POSTs it to the CO ingest endpoint.

## Architecture

```
┌──────────────────────────────────────────────────────┐
│ Fly Machine                                          │
│                                                      │
│  ┌──────────┐   stdout/stderr   ┌─────────────────┐ │
│  │  Your    │ ─────────────────▶│   co-agent      │ │
│  │  App     │                   │                 │ │
│  └──────────┘                   │  • ring buffer  │ │
│                                 │  • zstd + HMAC  │ │
│                                 │  • retry logic  │ │
│                                 └────────┬────────┘ │
└──────────────────────────────────────────│───────────┘
                                           │ HTTPS POST (batched)
                                           ▼
                              ingest.co.artelonga.com.br
```

The sidecar reads from **stdin** by default. Pipe your app's output into it:

```
your-app 2>&1 | co-agent
```

On a Fly Machine you can configure two processes in `fly.toml` and pipe one into the other, or use a shell wrapper as the entrypoint.

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `CO_UNIVERSE_ID` | ✅ | — | Universe this sidecar reports for |
| `CO_HMAC_KEY` | ✅ | — | HMAC-SHA256 key, hex-encoded (32 bytes) |
| `CO_INGEST_URL` | ❌ | `https://ingest.co.artelonga.com.br/v1/events` | Ingest endpoint |
| `CO_BATCH_SIZE` | ❌ | `200` | Events per batch before auto-flush |
| `CO_FLUSH_INTERVAL` | ❌ | `10` | Seconds between time-based flushes |
| `CO_MAX_RETRIES` | ❌ | `3` | Total POST attempts before dropping a batch |

## Generate an HMAC Key

```bash
# Generate and store locally
HMAC_KEY=$(openssl rand -hex 32)
echo $HMAC_KEY

# Set as a Fly secret
flyctl secrets set CO_HMAC_KEY="$HMAC_KEY" -a your-app
```

The ingest endpoint uses the same key to verify each incoming batch.

## Docker Image

```
ghcr.io/artelonga/co/co-agent:0.1.0
```

Pull and test locally:

```bash
docker pull ghcr.io/artelonga/co/co-agent:0.1.0

echo '{"message":"hello"}' | docker run --rm -i \
  -e CO_UNIVERSE_ID=my-universe \
  -e CO_HMAC_KEY=$(openssl rand -hex 32) \
  ghcr.io/artelonga/co/co-agent:0.1.0
```

## Adding co-agent as a Fly Machine Sidecar

Fly Machines support multiple processes via the `[processes]` table.  Add a
`sidecar` process that pipes your app's output into `co-agent`:

```toml
# fly.toml — add co-agent sidecar alongside your main app

app = "your-app"
primary_region = "gru"

[build]
  dockerfile = "Dockerfile"

# ── Processes ────────────────────────────────────────────────────────────────
[processes]
  # Main application process
  app = "/bin/sh -c '/app/your-binary 2>&1 | /usr/local/bin/co-agent'"

[env]
  # co-agent reads these at startup
  CO_UNIVERSE_ID    = "your-universe-slug"
  CO_BATCH_SIZE     = "200"
  CO_FLUSH_INTERVAL = "10"
  CO_MAX_RETRIES    = "3"

# CO_HMAC_KEY is a secret — set it with:
#   flyctl secrets set CO_HMAC_KEY=$(openssl rand -hex 32) -a your-app
```

If your `Dockerfile` already has `co-agent` installed, the above is all you
need.  To install it into an existing image, add a `COPY` from the published
image:

```dockerfile
# In your Dockerfile — copy the sidecar binary from the published image
COPY --from=ghcr.io/artelonga/co/co-agent:0.1.0 \
     /usr/local/bin/co-agent \
     /usr/local/bin/co-agent
```

### Alternative: Separate sidecar process via Fly Machines API

If you prefer a fully separate Machine rather than a combined process, create
a second Machine in the same app using the Fly Machines API and mount a shared
volume or named pipe between them.

## Payload Format

Each POST carries:

| Header | Value |
|--------|-------|
| `Content-Type` | `application/x-ndjson` |
| `Content-Encoding` | `zstd` |
| `X-Co-Timestamp` | Unix timestamp (seconds) |
| `X-Co-Universe` | Universe ID |
| `X-Co-Signature` | HMAC-SHA256 hex signature |

### HMAC Signature

The signature covers:

```
message = "<unix_timestamp>|<universe_id>|<hex(SHA256(compressed_body))>"
signature = HMAC-SHA256(key, message)
```

The ingest endpoint:
1. Checks `X-Co-Timestamp` is within ±5 minutes of server time (rejects clock-skewed requests).
2. Recomputes the signature and rejects mismatches with `401`.

### Event Schema

Each line in the decompressed body is a JSON object:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": "2026-05-01T12:34:56.789Z",
  "universe_id": "your-universe-slug",
  "kind": "log",
  "payload": { "message": "your log line here" }
}
```

`kind` is one of: `log`, `metric`, `trace`.

## Overflow & Reliability

| Behaviour | Policy |
|-----------|--------|
| Ring buffer full | Drop **oldest** event; increment `co_agent_dropped_events_total` |
| 5xx from ingest | Exponential backoff + jitter, up to `CO_MAX_RETRIES` attempts, then drop |
| 4xx from ingest | Immediate log + drop (bad payload, no point retrying) |
| Heartbeat | Every 60 s — logs `dropped_events_total` to stderr |
| SIGTERM / SIGINT | Flush remaining buffer, then exit |

Monitor `co_agent_dropped_events_total` in your alerting system.  A non-zero
value indicates backpressure — consider increasing `CO_BATCH_SIZE` or
`CO_FLUSH_INTERVAL`.

## Troubleshooting

**No events arriving at the ingest endpoint**

1. Check `CO_HMAC_KEY` is set and matches the key registered for your universe.
2. Verify the machine clock is within ±5 min of UTC (`date -u`).
3. Enable debug logging: set `RUST_LOG=co_agent=debug`.

**High dropped-event count**

- Increase `CO_RING_BUFFER_CAP` (env var, default 10 000).
- Increase `CO_BATCH_SIZE` to reduce POST frequency per event.
- Check ingest endpoint latency — if it is slow, the buffer fills faster.

**Authentication errors (401)**

- Regenerate the HMAC key and update both the Fly secret and the ingest configuration.

## Extending co-agent

`co-agent` is one implementation of the `CoAgent` trait defined in `co-core`:

```rust
pub trait CoAgent: Send + Sync {
    fn ship(&self, events: &[TelemetryEvent]) -> impl Future<Output = Result<()>> + Send;
    fn heartbeat(&self) -> impl Future<Output = Result<()>> + Send;
    fn flush(&self) -> impl Future<Output = Result<()>> + Send;
}
```

Future adapters (CO-124):
- **CF Worker tail** — receives Cloudflare Tail Worker events
- **Vercel Log Drain** — handles Vercel Log Drain webhook payloads
- **Browser SDK beacon** — Phase 2 browser-side beacon
