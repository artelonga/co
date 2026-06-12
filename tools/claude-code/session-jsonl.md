---
type: tool
id: session-jsonl
category: claude-code
tool_type: deterministic
command: claude
status: active
language: english
url: claude-code::content/README.md
---

# Session JSONL — Structured Session Output

Claude Code can write a structured JSONL log of the full session — every tool call,
assistant turn, and usage metric — to a file. CO reads this log for cost attribution,
usage metadata enrichment, and audit trails.

## When CO Uses This

- **CO-437** (usage metadata): co-auto reads the session JSONL after each task to
  extract per-task token usage (input, cache_read, cache_create, output) and attach
  them to the task record as `usage_metadata`.
- **co-auto model×universe matrix**: session JSONL provides the `model` field used to
  build the per-universe model usage breakdown.

## Minimal Example

```bash
# Write session log to a file
claude -p "Implement feature X" \
       --output-format stream-json \
       --session-output /tmp/session-$(date +%s).jsonl

# Parse token usage after the run
jq 'select(.type == "usage") | {input, output, cache_read}' /tmp/session-*.jsonl
```

## JSONL Schema (key fields)

```json
{ "type": "usage",     "model": "claude-sonnet-4-6", "input_tokens": 1234,
  "output_tokens": 567, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0 }
{ "type": "assistant", "message": { "content": [...] } }
{ "type": "tool_use",  "name": "Bash", "input": { "command": "..." } }
```

## Canonical Reference

See [[claude-code::content/README.md]] for the full session output format.
