# CO-388: Security Audit Pipeline — 11-Step CI Route Extension

CO adopts **Project Glasswing** posture: defenders ahead of attackers via early
integration of automated vulnerability scanning. This document describes step 11
of the deterministic CI route (CO-382) and how findings flow through CO's own
infrastructure (event bus → sprint board → release gate).

## The 11-Step CI Route

```
PR opened
 │
 ├── Step 1: cargo fmt --check         (ci.yml → test job)
 ├── Step 2: cargo clippy -D warnings  (ci.yml → test job)
 ├── Step 3: cargo test --workspace    (ci.yml → test job)
 ├── Step 4: openapi:check             (openapi-check.yml)
 ├── Step 5: e2e local (Playwright)    (ci.yml → e2e job)
 ├── Step 6: migration validation      (pr-route.yml — conditional)
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
 ├── Step 10: DoD verification (wave)  (release-gate.yml)
 ├── Step 11: Security audit gate      (release-gate.yml)
 └── Sprint review commit              (release-gate.yml)
```

## Step 11: Security Audit

### Per-PR (pr-route.yml `security-audit` job)

Triggers on every PR to `main`, unless:
- Draft PR → skip
- Docs-only PR (all changed files are `.md`) → skip
- Revert PR (title starts with "Revert") → skip

**Backend selection** (via `CO_SECURITY_BACKEND` env var):

| Value | Backend | When |
|-------|---------|------|
| unset | `LocalGrepBackend` | Default — always available |
| `claude` | `ClaudeSecurityBackend` | Requires `CO_SECURITY_API_KEY` |
| `disabled` | `NoOpBackend` | Dev/test only |

**Finding severity routing:**

| Severity | Action |
|----------|--------|
| Critical | Block merge + release-blocker PBI + 🚨 /agora alert |
| High | Block merge + release-blocker PBI + 🚨 /agora alert |
| Medium | Queue as next-sprint PBI (advisory, does not block merge) |
| Low | Log to atividades only |
| Info | Log to atividades only |

### Per-Wave (release-gate.yml Thursday 14:00 BRT)

The release gate checks `docs/scrum/security/security-audit-<PR>.json` for every
PR in the wave. If any file has `blocker_count > 0` with unresolved findings,
the release is blocked.

**Emergency override** (hotfix only — logged + alerted):

```yaml
# GitHub Actions → workflow_dispatch
ignore_security_findings: true
```

This appends a `[--ignore-security-findings override]` note to the audit trail
and publishes a `security.override_activated` event to the EDA bus.

## Architecture

### Event flow

```
scan_diff / scan_full
  → Finding detected
  → publish "security.finding_detected" (Visibility::System)
      ├── FindingsPersistor → security_findings table (DB)
      ├── PBIBacklogger     → work/co/security/SEC-<id>.md (if severity ≥ Medium)
      ├── ReleaseBlocker    → log error + "security.release_blocked" (if severity ≥ High)
      └── AtividadesPersistor → event_log (all events)
```

### Database schema (v71)

```sql
CREATE TABLE security_findings (
    id              TEXT PRIMARY KEY,           -- ULID
    pr_number       INTEGER NOT NULL,
    severity        TEXT NOT NULL CHECK (severity IN ('critical','high','medium','low','info')),
    category        TEXT NOT NULL,
    file_path       TEXT NOT NULL,
    line_start      INTEGER,
    line_end        INTEGER,
    description     TEXT NOT NULL,
    cwe             TEXT,
    cve_match       TEXT,
    suggested_patch TEXT,
    detected_at     TEXT NOT NULL,
    resolved_at     TEXT,
    resolution_kind TEXT CHECK (resolution_kind IS NULL OR
                                resolution_kind IN ('patched','accepted-risk','false-positive','wont-fix')),
    resolution_pr   INTEGER
);
```

### Admin API

All endpoints require GitHub admin auth (`CO_GESTAO_GITHUB_ADMINS`).

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/gestao/security/findings` | List findings (filterable by severity/resolved) |
| `GET` | `/api/v1/gestao/security/findings/:id` | Get a single finding |
| `PATCH` | `/api/v1/gestao/security/findings/:id` | Resolve a finding |
| `GET` | `/api/v1/gestao/security/scan/status` | Backend info + unresolved counts |
| `POST` | `/api/v1/gestao/security/scan` | Trigger a manual scan |

**Resolve a finding:**

```bash
curl -X PATCH https://co-artelonga.fly.dev/api/v1/gestao/security/findings/01J... \
  -H 'Authorization: Bearer <github-token>' \
  -H 'Content-Type: application/json' \
  -d '{"resolution_kind":"patched","resolution_pr":195}'
```

**Resolution kinds:** `patched` | `accepted-risk` | `false-positive` | `wont-fix`

## Cost Guardrails (Claude Security backend)

When `CO_SECURITY_BACKEND=claude`:

| Guardrail | Control |
|-----------|---------|
| Daily scan limit | `CO_SECURITY_MAX_SCANS_PER_DAY` (default: 50) |
| Skip drafts | Enforced in CI workflow |
| Skip docs-only PRs | Enforced in CI workflow |
| Skip revert PRs | Enforced in CI workflow |
| Cache by file hash | SHA-256 keyed in-memory, short-circuits unchanged files |

## Telemetry (via CO-380 EDA bus)

All security events have `Visibility::System` — only visible to admins.

| Event | When |
|-------|------|
| `security.finding_detected` | Every new finding |
| `security.release_blocked` | Critical/High finding detected |
| `security.finding_resolved` | Finding marked resolved |
| `security.override_activated` | `--ignore-security-findings` used |

Visible at `/agora` (pt-BR) and `/live` (en) for authenticated admins.

## Priority Scan Surfaces

TypeScript paths are the primary XSS attack surface:

- All vault write paths: `/api/v1/universes/*/vault/*`
- All auth flows: `/api/v1/auth/*`
- All WebSocket handlers: `/api/v1/events/*`
- All markdown render paths (innerHTML / dangerouslySetInnerHTML)

## Disclosure Flow

See [`docs/security-disclosure.md`](security-disclosure.md) for the manual
disclosure process for Critical findings.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CO_SECURITY_BACKEND` | `local-grep` | `local-grep` \| `claude` \| `disabled` |
| `CO_SECURITY_API_KEY` | — | API key when using Claude backend |
| `CO_SECURITY_MAX_SCANS_PER_DAY` | `50` | Daily scan cap (Claude backend only) |
