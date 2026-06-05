# CO Backup Snapshot Format

## Overview

CO backups are self-contained `.tar.gz` archives produced by the `LocalFsBackend`
(and future cloud backends).  Every snapshot is backend-independent: a snapshot
created on the local filesystem can be restored from S3 (or any other backend)
without any format conversion.

## Filename Convention

```
snapshot-<UTC-ISO8601-compact>-<sha256-prefix-16>.tar.gz
```

Example:
```
snapshot-20260605T142300Z-a1b2c3d4e5f6a7b8.tar.gz
```

A companion sidecar file `<filename>.json` is written alongside the tarball by
`LocalFsBackend` to allow fast listing without re-hashing:

```json
{
  "id": "snapshot-20260605T142300Z-a1b2c3d4e5f6a7b8.tar.gz",
  "created_at": "2026-06-05T14:23:00Z",
  "bytes": 7234567,
  "sha256": "a1b2c3d4e5f6a7b8..."
}
```

## Archive Structure

```
snapshot-<ts>-<sha>.tar.gz
├── manifest.json          — snapshot metadata (see below)
├── meta.db                — platform SQLite database (users, universes, entries…)
└── universes/
    ├── <universe-key>/
    │   ├── data.db        — per-universe SQLite (entries, tasks, relations…)
    │   ├── content/       — markdown files
    │   └── blobs/         — binary asset CAS (CO-146)
    └── ...
```

## manifest.json Schema

```json
{
  "created_at":    "2026-06-05T14:23:00Z",
  "co_version":    "2.40.0",
  "schema_version": 14,
  "universes":     ["template", "u-abc123", "u-def456"],
  "backup_backend": "local"
}
```

| Field            | Type   | Description                                    |
|------------------|--------|------------------------------------------------|
| `created_at`     | string | RFC-3339 UTC timestamp of snapshot creation    |
| `co_version`     | string | `CARGO_PKG_VERSION` at time of snapshot        |
| `schema_version` | int    | `MAX(version)` from `schema_version` table     |
| `universes`      | array  | Sorted list of universe keys included          |
| `backup_backend` | string | Name of the backend that created the snapshot  |

The `backup_backend` field lets a future restore tool know where to fetch the
snapshot from — important when multiple backends coexist.

## Backends

| Backend  | Feature flag   | Status      | Notes                                        |
|----------|---------------|-------------|----------------------------------------------|
| `local`  | (always on)    | Full        | Writes to `$CO_BACKUP_DIR` (default `/data/backups/`) |
| `s3`     | `backup-s3`   | Stub        | `unimplemented!` until v3.1+                 |
| `r2`     | `backup-r2`   | Stub        | Cloudflare R2, v3.1+                         |
| `fly`    | `backup-fly`  | Stub        | `flyctl volumes snapshots`, v3.1+            |
| `gcs`    | `backup-gcs`  | Stub        | Google Cloud Storage, v3.1+                  |
| disabled | —             | Short-circuit | `CO_BACKUP_BACKEND=disabled` skips the worker |

## Configuration

| Env var                    | Default             | Description                                  |
|----------------------------|---------------------|----------------------------------------------|
| `CO_BACKUP_BACKEND`        | `local`             | Backend selector: `local`, `s3`, `r2`, `fly`, `gcs`, `disabled` |
| `CO_BACKUP_DIR`            | `/data/backups/`    | Directory for `local` backend                |
| `CO_BACKUP_BUCKET`         | —                   | Bucket name for S3/R2/GCS                    |
| `CO_BACKUP_REGION`         | —                   | Region for S3/R2/GCS                         |
| `CO_BACKUP_RETENTION_DAYS` | `30`                | Delete snapshots older than N days           |
| `CO_BACKUP_INTERVAL_HOURS` | `24`                | Worker tick interval in hours                |

## Admin API

Both endpoints require a valid JWT for `CO_SEED_ADMIN_EMAIL`.

```
POST /api/v1/admin/backup/snapshot
```
Triggers a snapshot immediately (background task).  Returns `202 Accepted`.

```json
{ "status": "accepted", "message": "Snapshot started in background" }
```

```
GET /api/v1/admin/backup/snapshots
```
Lists stored snapshots (newest first).

```json
[
  {
    "id": "snapshot-20260605T142300Z-a1b2c3d4.tar.gz",
    "created_at": "2026-06-05T14:23:00Z",
    "bytes": 7234567,
    "sha256": "a1b2c3d4e5f6a7b8..."
  }
]
```

## Restore

Restore tooling (`co restore --snapshot <id>`) is a separate CLI spec (post-v3.0).
To restore manually:

```bash
# 1. Stop the server
# 2. Extract the tarball
tar xzf snapshot-<ts>-<sha>.tar.gz -C /data

# 3. Verify
sqlite3 /data/meta.db "SELECT MAX(version) FROM schema_version;"

# 4. Restart the server
```

## Integrity Verification

The SHA-256 of the tarball is stored in both the sidecar `.json` and the
manifest inside the tarball.  To verify:

```bash
sha256sum snapshot-<ts>-<sha>.tar.gz
# compare with the `sha256` field in snapshot-<ts>-<sha>.tar.gz.json
```
