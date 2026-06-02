## CO-333 — wire the visitor-facing feedback widget into the SPA

CO-333 shipped the feedback widget module (`feedback-widget.js`) but `app.js` only loaded `feedback-panel.js` (the owner-side in-locus review badge). The visitor-facing floating button was unreachable — orphan-import pattern, same shape as CO-311's platforms.js bug.

### Fix
Add a dynamic `import('./modules/feedback-widget.js')` in `app.js`. The widget self-initializes on module load (mounts the bottom-left floating button + attaches to `window.CoFeedbackWidget`), so the import alone wires it.

### Effect
- Anonymous + authenticated users now see the feedback button on every page
- Click → modal → submit → POST `/api/v1/feedback` (CO-333's existing endpoint)
- Owner-side in-locus badge (already wired via `feedback-panel.js`) unchanged
