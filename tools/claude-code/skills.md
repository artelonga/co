---
type: tool
id: skills
category: claude-code
tool_type: deterministic
command: claude
status: active
language: english
url: claude-code::content/README.md
---

# Skills — Slash Commands

Skills (also called slash commands or plugins) extend Claude Code with reusable,
parameterized workflows. A skill is a markdown file in `.claude/skills/<name>/SKILL.md`
(or a plugin directory). CO defines its own skills for recurring operations like
code review, security audits, and co-auto orchestration.

## When CO Uses This

- **CO-429** (this task): `co-auto` is invoked as a skill from within a Claude Code
  session, giving the agent structured access to task context without loading it ad-hoc.
- **`/code-review`**, **`/simplify`**, **`/security-review`**: built-in CO skills
  available in every co session (registered in `.claude/settings.json`).
- **`/verify`**, **`/run`**: integration skills that launch and observe the app.

## Minimal Example

```markdown
<!-- .claude/skills/my-skill/SKILL.md -->
# My Skill

Run this to check database migration safety.

## Arguments

- `migration`: path to the .sql file to review.

## Instructions

Read the migration at {{migration}} and check for:
- Missing DEFAULT on NOT NULL columns
- References to columns added in the same migration
```

## Plugin Discovery

Plugins in `~/.claude/plugins/<name>/` are available globally across all projects.
Project-local skills live in `.claude/skills/`.

## Canonical Reference

See [[claude-code::content/README.md]] for skill authoring documentation.
