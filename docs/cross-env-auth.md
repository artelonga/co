# Cross-Environment Identity — Shared Admin + JWT

Phase 1 establishes a shared `JWT_SECRET` and admin credentials between
production and staging so that tokens issued in one environment validate
in the other. Phase 2 (deferred to CO-377-B) eliminates the shared-secret
risk via OIDC federation.

## Phase 1 — Shared secrets

### Prerequisites

1. Staging app (`co-artelonga-staging`) deployed and healthy at
   `staging.co.artelonga.com.br`.
2. `flyctl` authenticated with permissions to both apps.
3. SSH access to the prod machine (for secret extraction).

### Secrets sync runbook

Run once after staging is provisioned. Re-run after any `JWT_SECRET` rotation.

```bash
# 1. Extract secret values from the running prod machine
#    (flyctl secrets list shows digests only, not values)
JWT_SECRET=$(flyctl ssh console -a co-artelonga -C "printenv JWT_SECRET")
ADMIN_HASH=$(flyctl ssh console -a co-artelonga -C "printenv CO_SEED_ADMIN_PASSWORD_HASH")
GCP_ID=$(flyctl ssh console -a co-artelonga -C "printenv GOOGLE_CLIENT_ID")
GCP_SECRET=$(flyctl ssh console -a co-artelonga -C "printenv GOOGLE_CLIENT_SECRET")

# 2. Push to staging
flyctl secrets set \
  JWT_SECRET="$JWT_SECRET" \
  CO_SEED_ADMIN_EMAIL="yuri@artelonga.com.br" \
  CO_SEED_ADMIN_PASSWORD_HASH="$ADMIN_HASH" \
  GOOGLE_CLIENT_ID="$GCP_ID" \
  GOOGLE_CLIENT_SECRET="$GCP_SECRET" \
  -a co-artelonga-staging
```

Fly.io restarts staging automatically after `secrets set`. Wait ~30 s for
the health check to go green before proceeding to verification.

### Google OAuth callback URL

Both environments share the same Google OAuth client. You only need to add
the staging redirect URI once.

1. Open [Google Cloud Console](https://console.cloud.google.com/) →
   APIs & Services → Credentials.
2. Edit the OAuth 2.0 client ID used by `co-artelonga`.
3. Under **Authorized redirect URIs**, add:
   ```
   https://staging.co.artelonga.com.br/api/v1/auth/google/callback
   ```
4. Save.

No new client ID or secret is needed.

### Verification

```bash
# Health checks
curl -s https://co-artelonga.fly.dev/api/health
# → {"status":"ok","env":"production","version":"..."}

curl -s https://staging.co.artelonga.com.br/api/health
# → {"status":"ok","env":"staging","version":"..."}

# ── Test 1: prod token accepted on staging ──────────────────────────────────

# Login on prod (password-login; works in any env)
curl -sc /tmp/prod.txt -X POST https://co-artelonga.fly.dev/api/v1/auth/password-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"yuri@artelonga.com.br","password":"<your-password>"}' \
  | python3 -m json.tool
SESSION_PROD=$(awk '/session/{print $NF}' /tmp/prod.txt)

# Use prod session on staging — must return 200
curl -fs -b "session=$SESSION_PROD" \
  https://staging.co.artelonga.com.br/api/v1/me \
  | python3 -m json.tool
# → {"user_id":"...","email":"yuri@artelonga.com.br",...}

# ── Test 2: staging token accepted on prod (bi-directional) ─────────────────

curl -sc /tmp/staging.txt -X POST https://staging.co.artelonga.com.br/api/v1/auth/password-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"yuri@artelonga.com.br","password":"<your-password>"}' \
  | python3 -m json.tool
SESSION_STAGING=$(awk '/session/{print $NF}' /tmp/staging.txt)

curl -fs -b "session=$SESSION_STAGING" \
  https://co-artelonga.fly.dev/api/v1/me \
  | python3 -m json.tool
# → {"user_id":"...","email":"yuri@artelonga.com.br",...}
```

Both tests passing confirms:
- Shared `JWT_SECRET` is active on both apps.
- Admin password hash is identical in both seeds.
- Cross-env token validation works bi-directionally.

## `CO_STAGING_ADMIN_TOKEN` — scoped suite credential (CO-401)

The deep staging suite (CO-374) authenticates as an admin to exercise scenarios
that anonymous requests can't reach. Rather than ship a password into CI, a
**scoped API token** bridges the two sides:

- **Server side** — on every staging boot, the seeder
  (`Storage::seed_staging_admin_token`, gated on `CO_ENV=staging`) reads the
  `CO_STAGING_ADMIN_TOKEN` Fly secret and installs its SHA-256 hash as an
  `api_tokens` row owned by the dedicated `staging-admin` user (tier `admin`).
  Only the hash is stored — never the plaintext. Idempotent across boots.
- **CI side** — the same value is a GitHub Actions secret. `staging-suite.yml`
  passes it to Playwright as `CO_STAGING_ADMIN_TOKEN`; the suite sends it as
  `Authorization: Bearer <token>`. The Bearer header hashes to the row the
  seeder installed, so the request authenticates as `staging-admin`.

The token is scoped to the synthetic `staging-admin` user and only ever exists
in the staging environment (the boot gate refuses to seed it elsewhere). It is
**not** a prod credential and never validates against production data.

### Generate / rotate

```bash
# 1. Mint a fresh token value (must start with co_ to match the token convention)
NEW="co_$(openssl rand -hex 32)"

# 2. Set it on the staging Fly app (triggers a restart → seeder installs the hash)
flyctl secrets set CO_STAGING_ADMIN_TOKEN="$NEW" -a co-artelonga-staging

# 3. Mirror the SAME value into the GitHub Actions secret used by the suite
gh secret set CO_STAGING_ADMIN_TOKEN -b "$NEW" --repo institutional-pointset/co

# 4. (optional) Revoke the previous token row once the new one is live
flyctl ssh console -a co-artelonga-staging \
  -C "sqlite3 /data/co.db \"DELETE FROM api_tokens WHERE name='CO_STAGING_ADMIN_TOKEN' AND token_prefix <> substr('$NEW',1,11)\""
```

Both sides must hold the identical value — a drift between the Fly secret and
the GitHub secret makes the suite fall back to anonymous-only coverage (the
authed tests skip rather than fail).

## Risk model

| Risk | Mitigation |
|---|---|
| Staging compromise → forge prod tokens | Quarterly `JWT_SECRET` rotation across both apps |
| Shared admin password leak | Strong password; review login patterns in Fly logs |
| OAuth callback URL spoofing | Pinned redirect URI enforced by Google |
| Staging logs expose prod data | Staging uses fixture data only; no prod data seeded |

## Phase 2 — OIDC federation (CO-377-B, deferred to post-v3.0)

Phase 2 eliminates the shared-secret risk. Prod acts as an OIDC issuer;
staging validates tokens against prod's JWKS endpoint instead of a shared
HS256 secret.

### Architecture

```
Prod issues JWT (ES256)
  └─ signed with prod's EC private key
  └─ JWKS published at https://co.artelonga.com.br/.well-known/jwks.json

Staging validates JWT
  └─ fetches JWKS from prod on first use (cached)
  └─ verifies signature against prod's EC public key
  └─ no shared JWT_SECRET; staging cannot forge prod tokens
```

### Required work

| Step | Detail |
|---|---|
| `GET /.well-known/jwks.json` | Expose prod's ES256 public key. Partial: CO-211 handover uses ES256; JWKS endpoint exists but only serves the handover key. |
| `POST /api/v1/auth/exchange-session` | Prod endpoint that reissues a short-lived ES256 token for cross-env auth. |
| Staging `AuthMiddleware` | Try JWKS validation first; fall back to HS256 for staging-issued tokens. |
| Remove shared secret | Remove `JWT_SECRET` from staging secrets once JWKS validation is live. |

### Timeline

Defer until after v3.0 public launch. Re-evaluate when per-env role
separation becomes a requirement (e.g., admin on prod but read-only on
staging).

## Rotation schedule

| Secret | Frequency | Runbook |
|---|---|---|
| `JWT_SECRET` | Quarterly or on suspected compromise | [`docs/jwt-rotation.md`](jwt-rotation.md) |
| `CO_SEED_ADMIN_PASSWORD_HASH` | On password change | Re-hash + update both apps |
| `GOOGLE_CLIENT_SECRET` | Annually or on compromise | Google Console + update both apps |
| `CO_STAGING_ADMIN_TOKEN` | Quarterly or on suspected compromise | Regenerate + set Fly **and** GitHub secret (see [generate / rotate](#generate--rotate)) |
