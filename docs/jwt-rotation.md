# JWT Secret Rotation Runbook

`JWT_SECRET` is the HMAC-SHA256 key used to sign all session cookies (HS256).
This document describes how to rotate it safely with minimal disruption.

> **Note:** Rotating `JWT_SECRET` invalidates every active session. All users
> will be logged out and must authenticate again.

## When to rotate

- **Quarterly** — proactive schedule (align with VAPID rotation).
- **Immediately** on suspected compromise: leaked secret, unusual login
  patterns in Fly logs, or a staging security incident.
- **After a staging incident** — staging shares the same secret; a staging
  breach can yield tokens valid on prod.

## Rotation steps

### 1. Generate a new secret

```bash
NEW_SECRET=$(openssl rand -base64 48)
printf '%s\n' "$NEW_SECRET"   # copy to clipboard; do not store in shell history
```

### 2. Set on prod

```bash
flyctl secrets set JWT_SECRET="$NEW_SECRET" -a co-artelonga
```

Fly.io restarts the app automatically. Allow ~30 s for the health check.

### 3. Set on staging (shared-secret mode)

Skip this step if staging has migrated to OIDC federation (CO-377-B).

```bash
flyctl secrets set JWT_SECRET="$NEW_SECRET" -a co-artelonga-staging
```

### 4. Verify

```bash
# Prod health
curl -s https://co-artelonga.fly.dev/api/health
# → {"status":"ok"}

# New session works on prod
curl -s -X POST https://co-artelonga.fly.dev/api/v1/auth/password-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"yuri@artelonga.com.br","password":"<your-password>"}' \
  | python3 -m json.tool
# → 200 with user object

# Staging health (if applicable)
curl -s https://staging.co.artelonga.com.br/api/health
# → {"status":"ok","env":"staging"}
```

### 5. Cross-env re-sync verification

After rotating, re-run the cross-env token tests from
[`docs/cross-env-auth.md`](cross-env-auth.md) to confirm bi-directional
token acceptance.

### 6. Record the rotation

Append a row to the rotation log below.

---

## Rotation log

| Date | Rotated by | Environments | Reason |
|---|---|---|---|
| _(first rotation pending)_ | — | prod + staging | Initial secrets sync (CO-377) |

---

## Impact reference

| Component | Impact |
|---|---|
| Web session cookies | Invalidated — users logged out |
| API Bearer tokens (`/api/v1/auth/token`) | Invalidated |
| Handover tokens (ES256 via JWKS) | **Not affected** — use a separate EC key pair |
| Playwright test tokens (if using prod JWT in CI) | Re-issue before next test run |

## Emergency procedure (on-call)

If the secret is actively compromised:

1. Execute steps 1–5 above immediately.
2. Review recent logins:
   ```bash
   flyctl logs -a co-artelonga --no-tail | grep -i "login\|auth"
   ```
3. Check `activity_log` in SQLite for actions taken under suspect sessions:
   ```bash
   flyctl ssh console -a co-artelonga -C \
     "sqlite3 /data/co.db 'SELECT actor_id, action, created_at FROM activity_log ORDER BY created_at DESC LIMIT 50'"
   ```
4. If a specific session was stolen, there is no per-session revocation today
   (TO BE: token blocklist in CO-N). Rotation of `JWT_SECRET` is the only
   current remedy and logs out all users.
