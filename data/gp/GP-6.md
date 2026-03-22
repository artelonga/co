---
id: 6
title: User registration endpoint
status: todo
priority: critical
parent: 5
labels:
  - auth
  - server
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

GIVEN a new user wants to create an account,
WHEN they POST to `/api/v1/auth/register` with `{ username, password, display_name }`,
THEN:
- [ ] Username validated: 3-20 chars, alphanumeric + underscore, case-insensitive unique
- [ ] Password validated: minimum 8 characters
- [ ] Password hashed with Argon2id (salt from OsRng)
- [ ] UserProfile created in storage with nanoid user_id
- [ ] Wallet initialized with 10,000 chips (TX_INITIAL_GRANT)
- [ ] Returns `201 { user_id, username, display_name }`
- [ ] Duplicate username returns `409 Conflict`
- [ ] Invalid input returns `400 Bad Request` with field-level errors
- [ ] commit: `feat(server): add user registration endpoint`
