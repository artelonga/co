# Fly.io Ops Runbook

Patterns discovered during CO-281 (cost optimization). Each section covers one
operation, its revert, and when to use it.

---

## 1. Suspend an app (auto_stop_machines)

**When:** The app handles sporadic traffic and a ~250 ms resume latency is
acceptable. Prefer `"suspend"` over `"stop"` when the app holds state (e.g.
LiteFS primary lease) that a cold boot would re-negotiate.

**Pattern (CO-285):**

```toml
# fly.toml
[http_service]
  auto_stop_machines  = "suspend"   # was "stop" or false
  auto_start_machines = true
  min_machines_running = 0          # allow full scale-to-zero
```

Deploy:

```bash
flyctl deploy
```

Verify the machine suspends after idle (check `flyctl status -a <app>` after
a quiet window; state should transition from `started` to `suspended`).

**Revert:**

```toml
  auto_stop_machines = "stop"       # or false to disable entirely
  min_machines_running = 1          # restore always-on if needed
```

---

## 2. Scale memory (right-size RAM)

**When:** Profiling (CO-287 observation window) shows the machine's RSS stays
well below its declared RAM. A 2× headroom is healthy; 4× or more is a
candidate for downsizing.

**Observation window (before touching fly.toml):**

```bash
# Stream RSS from the running machine for 30 minutes
flyctl ssh console -a <app> -C 'while true; do cat /proc/1/status | grep VmRSS; sleep 60; done'
```

If peak RSS < 50% of declared RAM, halve the tier; if > 80%, consider upsizing.

**Pattern:**

```toml
# fly.toml
[[vm]]
  memory = "512mb"   # was "1024mb"; halve only after the observation window
  cpu_kind = "shared"
  cpus = 1
```

Deploy and monitor RSS again for the next 24 h before calling it stable.

**Revert:**

```toml
[[vm]]
  memory = "1024mb"
```

---

## 3. Extract a sidecar (when to split a worker out)

**When (CO-286 criteria):**

- A background worker (e.g. embedding generator, report builder) consumes
  significant RAM but is **idle most of the time** (< 20% utilization).
- The worker's idle RAM prevents the main app from scaling down efficiently.
- The worker can tolerate cold-start latency (it's triggered by a job queue,
  not a user request).

**Pattern:**

1. Move the worker's binary/process into a separate Fly app.
2. Configure the sidecar app with its own `fly.toml`:

```toml
app = "<app>-worker"
[http_service]
  auto_stop_machines  = "suspend"
  auto_start_machines = true
  min_machines_running = 0
```

3. Use a lightweight job queue (e.g. a `pending_jobs` SQLite table polled via
   the LiteFS replica, or a simple HTTP POST from the main app to the worker's
   internal address) to hand off work.
4. The main app no longer needs the worker's RAM allocation.

**When NOT to extract:**

- The worker is always busy (> 50% utilization) — suspending it wouldn't save
  meaningful cost.
- The main app and worker share an in-process data structure (mutex, channel)
  that can't be replaced with a serializable queue.

**Revert:** merge the worker code back into the main crate and remove the
sidecar app (`flyctl apps destroy <app>-worker`).

---

## Reverting any of the above

Every change above touches only `fly.toml`. To revert:

```bash
git revert <commit-that-changed-fly.toml>
flyctl deploy
```

Or restore the previous `fly.toml` manually and redeploy. Machine state
(volumes, secrets) is unaffected by `fly.toml` changes.

---

## References

- Baseline sizing and cost estimates: [`docs/infra/fly-baseline-2026-05.md`](fly-baseline-2026-05.md)
- Operations runbook (deploy flow, disk-full recovery): [`docs/OPERATIONS.md`](../OPERATIONS.md)
- CO-281 cost arc: `work/co/CO-281.md`
- CO-285 (suspend), CO-286 (sidecar), CO-287 (memory right-size): closed PRs
