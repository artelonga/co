# CO Architecture — Post-GitHub Model (v2.0)

## Overview

CO is a graph-based content management platform. Each **universe** is a namespace
that owns a collection of **entries** (Markdown files indexed in SQLite) and
optionally a **project** board. The platform runs as a single Rust binary
(`co-web`) on Fly.io backed by a single SQLite database and a per-universe file
tree on the Fly volume.

## Data model

```
universes (SQLite row + /data/universes/<key>/ directory)
  └── entries   (source-of-truth: .md files; index: SQLite)
  └── universe_members
  └── subscriptions
users
  └── api_tokens
schema_version
```

## Canonical sync path: Vault REST API

`PUT /api/v1/universes/:slug/vault/<path>` is the canonical way to push content.
Editors (Obsidian, CLI, scripts) use an API token (`POST /api/v1/auth/token`).
The git-clone-on-server path (CO-50, CO-55) was removed in v1.22.7 (CO-64).

## Migration system

Migrations live in `co-web/src/storage.rs::run_migrations()`. Every
`ALTER TABLE ADD COLUMN` uses `ensure_column` (idempotent, CO-137).
Current schema version: **23**.

## Forward references

- **CO-70** — `_universe.yaml` manifest format
- **CO-71** — per-universe schema validator
- **CO-77** — per-universe SQLite + LiteFS replicas
- **CO-61** — Sync Protocol v1 (op log, 3-way merge)
