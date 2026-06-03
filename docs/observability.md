# Observability — CO-291

CO emits structured logs to stderr by default.  Set `CO_TELEMETRY_OTLP_ENDPOINT`
to export OpenTelemetry traces to any OTLP-compatible collector (Jaeger, Grafana
Cloud, Honeycomb, etc.).

## Local Jaeger quickstart

### 1. Start Jaeger

```bash
docker run -d \
  --name jaeger \
  -p 16686:16686 \
  -p 4317:4317 \
  jaegertracing/all-in-one:latest
```

- **Port 4317** — OTLP gRPC (what CO-web sends to)
- **Port 16686** — Jaeger UI

### 2. Run co-web with OTLP enabled

```bash
CO_TELEMETRY_OTLP_ENDPOINT=http://localhost:4317 cargo run -p co-web
```

You should see this log line on startup:

```
INFO co_web::infra::telemetry: OTLP telemetry enabled endpoint=http://localhost:4317 service=co-web sampling=1
```

### 3. Open the Jaeger UI

```
http://localhost:16686
```

Select **Service → co-web**, then click **Find Traces**.  Within 10 seconds of
sending an HTTP request, you should see a trace with:

- A root span per HTTP request (from `TraceLayer::new_for_http()`)
- Child `db.query` spans for SQLite entry reads (`get_entry`, `list_entries`,
  `search_entries`)

## Environment variables

| Variable                       | Default  | Description                                      |
|-------------------------------|----------|--------------------------------------------------|
| `CO_TELEMETRY_OTLP_ENDPOINT`  | —        | OTLP gRPC endpoint.  Enables OTLP when set.     |
| `CO_TELEMETRY_SERVICE_NAME`   | `co-web` | Service name shown in the collector UI.          |
| `CO_TELEMETRY_SAMPLING_RATIO` | `1.0`    | Fraction of traces to sample.  Range: 0.0–1.0.  |
| `RUST_LOG`                    | —        | Filter directive (e.g. `co_web=debug`).          |

When `CO_TELEMETRY_OTLP_ENDPOINT` is **not** set, behavior is identical to
before: spans go to stderr via the standard `tracing-subscriber` fmt layer.

## Staging / production

- **Honeycomb**: set `CO_TELEMETRY_OTLP_ENDPOINT=https://api.honeycomb.io`
  and `CO_TELEMETRY_SERVICE_NAME=co-web-prod`.  Add your API key as a request
  header (via OTLP headers env var — see Honeycomb docs for the exact flag).
- **Grafana Cloud**: use the Grafana Cloud OTLP endpoint for your stack.
- **Self-hosted Collector**: point at an OpenTelemetry Collector that forwards
  to your preferred backend.

Setting up cloud backends is out of scope for CO-291 (ops decision when needed).

## Architecture

```
HTTP request
  └─ TraceLayer span (tower-http)      — root span, every request
       └─ db.query span (infra::storage) — child spans for SQLite reads
```

The `infra::telemetry::db_span()` helper creates additional child spans around
arbitrary storage calls.  See the function doc-comment for usage.
