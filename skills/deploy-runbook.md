# Deploy Runbook — CO Platform

## Environments

| Env | App | Config |
|-----|-----|--------|
| Production | `co-artelonga` | `fly.toml` |

No dedicated UAT environment — deploy direct to prod with smoke test (CO-feedback).

## Deploy Flow

```bash
# 1. Run tests
cargo test -p co-web
cargo clippy -p co-web -- -D warnings

# 2. Deploy
flyctl deploy

# 3. Smoke test
curl -s https://co-artelonga.fly.dev/api/health
```

## Smoke Test

`scripts/smoke-prod.sh` runs post-deploy checks:
- `GET /api/health` → `{"status":"ok"}`
- Template universe accessible
- Login endpoint responds

## Fly.io Config Files

| File | Purpose |
|------|---------|
| `fly.toml` | Production deployment |
| `Dockerfile` | Build image (`rust:1.88-slim` → `debian:bookworm-slim`) |
| `litefs.yml` | LiteFS SQLite replication config |

## Secrets

```bash
# Set JWT secret (one-time per env)
flyctl secrets set JWT_SECRET=$(openssl rand -base64 48) -a co-artelonga

# Seed admin user
flyctl secrets set CO_SEED_ADMIN_EMAIL=... CO_SEED_ADMIN_PASSWORD_HASH=... -a co-artelonga
```

## Rollback

CHANGELOG.md tracks the last stable version. To roll back:
```bash
git checkout v{PREV_VERSION}
flyctl deploy
```

## Logs

```bash
flyctl logs -a co-artelonga          # Live stream
flyctl logs -a co-artelonga --no-tail  # Recent only
flyctl ssh console -a co-artelonga   # Shell
```
