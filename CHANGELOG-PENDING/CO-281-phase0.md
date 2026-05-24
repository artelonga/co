## CO-281 — Phase 0 baseline snapshot

Captured the current Fly.io deployment baseline for the 5 deployable apps
(plus the unconfirmed `artelonga-dev` variant) to `docs/infra/fly-baseline-2026-05.md`.
This is pure measurement — no `fly.toml` edits, no deploys — and gives Phases
1-4 a fixed reference point to measure savings against.

### Why

Per CO-281, before changing any sizing we wanted a written snapshot of every
app's machine size, `auto_stop_machines` setting, `min_machines_running`, and
estimated monthly cost. The baseline already surfaces useful signal: real
total is ~$24-26/mo for machines (vs the spec's $13-15/mo pre-flight guess),
dominated by `quilombo-araucaria` running at 2 GB always-on for its video
upload workload — meaning Phase 1's target band is reachable from
`min_machines_running` flips alone, before any embedding-sidecar extraction.
