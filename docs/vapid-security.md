# VAPID Key — Security Model + Threat Analysis

What a leak of CO's VAPID private key would actually enable, what it
wouldn't, and how we mitigate.

## What VAPID is

VAPID (Voluntary Application Server Identification, [RFC 8292]) is a
self-authentication scheme for **the sender of a Web Push message**.

The CO server holds an ES256 (P-256 ECDSA) keypair:

- **Public key** — exposed via `GET /api/v1/notifications/vapid-public-key`.
  Browsers fetch it during `PushManager.subscribe()` and bind it to each
  subscription endpoint. **Public on purpose** — no security value.
- **Private key** — held in Fly secrets as `VAPID_PRIVATE_KEY`. Used by
  the push worker to sign a short-lived JWT (`Authorization: vapid
  t=<jwt>, k=<public_key>`) that goes alongside each push request to a
  browser's push service (Mozilla autopush, Apple, FCM).

Push services validate the JWT against the public key they recorded at
subscription time. **A request without a valid VAPID JWT is rejected.**

[RFC 8292]: https://datatracker.ietf.org/doc/html/rfc8292

## What VAPID is NOT

| What it doesn't do | Why this matters |
|---|---|
| **Does not encrypt the payload.** | Payload encryption uses the *subscriber's* `p256dh` + `auth` keys (RFC 8291), stored per-user in `push_subscriptions`. VAPID is sender auth, not message confidentiality. |
| **Does not authenticate the recipient.** | Anyone who knows a user's subscription endpoint + `p256dh` + `auth` keys can encrypt a payload to that user. VAPID just says "this is the legitimate sender." |
| **Does not grant access to user accounts.** | VAPID is for the push channel only. Compromise doesn't yield session tokens, passwords, or any CO data. |
| **Does not enable read access to push history.** | The push service doesn't keep messages once delivered. CO's DB records `delivered_push_at` timestamps but not message bodies (we strip body before delivery; only `tag` + `url` go on the wire). |

## Compromise threat model

### What an attacker with ONLY `VAPID_PRIVATE_KEY` can do

**Almost nothing useful.** Without subscription data, they can sign valid
VAPID JWTs but have no `endpoint` / `p256dh` / `auth` keys to target. A
push request needs all three.

### What an attacker with `VAPID_PRIVATE_KEY` + `push_subscriptions` table can do

**Spoof push notifications to your users that appear to come from CO.**

Concretely:

1. Fetch a target user's `endpoint`, `p256dh`, `auth` from the leaked DB.
2. Build a payload (title, body, url) of their choosing.
3. Encrypt with `p256dh` + `auth` (subscriber-keyed encryption).
4. Sign a VAPID JWT with the leaked private key.
5. POST to the user's `endpoint` URL.
6. The push service validates the VAPID JWT, sees it matches the public
   key bound at subscription, **accepts the request as legitimate from CO**.
7. The user's browser shows a system notification that **looks
   indistinguishable from a real CO notification.**

**Realistic attacks enabled by this:**

| Attack | Plausibility |
|---|---|
| Phishing — "Your password expires in 1 hour, click here" with attacker URL | High value to attacker; legitimate-looking |
| Brand damage — push spam to all subscribers | Moderate; users would distrust real notifications |
| Targeted social engineering — push to a specific user mentioning their actual context | High if attacker has DB access to enrich (room names, contacts, etc.) |
| Cross-site script delivery | **Not possible** — `notificationclick` only opens URLs; the system notification UI doesn't execute scripts |
| Account takeover | **Not directly** — phishing-only, requires user interaction with the spoofed link |

### What VAPID compromise does NOT enable

- Reading any user's data — VAPID is push-channel-only
- Decrypting past push messages — once delivered, they're discarded
- Adding/removing subscriptions — that requires CO API auth
- Modifying notification preferences — that requires session JWT
- Impersonating the user — push goes server→user, not user→server

## Severity assessment

**Standalone leak: LOW severity.**

The VAPID private key alone is useless. An attacker still needs:
- Subscription endpoints (separately leaked from `push_subscriptions`)
- Public p256dh + auth keys for each target (same)

**Combined with `push_subscriptions` table leak: MEDIUM severity.**

Phishing-grade attack surface on every subscribed user. Mitigated by:
- Users learning to verify the URL in any notification before clicking
- Rotating VAPID + invalidating all subscriptions (forces re-subscribe)
- Notification UI showing the origin domain (browser-enforced)

## Storage + rotation

### Where the private key MUST live

| Location | Status |
|---|---|
| Fly secrets (`VAPID_PRIVATE_KEY`) | ✅ Primary — encrypted at rest, injected as env var at runtime only |
| Password manager (1Password / Bitwarden) | ✅ Required backup for rotation / disaster recovery |

### Where it MUST NOT live

| Location | Risk |
|---|---|
| Git repository (any branch) | Public-facing risk via repo clone / history |
| Source code, even commented-out | Same |
| Plaintext on disk past the generation moment | Disk forensics, accidental backup, `find / -name "*.key"` |
| Chat with AI / Slack / email | Logged on third-party server; out of your control |
| Bash history (`HISTFILE`) | Run `unset HISTFILE` before any command containing the key, or use `set +o history` |
| Shell environment variables outside the secret-setting moment | Other processes can read `/proc/<pid>/environ` |

### Rotation procedure

When to rotate:

- **Immediate**: suspected leak (key in logs, chat, screenshot, git push, lost laptop)
- **Annual**: policy-based defense in depth
- **On personnel change**: anyone who had access to set the key leaves

How to rotate:

1. Generate fresh keypair locally (see `/tmp/vapidgen.sh` template in
   the team docs)
2. `flyctl secrets set VAPID_PUBLIC_KEY=... VAPID_PRIVATE_KEY=...` on prod
3. Restart the Fly machine
4. Update password manager entry with the new keys (delete the old one
   after confirmation)
5. **All existing browser subscriptions become invalid.** Users must
   re-subscribe on their next visit. Acceptable trade-off.

### Operational hygiene

- The VAPID private key is **a single value**, easy to audit access to
- `flyctl secrets list -a co-artelonga` shows the digest (not the value)
- Run `flyctl secrets unset VAPID_PRIVATE_KEY` to immediately invalidate
  push without rotating (graceful degradation: worker logs but doesn't
  deliver)

## Detection

How would we notice a VAPID compromise?

- **Push notification reports from users** — "I got a weird message"
- **Anomalous patterns** in push delivery logs (`flyctl logs |
  grep "push delivered"`) — bursts to many users at odd times, payloads
  with URLs not pointing at co.artelonga.com.br
- **Subscriber complaints** about phishing-shaped content

No tamper-evident logging today. If high-volume push becomes a primary
product, consider adding a push-payload-audit table (signed hash of
each delivered payload, append-only, alarmed on anomaly).

## TL;DR

- **The VAPID private key alone is low-value** to an attacker
- **Combined with the subscriber DB, it's a phishing toolkit**
- **Treat it as MEDIUM-sensitivity** — same tier as Resend API key
- **Store in Fly secrets + password manager only**
- **Rotate annually or on suspicion**
- **Web Push payload confidentiality is separately guaranteed** by
  RFC 8291 subscriber-keyed encryption
