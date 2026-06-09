# Migration validation checklist (CO-376)

Before a release that contains new migrations is deployed, validate those migrations
against a **copy** of real data. The goal is to catch the class of failure recorded in
the 1.22.4 prod incident (an unguarded read of a freshly-migrated column) *before* it
reaches prod — without ever touching a live database.

## Tooling: `migrate_check`

`co-web/src/bin/migrate_check.rs` applies the current binary's migrations against an
extracted snapshot and runs read-only smoke assertions:

```bash
cargo run --bin migrate_check -- <extracted-snapshot-dir>
```

It:

1. records a pre-migration baseline (raw connections, no migration);
2. runs **meta-db** migrations via `Storage::new(dir)` (and the entry-split);
3. opens each universe so **per-universe pool** migrations run;
4. asserts the wave's tables/columns exist and are selectable, the `yuri` admin user
   survived, row counts that should be stable are unchanged, and the conserved entry
   total (meta + all universes) did not drift more than ±5%.

Exit code: **0** iff every check passed, **1** if any failed, **2** on bad invocation.
The failing check (and its query error) is printed.

## CI-sandbox flow (no live DB touched)

The snapshot is produced by the pluggable backup backend and is a single `.tar.gz`
(`manifest.json` + `meta.db` + `universes/{l1}/{l2}/{key}/data.db`). Obtain one of:

**A. Trigger + download via the admin API** (requires an admin session/token):

```bash
# 1. trigger a WAL-safe snapshot (uses rusqlite Backup; does not lock writers)
curl -s -X POST https://co-artelonga-staging.fly.dev/api/v1/admin/backup/snapshot \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# 2. list to get the newest snapshot id
curl -s https://co-artelonga-staging.fly.dev/api/v1/admin/backup/snapshots \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

**B. Or fetch the nightly snapshot artifact** the backup worker writes to the configured
backend (default `local` at `/data/backups/snapshot-*.tar.gz`).

Then, inside a CI runner or a throwaway local dir:

```bash
mkdir -p /tmp/snap && tar xzf snapshot-*.tar.gz -C /tmp/snap
cargo run --bin migrate_check -- /tmp/snap
```

A non-zero exit blocks the release. A green run is the go/no-go signal for applying the
migrations in production.

## Reading the output

- `meta schema_version advanced` — the meta-db reached the new version.
- `bridge_state` / `sync_conflicts` tables, `universes.source_*` columns,
  `entries.source_marker` — wave-specific shape checks; extend per release.
- `user count unchanged` / `universe count unchanged` — additive migrations must not
  change these; a delta means seeding ran on a fresh DB (expected only when validating
  an empty DB, not a real snapshot).
- `entry total (meta + universes)` — conserved across the entry-split and additive pool
  migrations; must stay within ±5%.

## Deferred (CO-376 follow-up — not yet built)

This is the MVP. Still to come:

- A **gating GitHub Action** that runs `migrate_check` automatically on any PR touching
  `co-web/src/storage/migrations.rs` or `co-web/src/platform/universe_pool.rs`, pulling
  the latest staging snapshot read-only and blocking merge on failure.
- `GET /api/v1/admin/migrations/snapshots` — list in-flight validation runs.
- Custom per-migration assertions in `migrations/<vN>.smoke.sql`.
- 24h retention automation for failed-run snapshots.
