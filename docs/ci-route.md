# CO-382: CI/CD Route — Deterministic 11-Step Pipeline

Every PR follows a fixed 11-step route. Steps 1–6, 10, and 11 block merge.
Steps 7–9 run post-merge and block the Thursday release.

## The 10-Step Route

```
PR opened
 │
 ├── Step 1: cargo fmt --check         (ci.yml → test job)
 ├── Step 2: cargo clippy -D warnings  (ci.yml → test job)
 ├── Step 3: cargo test --workspace    (ci.yml → test job)
 ├── Step 4: openapi:check             (openapi-check.yml — PRs touching routes/spec)
 ├── Step 5: e2e local (Playwright)    (ci.yml → e2e job)
 ├── Step 6: migration validation      (pr-route.yml — only when migration file present)
 ├── Step 11: security audit           (pr-route.yml — skips drafts/docs/reverts)
 │
 ├── All green? → Mergeable
 │
 └── Merged to main
      │
      ├── Step 7: Deploy to staging    (staging-deploy.yml)
      ├── Step 8: Contract probe       (staging-suite.yml)
      └── Step 9: E2E staging suite    (staging-suite.yml)

Thursday 14:00 BRT
 │
 ├── Step 10: DoD verification (wave)  (release-gate.yml + pr-route.yml per PR)
 ├── Step 11: Security findings gate   (release-gate.yml — blocks if Critical/High unresolved)
 └── Sprint review commit              (release-gate.yml → scrum/sprint-review.ts)
```

## Step Details

| Step | Workflow | Blocks Merge? | Details |
|------|----------|--------------|---------|
| 1. cargo fmt | `ci.yml` | ✅ | `cargo fmt --all -- --check` |
| 2. cargo clippy | `ci.yml` | ✅ | `cargo clippy --workspace -- -D warnings` |
| 3. cargo test | `ci.yml` | ✅ | `cargo test --workspace` |
| 4. OpenAPI check | `openapi-check.yml` | ✅ | Only on route/spec file changes |
| 5. E2E local | `ci.yml` | ✅ | Playwright chromium-desktop against ephemeral server |
| 6. Migration validation | `pr-route.yml` | ✅ | Conditional: triggers only if `co-web/src/db/migrations/v*.sql` changed |
| 7. Staging deploy | `staging-deploy.yml` | ❌ (post-merge) | `flyctl deploy --config fly.staging.toml` |
| 8. Contract probe | `staging-suite.yml` | ❌ (post-merge) | Health + key endpoint smoke tests vs staging |
| 9. E2E staging | `staging-suite.yml` | ❌ (post-merge) | Full Playwright suite vs `staging.co.artelonga.com.br` |
| 10. DoD verification | `pr-route.yml` + `release-gate.yml` | ✅ (per PR) + blocks release (wave) | Parses `## Acceptance` from `work/co/CO-N.md`, maps to test patterns |
| 11. Security audit | `pr-route.yml` | ✅ (Critical/High findings) | LocalGrepBackend default; Claude backend optional; skips drafts/docs/reverts |

## Workflow Files

| File | Trigger | Steps |
|------|---------|-------|
| `.github/workflows/ci.yml` | push/PR to main | 1, 2, 3, 5 |
| `.github/workflows/openapi-check.yml` | PR touching routes/spec | 4 |
| `.github/workflows/pr-route.yml` | PR to main | 6, 10 |
| `.github/workflows/staging-deploy.yml` | push to main | 7 |
| `.github/workflows/staging-suite.yml` | push to main | 8, 9 |
| `.github/workflows/release-gate.yml` | Thursday 14:00 BRT (cron) | 10 (wave), 11 (security gate), sprint review |

## DoD Verification (Step 10)

DoD verification is run per-PR by `scripts/dod/verify.ts` and aggregated
per-wave by `release-gate.yml` on Thursday.

### How it works

1. Read `work/co/CO-N.md` (spec for the task)
2. Parse the `## Acceptance` checklist (every `- [ ]` line)
3. Derive a regex pattern from each item's text
4. Search `co-web/e2e/` and `co-web/e2e-generated/` for matching test names
5. A match with `test.fixme()` → ❌ (stub, not implemented)
6. A match without `test.fixme()` → ✅
7. No match → ❌
8. Post a DoD table as a PR comment
9. Exit non-zero (blocking merge) only on `blocking_failures > 0` — a matched real test failing. Stub/no-match items are ⚠️ *pending* (advisory)
10. Save JSON report to `docs/scrum/dod/CO-N.json`

### Stub generation

When a spec has no tests yet, generate stubs with:

```bash
cd co-web
npm run dod:verify -- --spec CO-N --generate-stubs
```

This creates `co-web/e2e-generated/CO-N-dod.spec.ts` with `test.fixme()` placeholders.
Replace each `// TODO: implement` + `test.fixme()` with a real test to satisfy the item.

### Running locally

```bash
cd co-web

# Verify a spec
npm run dod:verify -- --spec CO-382

# Generate stubs for a new spec
npm run dod:verify -- --spec CO-N --generate-stubs

# Verify + post PR comment (requires GH_TOKEN, PR_NUMBER, REPO env vars)
npm run dod:verify -- --spec CO-N --post-comment
```

## Sprint Review (Thursday 14:30 BRT)

The sprint review is auto-generated 30 minutes before the release window:

```bash
cd co-web
npm run scrum:sprint-review
```

Reads `docs/scrum/dod/*.json` + git history → commits
`docs/scrum/sprints/sprint-<N>.md`.

## Release Gate (Thursday 15:00 BRT)

The operator runs `scripts/release-commit.sh <VERSION> <THEME>` to cut the release.
Before building the changelog, the script checks `docs/scrum/dod/CO-N.json` for
every pending task (files in `CHANGELOG-PENDING/`).

If any task has `blocking_failures > 0` (or no DoD report at all), the release is **blocked**. Pending items (stubs / unmapped) are advisory and reported but do not block.

**Emergency override** (hotfixes only — logged in commit):

```bash
scripts/release-commit.sh 3.0.1 hotfix --ignore-dod
```

The override flag appends `[--ignore-dod override]` to the release theme so the
deviation is visible in the git log and CHANGELOG.

## CI Event Bus (CO-380)

CI workflows publish events to the CO event bus for live timeline visibility:

| Event | Published by | Live icon |
|-------|-------------|-----------|
| `ci.step.passed` | CI steps | 🛠️ |
| `ci.step.failed` | CI steps | 🛠️ |
| `ci.dod.verified` | `pr-route.yml` | 🛠️ |
| `ci.dod.failed` | `pr-route.yml` | 🛠️ |
| `release.gate.passed` | `release-gate.yml` | 🚀 |
| `release.gate.blocked` | `release-gate.yml` | 🚫 |

Visible at `/agora` (pt-BR) and `/live` (en) live timeline.
All CI events have `visibility: System` — only visible to admins.
