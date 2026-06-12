---
type: tool
id: hooks
category: claude-code
tool_type: deterministic
command: claude
status: active
language: english
url: claude-code::content/examples/settings/README.md
---

# Hooks — Lifecycle Event Scripts

Hooks are shell commands wired into Claude Code's lifecycle events via
`.claude/settings.json`. They run automatically at defined points (before/after tool
calls, on session stop, etc.) — outside the agent's context window. CO uses them for
automated behaviours that cannot be expressed as agent instructions.

## When CO Uses This

- **CO-428** (digest consumer): a `PostToolUse` hook can trigger `co universe digest`
  after each file write, keeping the digest cache warm.
- **co-auto**: `PreToolUse` guards (e.g., validate bash commands before execution) run
  as hooks so they execute even if the agent forgets to check.
- **session teardown**: `Stop` hooks commit telemetry and flush the session log to a
  JSONL file consumed by CO-437.

## Minimal Example

```json
{
  "hooks": {
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "co session flush --session $CLAUDE_SESSION_ID"
          }
        ]
      }
    ]
  }
}
```

## Supported Events

| Event | When |
|-------|------|
| `PreToolUse` | Before any tool call |
| `PostToolUse` | After any tool call |
| `Notification` | On agent notification |
| `Stop` | When the session ends |

## Canonical Reference

See [[claude-code::content/examples/settings/README.md]] for hook configuration examples.
