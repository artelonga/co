---
id: e2e-release-checklist
title: "Full E2E test plan — sign up / login / logout / forgot pw across all entry points"
type: page
status: active
priority: high
created_at: 2026-05-12T00:00:00Z
---

# Full E2E Release Checklist

Walks every signup entry point through the canonical lifecycle: **sign up
→ logged in → logout → forgot password → logged in again**, with
multi-universe SSO + recently-added artelonga signup arm.

Run this whenever a release touches auth, identity, or membership.
Currently 2.3.3.

---

## Pre-test setup

### 1. Three fresh incognito browsers (or three browser profiles)

Each profile represents a different user. Suggested:

| Profile | Email | Persona |
|---|---|---|
| **A** | `e2e-yuri@test.local` | Power user — owns universes, will be admin |
| **B** | `e2e-mariana@test.local` | Regular user — gets invited |
| **C** | `e2e-pedro@test.local` | Cross-domain user — signs up via quilombo or artelonga |

Pre-test:

```bash
# Make sure prod is the version under test
curl -s https://co-artelonga.fly.dev/api/health
# → {"status":"ok","version":"2.3.3"}

# Make sure quilombo prod also up
curl -s https://quilomboaraucaria.org/api/quilombo/auth/me
# → 401 (no session) is the right answer

# Tail logs in a second terminal for the duration of the test
flyctl logs -a co-artelonga | tee /tmp/e2e-$(date +%Y%m%d).log
```

### 2. Universes to exercise

Test must touch each visibility class at least once:

- **Template** (`/` — anonymous-readable, auto-cloned for new users)
- **Public-subscribable** (e.g., `co`, `quilomboaraucaria`, `yggdrasil`)
- **Private + invitation-required** (create one during test)
- **Sub-universe with parent_key** (e.g., a child of `tempo`)

---

## Test arm 1 — CO direct (`co.artelonga.com.br`)

### 1.1 Sign up via passwordless email (CO-190)

In Profile A, incognito, `https://co.artelonga.com.br`:

- [ ] Landing renders template board (tutorial tasks visible)
- [ ] Click **"Entrar"** → modal opens with email-first surface
- [ ] Enter `e2e-yuri@test.local` → "Continuar com email"
- [ ] **Resend should send a 6-digit code.** Check inbox / Resend dashboard.
- [ ] Enter code → click "Confirmar"
- [ ] **Lands logged in.** Header shows display name; sidebar shows owned universe.
- [ ] `GET /api/v1/auth/me` returns user with `usuario: 'e2e-yuri'` (auto-derived).
- [ ] User badge in top-right shows display name.
- [ ] Tutorial board is now THEIR clone, editable.

**Side effects to verify:**

- [ ] `users.origin` is `'co'` (or NULL — origin tracking is CO-205, pending)
- [ ] `quilombo_usuarios` row auto-created via reverse bridge (CO-184)
- [ ] `notification_preferences` row created with defaults
- [ ] `user_recovery_channels` has verified email channel

### 1.2 Logout

- [ ] Click user badge → "Sair"
- [ ] Sidebar collapses to anonymous view
- [ ] `GET /api/v1/auth/me` returns 401
- [ ] Universe `/` still shows template board (anonymous-readable)

### 1.3 Forgot password (CO-176)

User just signed up via passwordless — they don't *have* a password.
Forgot-password still needs to work for the email recovery flow:

- [ ] Click "Entrar" → "Esqueci minha senha"
- [ ] Enter usuario `e2e-yuri` + email `e2e-yuri@test.local`
- [ ] Code arrives via Resend
- [ ] Enter code → set new password
- [ ] Land logged in with that password
- [ ] Logout
- [ ] Try logging in with username + password → success
- [ ] `flyctl logs | grep "Recovery code emailed"` shows the send

### 1.4 Cross-domain SSO

While logged in on `co.artelonga.com.br`:

- [ ] Navigate to `https://quilomboaraucaria.org` (Profile A, same window)
- [ ] You should NOT be auto-logged in (different apex; expected)
- [ ] Click "Entrar" → "Continuar com Google" (or email)
- [ ] Same identity resolves — `e2e-yuri` is already linked via the bridge
- [ ] You're logged in on quilombo with the same `linked_co_user_id`

---

## Test arm 2 — Quilombo origin (`quilomboaraucaria.org`)

In Profile B, fresh incognito:

### 2.1 Sign up via Google OAuth on quilombo

- [ ] `https://quilomboaraucaria.org/entrar`
- [ ] Click "Continuar com Google"
- [ ] Google consent screen → choose `e2e-mariana@test.local`
- [ ] Lands logged in on quilombo
- [ ] **Side effect:** CO account auto-created via CO-172 bridge

Verify on co side (open new tab, same browser):

- [ ] `https://co.artelonga.com.br` shows logged-in state automatically (same apex if `.artelonga.com.br` cookie set, OR show login modal with one-click via stored handover token)
- [ ] If not auto-logged: click "Entrar" → Google → same user → succeeds
- [ ] `GET /api/v1/auth/me` returns Mariana with the same Google sub

### 2.2 Logout from quilombo

- [ ] On quilombo, click "Sair"
- [ ] Reload — anonymous
- [ ] CO is also logged out (or independent — both behaviors acceptable, document which)

### 2.3 Forgot password from quilombo

- [ ] On quilombo, "Esqueci minha senha" → enter usuario + email
- [ ] **Bounces to** `co.artelonga.com.br/recover?return_to=...` (CO-176 redirect)
- [ ] CO recovery form loads, code sent
- [ ] After reset, redirected back to quilombo logged in

---

## Test arm 3 — Yggdrasil game lobby

In Profile A (or fresh Profile C):

### 3.1 Anonymous lobby

- [ ] `https://co.artelonga.com.br/yggdrasil/yggdrasil`
- [ ] Lobby renders; chat panel renders inline (not drawer)
- [ ] Without login: chat panel shows login prompt, no rooms listed

### 3.2 Signed-in lobby

Log in via passwordless or Google, then revisit lobby:

- [ ] Chat panel loads with `general` room
- [ ] Can post messages
- [ ] Other Profile B can join same lobby and see live messages

### 3.3 Sign up via yggdrasil entry point

- [ ] Fresh incognito on `/yggdrasil/yggdrasil` while anon
- [ ] Click "Entrar" in the inline panel → email signup
- [ ] After verify, you're back in the lobby, logged in
- [ ] **CO-205 will add origin tracking** — verify `users.origin = 'yggdrasil'` after CO-205 lands

---

## Test arm 4 — Artelonga signup (CO-205, pending)

⚠️ Requires CO-205 to land first. Until then, document expected
behavior:

### 4.1 Anonymous on artelonga.com.br

- [ ] `https://artelonga.com.br` loads (static)
- [ ] Signup form visible (after AL-N frontend ticket lands)

### 4.2 Sign up via artelonga form

- [ ] Submit email → POST hits `co.artelonga.com.br/api/v1/auth/onboard-with-email` with CORS
- [ ] Code arrives, enter it
- [ ] Cookie set on `.artelonga.com.br` → visible from artelonga.com.br
- [ ] Reload artelonga.com.br → "Welcome back" / logged-in state
- [ ] `users.origin = 'artelonga'`

### 4.3 Verify cross-universe access from artelonga origin

- [ ] After artelonga signup, navigate to `co.artelonga.com.br/` → already logged in
- [ ] Same user, same identity
- [ ] Same Mariana / yuri user shows up

---

## Test arm 5 — Multi-universe membership

In Profile A, logged in:

### 5.1 Create + own a new universe

- [ ] Click "Criar universo" → name "Test E2E"
- [ ] Land on `/test-e2e` as owner
- [ ] Sidebar shows it under "Meus universos"

### 5.2 Invite Profile B

- [ ] Open settings → "Convidar pessoas" → email `e2e-mariana@test.local`
- [ ] Mariana gets email (or check `flyctl logs | grep invitation`)

### 5.3 Profile B accepts invitation

- [ ] Open the invitation link in Profile B's incognito
- [ ] Preview shows correct universe + inviter
- [ ] Click "Aceitar" — if logged out, signs in; if logged in but
      identity-mismatch, shows clear message
- [ ] After accept, Mariana sees "Test E2E" in her sidebar under "Comunidades" with role chip

### 5.4 Chat in the new universe

- [ ] Both A and B open Test E2E
- [ ] A posts a chat message → B sees it live in `general` room
- [ ] A edits the message → B sees the `(editado)` tag live
- [ ] B sends a DM to A
- [ ] A gets notified (in-app bell badge + email digest within ~60s)

### 5.5 Sub-universe (parent_key)

If `tempo` universe is accessible:

- [ ] Navigate to `/tempo`
- [ ] Sidebar tree default-expands; subuniverses (tempo-bahia, tempo-rs, etc.) visible
- [ ] Click a subuniverse → switches context

---

## Test arm 6 — Notifications (Phase 5)

### 6.1 In-app notification center

- [ ] 🔔 bell visible in header
- [ ] After receiving a chat or DM, badge increments live
- [ ] Click bell → dropdown shows recent
- [ ] Click a notif → marks read, navigates to source

### 6.2 Email digests (CO-200)

- [ ] In settings, set `email_digest_freq = instant` for `e2e-yuri`
- [ ] From Profile B, send a chat message to a room A belongs to
- [ ] Within 60s, A receives an email digest from `notificacoes@…`
- [ ] Verify `flyctl logs | grep "notif email worker"`

### 6.3 Web push (CO-201)

- [ ] Settings → "Ativar notificações" → permission prompt → grant
- [ ] Close all tabs
- [ ] Profile B sends a DM
- [ ] System notification appears on A's OS
- [ ] Click notification → tab focuses on DM

⚠️ Requires VAPID secrets set on Fly. Without them, push degrades to log-only.

---

## Test arm 7 — Recovery + edge cases

### 7.1 Forgot password for a quilombo-bridged user (legacy)

- [ ] Create a fresh quilombo user via quilombo signup
- [ ] Logout
- [ ] On co.artelonga.com.br, "Esqueci minha senha" → enter quilombo usuario + email
- [ ] **Lazy-bridge fires** if needed; code arrives
- [ ] After reset, login works on both co and quilombo

### 7.2 Identity-mismatch on invitation

- [ ] Profile A invites email `e2e-pedro@test.local`
- [ ] Click link in Profile B's browser (logged in as Mariana)
- [ ] Preview page shows "Esse convite é para outro email/usuário"
- [ ] Affordance to sign out + sign in with correct account works

### 7.3 Storage lock under load

Issue 2.3.x family — if you've manually triggered any panic
condition, verify it doesn't cascade:

- [ ] Send 5 chat messages rapidly
- [ ] Open 3 universes back-to-back via sidebar clicks
- [ ] Browser back/forward several times — each loads correctly (CO-2.3.3)
- [ ] Refresh — health should still be 200; no "storage lock" errors
- [ ] `flyctl logs | grep poisoned` should be empty for the test window

---

## Backend checks during the test

Run these from a separate terminal while the test is in progress:

```bash
# Watch for poisoned locks
flyctl logs -a co-artelonga | grep -E "poisoned|panic"

# Watch for any 500s
flyctl logs -a co-artelonga | grep -E "Status code: 5"

# Watch notif workers active
flyctl logs -a co-artelonga | grep -E "notif (email|push)"

# Verify VAPID is configured (or not) for push
curl -s https://co-artelonga.fly.dev/api/v1/notifications/vapid-public-key
```

After the test, check counts in DB:

```bash
flyctl ssh console -a co-artelonga -C "sqlite3 /data/co.db '
SELECT origin, COUNT(*) FROM users GROUP BY origin;
SELECT event_type, COUNT(*) FROM user_notifications GROUP BY event_type;
SELECT kind, COUNT(*) FROM chat_rooms GROUP BY kind;
'"
```

---

## Exit criteria

✅ All 7 test arms pass → release is green.

⚠️ Any storage-lock-poisoned errors → escalate to CO-203 (parking_lot
migration) before re-running.

⚠️ Any 500 from auth/me/recovery/onboarding → file a bug ticket
referencing this checklist.

---

## Recovery if prod breaks during the test

```bash
# Storage lock poisoned cascade
flyctl machine restart 1850920b111d38 -a co-artelonga

# Bad deploy
git log --oneline -5
# Identify last-known-good
flyctl releases -a co-artelonga
flyctl deploy --image registry.fly.io/co-artelonga:deployment-<id>

# Test data cleanup
flyctl ssh console -a co-artelonga
sqlite3 /data/co.db "DELETE FROM users WHERE email LIKE '%@test.local';"
# (cascades via FK to user_notifications, chat_room_members, etc.)
```

---

## Open questions / scope notes

- **Push notifications (test arm 6.3)** requires VAPID setup; if you
  haven't done that yet, that test arm is N/A.
- **CO-205 artelonga signup (test arm 4)** is pending implementation.
  When CO-205 lands, this section can be exercised.
- **Test data isolation** — the test creates real users on prod. After
  the test, run the cleanup SQL above to remove `*@test.local` users.

---

## Related tickets

- CO-203 parking_lot::Mutex migration — must land before this test
  becomes reliable on a busy prod
- CO-204 message origin telemetry — affects DM rendering, not signup
- CO-205 artelonga signup backend — required for test arm 4
- CO-197 co-auto FF reliability — orthogonal but worth fixing
