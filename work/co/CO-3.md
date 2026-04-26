---
id: 3
title: "Fix timeline: stable header, dependency arrows, proper zoom"
status: done
priority: high
parent: 1
labels:
  - board
  - ui
created_at: 2026-04-01T00:00:00Z
updated_at: 2026-04-01T22:58:08.344936+00:00
---

GIVEN the timeline view exists in `co-web/static/variants/a/app.js`,
WHEN the user scrolls or zooms the timeline,
THEN the header stays fixed, tasks show parent-child dependency arrows, and zoom levels render correctly.

## Acceptance Criteria

- [ ] Sticky header row (position: sticky, top: 0, z-index above swimlanes)
- [ ] Dependency arrows drawn between parent and child tasks (SVG or canvas overlay)
- [ ] Week zoom: 7-day columns with day labels
- [ ] Month zoom: 30-day columns with week labels
- [ ] Quarter zoom: 90-day view with month labels
- [ ] Drag-to-resize due_date still works after changes
- [ ] commit: `fix(board): stable timeline header and dependency arrows`
