---
id: 4
title: "Dashboard: velocity chart, completion trend, workload by assignee"
status: done
priority: high
parent: 1
labels:
  - board
  - ui
created_at: 2026-04-01T00:00:00Z
updated_at: 2026-04-01T23:34:40.011277+00:00
---

GIVEN the dashboard view shows only basic status counts,
WHEN the user views the dashboard,
THEN it shows velocity (tasks done per week), completion trend (burndown), and task distribution.

## Current state

Dashboard is rendered in `co-web/static/variants/a/app.js` with minimal stats:
status distribution (bar), overdue count, upcoming deadlines list, recently updated list.

## Acceptance Criteria

- [ ] Velocity chart: tasks completed per week for the last 8 weeks (bar chart, using activity_log data)
- [ ] Burndown/burnup: remaining vs completed tasks over time (line chart)
- [ ] Task distribution by label (horizontal bar chart)
- [ ] Overdue tasks with aging indicators (days overdue, color-coded)
- [ ] Charts rendered with pure SVG (no external chart library)
- [ ] commit: `feat(board): dashboard with velocity, burndown, and distribution charts`
