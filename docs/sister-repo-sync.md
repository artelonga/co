# Sister-Repo Sync (CO-337)

CO universes can pull their content from a remote git repository on the production
machine, mirroring what `local_repo_path` does on localhost.

## Overview

The sync resolves a content source per universe in this order:

1. **`local_repo_path`** (set AND exists) → used as-is (CO-330 behavior; dev convenience)
2. **`remote_url`** (set, local not available) → shallow-cloned to `<data-dir>/remote-repos/<key>/`
3. **Neither** → no content seeded

This means: on localhost, the local checkout wins. On prod (where `~/projects/comunicacao`
doesn't exist), the remote clone takes over automatically.

## Cadence

- On every `co serve` boot
- Every 15 minutes via the `remote_sister_repo_sync` background worker

Override the interval with `CO_REMOTE_SYNC_INTERVAL_SECS` (seconds):

```bash
flyctl secrets set CO_REMOTE_SYNC_INTERVAL_SECS=300 -a co-artelonga  # 5 min
```

## Configuring a universe

### Via the API

```bash
PATCH /api/v1/universes/<key>/source
Content-Type: application/json
Authorization: Bearer <token>

{
  "remote_url": "https://github.com/artelonga/comunicacao",
  "remote_ref": "main",
  "content_subdirs": ["docs", "content", "sources"]
}
```

All fields are optional; only supplied fields are updated.

### Via sqlite3 on the Fly machine (one-time backfill)

```bash
flyctl ssh console -a co-artelonga -C "sqlite3 /data/meta.db \"
  UPDATE universes
  SET remote_url='https://github.com/artelonga/comunicacao',
      remote_ref='main'
  WHERE key='comunicacao';
\""
# Restart to trigger the first sync
flyctl machine restart -a co-artelonga
```

Repeat for each sister universe (`mbya`, `topologia`, `artelonga`, `yggdrasil`,
`rfq`, `quilomboaraucaria`).

## Schema columns (migration v56)

| Column | Type | Description |
|--------|------|-------------|
| `remote_url` | `TEXT` | HTTPS or SSH git URL |
| `remote_ref` | `TEXT` | Branch, tag, or SHA (default: `main`) |
| `remote_last_sync` | `TEXT` | ISO-8601 timestamp of last successful sync |

## Authentication

### Public repos (no auth needed)

Most sister repos are public on GitHub. `https://github.com/artelonga/<repo>` clones
without any credentials.

### HTTPS with a Personal Access Token

```bash
flyctl secrets set CO_GIT_TOKEN=<github-pat> -a co-artelonga
```

The token is injected as an HTTP `Authorization` header — it never appears in the
process argument list.

### SSH key (private repos)

1. Generate a deploy key: `ssh-keygen -t ed25519 -f /tmp/co-deploy -N ""`
2. Add the public key to the repo's Deploy Keys on GitHub (read-only)
3. Mount the private key as a Fly secret file:

```bash
flyctl secrets set CO_GIT_SSH_KEY_PATH=/etc/co-deploy -a co-artelonga
flyctl secrets set CO_GIT_SSH_KEY="$(cat /tmp/co-deploy)" -a co-artelonga
# Then mount it via fly.toml [files] or a pre-start script
```

When `CO_GIT_SSH_KEY_PATH` is set, git receives
`GIT_SSH_COMMAND=ssh -i <path> -o StrictHostKeyChecking=no -o BatchMode=yes`.

## Observability

Sync events are logged at `INFO` level:

```
CO-337: synced remote repo for comunicacao from https://github.com/artelonga/comunicacao
```

Failures are logged at `WARN` and do not abort the boot sequence:

```
CO-337: failed to sync remote repo for mbya: git fetch failed ... (exit 128)
```

Check `remote_last_sync` via sqlite to see when each universe last synced:

```bash
flyctl ssh console -a co-artelonga -C \
  "sqlite3 /data/meta.db 'SELECT key, remote_url, remote_last_sync FROM universes WHERE remote_url IS NOT NULL;'"
```

## Not in scope

- Push / write-back to the remote — CO is a read-only consumer
- Partial clones / sparse checkout — full shallow clone (`--depth=1`)
- Conflict resolution between local and remote — local always wins
- Webhook-driven instant sync — planned for Phase 2
