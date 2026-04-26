---
id: 2
title: Subtask tree rendering with expand/collapse in all views
status: done
priority: critical
parent: 1
labels:
  - board
  - ui
created_at: 2026-04-01T00:00:00Z
updated_at: 2026-04-01T22:57:43.784727+00:00
---

GIVEN tasks have a `parent` field linking child to parent,
WHEN viewing kanban, table, or timeline views,
THEN subtasks render nested under their parent with indent and expand/collapse toggle.

## Current state

The board UI is in `co-web/static/variants/a/app.js` (~2150 lines) and `style.css` (~1785 lines).
Functions `getSubtasks()` and `getSubtaskProgress()` exist (app.js ~line 215-224) but only show a count badge.
All views render tasks as a flat list with no hierarchy.

## Acceptance Criteria

- [ ] Kanban: parent card shows expandable subtask list below it (click to toggle)
- [ ] Table: indented rows with visual tree connector lines (like a file tree)
- [ ] Timeline: subtasks grouped under parent swimlane, collapsible
- [ ] Task modal: show parent link and list of subtasks
- [ ] Persist expand/collapse state in localStorage
- [ ] commit: `feat(board): subtask tree rendering with expand/collapse`
