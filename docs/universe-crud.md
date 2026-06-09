# Universe CRUD — add a universe from web or CLI

A universe is a **CRUD resource**, and its markdown entries are a sub-resource.
Both the web API and the `co` CLI write to the same `Storage` layer, so "add a
universe" is the same operation through either front-end — **no `seed.rs` edit, no
deploy** (per `feedback_no_hardcoded_content_mappings`: bindings live in the DB).

```
            ┌──────────────┐         ┌──────────────┐
   web ───▶ │  REST + Vault │ ──┐     │   co launch  │ ◀─── local CLI
            └──────────────┘   │     └──────────────┘
                               ▼            │
                        ┌─────────────────────────┐
                        │   Storage (SQLite)        │
                        │   universes + per-universe │
                        │   data.db (entries)        │
                        └─────────────────────────┘
```

## Resource model

- **Universe** — a row in the `universes` table (`key`, `name`, `visibility`,
  `local_repo_path` / `remote_url` binding, …).
- **Entries** — markdown files, stored per-universe in `data.db`, addressed by path.

## A) Web — HTTP CRUD

### Universe

| Op | Endpoint | Handler |
|----|----------|---------|
| **Create** | `POST /api/v1/universes` `{key,name,description}` | `create_universe` |
| **Read** | `GET /api/v1/universes/{slug}` · `/public` · `/search` · `/{slug}/projects` | `get_universe_info` … |
| **Update** | `PUT /api/v1/universes/{slug}` `{name?,description?,visibility?}` | `update_universe` |
| **Bind source** | `PATCH /api/v1/universes/{slug}/source` `{local_repo_path?,remote_url?,remote_ref?,content_subdirs?}` | `patch_universe_source` |
| **Delete** | `DELETE /api/v1/universes/{slug}` | `delete_universe` |
| extra | `POST /{slug}/clone` · `POST /{slug}/duplicate` | |

### Entries (content) — the Vault API

| Op | Endpoint |
|----|----------|
| **Create/Update** | `PUT /api/v1/universes/{slug}/vault/{path}` (markdown body) |
| **Read** | `GET /api/v1/universes/{slug}/vault/{path}` · `GET …/vault/` (list) |
| **Delete** | `DELETE /api/v1/universes/{slug}/vault/{path}` |
| browse | `…/vault/tree` · `…/vault/tags` · `POST …/vault/search` |

Auth: bearer token from `POST /api/v1/auth/token`.

**Add a universe from the web** = `POST /universes` → (optional `PATCH …/source`) →
loop `PUT …/vault/{path}` for each `content/*.md`. `scripts/bulk-upload.py` wraps the
Vault loop.

## B) Local — `co` CLI

`co launch` is the create + bind + seed abstraction in one command. From inside the
universe's directory:

```bash
cd ~/projects/grcsamazonia
co launch --key grcsamazonia --name "Escola de Samba Amazônia" --public
# → Universe 'grcsamazonia' provisioned: 16 pages, 0 tasks across 0 projects
```

Under the hood (`co-cli/src/commands/launch.rs`) it:

1. `ensure_local_universe(key, name, public)` — **create**;
2. `update_universe_source(key, local_repo_path=<repo root>)` — **bind** (so
   `seed_orchestrator` re-ingests on the next `co serve` *without a hardcoded list*);
3. `seed_universe_from_local_repo(["docs","content"])` + work tasks — **seed entries**.

`co serve` then serves it at `/{key}`. `co init` / `co new` create spaces / content
*within* a universe.

## The gap: CLI → remote

| Path | Status |
|------|--------|
| Local dev (`co launch` → local SQLite) | ✅ |
| Prod deployed (Vault API `PUT` from web / script) | ✅ |
| **CLI → remote running server over HTTP** | ❌ no `co push` |

The CLI writes only local storage; pushing a local universe's content to a deployed
server today means hitting the Vault API via a script. The missing edge — a
`co push --remote <url> --token <t>` that wraps `POST /universes` + Vault `PUT`s so the
CLI and web hit identical endpoints — is specced in **CO-392**.

## Recommended end state

1. `POST /universes` + Vault `PUT`s as the **canonical add path**; retire the
   hardcoded universe list in `seed.rs` (`co launch` already proves the no-hardcoding
   path).
2. Ship **CO-392 `co push`** so "add a universe" is **one verb, identical semantics,
   web or CLI, no deploy**.
