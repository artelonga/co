---
id: 5
title: Authentication & User Identity
status: todo
priority: critical
labels:
  - epic
  - auth
  - base-app
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

AS A user of the game platform,
I NEED to register, log in, and maintain a persistent identity across all universes,
SO THAT my profile, game stats, wallet, and universe memberships are tied to my account and accessible from any client (web, Godot).

## Scope
- Username + password registration (Argon2id hashing)
- JWT-based session tokens (HMAC-SHA256)
- User profile (display name, bio, avatar URL, created_at)
- Auth middleware for all protected API routes
- SvelteKit auth flow (login page → HttpOnly cookie → hooks guard)

## Versioning
- feat: register/login → v0.2.0
