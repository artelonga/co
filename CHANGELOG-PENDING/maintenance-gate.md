## maintenance-gate — CO_MAINTENANCE_MODE planned-maintenance gate

Added a `maintenance_gate` middleware: when the `CO_MAINTENANCE_MODE` secret is
set, every request except `/api/health` returns `503` immediately, **without
touching the storage lock** — so a one-time exclusive DB operation (a migration,
or a future off-traffic `event_log` VACUUM per CO-463) can run with zero request
contention. `/api/health` stays `200` so Fly keeps the machine alive. Inert by
default (flag unset); toggle via secret, no code change. Reusable infra salvaged
from the retired in-process reclaim experiment.
