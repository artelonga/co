---
title: Auto-sync — weekly upload of admin repos to prod
created: 2026-05-03
related: CO-145
---

# Weekly auto-sync

Every Sunday at 09:00, your local Mac runs `scripts/sync-all.sh`, which:

1. Pulls fresh credentials from the macOS keychain.
2. POSTs to `/api/v1/auth/password-login` to refresh the session cookie.
3. For each of `quilomboaraucaria` / `artelonga` / `rfq` / `co`: `git pull --ff-only`, then runs `scripts/bulk-upload-binary.py` against `co-artelonga.fly.dev`.
4. Logs to `~/.co/sync.log`.

The bulk uploader is idempotent (sha256 content addressing for binaries; INSERT OR REPLACE for vault entries), so re-running is a no-op when nothing changed.

## One-time setup

```bash
# 1. Stash the admin password in your keychain (replace YOUR_PASSWORD):
security add-generic-password \
    -a yuri@artelonga.com.br \
    -s co-prod-admin \
    -w 'YOUR_PASSWORD' \
    -U

# 2. Verify the launchd job is loaded:
launchctl list | grep co-sync
# → -  0  com.artelonga.co-sync

# 3. Test it once before next Sunday:
bash ~/projects/co/scripts/sync-all.sh
tail -50 ~/.co/sync.log
```

## Files

| Path | Purpose |
|---|---|
| `~/projects/co/scripts/sync-all.sh` | The wrapper (pull + login + upload all 4) |
| `~/projects/co/scripts/bulk-upload-binary.py` | The two-pass uploader |
| `~/Library/LaunchAgents/com.artelonga.co-sync.plist` | The schedule (Sundays 09:00) |
| `~/.co/cookie.txt` | Refreshed session cookie (also symlinked to `/tmp/c.txt`) |
| `~/.co/sync.log` | Per-run log |
| `~/.co/launchd-stdout.log` / `.../launchd-stderr.log` | launchd's view |

## Tweaks

- **Change the schedule:** edit `~/Library/LaunchAgents/com.artelonga.co-sync.plist`, then `launchctl unload && launchctl load -w` it.
- **Add a repo:** append a `slug|/path` pair to the `REPOS` array in `sync-all.sh`.
- **Pause:** `launchctl unload ~/Library/LaunchAgents/com.artelonga.co-sync.plist`.
- **Resume:** `launchctl load -w ~/Library/LaunchAgents/com.artelonga.co-sync.plist`.
- **Fully remove:** unload, then `rm ~/Library/LaunchAgents/com.artelonga.co-sync.plist`.

## Why local launchd, not cloud cron?

Anthropic's `/schedule` cloud cron can't reach `/Users/artelonga/projects/...` — it runs in Anthropic's environment, not yours. Auto-sync needs your local file tree as the source. macOS's launchd is the right tool.

If you want to lift this to a server-side pull (CO-91 territory), the design is: have prod periodically clone/pull from `github.com/artelonga/<repo>` directly. That's a deploy + new endpoint and outside this loop's scope.
