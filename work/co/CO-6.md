---
id: 6
title: Add assignee field to task model, API, and UI
status: done
priority: high
parent: 1
labels:
  - board
  - api
created_at: 2026-04-01T00:00:00Z
updated_at: 2026-04-01T23:14:51.356276+00:00
---

GIVEN tasks have no assignee field,
WHEN creating or editing a task,
THEN an assignee (email or name) can be set and displayed in all views.

## Files to modify

- `co-web/src/models.rs` — add `assignee: Option<String>` to Task, CreateTask, UpdateTask
- `co-web/src/storage.rs` — migration v5: `ALTER TABLE tasks ADD COLUMN assignee TEXT`; update all task CRUD queries to include assignee
- `co-web/static/variants/a/app.js` — show assignee avatar/name in kanban card, table column, task edit modal
- `co-web/static/variants/a/style.css` — assignee badge styling
- `co-web/tests/storage_tests.rs` — update schema version assertion to v5
- `co-web/openapi.yaml` — update Task schema

## Acceptance Criteria

- [ ] Task struct has `assignee: Option<String>`
- [ ] CreateTask and UpdateTask accept `assignee` field
- [ ] SQLite migration adds `assignee TEXT` column to tasks table
- [ ] Storage CRUD reads/writes assignee
- [ ] Kanban card shows assignee initials badge
- [ ] Table view has assignee column
- [ ] Task edit modal has assignee input field
- [ ] `cargo test` passes
- [ ] commit: `feat(board): add assignee field to tasks`
