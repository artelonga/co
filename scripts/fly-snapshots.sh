#!/usr/bin/env bash
# Fly volume snapshot helper — call weekly (cron or launchd).
#
# For each app that has a persistent volume, this script:
#   1. SSH's into the running machine
#   2. Runs sqlite3 .backup on the active DB(s) inside the volume
#   3. Copies the backup file out via flyctl ssh sftp
#   4. Saves locally as ~/co-backups/<app>/<YYYY-MM-DD>/<db-name>.db
#
# Designed to be safe to interrupt and idempotent. Skips apps where no
# machines are running (machine restart counts the snapshot as missed).
#
# Usage:
#   bash scripts/fly-snapshots.sh            # snapshot all configured apps
#   bash scripts/fly-snapshots.sh co-artelonga  # snapshot one app
#
# Recommended cadence: weekly via cron
#   0 4 * * 1 /Users/artelonga/projects/co/scripts/fly-snapshots.sh >> ~/co-backups/fly-snapshots.log 2>&1

set -euo pipefail

BACKUP_ROOT="${BACKUP_ROOT:-$HOME/co-backups}"
TODAY="$(date +%Y-%m-%d)"

# (app, db-paths-to-snapshot)
# db-paths is space-separated list of paths inside the machine's volume
APPS=(
    "co-artelonga:/data/meta.db /data/game.db"
    "yggdrasil-artelonga:/data/yggdrasil.db /data/yggdrasil-sementes.db"
    "quilombo-araucaria:/app/data/quilombo.db"
    "rfq:/app/artifacts"   # rfq stores JSONL ring buffers, not SQLite — copy the dir
)

snapshot_app() {
    local app="$1"
    local dbs="$2"
    local outdir="$BACKUP_ROOT/$app/$TODAY"
    mkdir -p "$outdir"

    echo ""
    echo "=== $app ==="

    # Check if the app has a running machine; flyctl ssh fails on stopped apps.
    if ! flyctl status -a "$app" 2>/dev/null | grep -q "started"; then
        echo "  no started machine — skipped"
        return
    fi

    for db in $dbs; do
        local name
        name="$(basename "$db")"
        echo "  snapshotting $db → $outdir/$name"

        if [[ "$db" == *".db" ]]; then
            # SQLite — use .backup for a consistent online snapshot
            local tmp="/tmp/snap-$(date +%s)-$name"
            if ! flyctl ssh console -a "$app" -C "sqlite3 $db \".backup $tmp\"" >/dev/null 2>&1; then
                echo "  ✗ sqlite backup failed"
                continue
            fi
            if ! flyctl ssh sftp get "$tmp" "$outdir/$name" -a "$app" >/dev/null 2>&1; then
                echo "  ✗ sftp get failed"
                continue
            fi
            flyctl ssh console -a "$app" -C "rm -f $tmp" >/dev/null 2>&1 || true
            local size
            size="$(stat -f %z "$outdir/$name" 2>/dev/null || stat -c %s "$outdir/$name")"
            echo "  ✓ $name ($size bytes)"
        else
            # Directory (e.g., rfq artifacts) — tar + sftp
            local tar="$outdir/$name.tar.gz"
            if ! flyctl ssh console -a "$app" -C "tar czf /tmp/snap.tar.gz $db && cat /tmp/snap.tar.gz" \
                > "$tar" 2>/dev/null; then
                echo "  ✗ dir snapshot failed"
                continue
            fi
            flyctl ssh console -a "$app" -C "rm -f /tmp/snap.tar.gz" >/dev/null 2>&1 || true
            local size
            size="$(stat -f %z "$tar" 2>/dev/null || stat -c %s "$tar")"
            echo "  ✓ $name.tar.gz ($size bytes)"
        fi
    done
}

# --- main ---

if [[ $# -gt 0 ]]; then
    # Single-app mode
    for entry in "${APPS[@]}"; do
        app="${entry%%:*}"
        dbs="${entry#*:}"
        if [[ "$app" == "$1" ]]; then
            snapshot_app "$app" "$dbs"
            exit 0
        fi
    done
    echo "Unknown app: $1"
    echo "Configured apps:"
    for entry in "${APPS[@]}"; do
        echo "  ${entry%%:*}"
    done
    exit 1
fi

# All-apps mode
echo "Fly snapshots — $TODAY"
for entry in "${APPS[@]}"; do
    app="${entry%%:*}"
    dbs="${entry#*:}"
    snapshot_app "$app" "$dbs"
done

echo ""
echo "Done. Backups in $BACKUP_ROOT"

# Cleanup: retain last 8 weeks per app
echo ""
echo "Pruning backups older than 56 days..."
find "$BACKUP_ROOT" -type d -mtime +56 -depth 2 -exec rm -rf {} + 2>/dev/null || true
echo "Done."
