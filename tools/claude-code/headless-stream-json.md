---
type: tool
id: headless-stream-json
category: claude-code
tool_type: deterministic
command: claude
status: active
language: english
dependencies: claude
url: claude-code::content/README.md
---

# Headless Mode + stream-json Output Format

Claude Code's headless mode (`-p`/`--print`) runs a single prompt non-interactively and exits.
Combined with `--output-format stream-json`, each assistant turn is emitted as a newline-delimited
JSON stream, which CO reads to parse structured task output without a PTY.

## When CO Uses This

- **CO-417** (`co source add github`): materialize a GitHub repo as universe entries by piping
  Claude's analysis through stream-json parsing.
- **co-auto pipeline**: each task execution uses `claude -p "..." --output-format stream-json`
  so the orchestrator captures structured results, exit codes, and token usage.

## Minimal Example

```bash
claude -p "List all .md files in this repo" \
       --output-format stream-json \
  | jq -c 'select(.type == "assistant") | .message.content[]?.text'
```

## Key Flags

| Flag | Purpose |
|------|---------|
| `-p / --print` | Headless mode — run prompt and exit |
| `--output-format stream-json` | NDJSON stream, one object per event |
| `--no-interactive` | Alias for headless, useful in scripts |

## Canonical Reference

See [[claude-code::content/README.md]] for the full CLI reference.
