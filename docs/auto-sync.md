---
title: Auto-sync — continuous delta sync to prod (every 5s)
created: 2026-05-03
updated: 2026-05-03
related: CO-145, CO-151
---

# Continuous auto-sync

A long-running daemon (`co-watch.py`) polls the 4 admin repos every 5 seconds, computes a delta against the previous snapshot, and propagates changes to prod:

- **New / modified `.md`** → `PUT /api/v1/universes/{slug}/vault/{path}` (with `![](relative.jpg)` → `![](sha256:…)` rewriting)
- **New / modified binary** (image, video, pdf, audio, svg) → `POST /api/v1/universes/{slug}/assets` (idempotent by sha256)
- **Deleted file** → `DELETE /api/v1/universes/{slug}/vault/{path}`

The daemon is managed by **launchd** (`com.artelonga.co-sync`) with `KeepAlive=true` and `RunAtLoad=true` — it starts at login and auto-restarts on crash (30s throttle).

## Latency

| Operation | Observed end-to-end |
|---|---|
| Touch `.md` → on prod | 4 – 8 s |
| Delete `.md` → 404 on prod | 5 – 9 s |

The 5s poll interval is the dominant factor. CO-151 upgrades to protobuf-over-WebSocket for sub-500ms latency.

## One-time setup

```bash
# 1. Stash the admin password in your keychain:
security add-generic-password \
    -a yuri@artelonga.com.br \
    -s co-prod-admin \
    -w 'YOUR_PASSWORD' \
    -U

# 2. Verify the launchd job is running:
launchctl list | grep co-sync
# → 74531  0  com.artelonga.co-sync   (PID = currently running)

# 3. Tail the log to see ticks:
tail -f ~/.co/watch.log
```

The session cookie is auto-refreshed on 401. If `~/.co/cookie.txt` ever goes stale, the next 401 triggers a fresh `password-login` from the keychain.

## Files

| Path | Purpose |
|---|---|
| `~/projects/co/scripts/co-watch.py` | The continuous watcher daemon |
| `~/projects/co/scripts/bulk-upload-binary.py` | The bootstrap uploader (idempotent; for first import) |
| `~/projects/co/scripts/sync-all.sh` | The pull-then-bulk-upload one-shot script (now superseded by the watcher; kept as a manual fallback) |
| `~/Library/LaunchAgents/com.artelonga.co-sync.plist` | launchd plist (continuous, KeepAlive) |
| `~/projects/co/scripts/co-sync.plist` | Mirror of the plist for repo tracking |
| `~/.co/cookie.txt` | Session cookie (also symlinked at `/tmp/c.txt`) |
| `~/.co/watch.log` | Per-tick log |
| `~/.co/launchd-stdout.log` / `.../launchd-stderr.log` | launchd's view of the daemon |

## Tweaks

- **Pause:** `launchctl unload ~/Library/LaunchAgents/com.artelonga.co-sync.plist`
- **Resume:** `launchctl load -w ~/Library/LaunchAgents/com.artelonga.co-sync.plist`
- **Tighter polling:** edit `POLL_INTERVAL` in `co-watch.py` (default 5 s).
- **Add a repo:** append a `(slug, path)` to the `REPOS` list in `co-watch.py`.
- **Restart now:** `launchctl kickstart -k gui/$(id -u)/com.artelonga.co-sync`.
- **Fully remove:** unload, then `rm ~/Library/LaunchAgents/com.artelonga.co-sync.plist`.

## Wire format (v1)

JSON over the existing REST endpoints. Per-tick fan-out: one HTTP request per changed file. Reuses TLS keep-alive via urllib's connection pool.

CO-151 upgrades this to protobuf `SyncDelta` over WebSocket with zstd compression and bidirectional flow (server can push deltas to connected clients).

## Bootstrap vs continuous

The first time you point the daemon at a repo, the initial snapshot treats existing files as already-synced — **no upload storm on first start**. If the prod universe is missing those files, run the one-shot bootstrap first:

```bash
bash ~/projects/co/scripts/sync-all.sh
```

That POSTs every binary and PUTs every markdown once, then the watcher picks up deltas from there. (The bootstrap is itself idempotent — running it on an already-synced repo uploads zero new bytes.)

## Why local launchd, not cloud cron?

Anthropic's `/schedule` cloud cron can't reach `/Users/artelonga/projects/...` — it runs in Anthropic's environment, not yours. Local file watching needs local processes. macOS launchd is the right tool.

If you want to lift this to a server-side pull (CO-91 territory), the design is: have prod periodically clone/pull from `github.com/artelonga/<repo>` directly. That's an extra deploy + new endpoint, and gives up the live-edit-to-prod feedback loop. Local watcher is faster.
