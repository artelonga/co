# E2E Test Layer — co-web

## Layer overview

| Layer | Location | Tool | Wall-clock target | Run command |
|-------|----------|------|--------------------|-------------|
| Lib (unit/integration) | `co-web/tests/` | `cargo test` | < 30 s | `scripts/co-test lib` |
| Component | `co-web/components/__tests__/` | Vitest + happy-dom | < 10 s | `scripts/co-test components` |
| E2E per-feature | `co-web/e2e/*.spec.ts` | Playwright | < 3 min (local) | `scripts/co-test e2e` |
| Smoke | `co-web/e2e/smoke.spec.ts` | Playwright | < 1 min | `scripts/co-test smoke` |

## When to add each type

**Component test** (Vitest, no browser, no server):
- DOM logic: view-tab switching, search filtering, sidebar toggle, keyboard shortcuts
- Pure UI state: theme switcher, i18n label swap, empty-state visibility
- Anything that can be tested with `document.createElement` + event dispatch

**E2E test** (Playwright, real browser + real server):
- Auth flow end-to-end: session cookie, protected routes
- Full CRUD round-trip: API mutation → DOM update verification
- Cross-view consistency: same task visible in kanban + table + timeline
- Network behavior: HTTP status codes, cookie headers

## File naming convention

| Pattern | Layer | Example |
|---------|-------|---------|
| `e2e/smoke.spec.ts` | Smoke (always runs) | Health check + basic page load |
| `e2e/<feature>.spec.ts` | E2E per-feature | `board-ux.spec.ts`, `auth.spec.ts` |
| `e2e/archived/<name>.spec.ts` | Archived (not run) | `auth-crdt.spec.ts` |
| `components/__tests__/<component>.test.ts` | Component | `view-tabs.test.ts` |

## Running tests locally

```bash
# Quick smoke check (auto-starts co-web)
scripts/co-test smoke

# Full e2e suite (auto-starts co-web on port 54321)
scripts/co-test e2e

# Only files changed since main
scripts/co-test e2e --since main

# Component tests (no server needed)
scripts/co-test components

# Rust lib tests
scripts/co-test lib

# Review test counts + suspected redundancy
scripts/co-test review

# Fail CI if any spec file exceeds 30-test limit
scripts/co-test review --fail-on-bloat
```

## Guard rails

- **30-test limit per spec file**: `scripts/co-test review --fail-on-bloat` exits 1 if any `*.spec.ts` exceeds this. CI prints a warning but does not hard-fail (PR authors should check locally).
- **Archived specs**: moved to `e2e/archived/` when a feature is fully shipped and its acceptance proof is no longer needed for regression. Playwright's `testIgnore` excludes `archived/`, `wave-2/`, and `interactions/` from all runs.
- **Test manifest**: `tests/manifest.yaml` lists all spec files for tracking.

## Archived specs

Files in `e2e/archived/` proved a feature shipped and are kept for reference. They are **not run** by CI or `scripts/co-test`. To restore a spec, move it back to `e2e/`.

## Auth fixtures — migration path (CO-303)

The `apiContext` fixture in `fixtures.ts` currently authenticates via
`POST /api/v1/auth/uat-login` — a hidden backdoor only available when
`CO_ENV=uat|test`. This covers the happy-auth path in CI but exercises a code
path real users never touch.

After CO-303, the server returns `dev_code` inline in
`POST /api/v1/auth/onboard-with-email` responses when `CO_ENV=test`. Migrating
fixtures to this path would give e2e full coverage on the production login flow:

```ts
// Future: higher-fidelity auth fixture (CO-303-B)
const sendRes = await ctx.post('/api/v1/auth/onboard-with-email', {
  data: { email: 'yuri@uat.local' },
});
const { dev_code } = await sendRes.json();
await ctx.post('/api/v1/auth/onboard-with-email/verify', {
  data: { email: 'yuri@uat.local', code: dev_code },
});
```

This migration is tracked as **CO-303-B**. The current `uat-login` fixture is
kept for backwards compatibility until that follow-up ships.
