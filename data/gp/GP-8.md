---
id: 8
title: JWT auth middleware for protected routes
status: todo
priority: critical
parent: 5
labels:
  - auth
  - server
  - middleware
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

GIVEN authenticated endpoints need JWT validation,
WHEN a request includes `Authorization: Bearer <jwt>`,
THEN:
- [ ] Middleware extracts Bearer token from header
- [ ] JWT decoded and verified using `JWT_SECRET`
- [ ] Expired tokens return `401`
- [ ] Invalid signatures return `401`
- [ ] Missing header returns `401`
- [ ] Valid token injects `UserId` into request extensions
- [ ] Existing game service token-map auth replaced with JWT validation
- [ ] All protected routes use this middleware
- [ ] commit: `feat(server): JWT auth middleware replacing token map`
