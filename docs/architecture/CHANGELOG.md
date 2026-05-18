# CO Architecture — Iceberg-compatible Change Log

> This file is the **bootstrap log** for the `co-arch` subuniverse.
> Once registered into a live CO instance, every write to `docs/architecture/`
> appends an event row to CO's `entry_events` table — that table is the
> authoritative log (Parquet/Iceberg export tracked in `co::public/transaction-log.md`).
>
> This file exists so the subuniverse has a deterministic state history even
> when offline / before registration.

## 2026-05-18 — initial audit
- as-is.md created
- api-catalog.md created
- refactor-plan.md created
- task-backlog-summary.md created  *(co only)*
- `_universe.yaml` + `schema.yaml` template applied via `scripts/apply-docs-subuniverse.py`

## Iceberg schema (target)

After registration in CO, this subuniverse's events flow into:

```
table: <co-iceberg-warehouse>.entries_events.<slug>_arch
schema:
  event_id        STRING (uuid)
  event_at        TIMESTAMP
  actor           STRING
  action          STRING  ENUM(created, updated, deleted, renamed)
  entry_path      STRING
  prev_body_hash  STRING NULLABLE
  new_body_hash   STRING NULLABLE
  frontmatter     JSON
partitioning: days(event_at)
```

The `entry_events` schema in CO (CO 2.7.25+) is forward-compatible with this Iceberg shape.
