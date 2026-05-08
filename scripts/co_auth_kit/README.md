# CO Auth Kit — Single Sign-On Integration

CO-166: Cross-deployment SSO for CO universes.

Two integration mechanisms are supported depending on whether the deployments
share an apex domain.

---

## Mechanism A — Cookie sharing (`.artelonga.com.br` subdomains)

When CO sets `CO_COOKIE_DOMAIN=.artelonga.com.br`, the `session` cookie is
automatically shared across all subdomains. Any application on
`*.artelonga.com.br` receives the cookie on every request and can verify it
using this kit without additional redirects.

### How it works

1. User logs in at `co.artelonga.com.br` → receives `session=<JWT>` cookie
   with `Domain=.artelonga.com.br`.
2. Browser sends the same cookie to `app.artelonga.com.br`, `api.artelonga.com.br`, etc.
3. Each app calls `verifyCoSession(token, "https://co.artelonga.com.br")` to
   validate the token against the public JWKS (no secret sharing needed).

### Configuration

Set on the CO server:
```
CO_COOKIE_DOMAIN=.artelonga.com.br
```

No client-side configuration needed; the cookie is shared automatically.

---

## Mechanism B — OIDC for cross-apex deployments

When apps live on different top-level domains, use the standard OpenID Connect
authorization code flow with PKCE (S256).

### Discovery

```
GET https://co.artelonga.com.br/.well-known/openid-configuration
GET https://co.artelonga.com.br/.well-known/jwks.json
```

### Register your app (gestão admin required)

```bash
curl -X POST https://co.artelonga.com.br/api/v1/gestao/oauth/clients \
  -H "Authorization: Bearer <github-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My App",
    "redirect_uris": ["https://myapp.example.com/callback"],
    "scopes": "openid profile email"
  }'
# → { client_id, client_secret, ... }
```

### Authorization code flow

1. Redirect user to:
   ```
   https://co.artelonga.com.br/oauth/authorize
     ?response_type=code
     &client_id=co_<your-client-id>
     &redirect_uri=https://myapp.example.com/callback
     &scope=openid%20email
     &code_challenge=<SHA256(code_verifier) base64url>
     &code_challenge_method=S256
   ```

2. CO redirects back with `?code=<auth-code>`.

3. Exchange code for tokens:
   ```bash
   curl -X POST https://co.artelonga.com.br/oauth/token \
     -d grant_type=authorization_code \
     -d code=<auth-code> \
     -d redirect_uri=https://myapp.example.com/callback \
     -d client_id=co_<your-client-id> \
     -d client_secret=<your-secret> \
     -d code_verifier=<your-verifier>
   # → { access_token, id_token, token_type, expires_in, scope }
   ```

4. Fetch user profile:
   ```bash
   curl -H "Authorization: Bearer <access_token>" \
     https://co.artelonga.com.br/oauth/userinfo
   # → { sub, email, name, tier }
   ```

---

## TypeScript adapter (`co_auth.ts`)

Compatible with Deno and Node.js (requires Web Crypto API).

```typescript
import { verifyCoSession, getCoSessionCookie } from "./co_auth.ts";

// In your request handler:
const token = getCoSessionCookie(req.headers.get("cookie") ?? "");
if (!token) {
  return Response.json({ error: "not authenticated" }, { status: 401 });
}

const user = await verifyCoSession(token, "https://co.artelonga.com.br");
if (!user) {
  return Response.json({ error: "invalid or expired token" }, { status: 401 });
}

console.log(user.sub, user.email, user.tier);
```

No secret sharing required. The public key is fetched from JWKS and cached
for 5 minutes.

---

## Python adapter (`co_auth.py`)

Requires `PyJWT` and `cryptography`:

```bash
pip install PyJWT cryptography
```

```python
from co_auth import verify_co_session, get_co_session_cookie

# In your request handler (Flask/FastAPI/Django):
cookie_header = request.headers.get("Cookie", "")
token = get_co_session_cookie(cookie_header)
if not token:
    return {"error": "not authenticated"}, 401

user = verify_co_session(token, "https://co.artelonga.com.br")
if not user:
    return {"error": "invalid or expired token"}, 401

print(user["sub"], user["email"], user["tier"])
```

---

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CO_COOKIE_DOMAIN` | (none) | When set, adds `Domain=<value>` to session cookies. Use `.artelonga.com.br` for subdomain sharing. |
| `CO_JWT_PRIVATE_KEY` | (auto-generated) | PKCS8 PEM of the EC P-256 signing key. Set in production so keys survive restarts. |
| `CO_QUILOMBO_LEGACY_LOGIN` | `true` | When `false`, `POST /api/v1/quilombo/auth/login` returns 410 Gone (deprecation path). |

---

## "Entrar com CO" button concept

For sites that want a "Login with CO" button (similar to "Login with GitHub"):

1. Generate a PKCE verifier/challenge pair (see RFC 7636).
2. Store `code_verifier` in session/cookie.
3. Redirect user to the CO `/oauth/authorize` endpoint.
4. On callback, exchange the code using the stored `code_verifier`.
5. Use the returned `id_token` to identify the user.

This gives your app a CO-verified identity without any shared secrets.
The verification is done via the public JWKS, so no CO server credentials
are needed on your side.
