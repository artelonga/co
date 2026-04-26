# Breaking Changes — v1.0 → v1.2

This document records every breaking change introduced between CO v1.0 and v1.2.
If you are upgrading a self-hosted CO instance or integrating against the API,
read each section and follow the migration instructions.

---

## Database — Universe visibility model (CO-49)

### What changed

The three boolean columns that controlled universe access were replaced by a
single `visibility` TEXT enum.

| Old column                      | New value                  |
|---------------------------------|----------------------------|
| `is_template = 1`               | `visibility = 'template'`  |
| `requires_login = 1`            | `visibility = 'requires_login'` |
| `is_public = 1, requires_login = 0` | `visibility = 'public-subscribable'` |
| all zeroes (private)            | `visibility = 'private'`   |

Migration v20 (in `storage.rs`) runs automatically on server startup and
backfills `visibility` for every existing row. The old columns (`is_template`,
`is_public`, `requires_login`) are retained for read compatibility but are no
longer the authoritative source; all new code reads `visibility`.

### Action required

No manual action for Fly.io deploys — migration runs on startup.

For manual SQLite upgrades, run:

```sql
ALTER TABLE universes ADD COLUMN visibility TEXT NOT NULL DEFAULT 'private';
UPDATE universes SET visibility = 'template' WHERE is_template = 1;
UPDATE universes SET visibility = 'requires_login'
    WHERE is_template = 0 AND requires_login = 1;
UPDATE universes SET visibility = 'public-subscribable'
    WHERE is_template = 0 AND requires_login = 0 AND is_public = 1;
INSERT INTO schema_version (version) VALUES (20);
```

---

## API — Universe access model (CO-49)

### What changed

`GET /api/v1/universes/:slug` now enforces a deterministic access model based
on `visibility` instead of ad-hoc boolean checks.

| `visibility`             | Anonymous           | Authenticated (non-member) | Member / Owner |
|--------------------------|---------------------|----------------------------|----------------|
| `template`               | 200 read-only       | 200 read-only              | 200 read-only  |
| `private`                | 404                 | 404                        | 200 read+write |
| `requires_login`         | 401                 | 200 read-only              | 200 read+write |
| `public-subscribable`    | 200 metadata only   | 200 metadata only          | 200 read+write |

Previously, some `private` universes returned 403; they now return 404 to avoid
leaking universe existence to unauthorized callers.

### Action required

Update any client code that relied on 403 for private universes — expect 404.

---

## API — Subscriptions endpoint added (CO-49)

A new `subscriptions` table and endpoints enable following `public-subscribable`
universes:

- `POST /api/v1/universes/:slug/subscribe` — subscribe (requires auth)
- `DELETE /api/v1/universes/:slug/subscribe` — unsubscribe (requires auth)
- `GET /api/v1/universes/:slug/subscribers` — subscriber count (public)

These endpoints did not exist in v1.0.

---

## Anonymous user flow — auto-clone removed (CO-22 → current)

### What changed

In earlier builds, visiting the app as an anonymous user automatically cloned
the template universe into a personal `anon-*` universe. This behavior was
removed. Anonymous users now see the template universe in read-only mode.

A clone is only created when the user explicitly requests one
(`POST /api/v1/universes/:slug/clone`).

### Action required

Any `anon-*` universes created by the old auto-clone flow are cleaned up on
server startup via `cleanup_anon_universes()`. No manual cleanup is required,
but you can verify with:

```sql
SELECT COUNT(*) FROM universes WHERE key LIKE 'anon-%';
-- expect: 0 after first restart post-upgrade
```

---

## Project key namespace (CO-21)

### What changed

User universes previously allowed creating a project with key `CO`, which
collided with the template universe's project key. New universes now derive
their default project key from the universe slug:
`{SLUG_UPPER[0..4]}P` (e.g., universe `alice` → project key `ALICP`).

Existing template-universe data is unaffected. Any user project already
named `CO` retains its key — only new projects created after this change
use the derived key.

### Action required

None for existing data. No cross-universe task leaks are possible because
`list_tasks` always filters by both `project_key` AND `universe_key` via
the `entries` table.

---

## Legacy task API — deprecated (phase-out in v1.3)

The following routes still work in v1.2 but are deprecated:

```
GET    /api/projects/:key/tasks
POST   /api/projects/:key/tasks
PUT    /api/projects/:key/tasks/:id
DELETE /api/projects/:key/tasks/:id
GET    /api/projects/:key/tasks/:id/comments
POST   /api/projects/:key/tasks/:id/comments
```

Both the legacy routes and the new entries routes read from the same `entries`
table, so there is no data inconsistency. The legacy routes will be removed in
v1.3.

**Migrate to:** `GET /api/v1/universes/:slug/entries?type=task`

---

## Theme engine — server-side CSS (CO-30)

### What changed

Universe themes are now generated server-side as CSS custom-property blocks
served at `GET /api/v1/universes/:slug/theme.css`. Clients include this
stylesheet via a `<link id="co-theme-css">` tag.

The old approach embedded theme tokens directly in the page HTML.

### Conflict with palette switcher

The client-side palette switcher (`experiment.js`) currently removes the
server theme `<link>` when a user selects a named palette. This means the
user's palette choice is stored only in `localStorage` and is not persisted
across browsers or devices.

**Planned fix (v1.3):** palette selection will persist server-side via
`PUT /api/v1/universes/:slug/config` so that `theme_preset` is the single
source of truth.

---

## CRDT (Yjs) + CLI sync coexistence

Yjs real-time sync (web) and CLI sync both write to the `entries` table.
They are compatible because:

- Yjs flushes dirty entries to the database on idle
- CLI sync detects changes via `body_hash`; it pulls after Yjs has committed

Running Yjs and CLI sync simultaneously is safe as long as CLI sync is not
also pushing concurrently. Simultaneous push from both the Obsidian plugin and
the CLI can produce duplicate pushes; this is tracked for v1.3 (lockfile or
last-write-wins strategy).
