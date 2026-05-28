# CO Staging Mode — Pre-Deploy Verification

`co serve --staging` starts a local server that behaves like a constrained
production environment: storage is slow, blob ops occasionally fail, the cache
evicts under pressure, and workers fail at a low rate.

Use it to verify that your changes handle the realistic failure modes that
exist in production before you deploy.

## Quick Start

```bash
# Start with default staging settings (50ms latency, 5% error rate)
co serve --staging

# Verify latency is injected
time curl -s http://localhost:54321/api/health > /dev/null
# → real 0m0.055s  (≥50ms slower than without --staging)

# Hammer blob upload endpoint to see ~5% failures
for i in $(seq 1 100); do
  curl -s -o /dev/null -w "%{http_code}\n" -X POST \
    http://localhost:54321/api/v1/universes/template/assets \
    -H "Authorization: Bearer $TOKEN" \
    -F "file=@test.png"
done | sort | uniq -c
# Expected: ~95 "200", ~5 "503" (or 500 for simulated errors)
```

## CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--staging` | off | Enable staging simulation |
| `--staging-latency-ms` | 20 | Latency added per storage call and per HTTP request (ms) |
| `--staging-error-rate` | 0.05 | Fraction of blob ops and worker enqueues that return errors |

## Decorator Plugs

Four simulation decorators are active when `--staging` is set. Each can be
disabled individually via environment variable:

| Decorator | Env var to disable | What it simulates |
|-----------|--------------------|-------------------|
| `LatencyInjectedStorage` | `CO_STAGING_LATENCY=false` | Slow storage (adds latency to every storage call) |
| `FlakyBlobStore` | `CO_STAGING_FAULT_INJECTION=false` | Blob storage intermittent failures (503s) |
| `EvictingCache` | `CO_STAGING_EVICTION=false` | Cache pressure (random entry evictions) |
| `RetryProneWorkerExecutor` | `CO_STAGING_WORKER_FAILURE=false` | Transient job-queue failures |

### Mix-and-Match Examples

```bash
# Latency only (no fault injection)
CO_STAGING_FAULT_INJECTION=false co serve --staging

# Fault injection only (no latency, no eviction)
CO_STAGING_LATENCY=false CO_STAGING_EVICTION=false co serve --staging

# Higher error rate with custom latency
co serve --staging --staging-latency-ms 100 --staging-error-rate 0.10
```

## How Each Plug Works

### LatencyInjectedStorage

Wraps the `Storage` trait implementation and calls `std::thread::sleep(latency_ms)`
before forwarding each storage call. Because the storage trait is synchronous,
the sleep blocks the worker thread — intentionally simulating a slow database.

The same latency is also added to every HTTP request via a tower middleware
layer, so even lightweight endpoints like `/api/health` are measurably slower.

### FlakyBlobStore

Wraps any `BlobStore` implementation and fails every `ceil(1 / error_rate)`th
call with an error message containing `"staging: simulated 503"`. The counter
is shared across all method calls (`put`, `get`, `delete`, `exists`,
`presign_url`) and is deterministic — call N that is a multiple of
`fail_every_n` always fails, regardless of which method it is.

With the default `error_rate = 0.05`, this means 1 in 20 blob operations fails.

### EvictingCache

Wraps any `Cache<K, V>` implementation. On every `put`, if the call counter
hits the eviction interval, the just-inserted entry is immediately invalidated.
The cache miss then forces the caller to re-fetch from storage — exercising the
production cache-miss path.

For the server's `CacheLayer` (manifest cache, query result cache), a
background task evicts cache entries at a random rate every 5 seconds.

### RetryProneWorkerExecutor

Wraps any `WorkerExecutor`. Every `ceil(1 / error_rate)`th `enqueue` call
returns `Err` without dispatching the job. `run_now`, `cancel`, `status`, and
`statuses` are always forwarded unchanged. This forces calling code to handle
the case where a job could not be enqueued — covering the retry path.

## Not in Scope for This Release

- **OTLP export** — staging metrics are visible in the server log, not yet
  forwarded to an OpenTelemetry collector (CO-299).
- **Incident replay** — reproducing specific production failure shapes from
  logs is a separate epic.
- **TestServer** — `co serve --staging` is for manual pre-deploy verification;
  the `TestServer` abstraction for automated integration tests is CO-300.
