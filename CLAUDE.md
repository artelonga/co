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

| Label | Version Bump | Example |
|-------|--------------|---------|
| `type:feat` | Minor (x.**Y**.0) | 0.12.0 → 0.13.0 |
| `type:fix` | Patch (x.y.**Z**) | 0.12.0 → 0.12.1 |
| `type:docs` | Patch (x.y.**Z**) | 0.12.0 → 0.12.1 |
| `type:refactor` | Patch (x.y.**Z**) | 0.12.0 → 0.12.1 |
| `type:chore` | No bump | - |

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
| **Space** | Project namespace directory (`monica/`, `work/`) |
| **Scope** | Subdirectory within a space (`private/`, `shared/`) |
| **Context** | User-provided content/prompts |

## Open Issues (v1.0 Roadmap)

Check current status: `gh issue list --state open --label "milestone:v1.0"`

Priority order for v1.0:
1. #48 - Collaborative Content Creation (User + Agent)
2. #36 - GitHub as Source of Truth
3. #47 - Space Isolation & Commit Guards
4. #49 - Terminology Refactor
5. #38-43 - Additional features

## Getting Help

- Review existing code patterns before implementing new features
- Check closed PRs for similar implementations
- Use `gh issue view <n> --comments` for discussion context
