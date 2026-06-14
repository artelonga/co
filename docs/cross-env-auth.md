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

## Token capability scopes (CO-448 — least-privilege API tokens)

Until CO-448 an `api_tokens` row had **no scope column**: a token inherited the
owner's *tier* (all-or-nothing). That is why the CO-401 staging-suite token had
to be **admin-pleno** — a leaked secret would have granted full admin. CO-448
adds a **least-privilege**, **hybrid** model: an issuer requests **exactly** the
capabilities a token needs.

### Model — capabilities + named bundles

- **Capability** — a `recurso:ação` string, e.g. `entries:read`, `chat:write`.
- **Bundle** — a named preset that expands to a set of capabilities. Issuers may
  pass either raw capabilities, bundle names, or a mix; the request is resolved
  and **expanded at issuance**, and the expanded set is what is persisted in
  `api_tokens.scopes` (a JSON array), so a token is **auditable** — you can read
  exactly what it can do.

| Bundle | Expands to |
|---|---|
| `read`  | `entries:read`, `universes:read`, `telemetry:read` |
| `write` | `read` ∪ `entries:write`, `universes:write` |
| `admin` | `write` ∪ `gestao:read`, `funnel:read`, `chat:read`, `chat:write`, `deployments:read` |
| `agent` | `write` ∪ `agent:dispatch` |

Initial capability vocabulary (extensible — new surfaces add capabilities
additively, e.g. the CO-288 cost panel maps to `deployments:read`):

`entries:read`, `universes:read`, `telemetry:read`, `gestao:read`,
`funnel:read`, `chat:read`, `deployments:read`, `entries:write`,
`universes:write`, `chat:write`, `agent:dispatch`.

### Issuing a scoped token

```bash
# Least-privilege: only what the staging suite (CO-374) exercises.
curl -X POST https://co.artelonga.com.br/api/v1/auth/token \
  -H "Authorization: Bearer $JWT" -H 'Content-Type: application/json' \
  -d '{"name":"staging-suite","scopes":["chat:read","telemetry:read"]}'
# → { id, name, token, expires_at, scopes: ["chat:read","telemetry:read"] }
```

An unknown capability or bundle name is rejected with `400 Bad Request` (a typo
fails loudly rather than minting a misleading token). Omitting `scopes` (or an
empty list) issues a **NULL-scope** token — see compatibility below.

### Enforcement

Each protected endpoint declares the capability it requires via the
`Scoped<C>` extractor (`co-web/src/auth/extractors.rs`). The gate resolves the
caller, and:

- **API token, scopes present** — admitted iff the resolved set contains the
  required capability; otherwise **403**. No escalation — a token never gains a
  capability outside its persisted set.
- **API token, scopes NULL** — inherits the owner's tier (pre-CO-448 behavior).
  On an admin surface the owner must still be an admin.
- **JWT / session** — a full user session; capabilities only restrict tokens
  (an admin surface still requires `tier == "admin"`).

| Endpoint | Required capability |
|---|---|
| `GET /api/v1/admin/chat/origin-breakdown` | `chat:read` (admin surface) |

(The table grows as more surfaces adopt `Scoped<C>`; the public API CO-278 maps
each route to its capability as it lands.)

### Compatibility

Tokens issued before CO-448 have `scopes = NULL` and are **unchanged** — they
keep inheriting the owner's tier (vault sync, co-auto reporting, …). Only tokens
issued *with* a `scopes` value are capability-restricted. A leaked scoped secret
grants only that token's set, never owner-tier admin.

## `CO_STAGING_ADMIN_TOKEN` — the staging-suite token (CO-401)

The CO-374 deep staging suite authenticates with a single secret,
`CO_STAGING_ADMIN_TOKEN`. Per CO-401 it is a **CO-448 capability-scoped** token
(NOT an admin-tier NULL-scope token): a leaked CI secret grants only the suite's
capabilities, never full admin.

### How it works (seed-on-boot, no manual minting)

The secret is **self-contained**: its raw value is the token. On every staging
boot (`CO_ENV=staging`), `seed_orchestrator::seed_staging_fixtures` →
`seed_staging_admin_token`:

1. ensures the admin-tier `staging-admin` user exists,
2. registers an `api_tokens` row whose `token_hash = SHA-256(CO_STAGING_ADMIN_TOKEN)`
   with the least-privilege scope set below.

So the **same value** lives in two places — the Fly secret on
`co-artelonga-staging` (so the token validates) and the GitHub Actions secret
(so the suite sends it). Nothing is minted via the API; rotation is just
"set a new value in both places".

Scope set (`STAGING_SUITE_CAPABILITIES`, least-privilege — the admin *reads* the
suite touches plus entry/universe read+write, but **not** `chat:write`,
`deployments:read`, or `agent:dispatch`):

```
entries:read  entries:write  universes:read  universes:write
gestao:read   funnel:read    chat:read       telemetry:read
```

### Setup / rotation runbook

```bash
# 1. Generate a fresh opaque secret (the `co_` prefix matches issued tokens).
TOKEN="co_$(openssl rand -hex 32)"

# 2. Push it to the staging app — the boot seeder registers it (scoped) on
#    restart, retiring any previous staging-suite token automatically.
flyctl secrets set CO_STAGING_ADMIN_TOKEN="$TOKEN" -a co-artelonga-staging

# 3. Mirror it into GitHub Actions so the suite sends the same value.
gh secret set CO_STAGING_ADMIN_TOKEN --body "$TOKEN" --repo institutional-pointset/co

# 4. Verify — a scoped read succeeds, an out-of-scope write is 403.
curl -fs -H "Authorization: Bearer $TOKEN" \
  https://staging.co.artelonga.com.br/api/v1/universes/recursion-a | python3 -m json.tool
```

Seeding is idempotent and rotation-safe: re-setting the same value is a no-op;
setting a **new** value purges the old `staging-admin` token so only the live
secret remains.

| Secret | Frequency | Runbook |
|---|---|---|
| `CO_STAGING_ADMIN_TOKEN` | Quarterly or on suspected compromise | Steps 1–4 above (Fly + GitHub, then restart staging) |

> **Why a scoped token and not the admin password?** The earlier draft (held)
> shared the admin password / an admin-tier NULL-scope token with CI — a leak
> meant full admin. The scoped token is least-privilege: a leak grants only the
> capabilities above, and the staging app only ever holds fixture data.
