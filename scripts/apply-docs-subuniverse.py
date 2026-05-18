#!/usr/bin/env python3
"""Apply the docs-subuniverse template to every code repo.

Each repo's `docs/architecture/` becomes a registered CO sub-universe with:
  - `_universe.yaml` — universe metadata + schema slot
  - `schema.yaml`    — content_types: as-is, api-catalog, refactor-plan, task-backlog, change-log
  - `CHANGELOG.md`   — iceberg-compatible append-only state log header (entries logged via entry_events at runtime)

Idempotent — overwrites templates in place.
"""
from pathlib import Path

REPOS = {
    "co":          ("/Users/artelonga/projects/co/docs/architecture",          "CO Architecture",          "co"),
    "rfq-gateway": ("/Users/artelonga/projects/rfq-gateway/docs/architecture", "RFQ Gateway Architecture", "rfq-gateway"),
    "quilombo-blog": ("/Users/artelonga/projects/quilombo-blog/docs/architecture", "Quilombo Blog Architecture", "quilombo-blog"),
    "ArteLonga":   ("/Users/artelonga/projects/ArteLonga/docs/architecture",   "ArteLonga Architecture",   "artelonga"),
    "yggdrasil":   ("/Users/artelonga/projects/yggdrasil/docs/architecture",   "Yggdrasil Architecture",   "yggdrasil"),
    "comunicacao": ("/Users/artelonga/projects/comunicacao/docs/architecture", "Comunicacao Architecture", "comunicacao"),
}

UNIVERSE_TEMPLATE = """---
slug: {slug}-arch
name: {name}
description: >-
  Architecture audit + refactor backlog for the {parent_slug} repo.
  Sub-universe of `{parent_slug}` — auto-tracked via CO's `entry_events`
  append-only log (Iceberg-compatible per `co::public/transaction-log.md`).
visibility: private
parent_universe: {parent_slug}
kind: docs-subuniverse
schema: schema.yaml
content_root: .
expected_files:
  - as-is.md
  - api-catalog.md
  - refactor-plan.md
  - task-backlog-summary.md
created_at: 2026-05-18T00:00:00Z
---
"""

SCHEMA_TEMPLATE = """name: Docs Subuniverse
description: >-
  Canonical schema for any repo's `docs/architecture/` folder.
  Each file in `expected_files` maps to a content_type below.
  Every write produces an `entry_events` row (Iceberg-compatible append-only log).
schema_version: 1
content_types:
  - name: as-is
    description: C4-aligned current-state snapshot (Context + Containers + Components, dependency graph, API catalog reference).
    expected_path: as-is.md
    frontmatter:
      audit_date: { type: date, required: true }
      c4_levels: { type: array, items: enum, values: [context, container, component, code] }
      dep_graph_format: { type: enum, values: [mermaid, dot, plantuml] }
  - name: api-catalog
    description: Endpoint catalog organized by surface, with verb + path + status + auth + response shape.
    expected_path: api-catalog.md
    frontmatter:
      catalog_date: { type: date, required: true }
      endpoint_count: { type: number }
      drift_check_script: { type: string }
  - name: refactor-plan
    description: Gap analysis vs the seven design principles plus proposed REPO-N tasks (scope, acceptance, blast radius, priority).
    expected_path: refactor-plan.md
    frontmatter:
      audit_date: { type: date, required: true }
      task_count: { type: number }
      first_task_id: { type: number }
      principle_scorecard: { type: array, items: object }
  - name: task-backlog-summary
    description: Cross-cutting backlog roll-up grouped by repo → epic → user story, with conventional commit + semver per task.
    expected_path: task-backlog-summary.md
    frontmatter:
      summary_date: { type: date, required: true }
      epic_count: { type: number }
      story_count: { type: number }
      net_semver_bump: { type: enum, values: [major, minor, patch, none] }
  - name: change-log
    description: >-
      Optional iceberg-compatible change log for this subuniverse.
      Each row mirrors an `entry_events` event (created, updated, deleted) — used when the subuniverse
      lives outside CO's runtime and needs to bootstrap state.
    expected_path: CHANGELOG.md
    frontmatter:
      iceberg_namespace: { type: string }
      first_event_at: { type: date }
      schema_evolution: { type: array, items: object }
"""

CHANGELOG_TEMPLATE = """# {name} — Iceberg-compatible Change Log

> This file is the **bootstrap log** for the `{slug}-arch` subuniverse.
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
"""

for repo_key, (path_str, display_name, parent_slug) in REPOS.items():
    p = Path(path_str)
    if not p.exists():
        print(f"[{repo_key}] SKIP — {p} missing")
        continue
    slug = parent_slug
    (p / "_universe.yaml").write_text(UNIVERSE_TEMPLATE.format(slug=slug, name=display_name, parent_slug=parent_slug))
    (p / "schema.yaml").write_text(SCHEMA_TEMPLATE)
    (p / "CHANGELOG.md").write_text(CHANGELOG_TEMPLATE.format(slug=slug, name=display_name))
    print(f"[{repo_key}] applied → {p}")

print("\nDone. Each repo now has a registered docs-subuniverse template.")
