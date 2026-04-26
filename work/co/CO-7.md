---
id: 7
title: Auth-protect board write operations
status: done
priority: high
parent: 1
labels:
  - board
  - api
  - security
created_at: 2026-04-01T00:00:00Z
updated_at: 2026-04-01T23:28:56.679706+00:00
---

GIVEN all board CRUD routes (POST/PUT/DELETE) are currently unauthenticated,
WHEN a user tries to create, edit, or delete tasks or projects,
THEN the server requires a valid JWT Bearer token.

## Files to modify

- `co-web/src/server.rs` — split the `api` Router into `board_public` (GET routes) and `board_protected` (POST/PUT/DELETE routes with `require_auth` middleware)

## Acceptance Criteria

- [ ] GET /api/projects, GET /api/projects/{key}/tasks etc. remain public (no auth)
- [ ] POST /api/projects requires JWT
- [ ] POST /api/projects/{key}/tasks requires JWT
- [ ] PUT /api/projects/{key}/tasks/{id} requires JWT
- [ ] DELETE /api/projects/{key}/tasks/{id} requires JWT
- [ ] POST bulk-update and bulk-delete require JWT
- [ ] POST comments require JWT
- [ ] Returns 401 Unauthorized with JSON error body when no token
- [ ] `cargo test` passes (existing tests may need updating if they write without auth)
- [ ] commit: `feat(board): auth-protect write operations`
