# Contributing to CO

Thank you for your interest in contributing to CO. This document covers the development setup, workflow, and conventions.

## Table of Contents

- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Development Workflow](#development-workflow)
- [Issue Labels](#issue-labels)
- [Code Style](#code-style)
- [Testing](#testing)
- [Pull Request Process](#pull-request-process)

---

## Development Setup

### Prerequisites

- Rust (stable, via [rustup](https://rustup.rs))
- Node.js 20+ (for co-web E2E tests)
- SQLite (usually pre-installed)

### Clone and build

```bash
git clone https://github.com/artelonga/co
cd co
cargo build --workspace
```

### Run the web server locally

```bash
JWT_SECRET=dev-secret cargo run -p co-web
# Open http://localhost:3000/co
```

### Run the CLI

```bash
cargo install --path co-cli
co --help
```

### Run tests

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

---

## Project Structure

```
co/
├── core/        # Graph database, Markdown parser, content types
├── co-cli/      # CLI binary (co)
├── co-web/      # Axum HTTP server, board UI, REST API
├── co/          # co-engine crate
├── game-core/   # Game engine integration
├── co-deploy/   # Ansible deployment playbooks
└── CHANGELOG.md
```

---

## Development Workflow

All changes require a GitHub issue. No PR without a linked issue.

### 1. Pick or create an issue

Browse [open issues](https://github.com/artelonga/co/issues) or create one using the provided templates.

### 2. Create a branch

| Label | Branch pattern |
|-------|----------------|
| `type:feat` | `feat/issue-<n>-short-description` |
| `type:fix`  | `fix/issue-<n>-short-description`  |
| `type:docs` | `docs/issue-<n>-short-description` |
| `type:refactor` | `refactor/issue-<n>-short-description` |

```bash
git checkout -b feat/issue-42-my-feature
```

### 3. Implement with TDD

Follow the Red-Green-Refactor cycle:

1. **Red** — write a failing test first
2. **Green** — write the minimum code to pass
3. **Refactor** — clean up, run fmt + clippy, confirm tests pass

### 4. Include a version bump (in the same PR)

- Update `Cargo.toml` workspace version (feat → minor, fix/docs/refactor → patch)
- Update `co-cli/Cargo.toml` version field
- Add an entry to `CHANGELOG.md`

### 5. Open a PR

Reference the issue: `Closes #<n>`. Fill in the PR template.

---

## Issue Labels

### Type labels (one per issue)

| Label | Description | Version bump |
|-------|-------------|--------------|
| `type:feat` | New feature | Minor |
| `type:fix` | Bug fix | Patch |
| `type:docs` | Documentation | Patch |
| `type:refactor` | Code restructuring without behaviour change | Patch |
| `type:chore` | Maintenance, config, dependencies | None |

### Module labels (combine with type)

| Label | Subsystem |
|-------|-----------|
| `module:content` | Content types and parsing |
| `module:tools` | Tools and extensions |
| `module:writer` | Writer agent system |
| `module:collab` | GitHub/collaboration |
| `module:space` | Spaces and namespaces |

---

## Code Style

- **Formatting:** `cargo fmt` (required, enforced by CI)
- **Lints:** `cargo clippy -- -D warnings` (zero warnings policy)
- **Tests:** every public function should have at least one test
- **Comments:** only where the logic is non-obvious
- **Commits:** conventional commits (`feat(scope): description`)

### Commit format

```
tipo(escopo): descrição curta

Co-Authored-By: Your Name <you@example.com>
```

Types: `feat`, `fix`, `docs`, `refactor`, `chore`, `test`

---

## Testing

### Unit tests

```bash
cargo test --workspace
```

### E2E tests (Playwright)

```bash
cd co-web
npm ci
npx playwright install --with-deps chromium
JWT_SECRET=test-secret npx playwright test --project=chromium-desktop
```

### Test rules

- Tests must not open network ports (use in-process test servers)
- Never bind to `0.0.0.0` in tests — use `127.0.0.1`
- Use `tempfile::tempdir()` for test databases
- No sleeps, no real network calls, no system-time dependencies
- Set `JWT_SECRET=test-secret` and `RUST_LOG=off` in test setup

---

## Pull Request Process

1. CI must pass (test, clippy, fmt check)
2. PR description must reference an issue (`Closes #n`)
3. Version bump and CHANGELOG entry included
4. At least one approving review required for merge
5. Squash-merge with conventional commit message

---

## Reporting Issues

Use the issue templates:
- **Bug report** — unexpected behaviour, crashes, wrong output
- **Feature request** — new capability or improvement

For security vulnerabilities, email [yuri@artelonga.com.br](mailto:yuri@artelonga.com.br) directly (do not open a public issue).
