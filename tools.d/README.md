# `tools.d/` — trusted external-tool manifests (CO-503)

This directory is the **trusted manifest location** for CO's canonical tool
contract (CO-503). Each `*.yaml` / `*.yml` file describes one *external*
(out-of-process) tool — a CLI/script (`command:`) or an HTTP service (`url:`) —
that becomes a first-class, LLM-callable CO tool **with no binary recompile**.

The fields mirror `tools/schema.yaml`:

| Field | Meaning |
|-------|---------|
| `name` | Tool identifier the LLM references. |
| `description` | What the tool does (the LLM uses this to decide when to call). |
| `tool_type` | `deterministic` (script) or `predictive` (LLM-backed). |
| `command` | Subprocess command line. Args are passed as JSON on **stdin**; JSON is read from **stdout**; non-zero exit = error. |
| `url` | HTTP endpoint. Args are POSTed as JSON; JSON is read back. |
| `input_schema` | JSON Schema for the arguments (at least `required:` is enforced). |
| `dependencies` | Required host binaries/services (documentation). |
| `category` | Free-form grouping. |
| `status` | Only `active` manifests are registered. |
| `secrets` | Env var names a **subprocess** is permitted to receive (see below). |

## Trust boundary (SECURITY)

* External tools are **opt-in**: set `CO_ENABLE_EXTERNAL_TOOLS=1`. Default is OFF.
* Manifests load **only** from this trusted, instance-controlled directory —
  **never** from universe/observed content. An untrusted author must not be able
  to introduce a `command`.
* Subprocess tools run with **no ambient secrets**: the child starts from a clean
  environment and receives only the variables named in `secrets:`, forwarded by
  name from the host env. Everything else is stripped.

The directory CO reads is resolved from `CO_TOOLS_DIR` (default: `tools.d` under
the working directory). See `co/src/canon_tool.rs` for the contract and
`co-web/src/integrations/canon_tools.rs` for the HTTP invoker + boot wiring.
