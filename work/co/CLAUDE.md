---
type: doc
title: CLAUDE.md — CO Dev Guide
---

# CO Platform — Development Guide

## Overview

**Stack**: Rust (axum, rusqlite, serde) + TypeScript SPA + SQLite + Fly.io

## Git Conventions

### Branch naming

```
feat/CO-<n>-<short-desc>
fix/CO-<n>-<short-desc>
refactor/CO-<n>-<short-desc>
```

### Commits (conventional)

```
feat(scope): description
fix(scope): description
refactor(scope): description
chore(scope): description
```

Footer: `Co-Authored-By: Claude <noreply@anthropic.com>`

## Forbidden Files — DO NOT Modify

- `Cargo.toml` (workspace version)
- `co-cli/Cargo.toml` (binary version)
- `CHANGELOG.md`

Write changelog entry to `CHANGELOG-PENDING/<TASK-ID>.md` instead.

## TDD

1. **RED**: write failing test first — `cargo test` should fail
2. **GREEN**: minimal implementation — `cargo test` passes
3. **REFACTOR**: clean up before commit

```bash
cargo test                        # all tests
cargo clippy -- -D warnings       # must be clean
cargo fmt                         # auto-format
```

## Module Map

| Module | Path | Notes |
|--------|------|-------|
| Core types | `core/src/` | Shared library |
| CLI | `co-cli/src/` | Commands |
| Web server | `co-web/src/` | Axum routes + storage |
| SPA | `co-web/static/variants/a/` | TypeScript |
| co-auto | `dev/co-auto/src/` | Agent pipeline |

## Key Patterns

### Adding a route

1. Handler in `co-web/src/routes/{module}_routes.rs`
2. Register in `co-web/src/server/router.rs`
3. Integration test in same file (use `tower::ServiceExt`, no real port)

### Database migrations

Add file to `co-web/src/db/migrations/v{N}_{name}.sql`:
```sql
ALTER TABLE entries ADD COLUMN col TEXT NOT NULL DEFAULT '';
```

Never swallow SELECT errors on new columns with `.ok()` — let missing columns fail loudly.

### AppState

Use `parking_lot::Mutex<Storage>` (not `std::sync::Mutex`). Never hold the lock across an `await` point.

## Test Isolation

- No real network ports — use `tower::ServiceExt` in-process
- Never bind `0.0.0.0` — use `127.0.0.1`
- Use `tempfile::tempdir()` for test databases
- Set `JWT_SECRET=test-secret` in test setup
