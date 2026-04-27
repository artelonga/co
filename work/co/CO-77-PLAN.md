---
title: "CO-77 — Per-universe SQLite + LiteFS — Detailed Implementation Plan"
status: planning
parent: 77
created_at: 2026-04-27T00:00:00Z
---

# CO-77 — Detailed implementation plan

This is the deep-dive plan that supplements `CO-77.md`. The headline spec is "shard storage per universe with LiteFS read replicas." This document answers *how* to land it without a downtime window or data loss, and what to measure as we go.

## 1. Why this is a 2.0 release

Every read and write today goes through `state.storage.lock()` (a single `Mutex<Storage>`). Per-universe SQLite changes the basic shape of every method on `Storage`:

```rust
// today
fn list_entries(&self, universe_key: &str) -> Vec<Entry>
// after CO-77
fn list_entries(&self, universe_key: &str) -> Vec<Entry>  // unchanged signature
//   …but internally opens <data>/universes/<aa>/<bb>/<key>/data.db
```

The signatures stay the same; the internals do not. Roughly 80 storage methods need to choose: route to `meta.db` (universe metadata, users, sessions) or to `<universe>/data.db` (entries, projects, tasks, ops, manifests). Any method that mixes both pays a multi-DB cost (read meta then read universe; or two-phase write).

Because most read paths and many write paths shift, this is a **2.0 release**, not 1.x. SemVer-wise we get to break backwards compat once; it's worth using the budget here so CO-63's manifest work (CO-71 generic JSON storage in particular) lands cleanly on top.

## 2. Layout

```
/data/
├── meta.db                                # tier-zero: small, hot, replicated
│   ├── users
│   ├── universes (key, name, owner_id, visibility, created_at, …)
│   ├── universe_members
│   ├── subscriptions
│   ├── sessions / api_tokens
│   └── schema_version (for meta migrations)
│
└── universes/
    ├── ab/cd/abcd1234-foo/                # 2-level fanout: SHA256(key)[0..2]/[2..4]
    │   ├── data.db                        # entries, projects, tasks, ops, manifests
    │   ├── data.db-wal
    │   ├── data.db-shm
    │   └── blobs/                         # local blob cache (CO-81 stores remote)
    │
    ├── ab/cd/abcd5678-bar/
    │   └── data.db
    │
    └── …
```

Fanout decision: hash the key (SHA256 hex), take chars 0..2 and 2..4. At 10M universes that's ~150 entries per leaf directory at the deepest level — well within ext4's comfortable range.

Universe-key collision avoidance: append a short suffix only when a true hash collision happens (vanishingly rare). For now, use the literal key as the directory name; if two universes share a hash prefix that's expected.

## 3. Connection pool

```rust
struct UniverseConnectionPool {
    inner: Mutex<lru::LruCache<String, Arc<Mutex<Connection>>>>,
    max_open: usize,        // default 1000
    data_dir: PathBuf,
}

impl UniverseConnectionPool {
    fn get(&self, universe_key: &str) -> Result<Arc<Mutex<Connection>>> {
        // 1. cache hit?  return.
        // 2. cache miss → open <data_dir>/universes/<aa>/<bb>/<key>/data.db
        //    apply WAL pragmas, run migrations if schema_version differs
        // 3. if cache full, evict LRU (Drop closes the connection)
        // 4. insert + return
    }
}
```

Failure modes:
- **Open errors** (corruption, missing dir) → bubble up as `AppError::Internal`; do not silently re-create
- **Migration mid-life** → all per-universe DBs share the same migration sequence; on cache-miss open, check `PRAGMA user_version` and run any pending steps. Migrations must be backward-compatible (additive, never destructive) so a half-migrated population works
- **Concurrent open of same universe** → `lru::LruCache` is single-threaded; outer `Mutex` serializes. Acceptable since open is fast (<1ms) and only happens on cache miss

## 4. Migration from monolithic `co.db`

The existing prod `co.db` holds ~1 GB of mixed data. Migration steps:

### Stage A — Schema reorganization (no data motion)

1. Add a `meta.db` at `/data/meta.db` with the global tables.
2. `INSERT INTO meta.universes SELECT … FROM co.universes;` (read-only on co.db)
3. Same for `users`, `universe_members`, `subscriptions`, `sessions`, `api_tokens`.
4. Don't touch `co.db` yet — it remains the source of truth for entries/projects/tasks.

### Stage B — Per-universe DB extraction (data motion, online)

For each universe in `meta.db`:
1. Compute target path `/data/universes/<aa>/<bb>/<key>/data.db`
2. `mkdir -p` it; create empty SQLite, apply scaffold migration to `schema_version=1`
3. `INSERT INTO universe.entries SELECT … FROM co.entries WHERE universe_key = ?`
4. Same for `projects`, `tasks`, `ops` (when CO-61 lands), etc.
5. Once the per-universe DB is consistent, set a `migration_complete` flag on `meta.universes`.
6. The application reads from `meta.universes.migration_complete` to decide:
   - If true → use per-universe DB
   - If false → use legacy co.db (fallback)

This lets the migration run in the background while the app stays up. New writes during migration go to whichever side is "current" per universe, with the flag flipped atomically per row.

### Stage C — Decommission `co.db`

1. After every universe shows `migration_complete = true`, run validation: pick 100 random entries from each universe DB, compare to legacy co.db.
2. Rename `co.db` → `co.db.legacy.YYYY-MM-DD`. Don't delete for 30 days.
3. Code paths that branched on `migration_complete` are removed in a follow-up release (1 release after migration).

### Failure modes during migration

- **Partial extraction crashes** → `migration_complete` flag protects readers. On restart, resume per-universe migration from scratch (idempotent: DELETE + INSERT into target).
- **Schema drift mid-migration** → migrations must be additive. If CO-77 ships alongside any other schema-changing PR, batch them in the same migration step.
- **Disk full during migration** → mid-extraction, target DB is incomplete; flag never flips; readers stay on legacy. Add retry on disk-space recovery.

## 5. LiteFS configuration

```toml
# /etc/litefs.yml
fuse:
  dir: "/data"

data:
  dir: "/var/lib/litefs"

http:
  addr: ":20202"

lease:
  type: "consul"            # or "static" for single-region v1
  hostname: "litefs.${FLY_REGION}.internal"
  consul:
    url: "http://consul:8500"
    key: "litefs/co/primary"

exec:
  - cmd: "/app/co-web"
```

LiteFS replicates each `*.db` file independently, including the new per-universe DBs and `meta.db`. Reads can route to replicas; writes always go to the primary (Fly's machine in `gru` for now).

Read routing logic in `Storage`:
```rust
fn read_handle(&self, universe_key: &str) -> Result<ReadHandle> {
    // Check LiteFS replica lag for this DB; if < 1s, route to replica.
    // Otherwise route to primary.
    // On strong-consistency reads (e.g., post-write), force primary.
}
```

For v1, **all reads go to primary**. Replica routing is a follow-up once the sharded layout is stable.

## 6. Cross-universe queries (rare but exist)

| Query | Today | After CO-77 |
|-------|-------|-------------|
| "List my universes" | one row in meta | one row in meta — unchanged |
| "Search across all my universes for entry X" | single SELECT | aggregator: list universe keys from meta, fan out per-universe SELECTs in parallel, merge |
| "Global event feed" | single SELECT | aggregator across N universe DBs OR a denormalized `global_events` table in meta (preferred for hot paths) |
| "Backup everything" | one file | iterate per-universe + meta, snapshot each |

Aggregators are bounded to logged-in user's universes (typically <100), so fanout stays cheap. For genuine cross-tenant queries (admin telemetry), a denormalized table in `meta.db` is the answer.

## 7. Backup strategy

- **Per-universe**: `sqlite3 /data/universes/<aa>/<bb>/<key>/data.db ".backup /tmp/<key>.db"` then `aws s3 cp` to object storage. Tag with `universe_key`, `version`, `timestamp`.
- **meta.db**: same, separate cadence (more frequent — every hour vs. per-universe daily).
- **LiteFS auto-snapshots**: configured for hourly via `litefs.yml` `retention.duration: 168h`.
- **Per-universe restore**: copy the backup file in place, restart the connection pool. No effect on other universes.

This is a major operational improvement over today's "back up the whole 1 GB co.db nightly."

## 8. Performance budgets

| Operation | Target (p99) | How measured |
|-----------|-------------|--------------|
| Connection-pool hit | < 1 ms | local LRU lookup |
| Connection-pool miss + open | < 10 ms | mostly fs metadata + WAL header read |
| Cross-universe aggregation (10 universes) | < 50 ms | parallel fanout, dominated by single slowest universe |
| Per-universe write throughput | 100 wps sustained | dedicated mutex per universe — no global serialization |
| `meta.db` read | < 1 ms | small DB, hot in OS cache |
| `meta.db` write | < 5 ms | small, infrequent (universe creates) |

## 9. Risks (in priority order)

1. **Migration data loss** — mitigated by online migration with flag, validation pass, 30-day legacy retention.
2. **LRU eviction during a write** — possible if cache is undersized. Mitigation: writes hold the connection's `Arc<Mutex<>>` for the duration; eviction sees `Arc::strong_count > 1` and skips.
3. **Disk fragmentation at 10M directories** — measured via Fly volume's filesystem; ext4 with `dir_index` mounted handles this. If we ever hit limits, switch to a flat namespace with hash-as-filename.
4. **LiteFS lag during high-write periods** — replica reads see stale data. Mitigation: route writes always to primary; reads requiring strong consistency also to primary.
5. **API token validation cost** — currently one `meta.db` read per request (cheap). At 1M tokens, an LRU on the validator helps.

## 10. Sequencing

| Order | What | Why |
|-------|------|-----|
| 1 | Land scaffolding code (UniverseConnectionPool, migration framework) — no behavior change yet | Reviewable; doesn't ship the migration |
| 2 | Add `meta.db` with global tables; mirror writes (write to both, read from co.db) | Catches schema-inference bugs early |
| 3 | Cut over reads to `meta.db` for global tables | Validates meta.db is correct |
| 4 | Per-universe DB extraction with `migration_complete` flag | The big migration; can take days to drain in production |
| 5 | Decommission co.db | Final sweep |

This is genuinely a multi-week effort, not a weekend hack. Plan accordingly.

## 11. What stays out of scope

- **Cross-universe transactions** — unsupported. Application code that needs cross-universe atomicity has to use sagas or live with eventual consistency.
- **Online schema migrations on live universe DBs** — hard to do safely. Schedule downtimes per universe (or per universe pool) when needed.
- **Per-universe encryption keys** — interesting but a separate epic.

## 12. Decision log

- **One file per universe vs. one schema per universe in shared file?** → File per universe. Schema-per-table doesn't shard locks at the SQLite level; you still have one file = one writer.
- **Sharding by user vs. by universe?** → Universe. A user has many universes; co-locating one user's universes on one machine is a future load-balancing question, not a sharding question.
- **Postgres + sharding vs. SQLite + LiteFS?** → SQLite. LiteFS gives us 80% of replication for 5% of operational cost. Postgres becomes the answer when (a) cross-tenant transactions become required or (b) >100 GB per universe is normal — neither is plausible in the next 18 months.
