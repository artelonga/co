# Deploy Manifest (`deploy.yaml`)

A universe carries `deploy.yaml` at its root to declare how it should be deployed
to a target platform. CO validates the file before any platform API is called,
so deployment errors are caught early with actionable messages.

---

## Schema reference

The formal JSON Schema is at [`work/schema/deploy.v1.json`](../work/schema/deploy.v1.json).

### Top-level fields

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `version` | **yes** | `integer` | Must be `1` |
| `target` | **yes** | enum | Deployment target (see below) |
| `domain` | no | string | Custom domain; deployer chooses default if absent |
| `runtime` | no | object | Runtime and build configuration |
| `bindings` | no | object | Storage and secret bindings |
| `scaling` | no | object | Auto-scaling min/max |
| `telemetry` | no | object | Observability sink and sampling rate |
| `backup` | no | object | Backup schedule and retention |

### `target` values

| Value | Platform |
|-------|----------|
| `static-on-r2` | Cloudflare R2 static hosting |
| `cloudflare-pages` | Cloudflare Pages |
| `fly` | Fly.io |
| `vercel` | Vercel |
| `fargate` | AWS Fargate |

### `runtime`

```yaml
runtime:
  kind: static        # static | node | rust | python | wasm
  build:
    command: co build  # optional; default: co build
    output: dist/      # optional; default: dist/
```

### `bindings`

```yaml
bindings:
  storage:
    type: r2           # r2 | s3 | gcs
    bucket: my-bucket
    encrypted: true    # default: false
  secrets:
    - STRIPE_KEY       # name only; value resolved from CO secrets
```

### `scaling`

```yaml
scaling:
  min: 0   # >= 0; default: 0
  max: 10  # >= min
```

### `telemetry`

```yaml
telemetry:
  sink: co-central   # co-central | none
  sampling: 1.0      # 0.0 to 1.0; default: 1.0
```

### `backup`

```yaml
backup:
  schedule: daily    # hourly | daily | weekly | none
  retention: 30d     # positive integer + d/h/w/m (e.g. 30d, 24h, 1w)
```

---

## Full example per target

### `static-on-r2`

Serves a statically-built universe from Cloudflare R2.

```yaml
version: 1
target: static-on-r2
domain: meu-portfolio.co.app
runtime:
  kind: static
  build:
    command: co build
    output: dist/
bindings:
  storage:
    type: r2
    bucket: u-meu-portfolio
    encrypted: true
  secrets:
    - CO_API_TOKEN
telemetry:
  sink: co-central
  sampling: 1.0
backup:
  schedule: weekly
  retention: 30d
```

---

### `cloudflare-pages`

Full-stack deployment on Cloudflare Pages with Workers.

```yaml
version: 1
target: cloudflare-pages
domain: blog.example.co.app
runtime:
  kind: static
  build:
    command: co build --mode pages
    output: dist/
bindings:
  storage:
    type: r2
    bucket: u-blog-assets
    encrypted: false
  secrets:
    - STRIPE_KEY
    - SENDGRID_API_KEY
telemetry:
  sink: co-central
  sampling: 0.5
backup:
  schedule: daily
  retention: 14d
```

---

### `fly`

Containerized Rust service on Fly.io.

```yaml
version: 1
target: fly
domain: api.myapp.fly.dev
runtime:
  kind: rust
  build:
    command: cargo build --release
    output: target/release/
bindings:
  storage:
    type: s3
    bucket: myapp-data
    encrypted: true
  secrets:
    - DATABASE_URL
    - JWT_SECRET
scaling:
  min: 1
  max: 10
telemetry:
  sink: co-central
  sampling: 0.1
backup:
  schedule: hourly
  retention: 7d
```

---

### `vercel`

Next.js / Node.js deployment on Vercel.

```yaml
version: 1
target: vercel
domain: my-app.vercel.app
runtime:
  kind: node
  build:
    command: npm run build
    output: .next
bindings:
  secrets:
    - NEXT_PUBLIC_API_URL
    - DATABASE_URL
    - NEXTAUTH_SECRET
telemetry:
  sink: none
backup:
  schedule: daily
  retention: 30d
```

---

### `fargate`

Long-running container on AWS Fargate.

```yaml
version: 1
target: fargate
domain: worker.internal.example.com
runtime:
  kind: python
  build:
    command: pip install -r requirements.txt
    output: .
bindings:
  storage:
    type: s3
    bucket: fargate-state
    encrypted: true
  secrets:
    - AWS_ACCESS_KEY_ID
    - AWS_SECRET_ACCESS_KEY
    - REDIS_URL
scaling:
  min: 2
  max: 50
telemetry:
  sink: co-central
  sampling: 0.25
backup:
  schedule: daily
  retention: 90d
```

---

## CLI usage

```bash
# Validate deploy.yaml in the current directory
co validate deploy

# Validate a specific file
co validate deploy path/to/deploy.yaml
```

Validation errors include the file path, line number (for YAML parse errors),
and the field path (for semantic errors):

```
error: deploy.yaml: field 'scaling.max': must be >= scaling.min (50) but got 10
error: deploy.yaml: YAML parse error: line 3, column 9: unknown variant `unknown-platform`
```

---

## Versioning

The `version` field is strictly validated. CO 2.x supports only `version: 1`.
When v2 is introduced it will require a migration tool; breaking changes will be
announced in `CHANGELOG.md`.
