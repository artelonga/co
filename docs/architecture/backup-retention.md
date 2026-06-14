# Backup retention & local junk sweep (CO-459)

> Local-first backup + reclamation for `/data`. Reconstructable, verifiable,
> and parametrized so the same math carries over to S3/StaaS (CO-458/CO-81).
> Builds on the CO-405 free-space guard + retention and the CO-365 backup
> backend trait.

## 1. Reconstructable local snapshot

`POST /api/v1/admin/backup` (admin-gated) writes a **directory** snapshot:

```
/data/backups/<ts>/
├── manifest.json              # index — sha256 + bytes for every file below
├── meta.db                    # VACUUM INTO copy (committed, consistent)
└── universes/<aa>/<bb>/<key>/
    ├── data.db                # VACUUM INTO copy
    └── blobs/<aa>/<bb>/<sha>  # byte copies
```

- **Consistency.** `meta.db` and every per-universe `data.db` are copied with
  SQLite `VACUUM INTO`, not a hot byte-copy of a live WAL file. A `VACUUM INTO`
  target reflects the last committed state, so the snapshot is never torn.
  `-wal`/`-shm` sidecars are intentionally skipped — their committed content is
  already folded into the vacuumed copy.
- **Verifiability.** `manifest.json` records `{path, sha256, bytes}` per file.
  Re-hashing the directory and comparing to the manifest **proves** the snapshot
  can be reconstructed (no missing/drifted files). The endpoint runs this check
  itself and returns `verified: true|false` alongside the manifest.

This coexists with the CO-365 tarball backend under the same `CO_BACKUP_DIR`:
tarballs are `*.tar.gz` files; these snapshots are `<ts>/` directories carrying a
`manifest.json`. Retention treats only the directory snapshots (see §3).

## 2. Junk sweep

`POST /api/v1/admin/sweep` (admin-gated). **Dry-run is the default**; add
`?apply=true` to delete. The dry-run returns the full report —
`{items[], reclaimable_bytes, removed: 0, dry_run: true}` — so an operator sees
*what* would go and *how much* it frees before anything is touched.

| Kind            | Candidate                                        | Safety guard (checked again at delete time)               |
|-----------------|--------------------------------------------------|-----------------------------------------------------------|
| `AnonUniverse`  | anon clones (`anon-`/`u-`) older than the TTL    | `DELETE … WHERE owner_id LIKE 'anon-%' …` — never re-homed |
| `TempFile`      | `*.tmp`, `*.lock`, `co-backup-*`, `snapshot-tmp-*` | filename pattern **and** mtime older than max-age        |
| `LogFile`       | rotated logs `*.log.N`, `*.log.gz`               | rotated suffix only — the live `*.log` is never matched   |
| `OrphanBlob`    | `assets` rows with `refcount <= 0`               | `DELETE … WHERE refcount <= 0` re-checked per blob        |
| `ExpiredBackup` | directory snapshots beyond retention             | CO-405 `select_prunable` (always keeps the newest)        |

Every removal is logged (`tracing::info!`) — no silent deletes
(`feedback_serve_only_indexed`). Nothing is removed without first re-confirming
its reference/owner guard, so a concurrent re-home or re-reference cancels the
delete.

## 3. Retention math (count + space, reuses CO-405)

Snapshots are pruned by the same decision function as the CO-405 tarball worker
(`storage::backup::worker::select_prunable`), applied to the directory snapshots
via `snapshot_dir::prunable_local_snapshots`. With snapshots ordered
newest → oldest (index `i`, cumulative bytes `C_i = Σ_{j≤i} bytes_j`):

```
keep   i == 0                                  (newest is ALWAYS a restore point)
prune  i >= retain_count                       (count cap)
   or  retain_max_bytes > 0 && C_i > retain_max_bytes   (space cap)
   or  created_at_i < now - retention_days     (age cap, secondary)
```

A snapshot is pruned if **any** cap trips; the newest is exempt from all of
them. Same knobs as CO-405 (one place to configure backup retention):

| Env var                      | Default   | Meaning                                        |
|------------------------------|-----------|------------------------------------------------|
| `CO_BACKUP_RETAIN_COUNT`     | `3`       | keep at most N snapshots                        |
| `CO_BACKUP_RETAIN_MAX_BYTES` | `0`       | cumulative size cap (`0` = unlimited)           |
| `CO_BACKUP_RETENTION_DAYS`   | `30`      | age cap (secondary)                             |
| `CO_BACKUP_DIR`              | `/data/backups/` | snapshot root (shared with CO-365 tarballs) |
| `CO_SWEEP_ANON_TTL_DAYS`     | `30`      | anon-clone expiry before sweep                  |
| `CO_SWEEP_TEMP_MAX_AGE_HOURS`| `24`      | min age before a temp/log file is swept         |

### Worked example (the 2026-06-11 outage)

Four restart-burst snapshots of 264/266/280/299 MB filled a 3 GB volume. With
`retain_count=3` the oldest (299 MB) is pruned; the newest three remain. A
`retain_max_bytes` of, say, `800 MB` would instead keep only the snapshots whose
running total stays ≤ 800 MB (264 + 266 = 530 MB ✓; +280 = 810 MB ✗ → prune from
the third on). Whichever cap trips first wins.

## 4. Where S3 / StaaS takes over (CO-458 / CO-81)

This is **"local backup for now."** The snapshot builder, manifest format, and
retention math are storage-agnostic: the same `<ts>/ + manifest.json` layout and
`select_prunable` policy apply unchanged when the destination becomes an object
store. The handoff point is the `BackupBackend` trait (CO-365): a future
`S3Backend`/`R2Backend` (post-CO-81) uploads the verified snapshot directory and
runs the identical retention selection against remote listings. Out of scope
here (by design): S3 upload, telemetry archival (CO-449), and StaaS billing
(CO-460).
