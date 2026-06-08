# Conflict Resolution — CO-385

CO-385 implements Mac-style conflict resolution for cross-device sync. When the
CO-384 federated bridge detects a differing `body_hash` between a local and a
remote entry, it records a conflict and surfaces it to the user via the
`sync.conflict_detected` EDA event.

## Conflict Kinds

| Kind | When |
|---|---|
| `both_modified` | Same path exists on both sides with different hashes |
| `local_only_new` | Entry only exists locally (remote never saw it) |
| `remote_only_new` | Entry only exists on the remote side |
| `local_deleted_remote_modified` | Local hash is empty (deleted); remote has content |
| `local_modified_remote_deleted` | Local has content; remote hash is empty (deleted) |

Same-hash pairs are always silently skipped (hash-skip optimization).

## Action Tree

Each `ConflictKind` exposes a subset of the 7 available actions:

| Action | Description | Applicable kinds |
|---|---|---|
| `keep_both` | Rename local to `_1` suffix; write remote at original path | `both_modified`, `remote_only_new` |
| `ignore` | No-op — mark resolved without changing entries | all |
| `replace` | Overwrite local with remote (remote wins) | `both_modified`, `remote_only_new`, `local_deleted_remote_modified` |
| `update` | 3-way line merge; emits conflict markers when both sides changed | `both_modified`, `local_modified_remote_deleted` |
| `upsert` | Merge existing diffs + bulk-insert remote-only entries | `both_modified` |
| `accept_delete` | Mirror remote delete — hard-delete the local entry | `local_only_new`, `local_deleted_remote_modified`, `local_modified_remote_deleted` |
| `keep_local` | Resurrect from local snapshot; discard remote | `local_only_new`, `local_deleted_remote_modified`, `local_modified_remote_deleted` |

### Suggested defaults

- **yggdrasil** universe: `replace` (remote wins — CO-383 integration)
- All other universes: `update` (3-way merge)

## 3-Way Text Merge

`update` and `upsert` run a line-based 3-way merge:

1. If `local == remote` → identical, return as-is.
2. If `local == ancestor` → only remote changed, return remote.
3. If `remote == ancestor` → only local changed, return local.
4. Both sides changed → emit git-style conflict markers:

```
<<<<<<< local
<local content>
=======
<remote content>
>>>>>>> remote
```

The user then resolves the markers manually and re-saves.

## Bulk Apply

Every resolve request accepts an optional `apply_to_all_matching: true` flag.
When set, the same action is applied to all other unresolved conflicts of the
same `ConflictKind` in the same universe (up to 100 at a time). A
`sync.conflict_resolved_bulk` event is published with the count.

## REST API

```
GET  /api/v1/me/sync/conflicts?universe=<key>
POST /api/v1/sync/conflicts/{id}/resolve
     Body: { "action": "<action>", "apply_to_all_matching": false }
```

Both endpoints require authentication (JWT or session cookie).

## EDA Events

| Event | Published by | Payload |
|---|---|---|
| `sync.conflict_detected` | Bridge handler | `conflict_id`, `path`, `kind`, `local_body_hash`, `remote_body_hash`, `source`, `resolve_url` |
| `sync.conflict_resolved` | Resolver | `conflict_id`, `path`, `action`, `universe`, `kind` |
| `sync.conflict_resolved_bulk` | Resolve endpoint | `count`, `action`, `universe` |

## UI

`co-web/static/variants/a/modules/sync/conflicts.js` provides:

- `renderConflictsPanel(container, universe)` — full panel with conflict list and a
  bulk-apply checkbox.
- `renderConflictRow(conflict, onAction)` — per-row with action buttons filtered
  by `ConflictKind`.
- `wireLiveConflictCta(eventBus, openPanel)` — wires the CO-381 live timeline to
  show a "Resolver →" toast whenever `sync.conflict_detected` arrives.

## Database Schema (migration v67)

```sql
CREATE TABLE sync_conflicts (
    id                  TEXT PRIMARY KEY,
    universe_key        TEXT NOT NULL,
    path                TEXT NOT NULL,
    local_body_hash     TEXT NOT NULL,
    remote_body_hash    TEXT NOT NULL,
    common_ancestor_hash TEXT,
    kind                TEXT NOT NULL,
    detected_at         TEXT NOT NULL,
    resolved_at         TEXT,
    resolution_action   TEXT,
    resolved_by         TEXT,
    FOREIGN KEY (universe_key) REFERENCES universes(key)
);
CREATE INDEX idx_sync_conflicts_unresolved ON sync_conflicts(detected_at) WHERE resolved_at IS NULL;
CREATE INDEX idx_sync_conflicts_universe   ON sync_conflicts(universe_key, detected_at DESC);
```
