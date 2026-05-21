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
6. Merge via `scripts/safe-merge-pr.sh <repo> <pr-number>` (never bare `gh pr merge --delete-branch`) and clean up branches

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
# Merge the PR safely — never use bare gh pr merge --delete-branch.
# GitHub may return mergeable=UNKNOWN (transient); bare --delete-branch deletes the
# branch even when the merge fails, silently closing the PR without merging.
scripts/safe-merge-pr.sh <repo> <pr-number>

git checkout main && git pull origin main
git branch -d <branch-name>
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
