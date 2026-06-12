---
type: tool
id: otel-telemetry
category: claude-code
tool_type: deterministic
command: claude
status: active
language: english
url: claude-code::content/README.md
---

# OpenTelemetry Telemetry

Claude Code emits OpenTelemetry spans and metrics when `CLAUDE_CODE_ENABLE_TELEMETRY=1`
is set. CO captures these traces to correlate task execution time, tool-call counts,
and model usage with the universe and task ID — feeding the observability layer.

## When CO Uses This

- **CO-437** (usage metadata enrichment): OTEL traces provide wall-clock latency and
  tool-call count per task, complementing the token counts in the session JSONL.
- **co-auto model×universe matrix**: the `universe` and `task_id` span attributes allow
  grouping traces by universe and cross-referencing with the model used.

## Minimal Example

```bash
# Send traces to a local OTLP collector
CLAUDE_CODE_ENABLE_TELEMETRY=1 \
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 \
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf \
  claude -p "Fix the bug in main.rs" --output-format stream-json
```

## Key Environment Variables

| Variable | Purpose |
|----------|---------|
| `CLAUDE_CODE_ENABLE_TELEMETRY` | `1` to enable OTEL export |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP receiver URL |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | `grpc` or `http/protobuf` |
| `OTEL_SERVICE_NAME` | Service name tag on all spans |

## Canonical Reference

See [[claude-code::content/README.md]] for telemetry configuration.
