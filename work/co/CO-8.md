---
id: 8
title: Delete project API endpoint
status: done
priority: medium
parent: 1
labels:
  - board
  - api
created_at: 2026-04-01T00:00:00Z
updated_at: 2026-04-06T13:09:25.621223+00:00
---

GIVEN there is no way to delete projects via the API,
WHEN an admin sends DELETE /api/projects/{key},
THEN the project and all its tasks, comments, and activity log entries are removed.

## Files to modify

- `co-web/src/server.rs` — add `.delete(delete_project)` route
- `co-web/src/storage.rs` — add `delete_project(&self, key: &str)` method with cascade deletes

## Acceptance Criteria

- [ ] DELETE /api/projects/{key} endpoint exists
- [ ] Deletes project row from projects table
- [ ] Cascade deletes: all tasks, comments, activity_log for that project
- [ ] Returns 204 on success
- [ ] Returns 404 if project not found
- [ ] `cargo test` passes
- [ ] commit: `feat(board): add delete project endpoint`
