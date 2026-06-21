# Claude Development Instructions for CO

This file contains instructions for Claude instances working on the CO project.

## Project Overview

**CO** is a CLI tool for graph-based content management, built in Rust.

- **Repository:** `artelonga/co`
- **Current Version:** Check `Cargo.toml` for latest
- **Stack:** Rust, clap, SQLite (rusqlite), serde
- **co-web module patterns:** see [`co-web/src/MODULES.md`](co-web/src/MODULES.md) for the five server-side conventions (directory, sub-state, extractor, event-bus, worker)

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

## Versioning Policy (CO-258)

**Version bumps and CHANGELOG entries are owned by the release commit, NOT by task PRs.**

### Files agents MUST NOT modify

- `Cargo.toml` (workspace version)
- `co-cli/Cargo.toml` (binary version)
- `CHANGELOG.md`

These are mutated exclusively by `scripts/release-commit.sh` after a wave of tasks merges.

### What agents write instead

Write your changelog entry to `CHANGELOG-PENDING/<TASK-ID>.md`:

```markdown
## <TASK-ID> — <title>

<description of what changed and why>

### Why
<optional — rationale or motivation>
```

**Do NOT touch `Cargo.toml`, `co-cli/Cargo.toml`, or `CHANGELOG.md`.**

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

> **Single source of truth:** [`docs/OPERATIONS.md` → "Environments & Deploy"](docs/OPERATIONS.md)
> is the authoritative description of environments and the deploy flow. The summary
> below mirrors it — if they ever disagree, OPERATIONS.md wins.
>
> **Fly ops runbook** (suspend/scale/sidecar patterns + reverts): [`docs/infra/fly-runbook.md`](docs/infra/fly-runbook.md)

### Environments

| Env | App | URL | Purpose |
|-----|-----|-----|---------|
| **Production** | `co-artelonga` (Fly `gru`) | `co-artelonga.fly.dev` / `co.artelonga.com.br` | Public-facing, the only **required** deploy target. |
| **Staging** (optional) | `co-artelonga-staging` | `co-artelonga-staging.fly.dev` | **Manual preview only.** Deployed by hand via `flyctl deploy --config fly.staging.toml`. NOT a release gate; the `staging-deploy.yml` workflow is a deliberate no-op (`FLY_API_TOKEN` is intentionally not a repo secret). |

There is **no UAT environment** (decommissioned). `fly.uat.toml` is dead.

### Deploy: prod-direct (canonical)

```bash
# 1. Local checks
cargo test
cargo clippy -- -D warnings

# 2. CO-421 read-only Playwright prod-usability gate (anonymous, never mutates prod)
cd co-web && BASE_URL=https://co.artelonga.com.br \
  npx playwright test e2e/prod-usability.spec.ts --project=desktop-chromium --workers=2

# 3. Pre-deploy gate — CO-446 disk check + a fresh green local pipeline report
bash scripts/pipeline-deploy-gate.sh

# 4. Deploy to production
flyctl deploy

# 5. Smoke-test production
bash scripts/smoke-prod.sh
```

There is **no "UAT-first" step**. Staging, if used, is an optional manual preview
(`flyctl deploy --config fly.staging.toml`), not a gate.

> **CO-446 — disk-full hardening.** A deploy that adds a migration writes a
> `schema_version` row at boot; on a near-full `/data` that write fails with
> `SQLITE_FULL` and the machine crash-loops (2026-06-11 + 2026-06-13 outages).
> `scripts/pipeline-deploy-gate.sh` checks `df -P /data` on prod and **blocks at
> > 85% full**. If it blocks, **extend the volume before deploying**:
> `flyctl volumes extend <vol> -s <GB>` then `flyctl machine stop`/`start`
> (a plain **restart does NOT resize the filesystem**). Full runbook:
> [`docs/OPERATIONS.md` → "Disk-full recovery"](docs/OPERATIONS.md). The boot
> path also pre-flights free space (`CO_MIGRATION_MIN_FREE_BYTES`, default 200 MiB)
> and now degrades to a clear `FATAL (CO-446)` log instead of a SQLite panic.

### Fly.io Configuration Files

| File | Target | Notes |
|------|--------|-------|
| `fly.toml` | Production (`co-artelonga`) | Default deploy target |
| `fly.staging.toml` | Staging (`co-artelonga-staging`) | Optional manual preview: `--config fly.staging.toml` |

### Secrets

```bash
# Set JWT_SECRET (required, one-time)
flyctl secrets set JWT_SECRET=$(openssl rand -base64 48) -a co-artelonga

# Verify
flyctl secrets list -a co-artelonga
```

### Dockerfile Notes

- Image: `rust:1.88-slim` (requires protobuf-compiler)
- Dependency cache layer: `Cargo.toml`/`Cargo.lock` → dummy build → real source build
- First deploy: ~5 min (no cache). Subsequent: ~1-2 min (deps cached).
- Runtime: `debian:bookworm-slim` with `ca-certificates` + `curl` (healthcheck)
- Non-root user `co`, data volume at `/data`

### Logs & Debugging

```bash
flyctl logs -a co-artelonga --no-tail   # Recent logs
flyctl logs -a co-artelonga              # Stream live
flyctl ssh console -a co-artelonga       # Shell into machine
flyctl status -a co-artelonga            # Machine state
```

### Prod smoke

After every prod deploy, gate on `scripts/smoke-prod.sh` (exits non-zero on any
failed invariant). Combine with the CO-421 read-only Playwright prod-usability
gate above. See [`docs/OPERATIONS.md`](docs/OPERATIONS.md) for the full check list.

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

## Prod Verification

After every prod deploy, verify the platform invariants. The authoritative,
read-only checks live in `scripts/smoke-prod.sh` and the CO-421 Playwright
prod-usability suite; the items below are a quick reference.

### Health & Data Seeding

```bash
# Health check (version tracks Cargo.toml — e.g. 3.15.0)
curl -s https://co-artelonga.fly.dev/api/health
# → {"status":"ok","version":"3.15.0","env":"production"}

# Template universe exists
curl -s https://co-artelonga.fly.dev/api/v1/universes/template | python3 -m json.tool
# → key: "template", is_template: true, name: "CO"

# Tutorial project exists
curl -s https://co-artelonga.fly.dev/api/v1/universes/template/projects
# → [{ key: "CO", name: "Aprenda CO" }]

# 12 tutorial tasks
curl -s "https://co-artelonga.fly.dev/api/projects/CO/tasks?u=template" | python3 -c "import sys,json; print(len(json.load(sys.stdin)), 'tasks')"
# → 12 tasks
```

### Database State (SQLite)

CO-77 split storage: global tables live in `meta.db`; per-universe data lives in
`/data/universes/<key>/data.db`.

```bash
flyctl ssh console -a co-artelonga

# Global state (universes, users, schema_version, ab_*, telemetry_events)
sqlite3 /data/meta.db
.tables
SELECT MAX(version) FROM schema_version;   # source of truth: co-web/src/storage/migrations/ (e.g. 88)
SELECT key, name, is_template, is_public, content_count FROM universes;

# Per-universe content (entries, entry_dates, entry_relations, op_log)
sqlite3 /data/universes/template/data.db
SELECT path, entry_type, title FROM entries LIMIT 5;
```

## Getting Help

- Review existing code patterns before implementing new features
- Check closed PRs for similar implementations
- Use `gh issue view <n> --comments` for discussion context
