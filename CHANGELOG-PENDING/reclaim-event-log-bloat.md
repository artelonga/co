## reclaim-event-log-bloat — one-time boot reclaim of the bridge-flood event_log bloat

The EDA bridge flood left ~18 GB of `bridge.event_sent`/`bridge.event_received`
rows in `event_log` (meta.db ≈ 19 GB). They're under the 30-day retention window
so they won't age out yet, and SQLite won't shrink the file in place — the prod
volume kept hitting disk-pressure.

### What changed
- New `Storage::reclaim_event_log_transport_bloat()`: deletes the `bridge.*`
  transport rows in **WAL-checkpointed batches** (so the write-ahead log can't
  balloon past one batch on a near-full disk), then runs a full `VACUUM`. The
  VACUUM rewrites only the small live remainder (temp ≈ live size, not the bloated
  file size, so it fits a tight volume) **and activates the `auto_vacuum=INCREMENTAL`**
  that's been latent since v087 — so future retention reclaims pages automatically.
- A one-shot boot task (`event_log_reclaim_boot_task`) runs it ~20 s after boot
  (server already bound + passing the storage-free `/api/health` check), gated by
  the new `CO_MAINTENANCE_RECLAIM_EVENT_LOG` flag. Idempotent — a no-op once the
  bloat is gone, so the flag is safe to leave set.
- `fly.toml` sets `CO_MAINTENANCE_RECLAIM_EVENT_LOG = "1"` so the next prod deploy
  performs the reclaim. Remove it on a later deploy once confirmed.

### Why
Reclaims the volume without the ~30-minute download-and-swap outage, and fixes the
root cause of the recurring disk-pressure (the file now shrinks and stays shrunk).
