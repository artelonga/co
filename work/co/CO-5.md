---
id: 5
title: Integrate variant palette switcher into board UI
status: done
priority: medium
parent: 1
labels:
  - board
  - ui
  - design
created_at: 2026-04-01T00:00:00Z
updated_at: 2026-04-01T23:51:52.701060+00:00
---

GIVEN CO supports 8 experiment variants (a-h) with different visual treatments,
WHEN the user clicks a palette switcher in the board header,
THEN the board UI applies the selected color scheme via CSS custom properties.

## Current state

The variant system exists: `co-web/static/variants/{a..h}/` each have style.css.
Variants are selected via `co_variant` cookie and `/api/experiment/variant` endpoint.
But there's no UI switcher — variant change requires API call or cookie edit.

## Acceptance Criteria

- [ ] Palette switcher button/dropdown in the board header (top-right area)
- [ ] Show variant name/preview swatch for each option
- [ ] On selection: POST to /api/experiment/variant to switch
- [ ] Apply new variant's CSS immediately (swap stylesheet or override custom properties)
- [ ] Persist via existing co_variant cookie
- [ ] commit: `feat(board): variant palette switcher in header`
