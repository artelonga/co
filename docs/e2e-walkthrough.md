# Full E2E Walkthrough — Step-by-step companion to `work/co/E2E-RELEASE-CHECKLIST.md`

The checklist is the **what**; this is the **how**. Designed for two people
(or one person + AI assistant) to execute together: one drives the browser,
the other watches backend state via logs / DB queries / API calls.

**Current target version:** 2.6.1 (CO) + yggdrasil PR #2 merged + artelonga
0.13.0+.

---

## Pre-flight (5–10 min)

### Required state

| Component | Endpoint | Expected |
|---|---|---|
| CO | `https://co-artelonga.fly.dev/api/health` | `version: 2.6.1` |
| Quilombo | `https://quilomboaraucaria.org/api/quilombo/auth/me` | `401` (anon, expected) |
| Yggdrasil | `https://yggdrasil-artelonga.fly.dev/health` | `200 ok` |
| ArteLonga | `https://artelonga.com.br/entrar/` | `200` |

### Required setup

1. **Browser profiles or incognito windows × 3** — one per persona:

   | Profile | Persona | Test email |
   |---|---|---|
   | **A** | Power user — invites others, posts chat | `e2e-yuri-{YYYYMMDD}@test.local` |
   | **B** | Invited collaborator | `e2e-mariana-{YYYYMMDD}@test.local` |
   | **C** | Cross-domain (quilombo / yggdrasil / artelonga) signup | `e2e-pedro-{YYYYMMDD}@test.local` |

   Use today's date in the email so re-runs don't conflict with previous
   E2E sessions.

2. **Two terminals open:**

   ```bash
   # Terminal 1 — watch CO logs
   flyctl logs -a co-artelonga
   
   # Terminal 2 — watch yggdrasil logs (for SSO arm)
   flyctl logs -a yggdrasil-artelonga
   ```

3. **VAPID secrets set** (optional for arms 1–5 + parts of 6; required
   for arm 6.3 push notifications):

   ```bash
   flyctl secrets list -a co-artelonga | grep VAPID
   # Should show three entries (PUBLIC, PRIVATE, SUBJECT)
   ```

4. **Cleanup SQL ready** (for post-test):

   ```bash
   # Save this somewhere — run after the test session
   flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "
   DELETE FROM users WHERE email LIKE \"%@test.local\";
   DELETE FROM quilombo_usuarios WHERE email LIKE \"%@test.local\";
   "'
   ```

---

## Arm 1 — CO direct passwordless onboarding (10 min)

Tests: CO-190 onboarding flow + CO-184 quilombo reverse bridge.

### Steps (Profile A driver)

| # | Browser action | Expected UI |
|---|---|---|
| 1 | Open `https://co-artelonga.fly.dev` in incognito | Template board renders, tutorial tasks visible, PT default |
| 2 | Click "Entrar" in sidebar | Email-first modal opens |
| 3 | Type `e2e-yuri-{YYYYMMDD}@test.local` → "Continuar com email" | Form flips to code-entry view |
| 4 | Get the 6-digit code from `flyctl logs -a co-artelonga` (look for `[MAIL]` lines OR `ResendProvider: email delivered`) | Code arrives within ~5s |
| 5 | Type the code → "Confirmar" | Lands logged in; sidebar shows owned universe; tutorial board is the user's clone |

### Verification (assistant runs)

```bash
# 1. Confirm user exists with origin populated
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "SELECT id, email, usuario, origin FROM users WHERE email LIKE \"%e2e-yuri%@test.local\";"'

# 2. Confirm quilombo reverse bridge fired
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "SELECT id, usuario, email, linked_co_user_id FROM quilombo_usuarios WHERE email LIKE \"%e2e-yuri%@test.local\";"'

# 3. Confirm verified email recovery channel
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "SELECT user_id, channel_type, verified_at FROM user_recovery_channels WHERE user_id = (SELECT id FROM users WHERE email LIKE \"%e2e-yuri%@test.local\");"'
```

### Pass criteria

- [ ] User row exists; `origin = 'co'` or NULL (CO-205 tracking)
- [ ] Matching `quilombo_usuarios` row with `linked_co_user_id` populated (CO-184 reverse bridge)
- [ ] `user_recovery_channels` row with `verified_at` not NULL (email auto-promoted)

---

## Arm 2 — Logout + forgot password (5 min)

Tests: CO-176 recovery + lazy bridge.

### Steps (Profile A continues from arm 1)

| # | Browser action | Expected UI |
|---|---|---|
| 1 | Click user badge → "Sair" | Sidebar collapses to anonymous; template banner returns |
| 2 | Click "Entrar" → "Esqueci minha senha" | Recovery modal with usuario + email fields |
| 3 | Type usuario from arm 1 + email | "Código enviado" toast |
| 4 | Grab the new code from logs | Resend log line OR `Recovery code emailed` |
| 5 | Enter code → set new password | Lands logged in with the new password |
| 6 | Logout again → log in with username + new password | Success |

### Verification

```bash
# Recovery should have fired through Resend (not silent no-match)
flyctl logs -a co-artelonga | grep -i "recovery code emailed\|Recovery request"
```

### Pass criteria

- [ ] Forgot-password sends a real email (not silent 202)
- [ ] New password works for subsequent login

---

## Arm 3 — Quilombo Google OAuth + lazy bridge (10 min)

Tests: CO-172 forward bridge + Google OAuth on quilombo.

### Steps (Profile B driver — fresh incognito)

| # | Browser action | Expected UI |
|---|---|---|
| 1 | `https://quilomboaraucaria.org/entrar` | Quilombo login form |
| 2 | Click "Continuar com Google" | Google OAuth consent screen |
| 3 | Choose `e2e-mariana-{YYYYMMDD}@test.local` Google account | Lands logged in on quilombo |
| 4 | New tab: `https://co-artelonga.fly.dev/` | Should NOT be auto-logged (different apex) |
| 5 | Click "Entrar" → "Continuar com Google" → same Google account | Logged in on CO with the same identity |

### Verification

```bash
# Same user_id across both surfaces
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "
SELECT u.id, u.email, u.google_sub, qu.usuario, qu.linked_co_user_id
FROM users u
LEFT JOIN quilombo_usuarios qu ON qu.linked_co_user_id = u.id
WHERE u.email LIKE \"%e2e-mariana%@test.local\";
"'
```

### Pass criteria

- [ ] Single `users` row with `google_sub` populated
- [ ] `quilombo_usuarios.linked_co_user_id` = `users.id` (CO-172 forward bridge)
- [ ] Cross-domain SSO works (clicking "Continuar com Google" twice with same Google account doesn't create duplicates)

---

## Arm 4 — Artelonga signup form + cross-domain cookie (10 min)

Tests: CO-205 CORS + origin tracking + AL-50 frontend.

### Steps (Profile C driver — fresh incognito)

| # | Browser action | Expected UI |
|---|---|---|
| 1 | `https://artelonga.com.br/entrar/` | Two-step form: email input + Google button |
| 2 | Type `e2e-pedro-{YYYYMMDD}@test.local` → "Continuar" | Form flips to code-entry view |
| 3 | Get code from `flyctl logs -a co-artelonga` | Code line in logs |
| 4 | Enter code → "Confirmar" | Redirects to `artelonga.com.br/` with "Olá, e2e-pedro" + "Sair" in header |
| 5 | Open devtools → Application → Cookies → `.artelonga.com.br` | Session cookie present |
| 6 | New tab: `https://co-artelonga.fly.dev/` | Already logged in (cross-subdomain cookie) |

### Verification

```bash
# Origin should be 'artelonga'
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "SELECT id, email, usuario, origin FROM users WHERE email LIKE \"%e2e-pedro%@test.local\";"'

# Test logout cross-domain (the 2.4.1 CSRF fix)
# In Profile C, click "Sair" → cookie cleared, redirects
```

### Pass criteria

- [ ] `users.origin = 'artelonga'` for this user (CO-205)
- [ ] Session cookie domain is `.artelonga.com.br` (cross-subdomain)
- [ ] `co.artelonga.com.br` auto-recognizes the session
- [ ] "Sair" on artelonga.com.br hits CO's logout endpoint with 200 (CSRF fix from 2.4.1)

---

## Arm 5 — Yggdrasil SSO handover (10 min)

Tests: CO-206 + YG PR #2.

### Steps (Profile A — still logged in from arm 1)

| # | Browser action | Expected UI |
|---|---|---|
| 1 | While logged in on `co.artelonga.com.br`, navigate to `https://yggdrasil-artelonga.fly.dev/games/poker` | Lands at poker page; lobby visible |
| 2 | OR: visit `https://yggdrasil-artelonga.fly.dev/login` → click "Continuar com Google" → choose Google account already logged into CO | Bounces through CO → handover-receive → lobby |
| 3 | Open devtools → check localStorage for `yggdrasil_jwt` (or similar) | Token present |
| 4 | Click "Mesa Carvalho" → sit | Bot Carvalho appears as opponent |
| 5 | Open Profile B incognito → repeat login → join same table | Bot disappears, real user joins |

### Verification

```bash
# Yggdrasil should have provisioned a local user with co_user_id link
flyctl ssh console -a yggdrasil-artelonga -C 'sqlite3 /data/yggdrasil.db "SELECT email, user_id, co_user_id FROM usuarios WHERE email LIKE \"%@test.local\";"'

# CO logs should show handover token mint
flyctl logs -a co-artelonga | grep "co-handover"
```

### Pass criteria

- [ ] No login screen on yggdrasil (handover token in URL → instant session)
- [ ] `yggdrasil/usuarios.co_user_id` matches `co/users.id`
- [ ] Two real users at the poker table sees the bot leave
- [ ] CO logs show JWKS handover happening

---

## Arm 6 — Multi-universe + chat + notifications (15 min)

Tests: CO-188/189 invitations + CO-193..198 chat + CO-199..202 notifications.

### Steps (Profile A inviter, Profile B invitee)

| # | Profile | Browser action | Expected |
|---|---|---|---|
| 1 | A | Create new universe: settings → "+ Novo universo" → name "Test E2E" | Lands on `/test-e2e` as owner |
| 2 | A | Settings → "Convidar pessoas" → email `e2e-mariana-{YYYYMMDD}@test.local` → "Enviar convite" | Invitation row appears in pending list; email sent |
| 3 | A | Open 💬 chat → "Geral" room → post "olá pessoal" | Message appears in scrollback |
| 4 | B | Click invitation link in email (or get the token from logs) | Lands on `/invitations/{token}` preview |
| 5 | B | Click "Aceitar" | Lands inside "Test E2E" as member |
| 6 | B | Open chat → sees A's "olá pessoal" | Real-time WS receive |
| 7 | B | Post a reply | A sees it live |
| 8 | A | Hover own message → ✏️ edit → change text | "(editado)" tag, B sees the edit live |
| 9 | A | Open 📩 Mensagens (DMs) → click member B → "Enviar mensagem" → "DM teste" | DM thread opens |
| 10 | B | 🔔 bell shows red dot with count | Click → notif row visible |

### Verification

```bash
# Invitation accepted = chat_room_members row exists
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "
SELECT crm.user_id, crm.role, cr.universe_key FROM chat_room_members crm
JOIN chat_rooms cr ON cr.id = crm.room_id
WHERE crm.user_id IN (SELECT id FROM users WHERE email LIKE \"%@test.local\")
ORDER BY cr.universe_key, crm.user_id;
"'

# Chat message landed
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "SELECT id, room_id, author_id, substr(body, 1, 60) FROM chat_messages ORDER BY created_at DESC LIMIT 5;"'

# DM auto-created the dm-* room
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "SELECT id, slug, kind FROM chat_rooms WHERE kind = \"dm\";"'

# Notification rows exist
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "SELECT event_type, COUNT(*) FROM user_notifications GROUP BY event_type;"'
```

### Pass criteria

- [ ] Invitation flow end-to-end works
- [ ] Both users see live chat messages
- [ ] Edits/deletes propagate via WS
- [ ] DM thread auto-created with `kind='dm'`
- [ ] Notifications captured per user, per event type

### Email digest sub-test (after VAPID is set)

```bash
# In settings, set Profile A's email_digest_freq to 'instant'
# From Profile B, send a chat message
# Within 60s, Profile A should receive an email from notificacoes@seguranca.artelonga.com.br

flyctl logs -a co-artelonga | grep "Notif email worker\|digest send"
```

### Web push sub-test (requires VAPID set)

```bash
# In Profile A settings → "Ativar notificações no navegador" → grant permission
# Close that tab
# From Profile B, send a DM
# OS-level notification should appear within ~5s
# Click → relevant tab opens

flyctl logs -a co-artelonga | grep "push delivered"
```

---

## Arm 7 — Recovery + edge cases (10 min)

Tests: 2.3.4 parking_lot poison-cascade prevention + edge identity flows.

### 7a — Identity-mismatch on invitation

| # | Action | Expected |
|---|---|---|
| 1 | Profile A invites email `e2e-foreign-{YYYYMMDD}@test.local` (a 4th, fresh email) | Email sent |
| 2 | Profile B (logged in as mariana) clicks the link | Preview shows "Esse convite é para outro email/usuário" |
| 3 | Page offers "Sair e entrar com a conta certa" | Affordance works |

### 7b — Legacy quilombo lazy-bridge

| # | Action | Expected |
|---|---|---|
| 1 | Manually insert a quilombo_usuarios row with no `linked_co_user_id` (simulating legacy) | Via SSH |
| 2 | Try forgot-password with that usuario + email | Lazy-bridge fires, code sent, login works |

### 7c — Storage lock stress test (2.3.4 verification)

```bash
# Hit /me/universes 20 times in parallel — should all succeed (no cascading 500s)
for i in $(seq 1 20); do
  curl -s -o /dev/null -w "%{http_code}\n" \
    -H "Cookie: session=$(getsession_from_browser_devtools)" \
    https://co-artelonga.fly.dev/api/v1/me/universes &
done | sort | uniq -c
# Expected: 20 of HTTP 200, 0 of HTTP 500
```

### 7d — Browser back/forward navigation (2.3.3 verification)

| # | Action | Expected |
|---|---|---|
| 1 | Profile A: navigate sidebar to 3 different universes in sequence | Each loads |
| 2 | Click browser back → back → back | Each back-click returns to the previous universe |
| 3 | Click forward → forward → forward | Each forward-click advances |

---

## Post-test cleanup (2 min)

```bash
# Remove all test users + their cascading data
flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "
DELETE FROM users WHERE email LIKE \"%@test.local\";
DELETE FROM quilombo_usuarios WHERE email LIKE \"%@test.local\";
DELETE FROM universes WHERE owner_id IN (
    SELECT id FROM users WHERE email LIKE \"%@test.local\"
);
"'

# Yggdrasil cleanup
flyctl ssh console -a yggdrasil-artelonga -C 'sqlite3 /data/yggdrasil.db "
DELETE FROM usuarios WHERE email LIKE \"%@test.local\";
"'
```

---

## Exit criteria

All 7 arms passing = **release is green**.

| Arm | Critical? | If it fails… |
|---|---|---|
| 1 — CO direct passwordless | Yes | Block release — core onboarding broken |
| 2 — Forgot password | Yes | Block release — recovery broken |
| 3 — Quilombo Google OAuth | Yes | Block release — SSO broken |
| 4 — Artelonga signup form | Yes | Block release — cross-domain broken |
| 5 — Yggdrasil SSO handover | Yes | Block release — multi-app identity broken |
| 6 — Multi-universe + chat + notif | Yes | Block release — communication broken |
| 7 — Recovery + edge cases | Mostly | 7a/7b/7d non-blocking; 7c (stress test) is critical |

---

## Roles during the test

For a 2-person run (or 1 person + AI):

| Role | Responsibilities |
|---|---|
| **Driver** | Clicks through browser flows, reports what they see, runs the cleanup at the end |
| **Verifier** | Watches logs, runs DB queries, confirms pass criteria for each arm, calls out anomalies |

For 1 person solo: do the driver actions, then run the verification queries after each arm.

---

## Related

- `work/co/E2E-RELEASE-CHECKLIST.md` — original checklist (this doc is the
  step-by-step companion)
- `docs/vapid-security.md` — VAPID compromise threat model + rotation
- `docs/analytics-api.md` — endpoint reference for arms that use analytics
