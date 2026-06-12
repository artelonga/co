# co-auto

Automated task execution pipeline (developer tool, separate from the user-facing
CO scaffold). Picks the next unblocked task under `work/<space>/`, builds layered
context, launches an executor (Claude Code), reviews against acceptance criteria,
and finalizes (status update, branch, optional PR).

```bash
co-auto                  # next unblocked task in the auto-detected space
co-auto CO-427           # a specific task
co-auto --cycle          # keep going through the backlog
co-auto --model sonnet   # pin a model for the whole run (overrides routing)
```

## Model routing (CO-427)

By default co-auto chooses the executor model **per task** — it is no longer one
global model for the whole run. Resolution follows a four-level precedence (first
match wins):

1. **`--model` on the CLI** — operator override. Pins every task in the run.
2. **`model:` in the task frontmatter** — per-task override (see example below).
3. **Priority→model policy** — `high → opus`, `medium → sonnet`, `low → haiku`
   (`critical` maps to the `high` tier). All configurable.
4. **Quality-first default** — `opus` when nothing else applies (owner decision,
   2026-06-12: quality-first; unspecified tasks run on Opus).

Model names are CLI aliases (`opus`/`sonnet`/`haiku`/…) passed through verbatim —
they are **not** validated against a hardcoded id list, because the `claude` CLI
is the authority and that list changes.

### Window downshift

After the model is resolved and **before launch**, co-auto runs a best-effort
window downshift. It reads CO-426's rolling 5h usage:

```
GET <CO_USAGE_ENDPOINT>/api/v1/usage/summary?window=5h
```

If the 5h total-token consumption has crossed the configured **soft limit**, the
model is degraded **one tier** (`opus→sonnet→haiku`) so the run stays inside the
budget. The decision and reason are printed to the launcher log and included in
the usage report (`model_requested` vs `model_used`, plus a `downshifted` record)
so the CO-426 dashboard can surface the degradation — quality drops are never
silent.

The downshift is **fail-open**: no endpoint configured, no network, an
unparseable response, no soft limit, or an already-lowest tier → the requested
model is kept unchanged. Routing is decided at launch; tasks already running are
never rebalanced.

### Configuration

Routing is configurable in the space's `project.yaml` under a `routing:` block,
with env overrides for the soft limit and endpoint. All fields are optional and
fall back to the defaults above.

```yaml
# work/<space>/project.yaml
key: CO
routing:
  default: opus       # quality-first fallback
  high: opus
  medium: sonnet
  low: haiku
  usage_soft_limit_5h_tokens: 5000000   # downshift one tier past this
  usage_endpoint: https://co-artelonga.fly.dev
```

| Setting | Source | Default |
|---|---|---|
| priority policy + default | `project.yaml` `routing:` | `opus`/`sonnet`/`haiku`, default `opus` |
| `usage_soft_limit_5h_tokens` | `routing:` **or** env `CO_AUTO_SOFT_LIMIT_5H_TOKENS` | unset (downshift disabled) |
| `usage_endpoint` | `routing:` **or** env `CO_USAGE_ENDPOINT` | unset (downshift disabled) |

When the soft limit is unset, the server-reported `soft_limit_5h` from
`/usage/summary` is used if present; otherwise no downshift occurs.

### Per-task `model:` frontmatter

Add a `model:` field to any task spec to override routing for that task. An
absent field is normal (routing decides); a present-but-invalid value (non-string
or empty) is ignored with a warning and routing falls through to the policy.

```yaml
---
id: 427
title: "Model routing"
status: todo
priority: high
model: opus        # ← per-task override; wins over the priority policy
---
```
