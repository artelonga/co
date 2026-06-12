---
type: tool
id: claude-md
category: claude-code
tool_type: deterministic
command: claude
status: active
language: english
url: claude-code::content/README.md
---

# CLAUDE.md — Project Memory System

`CLAUDE.md` files are read by Claude Code at session start and injected into the system
prompt as project-specific context. CO uses them to embed task specs, architecture docs,
and hard-won rules into every co-auto session — eliminating redundant context loading per
launch.

## When CO Uses This

- **CO-429** (this task): `.claude/co-auto-context.md` is referenced by co-auto as the
  canonical context source for every task run. It contains the current task's acceptance
  criteria, conventions, and module map.
- **co-auto pipeline**: `dev/co-auto/src/` writes a task-specific CLAUDE.md fragment
  into the worktree before launching `claude`, so the agent sees the task without needing
  it in the prompt.

## Memory Hierarchy

| File | Scope | When loaded |
|------|-------|------------|
| `~/.claude/CLAUDE.md` | User-global | Every session |
| `<project>/CLAUDE.md` | Project | Sessions in that directory |
| `<project>/.claude/CLAUDE.md` | Project (alt) | Sessions in that directory |

## Minimal Example

```markdown
# My Project

- Stack: Rust + Axum
- Always run `cargo fmt` before committing.
- Forbidden: never use `.ok()` to swallow a SELECT on a new migration column.
```

## Canonical Reference

See [[claude-code::content/README.md]] for the memory system documentation.
