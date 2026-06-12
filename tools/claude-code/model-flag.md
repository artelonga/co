---
type: tool
id: model-flag
category: claude-code
tool_type: deterministic
command: claude
status: active
language: english
dependencies: claude
url: claude-code::content/README.md
---

# --model Flag — Model Selection

The `--model` flag (or `ANTHROPIC_MODEL` env var) selects which Claude model Claude Code
invokes. CO-auto uses this to route each task to the appropriate model tier based on
task priority and rolling usage, as defined in CO-427.

## When CO Uses This

- **CO-427** (model routing): co-auto's priority policy maps `high→opus`, `medium→sonnet`,
  `low→haiku`. The per-task `model:` frontmatter overrides the policy. The resolved alias
  is passed verbatim as `--model <alias>`.
- **Window downshift** (CO-427): when the rolling 5h token usage crosses the soft limit,
  co-auto degrades one tier and passes the lower alias as `--model`.

## Minimal Example

```bash
# explicit override — use Opus regardless of priority
claude -p "Hard refactor" --model claude-opus-4-8

# let the environment variable set the default
ANTHROPIC_MODEL=claude-haiku-4-5-20251001 claude -p "Quick lookup"
```

## Model Aliases (CO-427)

| Alias | Resolved Model |
|-------|---------------|
| `opus` | `claude-opus-4-8` |
| `sonnet` | `claude-sonnet-4-6` |
| `haiku` | `claude-haiku-4-5-20251001` |

## Canonical Reference

See [[claude-code::content/README.md]] for the full list of supported model IDs.
