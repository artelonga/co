# CO-463 — Move `event_log` out of the hot global meta.db (scalability)

> Status: spec / scoped 2026-06-17. Origin: the EDA bridge flood that wrote 37.5M
> `bridge.event_sent` rows (~18 GB) into the global `meta.db`, filling the prod
> volume and proving the current design isn't horizontally scalable. The bridge
> *transport* flood is already fixed (3.15.3 — `AtividadesPersistor` no longer
> persists `bridge.*`); this task is the structural follow-up so it can't recur in
> any other shape.

## Evidence (2026-06-17 forensic, read from the live file header)
- prod `meta.db` = 18.65 GiB file, but **only 4.37 GiB is real data**; 14.28 GiB
  (76.6%) is freelist (empty) pages left by the reclaim deletes that ran but were
  never VACUUMed.
- `event_log` composition (June-16 snapshot): `bridge.event_sent` 37,454,261 rows
  (99.1%); every other type < 210k. i.e. the table is ~entirely transport noise.

## The scalability problems this exposed
1. **`event_log` lives in the hot transactional `meta.db`.** It is append-only and
   unbounded; at the bridge's ~73 events/s that was 6.3M rows/day. Even with the
   30-day retention task that is ~190M-row steady-state pressure in the DB that
   also serves users/auth/telemetry.
2. **One global `Mutex<Storage>` serializes every write.** This is *why* the
   reclaim was impossible: a VACUUM (or any long op, or a write spike) locks the
   whole app. 4 in-process reclaim attempts each took the machine down because the
   exclusive lock + the shared-cpu disk I/O starved `/api/health` → Fly restart.
3. **Maintenance can't be done online.** Shrinking the file needs `VACUUM` =
   exclusive lock for the op's duration; incompatible with serving traffic.

## Proposed direction (not yet owner-approved — needs design sign-off)
- **Separate the EDA log from `meta.db`.** Put `event_log` (+ `bridge_state`) in
  its own SQLite file (`/data/event_log.db`) so its growth and its VACUUM/retention
  never lock the app's transactional DB. Lowest-effort, biggest win.
- **Only persist domain events.** Transport/relay events (`bridge.*`) stay
  in-memory (observability ring buffer) and are never written. (Done for the known
  types; make the persist allowlist explicit/positive rather than a denylist.)
- **Wire CO-449 cold-tier.** The S3/Parquet archival exists but isn't active;
  age `event_log` to cold storage so the hot file stays small, and let
  `auto_vacuum=INCREMENTAL` (set in v087, still latent) actually reclaim online.
- **Longer term:** the global `Mutex<Storage>` is the real horizontal-scale ceiling
  (per-universe DBs exist via CO-77, but the global meta.db is the shared hot path).
  Track a follow-up to shard/replicate or move hot counters off the single mutex.

## Immediate state (no action needed on prod)
prod meta.db is effectively a 4.4 GiB DB in an 18.65 GiB file; the 14 GiB of free
pages are reused before the file grows, so there is **no disk urgency**. A one-off
`VACUUM` (temp Fly machine or off-box, never on the live shared-cpu machine — see
the 2026-06-17 reclaim post-mortem) reclaims the 14 GiB whenever convenient.
