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

## Getting Help

- Review existing code patterns before implementing new features
- Check closed PRs for similar implementations
- Use `gh issue view <n> --comments` for discussion context


---

## Current Task: GP-7 — Email-only login with verification code and JWT

---
id: 7
title: Email-only login with verification code and JWT
status: todo
priority: critical
parent: 5
labels:
  - auth
  - server
module: server
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

GIVEN a user wants to authenticate with email only (no password),
WHEN they use the two-step verification flow,
THEN:

Step 1 — Request code:
- [ ] POST `/api/v1/auth/login` with `{ email }` sends a 6-digit numeric code
- [ ] Code stored in redb with key `verify:{email}`, value `{ code, user_id, expires_at, attempts }`
- [ ] Code expires after 5 minutes
- [ ] Rate limit: max 3 code requests per email per 15 minutes
- [ ] If email not registered, returns `200` anyway (no email enumeration)
- [ ] Email sent via `MAIL_PROVIDER` env var (default: log to stdout for dev)

Step 2 — Verify code:
- [ ] POST `/api/v1/auth/verify` with `{ email, code }`
- [ ] On match: JWT signed with HMAC-SHA256 using `JWT_SECRET` env var
- [ ] JWT payload: `{ sub: user_id, email, tier: "player", exp: now + 7d, iat: now }`
- [ ] JWT set as httpOnly Secure SameSite=Strict cookie named `session`
- [ ] Returns `200 { user_id, email, display_name, expires_at }`
- [ ] Wrong code: decrement attempts, return `401` with `{ remaining_attempts }`
- [ ] 3 failed attempts: delete code, return `401 { error: "Code expired, request a new one" }`
- [ ] Expired code: return `410 Gone`

Dependencies:
- [ ] `jsonwebtoken` crate added to server
- [ ] `MailProvider` trait with `send(to, subject, body)` in `core/`
- [ ] `LogMailProvider` impl that prints to stdout (dev mode)
- [ ] commit: `feat(server): email-only auth with verification code and JWT`


---

## Parent Epic: GP-5 — Authentication & User Identity

---
id: 5
title: Authentication & User Identity
status: todo
priority: critical
labels:
  - epic
  - auth
  - base-app
module: meta
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

AS A user of the game platform,
I NEED to register, log in, and maintain a persistent identity across all universes,
SO THAT my profile, game stats, wallet, and universe memberships are tied to my account and accessible from any client (web, Godot).

## Scope
- Username + password registration (Argon2id hashing)
- JWT-based session tokens (HMAC-SHA256)
- User profile (display name, bio, avatar URL, created_at)
- Auth middleware for all protected API routes
- SvelteKit auth flow (login page → HttpOnly cookie → hooks guard)

## Versioning
- feat: register/login → v0.2.0


---

## Project Configuration

```yaml
name: Game Platform
key: GP
description: Full-stack game platform with plugin-based universe system, task/notes management, leaderboards, and SvelteKit web UI
created_at: 2026-03-22T00:00:00Z
next_id: 44

```

---

## Roadmap

# GP Roadmap — Execution Order for `co auto`

## Release Plan (Semver Milestones)

### v0.1.0 — Base App Foundation
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 1 | GP-2: Rust workspace structure | core, server | `feat(core)` | — |
| 2 | GP-3: Plugin trait + manifest | core | `feat(core)` | GP-2 |
| 3 | GP-4: Plugin loader + registration | server | `feat(server)` | GP-3 |

### v0.2.0 — Auth + Web Frontend
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 4 | GP-6: User registration endpoint | server | `feat(server)` | GP-2 |
| 5 | GP-7: Login endpoint + JWT | server | `feat(server)` | GP-6 |
| 6 | GP-8: JWT auth middleware | server | `feat(server)` | GP-7 |
| 7 | GP-10: SvelteKit project setup | web | `feat(web)` | GP-2 |
| 8 | GP-11: Auth pages (login/register) | web | `feat(web)` | GP-7, GP-10 |
| 9 | GP-12: Dashboard page | web | `feat(web)` | GP-8, GP-11 |

### v0.3.0 — Leaderboards + Profiles
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 10 | GP-14: Multi-user game stats | core | `feat(core)` | GP-8 |
| 11 | GP-15: Leaderboard endpoint | server | `feat(server)` | GP-14 |
| 12 | GP-16: Player profile endpoint | server | `feat(server)` | GP-14 |
| 13 | GP-17: Leaderboard page | web | `feat(web)` | GP-15 |
| 14 | GP-18: Player profile page | web | `feat(web)` | GP-16 |

### v0.4.0 — Tasks + Notes
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 15 | GP-20: Task board API integration | server | `feat(server)` | GP-8 |
| 16 | GP-21: Notes API (read-only) | server | `feat(server)` | GP-8 |
| 17 | GP-22: Task board page (kanban) | web | `feat(web)` | GP-20 |
| 18 | GP-23: Markdown note viewer | web | `feat(web)` | GP-21 |

### v0.5.0 — Universe CRUD + Viewer
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 19 | GP-25: Universe CRUD endpoints | server | `feat(server)` | GP-3, GP-8 |
| 20 | GP-27: Universe viewer component | web | `feat(web)` | GP-25 |
| 21 | GP-29: Universe browser/discovery | web | `feat(web)` | GP-25 |

### v0.6.0 — Universe Editor + Themes
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 22 | GP-26: Tile map editor | web | `feat(web)` | GP-25 |
| 23 | GP-28: Theme system + picker | web | `feat(web)` | GP-27 |

### Plugins Repo v0.1.0 (separate)
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 24 | GP-31: Plugin template | universes | `feat(universes)` | GP-3 |
| 25 | GP-32: Tetris plugin | universes | `feat(universes)` | GP-31 |
| 26 | GP-33: Snake+Invaders+PointSet+Poker | universes | `feat(universes)` | GP-31 |

### E2E (continuous)
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 27 | GP-35: Server E2E smoke test | testing | `test(server)` | GP-8, GP-15, GP-25 |

### co auto (meta — builds the tool that runs this roadmap)
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 0 | GP-37: Task selector | co-cli | `feat(cli)` | — |
| 0 | GP-38: Context builder | co-cli | `feat(cli)` | GP-37 |
| 0 | GP-39: Claude Code launcher | co-cli | `feat(cli)` | GP-38 |
| 0 | GP-40: Acceptance criteria reviewer | co-cli | `feat(cli)` | GP-39 |
| 0 | GP-41: Task status updater | co-cli | `feat(cli)` | GP-40 |
| 0 | GP-42: Auto-cycle loop | co-cli | `feat(cli)` | GP-41 |

## Port Assignments (Dedicated Test Environment)

| Service | Port | Purpose |
|---------|------|---------|
| Game Server | 8742 | Game API (stats, wallet, leaderboards) |
| Tasks Service (co-web) | 8743 | Task/notes API (projects, tasks, comments) |
| SvelteKit Dev | 5173 | Web frontend (dev server) |
| SvelteKit Preview | 4173 | Web frontend (production preview) |

## `co auto` Invocation

```bash
# Build the auto command first (GP-36 epic)
cd $CO_WORKSPACE
cargo build

# Then run the GP roadmap
co auto --space gp --cycle --stop-on-fail

# Or run a single task
co auto --space gp --task GP-2

# Dry run (show what would execute)
co auto --space gp --dry-run
```


---

## Execution Instructions

Execute the task **GP-7** above. Follow the acceptance criteria exactly.
Each `- [ ]` item is a required deliverable.
Use conventional commits: the task specifies the commit message format.
Run `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt` before committing.
After completing all criteria, commit with the specified message.