## CO-280 Phase 1 — sidebar restructure (Platforms / This universe / Tools)

Restructure the SPA sidebar into three labeled sections so users can immediately
distinguish the three IA layers that were previously rendered as one flat list:

1. **Platforms** (top) — hardcoded list of the 5 sister deployable units
   (co, artelonga, quilombo, yggdrasil, rfq). External-link icon shown when
   a platform's URL differs from `window.location.origin`; click opens that
   platform in a new tab.
2. **This universe** (middle) — the existing universe + project nav, now
   under a clearly labeled section header ("Este universo" / "This universe").
   Behavior preserved; only the surrounding header changes.
3. **Tools** (bottom, muted) — dev/operator affordances (Deployments,
   Changelog). Visually de-emphasized so they read as operator tools rather
   than end-user destinations.

### Why

Two user-reported symptoms shared the same root cause — the sidebar mixed three
distinct IA layers (deployable platforms, content universes, dev tools) with no
visual distinction:

- "5 sub-universes part of whole, clarify and review" — sister deployables
  rendered identically to projects inside the current universe.
- "sidebar.co_dev_ship button is weird" — dev/operator affordances sat
  alongside end-user navigation with no signal of their audience.

Phase 1 introduces the three-section scaffolding so future phases (breadcrumbs,
sub-universe tree, individual tool audits) have a stable home. CO-277's
recursive sub-universe tree (Phase 4) and Phase 2's breadcrumbs are deferred to
follow-up tickets.

### Files

- `co-web/static/variants/a/index.html` — three section containers in the
  sidebar (`#sidebar-platforms-section`, `.sidebar-this-universe`,
  `#sidebar-tools-section`).
- `co-web/static/variants/a/modules/sidebar/platforms.js` — new module,
  hardcoded `PLATFORMS` list + `renderPlatforms()`.
- `co-web/static/variants/a/modules/sidebar/tools.js` — new module,
  `renderTools()` with deployments + changelog links.
- `co-web/static/variants/a/modules/sidebar/render.js` — calls
  `renderPlatforms()` + `renderTools()` from `renderSidebar()`.
- `co-web/static/variants/a/modules/sidebar/index.js` — public re-exports.
- `co-web/static/variants/a/style.css` — `.sidebar-platforms`,
  `.sidebar-tools`, `.sidebar-platform-item`, `.sidebar-tool-item` styling.
- `co-web/static/shared/i18n.js` — pt + en keys for the three section labels
  and tool labels.
- `co-web/e2e/co-280-sidebar-sections.spec.ts` — asserts all three sections
  render and that tool items never leak into `#project-list`.
