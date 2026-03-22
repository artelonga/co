---
id: 7
title: Login endpoint with JWT issuance
status: todo
priority: critical
parent: 5
labels:
  - auth
  - server
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

GIVEN a registered user wants to authenticate,
WHEN they POST to `/api/v1/auth/login` with `{ username, password }`,
THEN:
- [ ] Username lookup finds user in storage
- [ ] Password verified against Argon2id hash
- [ ] On success: JWT signed with HMAC-SHA256 using `JWT_SECRET` env var
- [ ] JWT payload: `{ sub: user_id, username, exp: now + 24h, iat: now }`
- [ ] Returns `200 { token, user_id, username, display_name, expires_at }`
- [ ] Wrong password returns `401 Unauthorized` (no detail leak)
- [ ] Unknown username returns `401 Unauthorized`
- [ ] `jsonwebtoken` crate added to server dependencies
- [ ] commit: `feat(server): add JWT login endpoint`
