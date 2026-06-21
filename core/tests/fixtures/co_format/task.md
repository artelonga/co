---
id: 1
prefix: QUI
title: "Schema extensions for sync protocol v1 — sha256, ator, operacao, hlc"
type: task
status: todo
priority: high
parent: projects/sync.md
labels:
  - schema
  - sync
  - co-61
module: content
created_at: 2026-04-14T00:00:00Z
---
GIVEN the sync protocol carries logical deltas,
WHEN we add content-addressed identifiers,
THEN clients can dedup and resume safely.

## Acceptance

- [ ] sha256 column added
- [ ] ator recorded per delta
- [ ] hlc monotonic per actor
