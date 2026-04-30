## Development Conventions (CLAUDE.md)

# Claude Development Instructions for CO

This file contains instructions for Claude instances working on the CO project.

## Project Overview

**CO** is a CLI tool for graph-based content management, built in Rust.

- **Repository:** `institutional-pointset/co`
- **Current Version:** Check `Cargo.toml` for latest
- **Stack:** Rust, clap, SQLite (rusqlite), serde

## Before Starting Work

### 1. Fetch Latest State

```bash
# Update main branch
git checkout main && git pull origin main

# Check current version
cargo run -- --version

# List open issues
gh issue list --state open --limit 20

# View specific issue with comments
gh issue view <NUMBER> --comments
```

### 2. Understand the Issue

- Read the issue description and acceptance criteria carefully
- Check linked issues and PRs for context
- Review comments for additional requirements or decisions

## Development Workflow

### Issue-Driven Development

**All changes require an issue.** No PR without a linked issue.

1. Pick an issue from the backlog
2. Create a branch: `git checkout -b <type>/issue-<number>-<description>`
3. Implement following TDD (see below)
4. **Include version bump in the same PR** (see Versioning below)
5. Create PR referencing the issue with `Closes #<n>`
6. Merge and clean up branches

### Branch Naming

| Type | Pattern | Example |
|------|---------|---------|
| Feature | `feat/issue-<n>-<desc>` | `feat/issue-48-collab-content` |
| Fix | `fix/issue-<n>-<desc>` | `fix/issue-54-version-bump` |
| Docs | `docs/issue-<n>-<desc>` | `docs/issue-42-readme` |
| Refactor | `refactor/issue-<n>-<desc>` | `refactor/issue-49-terminology` |

### After Merge - Cleanup

**Always clean up after merge:**

```bash
git checkout main && git pull origin main
git branch -d <branch-name>
git push origin --delete <branch-name>
```

## Versioning Policy (#53)

**Version bump happens IN the issue PR.** One issue = one PR = one version bump.

See **Issue Labels & Git Mapping** below for version bump rules per label type.

### Version Bump (in same PR)

Before creating the PR, include these changes:

1. Update `Cargo.toml` (workspace version)
2. Update `co-cli/Cargo.toml` (version field)
3. Add entry to `CHANGELOG.md`

**Do NOT create separate issues/PRs for version bumps.**

## TDD: Red-Green-Refactor

**IMPORTANT:** Follow strict TDD to avoid technical debt.

### 1. RED - Write Failing Test First

```rust
#[test]
fn test_new_feature_behavior() {
    // Arrange
    let input = setup_test_data();

    // Act
    let result = new_feature(input);

    // Assert
    assert_eq!(result, expected_output);
}
```

Run: `cargo test` - should FAIL

### 2. GREEN - Minimal Implementation

Write the **minimum code** to make the test pass. No extras.

Run: `cargo test` - should PASS

### 3. REFACTOR - Clean Up (Critical!)

**Do not skip this step.** Before committing:

- [ ] Remove duplication
- [ ] Improve naming (variables, functions)
- [ ] Extract helper functions if needed
- [ ] Ensure code follows existing patterns
- [ ] Run `cargo fmt` and `cargo clippy -- -D warnings`
- [ ] All tests still pass

### Test Commands

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture

# Check formatting and lints
cargo fmt && cargo clippy -- -D warnings
```

## Code Quality Checklist

Before creating a PR:

- [ ] All tests pass: `cargo test`
- [ ] No clippy warnings: `cargo clippy -- -D warnings`
- [ ] Code formatted: `cargo fmt`
- [ ] No debug prints or commented code
- [ ] CHANGELOG.md updated (for version bumps)
- [ ] PR references issue number

## Project Structure

```
co/
├── core/                 # Library crate
│   └── src/
│       ├── lib.rs        # Public API
│       ├── config/       # Configuration handling
│       ├── content.rs    # Content parsing
│       ├── feature/      # Feature system, schema
│       ├── validate.rs   # Validation logic
│       └── ...
├── co-cli/               # Binary crate
│   └── src/
│       ├── main.rs       # CLI entry point
│       ├── commands/     # Command implementations
│       └── i18n/         # Internationalization
├── agents/               # Agent definitions
├── tools/                # Tool definitions
├── work/                 # Work item schemas
├── CHANGELOG.md          # Version history
└── Cargo.toml            # Workspace config
```

## Key Commands Reference

```bash
# Content management
co init <name>           # Create new space
co new <type> <name>     # Create content
co show <item>           # Display content
co validate all          # Validate workspace

# Query
co locate                # Search content
co locate --type task    # Filter by type

# Spaces
co space list            # List spaces
co space current         # Current space

# Development
co schema list           # List content types
co config show           # Show configuration
```

## Common Patterns

### Adding a New Command

1. Add subcommand to `co-cli/src/main.rs`
2. Create handler in `co-cli/src/commands/<name>.rs`
3. Add `pub mod <name>;` to `co-cli/src/commands/mod.rs`
4. Write tests in `co-cli/tests/cli/<name>.rs`

### Adding a New Content Type

1. Add to `work/schema.yaml` or create new feature schema
2. Register in feature registry if needed
3. Add validation rules if structured

### Modifying Validation

1. Update `core/src/validate.rs`
2. Add/update tests in same file
3. Consider backward compatibility

## Terminology

| Term | Definition |
|------|------------|
| **Space** | Project namespace directory (`private/`, `work/`, `public/`) |
| **Context** | User-provided content/prompts |

## Work Item Types

CO uses structured work items for development tracking:

| Type | Purpose | Format |
|------|---------|--------|
| **user-story** | Detailed request (feature or fix) | `AS A <role>, I NEED <feature>, SO THAT <benefit>` |
| **task** | Sub-item within a user-story | `GIVEN <context>, WHEN <action>, THEN <result>` |
| **epic** | Large feature grouping | Collection of related user-stories |
| **release** | Version milestone | Groups completed work items |

### Hierarchy

```
epic
└── user-story (type:feat or type:fix)
    ├── task (commit 1)
    ├── task (commit 2)
    └── task (commit 3)
```

A **user-story** becomes a GitHub issue with either `type:feat` or `type:fix` label.
**Tasks** are implemented as commits within the user-story's PR.

## Issue Labels & Git Mapping

Labels drive branch naming, commit prefixes, and version bumps:

| Label | Branch Prefix | Commit Prefix | Version Bump |
|-------|---------------|---------------|--------------|
| `type:feat` | `feat/issue-<n>-...` | `feat:` | Minor (x.**Y**.0) |
| `type:fix` | `fix/issue-<n>-...` | `fix:` | Patch (x.y.**Z**) |
| `type:docs` | `docs/issue-<n>-...` | `docs:` | Patch (x.y.**Z**) |
| `type:refactor` | `refactor/issue-<n>-...` | `refactor:` | Patch (x.y.**Z**) |
| `type:chore` | `chore/issue-<n>-...` | `chore:` | No bump |

### Module Labels

Combine with type labels to categorize by subsystem:

| Label | Subsystem |
|-------|-----------|
| `module:content` | Content types and parsing |
| `module:tools` | Tools and extensions |
| `module:writer` | Writer agent system |
| `module:collab` | GitHub/collaboration |
| `module:space` | Spaces and namespaces |

### Work Item → Git Flow

| Work Item | Git Artifact | Notes |
|-----------|--------------|-------|
| user-story | Issue → Branch → PR | Label determines type (feat/fix) |
| task | Commits within PR | One or more per user-story |
| epic | GitHub Milestone | Groups user-stories |
| release | Git tag | Semantic version (vX.Y.Z) |

## Open Issues (v1.0 Roadmap)

Check current status: `gh issue list --state open --label "milestone:v1.0"`

Remaining v1.0 work:
- #42 - Documentation Polish (current)
- #53 - Versioning Policy (implemented, close when satisfied)
- #70 - Forbidden character validation (new)

## Deployment

### Environments

| Env | App | URL | Purpose |
|-----|-----|-----|---------|
| **UAT** | `co-artelonga-uat` | `co-artelonga-uat.fly.dev` | Verify before prod. Fresh data per deploy. |
| **Production** | `co-artelonga` | `co-artelonga.fly.dev` | Public-facing. Never deploy untested changes. |

### Deploy Order: ALWAYS UAT First

```bash
# 1. Run tests locally
cargo test -p co-web
cargo clippy -p co-web -- -D warnings

# 2. Deploy to UAT
flyctl deploy --config fly.uat.toml

# 3. Verify UAT (see UAT Verification below)
curl -s https://co-artelonga-uat.fly.dev/api/health

# 4. Only after UAT passes — deploy to production
flyctl deploy
```

**Never** run `flyctl deploy` (prod) without first verifying on UAT.

### Fly.io Configuration Files

| File | Target | Notes |
|------|--------|-------|
| `fly.toml` | Production (`co-artelonga`) | Default deploy target |
| `fly.uat.toml` | UAT (`co-artelonga-uat`) | `--config fly.uat.toml` |

### Secrets (per environment)

```bash
# Set JWT_SECRET (required, one-time per env)
flyctl secrets set JWT_SECRET=$(openssl rand -base64 48) -a co-artelonga
flyctl secrets set JWT_SECRET=$(openssl rand -base64 48) -a co-artelonga-uat

# Verify
flyctl secrets list -a co-artelonga
flyctl secrets list -a co-artelonga-uat
```

### Dockerfile Notes

- Image: `rust:1.88-slim` (requires protobuf-compiler)
- Dependency cache layer: `Cargo.toml`/`Cargo.lock` → dummy build → real source build
- First deploy: ~5 min (no cache). Subsequent: ~1-2 min (deps cached).
- Runtime: `debian:bookworm-slim` with `ca-certificates` + `curl` (healthcheck)
- Non-root user `co`, data volume at `/data`

### Logs & Debugging

```bash
flyctl logs -a co-artelonga-uat --no-tail   # Recent logs
flyctl logs -a co-artelonga-uat              # Stream live
flyctl ssh console -a co-artelonga-uat       # Shell into machine
flyctl status -a co-artelonga-uat            # Machine state
```

### UAT Credentials (CO-44)

**Login: `yuri` / `uat`** — UAT only. Never use these credentials in production.

| Field | Value |
|-------|-------|
| Email | `yuri@uat.local` |
| Password | `uat` |
| Tier | `admin` |
| Endpoint | `POST /api/v1/auth/uat-login` |

```bash
# Login as yuri on UAT
curl -s -X POST https://co-artelonga-uat.fly.dev/api/v1/auth/uat-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"yuri@uat.local","password":"uat"}'
# → { user_id, email, display_name, expires_at }
```

The `uat-login` endpoint returns **404 in production** (`CO_ENV` unset). Only available on UAT.

### Password-login in Production (CO-85)

Admin users with `password_hash` set can log in via `POST /api/v1/auth/password-login` in **any environment** (including prod). This endpoint has no env gate.

```bash
# Login as admin on prod
curl -sc cookies.txt -X POST https://co-artelonga.fly.dev/api/v1/auth/password-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"yuri@artelonga.com.br","password":"<your-password>"}'
# → 200, Set-Cookie: session=<JWT>
```

The admin user is seeded at startup via env vars:
```bash
flyctl secrets set CO_SEED_ADMIN_EMAIL=yuri@artelonga.com.br \
                   CO_SEED_ADMIN_PASSWORD_HASH="$HASH" \
                   -a co-artelonga
```

Generate the hash locally:
```bash
HASH=$(printf 'mySecretPassword' | argon2 "$(openssl rand -hex 16)" -id -t 3 -m 16 -p 1 -e)
```

### UAT Database Reset

Touch the reset flag and restart the machine to wipe all non-user data:

```bash
# 1. Create reset flag (survives until machine restarts)
flyctl ssh console -a co-artelonga-uat -C "touch /data/uat-reset.flag"

# 2. Restart the machine to trigger the reset on startup
flyctl machine restart -a co-artelonga-uat

# 3. Verify reset completed (check logs)
flyctl logs -a co-artelonga-uat --no-tail | grep "UAT: reset"
```

On startup with the flag present, the server will:
1. Back up all user accounts (so yuri persists across resets)
2. Delete the SQLite database
3. Clean up anonymous universe directories
4. Run all migrations fresh
5. Restore users + re-seed template universe
6. Delete the flag file

### UAT Yuri Login Health Check

Include this in post-deploy verification:

```bash
# Verify yuri login works
RESP=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST https://co-artelonga-uat.fly.dev/api/v1/auth/uat-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"yuri@uat.local","password":"uat"}')
[ "$RESP" = "200" ] && echo "UAT login OK" || echo "UAT login FAILED ($RESP)"

# Verify co-dev board accessible (requires token from login)
TOKEN=$(curl -sc /tmp/uat-cookies.txt -X POST \
  https://co-artelonga-uat.fly.dev/api/v1/auth/uat-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"yuri@uat.local","password":"uat"}' | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('user_id',''))")
echo "Logged in as: $TOKEN"
```

## UAT Verification Spec

After every deploy to UAT, verify the following:

### 1. Health & Data Seeding

```bash
# Health check
curl -s https://co-artelonga-uat.fly.dev/api/health
# → {"status":"ok","version":"1.0.0"}

# Template universe exists
curl -s https://co-artelonga-uat.fly.dev/api/v1/universes/template | python3 -m json.tool
# → key: "template", is_template: true, name: "CO"

# Tutorial project exists
curl -s https://co-artelonga-uat.fly.dev/api/v1/universes/template/projects
# → [{ key: "CO", name: "Aprenda CO" }]

# 7 tutorial tasks
curl -s "https://co-artelonga-uat.fly.dev/api/projects/CO/tasks?u=template" | python3 -c "import sys,json; print(len(json.load(sys.stdin)), 'tasks')"
# → 7 tasks
```

### 2. Anonymous User Flow (no login)

1. Open `https://co-artelonga-uat.fly.dev` in incognito
2. Board loads in Portuguese with 7 tutorial tasks in "A fazer" column
3. Banner visible: "CO — Gestão de conteúdo em grafo"
4. Drag task → board updates (auto-clone happens silently on first load)
5. Create task via "+ Nova Tarefa" → works
6. Refresh page → board persists (same clone loaded from localStorage)
7. Create up to 100 entries → all succeed
8. Entry 101 → "Crie uma conta para continuar" modal

### 3. Theme Switching

1. Click theme dropdown in header
2. All 12 themes visible: Modern, Scholarly Light/Dark, Relic Light/Dark, Medieval, Steampunk, Cyberpunk, Matrix, Garden, Terminal, Retro
3. Switch to each theme → colors change instantly, no reload
4. Refresh → theme persists (localStorage)

### 4. Language Toggle

1. Default: Portuguese ("Projetos", "A fazer", "Concluído")
2. Click language toggle → English ("Projects", "To do", "Done")
3. Refresh → language persists (cookie)

### 5. Login Flow

```bash
# Request login code (dev: code printed to server logs)
curl -X POST https://co-artelonga-uat.fly.dev/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"email":"test@example.com"}'

# Check server logs for verification code
flyctl logs -a co-artelonga-uat --no-tail | grep "Verification code"

# Verify code → get JWT
curl -X POST https://co-artelonga-uat.fly.dev/api/auth/verify \
  -H 'Content-Type: application/json' \
  -d '{"email":"test@example.com","code":"XXXXXX"}'
# → { user_id, email, display_name, expires_at }
```

In the browser:
1. Click "Criar conta" or "Entrar"
2. Enter email → check server logs for code → enter code
3. After login: anonymous clone is claimed, user has full access
4. User badge appears in header with display name

### 6. Access Levels

| Level | Can do | How to test |
|-------|--------|-------------|
| **Anonymous (no login)** | View template, auto-clone on load, CRUD up to 100 entries, switch themes/language | Incognito window |
| **Anonymous (clone owner)** | Full CRUD on own clone, up to 100 entries | Same browser, refresh |
| **Logged-in (own universe)** | Unlimited entries, CRDT collaboration, all themes, shareable URL | Login via email code |
| **Logged-in (other's universe)** | View only (if public) | Visit another user's `/co/{slug}` |
| **Admin** | Gestão API access | `GESTAO_GITHUB_ADMINS` env var |

### 7. Database State (SQLite)

```bash
# SSH into UAT machine
flyctl ssh console -a co-artelonga-uat

# Check database
sqlite3 /data/co.db

# Tables
.tables
# → entries, projects, schema_version, tasks, universe_members, universes, users, ...

# Schema version
SELECT MAX(version) FROM schema_version;
# → 13

# Universes
SELECT key, name, is_template, is_public, content_count FROM universes;
# → template | CO | 1 | 1 | 0
# → u-xxxxx | Meu CO | 0 | 0 | N  (anonymous clones)

# Entries (template)
SELECT path, entry_type, title FROM entries WHERE universe_key = 'template' LIMIT 5;
# → projects/CO/_project.md | project | Aprenda CO
# → projects/CO/1.md | task | Arraste esta tarefa...

# Users (after login test)
SELECT id, email, display_name, tier FROM users;
```

### 8. Entry Abstraction

```bash
# Entries API
curl -s "https://co-artelonga-uat.fly.dev/api/v1/universes/template/entries?type=task" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['total'], 'entries')"
# → 7 entries

# Tags
curl -s "https://co-artelonga-uat.fly.dev/api/v1/universes/template/entries/tags"
# → [{"tag":"tutorial","count":7}, ...]

# Tree
curl -s "https://co-artelonga-uat.fly.dev/api/v1/universes/template/entries/tree?type=task"
# → Hierarchical JSON with parent/child nesting
```

### 9. Vault API (Obsidian Compat)

```bash
# Requires API token (login first, then generate)
TOKEN="..." # from POST /api/v1/auth/token

curl -H "Authorization: Bearer $TOKEN" \
  "https://co-artelonga-uat.fly.dev/api/v1/universes/template/vault/notes"
# → File listing

curl -H "Authorization: Bearer $TOKEN" \
  "https://co-artelonga-uat.fly.dev/api/v1/universes/template/vault/tags"
# → Tag counts
```

### 10. E2E Tests (Playwright)

```bash
cd co-web

# Run against UAT
BASE_URL=https://co-artelonga-uat.fly.dev npx playwright test

# Run specific suite
BASE_URL=https://co-artelonga-uat.fly.dev npx playwright test e2e/smoke.spec.ts
BASE_URL=https://co-artelonga-uat.fly.dev npx playwright test e2e/universe.spec.ts
```

## Getting Help

- Review existing code patterns before implementing new features
- Check closed PRs for similar implementations
- Use `gh issue view <n> --comments` for discussion context


---

## Current Task: CO-137 — Investigate why migration v22 didn't apply on prod + harden ALTER ADD COLUMN against partial-application

---
id: 137
title: "Investigate why migration v22 didn't apply on prod + harden ALTER ADD COLUMN against partial-application"
type: user-story
status: todo
priority: high
labels:
  - ops
  - storage
  - migrations
  - hotfix-followup
module: co-web
created_at: 2026-04-30T00:00:00Z
updated_at: 2026-04-30T00:00:00Z
---

## As

a CO operator who shipped 1.22.0 → 1.22.3 and watched prod return 404 for every universe

## I Need

a diagnostic pass on **why migration v22 didn't add the `parent_key` column to prod's `universes` table** — despite the binary booting cleanly at 1.22.0, 1.22.3, and 1.22.4 — plus a **structural fix** to the migration framework so future ALTER-ADD-COLUMN migrations can't silently no-op the same way

## To

close out the prod incident from 2026-04-30 with root cause understood (not just papered over by 1.22.4's defensive read path), and prevent the next migration from leaving prod in an inconsistent state

## Given / When / Then

GIVEN 1.22.4 ships a defensive `get_universe` that tolerates a missing `parent_key` column,
AND prod's API works again but `parent_key=None` on the timeline trio (UAT shows `parent_key="template"`),
AND the boot logs on prod show quilombo + yggdrasil being **re-seeded** every boot since 1.22.0 (because their `*_exists` checks call `get_universe`, which was returning None due to the missing column),
WHEN we (a) confirm via direct schema inspection whether `parent_key` exists on prod, (b) confirm whether `schema_version` rows exist for v21 and v22, and (c) replace ALTER-ADD-COLUMN with a `pragma_table_info`-guarded helper,
THEN we know exactly why v22 didn't take, and the same failure mode is structurally impossible for the next migration.

## Diagnostic plan (do first, before changing migrations)

1. **Confirm column presence on prod.** Add a temporary admin-only endpoint `GET /api/v1/admin/_schema_check` (or use a one-shot Fly machine running a debug build) that runs:
   ```rust
   // returns Vec<(cid, name, type, notnull, dflt, pk)> for the universes table
   conn.prepare("SELECT cid, name, type, [notnull], dflt_value, pk FROM pragma_table_info('universes')")
       .query_map([], …)
   ```
   Compare prod vs UAT. Pay attention to `parent_key` and `git_*` columns (CO-50, v21).

2. **Confirm schema_version rows.** Same endpoint, additionally:
   ```rust
   conn.prepare("SELECT version FROM schema_version ORDER BY version").query_map([], …)
   ```
   Expected on a healthy DB: a sequence ending at 22.

3. **Cross-reference with boot logs.** `flyctl logs -a co-artelonga --no-tail` for the 1.22.0 deploy timestamp. Look for any SQLite error or panic that the binary swallowed before reaching `Project Board http://localhost:3000`.

## Working hypotheses (rank by likely)

1. **Migration v22 silently failed at INSERT INTO schema_version.** Maybe a write contention or pre-existing `version=22` row from some manual touch — `execute_batch` would then succeed up to ALTER but fail at INSERT. Result: column exists, but the v22 block tries to run again on every boot, hits "duplicate column name", panics, restarts. Contradicted by the binary running cleanly — but worth verifying.
2. **Migration v22 ran but in a transaction that got rolled back.** Unlikely with `execute_batch` (no implicit transaction in SQLite by default), but worth checking WAL.
3. **Prod's DB file is on a different volume snapshot than expected.** Some Fly volume restore that left it at an older state. Should be visible via `flyctl volumes list`.
4. **(Least likely) some `let _ = ...` swallowed the migration failure.** Audit `run_migrations` for any place where a `let _ = self.conn.execute_batch(...)` pattern exists. If one is found, that's the bug.

## Structural fix (after diagnosis)

Replace direct `ALTER TABLE … ADD COLUMN` calls with a helper that:

```rust
fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_def: &str,  // e.g. "TEXT" or "TEXT NOT NULL DEFAULT 'main'"
) -> rusqlite::Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
        params![table, column],
        |_| Ok(true),
    ).optional()?.unwrap_or(false);
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {column_def};"))?;
        Ok(true)  // added
    } else {
        Ok(false)  // already present, no-op
    }
}
```

Then v22 (and every future column-add migration) becomes:

```rust
ensure_column(&self.conn, "universes", "parent_key", "TEXT").expect("...");
self.conn.execute_batch("CREATE INDEX IF NOT EXISTS …").expect("...");
self.conn.execute("INSERT OR IGNORE INTO schema_version (version) VALUES (22)", []).expect("...");
```

This makes ALTER-ADD-COLUMN structurally idempotent — re-running a partially-applied migration recovers cleanly instead of panicking on "duplicate column name."

Apply the same pattern retroactively to v17–v21 if the diagnostic shows any of them are also partial.

## Acceptance

- [ ] Diagnostic endpoint or script confirms exact prod schema state for `universes` (column list + schema_version rows)
- [ ] Root cause documented — written into `feedback_migration_column_reads.md` + this ticket's resolution
- [ ] `ensure_column` helper exists in `co-web/src/storage.rs` and is unit-tested
- [ ] v22 (and any future column-add migration) uses `ensure_column`
- [ ] Backfill: prod's universes table has `parent_key` column after this ticket ships, and the timeline trio shows `parent_key="template"` in `GET /api/v1/universes/tempo`
- [ ] Quilombo + yggdrasil seeders no longer log `Seeding …` on every boot (confirms `*_exists` checks are working again)

## Out of scope

- Cross-region replication / LiteFS readiness (CO-77 territory)
- Migrating to a different storage engine (Postgres, etc.)
- Schema versioning tool changes beyond the ALTER-ADD-COLUMN helper

## Related

- `feedback_migration_column_reads.md` — the lesson learned saved post-incident
- 1.22.4 commit — defensive read path that bridges this gap
- Ticket-spec for CO-77 — per-universe SQLite (changes the migration model significantly; this ticket should land before CO-77)


---

## Project Configuration

```yaml
name: CO Platform
key: CO
description: CO open source platform — board UI, API, CLI, desktop apps
created_at: 2026-04-01T00:00:00Z
next_id: 140

```

---

## Roadmap

# CO Platform — Execution Roadmap

## Phase 1–2: Board (done)

1–7. CO-2..CO-8: Board API + UI overhaul ✅

## Phase 3: Public MVP — artelonga.com.br/co (Epic: CO-20)

### 3a: Core architecture
8. CO-21: Universe CRUD API (slug routing, create, clone, delete) ✅
9. CO-36: **Entry abstraction** (every entity = .md file, SQLite = index)
10. CO-24: Content/form separation (universe config → presentation, entries → content)

### 3b: Platform features
11. CO-23: Usage gate (100 entries free, then account required)
12. CO-25: Theme gating (Scholarly + Relic free, full set for logged-in)
13. CO-30: Dynamic CSS engine (runtime token generation)

### 3c: Editor & collaboration
14. CO-29: CodeMirror 6 editor (open to all, no login)
15. CO-31: CRDT sync (Yjs + WebSocket, login required + sharing gate)

### 3d: Frontend & i18n
16. CO-26: Web UI i18n (pt-BR / en toggle)
17. CO-22: Template universe (seed data, read-only, "Criar universo" CTA) ✅
18. CO-27: Landing page at /co (hero, login, criar universo)

### 3e: Deploy & quality
19. CO-32: Ansible deployment (provision, deploy, backup)
20. CO-33: E2E test suite (Playwright, full MVP flow)

### 3f: Release
21. CO-28: Open source repo setup (LICENSE, README, CI, Docker)

## Phase 4: Obsidian Ecosystem (v1.1)

22. CO-35: Vault REST API + Clipper support (file CRUD, search, clipper paste)
23. CO-34: Obsidian plugin (sync universe ↔ vault, wikilinks, community submission)

## Dependencies — execution order

```
CO-21 (universe CRUD) ✅
  └── CO-36 (entry abstraction)       ← CRITICAL: new foundation
        ├── CO-24 (content/form)      ← depends on entries
        │     ├── CO-25 (theme gate)
        │     └── CO-30 (dynamic CSS)
        ├── CO-23 (usage gate)        ← counts entries, not table rows
        └── CO-22 (template) ✅
CO-29 (CodeMirror)                    ← independent
  └── CO-31 (CRDT)                    ← after CO-29 + CO-36
CO-26 (i18n)                          ← independent
CO-27 (landing page)                  ← after CO-22 + CO-26
CO-32 (Ansible)                       ← independent
CO-33 (E2E tests)                     ← after all features
CO-28 (OSS release)                   ← last MVP task
  └── CO-35 (vault API)              ← post-MVP
        └── CO-34 (Obsidian plugin)
```

### Parallel execution groups for co auto
- **Group 1:** CO-36 (entry abstraction — critical path, builds on CO-21)
- **Group 2:** CO-24, CO-23, CO-29, CO-26 (after CO-36, except CO-29/CO-26 which are independent)
- **Group 3:** CO-25, CO-30, CO-31 (depend on group 2)
- **Group 4:** CO-27, CO-32 (depend on group 3)
- **Group 5:** CO-33 (E2E, needs everything)
- **Group 6:** CO-28 (release, last MVP)
- **Group 7:** CO-35 → CO-34 (Obsidian)
- **Group 8:** CO-37 (design alignment + Obsidian Tasks compat + v1.0 release tag)
- **Group 9:** CO-38 (Yggdrasil RPG universe)

## Phase 5: Polish, Telemetry, UAT (post-v1.0)

| ID | Task | Priority | Depends on |
|----|------|----------|-----------|
| CO-39 | Markdown rendering pipeline (minor path) | high | — |
| CO-40 | UI adequation (placeholder for spec) | medium | — |
| CO-41 | Deploy quilomboaraucaria as Co universe | high | — |
| CO-42 | Content page redesign (folders, cards, viewer, dados) | critical | CO-39 |
| CO-43 | Hidden dev board (Yuri admin) | high | — |
| CO-44 | UAT environment (yuri/uat, auto-reset) | high | CO-43 |
| CO-45 | UAT → dev change promotion | high | CO-44 |
| CO-46 | User telemetry system | high | — |
| CO-47 | Privacy policy update + tracked data list | high | CO-46 |
| CO-48 | Schema documentation MVP (data only) | medium | — |

### Execution order

```
CO-39 (markdown pipeline)        ← unblocks CO-42
  └── CO-42 (content redesign)   ← critical UX work
CO-43 (dev board)                ← independent
  └── CO-44 (UAT env)            ← needs dev board
        └── CO-45 (UAT→dev sync) ← needs UAT
CO-46 (telemetry)                ← independent
  └── CO-47 (privacy update)     ← needs telemetry data list
CO-41 (quilomboaraucaria)        ← independent
CO-48 (schema docs)              ← independent, foundation work
CO-40 (UI adequation)            ← awaiting spec
```


---

## Completed Tasks (already merged — do NOT re-implement)

- CO-30 — Dynamic CSS engine — token generation from universe config at runtime (DONE, already merged into main)
- CO-92 — Unified timeline view — events from any universe with linear+log scrolling (DONE, already merged into main)
- CO-2 — Subtask tree rendering with expand/collapse in all views (DONE, already merged into main)
- CO-82 — UAT mirrors prod content on reset — HTTP pull of yuri's universes (DONE, already merged into main)
- CO-20 — MVP: plataforma pública multi-tenant em artelonga.com.br/co (DONE, already merged into main)
- CO-45 — UAT → dev change promotion — state tracking + version control backend (DONE, already merged into main)
- CO-65 — Visibility on PUT — let owners flip universe visibility via API (DONE, already merged into main)
- CO-34 — Obsidian plugin — sync CO universe ↔ Obsidian vault (DONE, already merged into main)
- CO-6 — Add assignee field to task model, API, and UI (DONE, already merged into main)
- CO-41 — Deploy quilomboaraucaria as Co universe — import content + UI from quilombo-blog (DONE, already merged into main)
- CO-24 — Content/form separation — universe config drives presentation, entries drive content (DONE, already merged into main)
- CO-35 — Vault REST API + Obsidian Clipper support (DONE, already merged into main)
- CO-50 — Universe-as-repo — each universe backed by a Git repo, built at runtime from main (DONE, already merged into main)
- CO-7 — Auth-protect board write operations (DONE, already merged into main)
- CO-40 — UI adequation — implement spec for two theme versions (TBD) (DONE, already merged into main)
- CO-25 — Theme gating — Scholarly + Relic default, full set for logged-in users (DONE, already merged into main)
- CO-31 — CRDT sync — Yjs + WebSocket, login required, 'Crie uma conta pra colaborar' (DONE, already merged into main)
- CO-60 — Invite + review system — role-based access, invite flow, task review workflow (DONE, already merged into main)
- CO-3 — Fix timeline: stable header, dependency arrows, proper zoom (DONE, already merged into main)
- CO-83 — Mermaid.js diagram rendering — C4, ER, flowcharts, sequence, state, class (DONE, already merged into main)
- CO-21 — Universe CRUD API — create, list, get, delete with slug routing (DONE, already merged into main)
- CO-44 — UAT environment — yuri/uat account, auto-reset DB, CO board pre-seeded (DONE, already merged into main)
- CO-8 — Delete project API endpoint (DONE, already merged into main)
- CO-59 — co auto v2 — single argument repo-based workflow with worktrees (DONE, already merged into main)
- CO-49 — User access model spec — deterministic API for anonymous, logged-in, and subscribed users (DONE, already merged into main)
- CO-38 — Yggdrasil — universe of universes: minigames hub with profiles + rankings (login-gated) (DONE, already merged into main)
- CO-28 — Open source repo setup — LICENSE, README, contributing guide (DONE, already merged into main)
- CO-39 — Markdown rendering pipeline — unify CodeMirror, marked, CRDT, Capacitor/Electron (DONE, already merged into main)
- CO-29 — CodeMirror 6 editor — markdown editing with live preview, open to all (DONE, already merged into main)
- CO-48 — Schema documentation MVP — data only (mermaid ERD rendering deferred) (DONE, already merged into main)
- CO-36 — Entry abstraction — .md files (truth), SQLite (index), protobuf (wire) (DONE, already merged into main)
- CO-67 — Prod universe seed — artelonga, quilomboaraucaria, rfq with content (DONE, already merged into main)
- CO-53 — co-dev public universe — Co development board with all CO-* tasks, subscribable (DONE, already merged into main)
- CO-4 — Dashboard: velocity chart, completion trend, workload by assignee (DONE, already merged into main)
- CO-43 — Hidden dev board — private universe showing CO development tasks (Yuri only) (DONE, already merged into main)
- CO-26 — Web UI i18n — pt-BR / en toggle for all board strings (DONE, already merged into main)
- CO-84 — Extract co auto into dev/co-auto crate — trait-based composable pipeline (DONE, already merged into main)
- CO-57 — Adaptation audit — reconcile existing implementations (CO-1–CO-48) with new architecture (DONE, already merged into main)
- CO-32 — Ansible deployment — provision, deploy, backup playbooks for Fly.io + VPS (DONE, already merged into main)
- CO-22 — Template universe — seed data, read-only for visitors, 'Criar universo' CTA (DONE, already merged into main)
- CO-47 — Privacy policy update — telemetry section + comprehensive data tracked list (DONE, already merged into main)
- CO-33 — E2E test suite — Playwright for full MVP flow (DONE, already merged into main)
- CO-1 — Board UI Overhaul (DONE, already merged into main)
- CO-23 — Usage gate — 100 entries free, then account required (DONE, already merged into main)
- CO-46 — Full user telemetry — privacy-respecting tracking for debugging + improvement (DONE, already merged into main)
- CO-37 — Design alignment — Scholarly Automaton + Relic Archive aesthetic for v1.0 release (DONE, already merged into main)
- CO-66 — API hygiene — 500→409 on duplicate key, fix seed description override, no-auto-stop UAT (DONE, already merged into main)
- CO-52 — Universe search + subscription — discover and subscribe to public universes (DONE, already merged into main)
- CO-5 — Integrate variant palette switcher into board UI (DONE, already merged into main)
- CO-42 — Content page redesign — folders, rendered cards, zoom viewer, view dados, hide tasks (DONE, already merged into main)
- CO-27 — Landing page at /co — template board with hero, login, criar universo (DONE, already merged into main)
- CO-85 — Password-login on prod — replace email-code friction with Argon2id auth (DONE, already merged into main)

---

## Execution Instructions

**YOUR TASK IS: CO-137 — Investigate why migration v22 didn't apply on prod + harden ALTER ADD COLUMN against partial-application**

IMPORTANT: Only implement CO-137. Do NOT implement or modify any other task.
Dependencies listed in the roadmap (e.g., 'Depends On: GP-8') mean those tasks are ALREADY DONE and merged into main. Their code is already in the codebase. Do not look for them or re-implement them.

Follow the acceptance criteria exactly. Each `- [ ]` item is a required deliverable.
Use conventional commits: the task specifies the commit message format.
Run `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt` before committing.
After completing all criteria, commit with the specified message.

## Test Isolation Rules

- All tests MUST run without opening network ports. Use in-process test servers (e.g., `axum::test::TestClient`, `tower::ServiceExt`) instead of spawning HTTP listeners.
- Never bind to `0.0.0.0`. If a test requires a port, bind to `127.0.0.1` only.
- Use temp directories for test databases (e.g., `tempfile::tempdir()`) — never write to user paths.
- Tests must be fully deterministic: no sleeps, no real network calls, no system time dependencies.
- Set `JWT_SECRET=test-secret` and `RUST_LOG=off` in test harness setup.