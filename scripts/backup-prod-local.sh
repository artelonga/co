#!/usr/bin/env bash
# Local snapshot of prod (no S3 / no R2 — pure local copy).
#
# Usage:
#   bash scripts/backup-prod-local.sh
#   APP=co-artelonga DEST=~/co-backups bash scripts/backup-prod-local.sh
#   APP=co-artelonga-uat bash scripts/backup-prod-local.sh   # snapshot UAT instead
#
# Requires: flyctl (logged in). No AWS / cloud creds needed.
#
# Output (in $DEST, default ~/co-backups):
#   data-<YYYYMMDD-HHMMSS>.tar.gz — entire /data/ tree (meta.db, per-universe
#                                   data.db files, content .md, /data/co/, etc.)
#
# Restore (manual, full-volume):
#   1. flyctl sftp put -a co-artelonga ./data-<date>.tar.gz /tmp/data.tar.gz
#   2. flyctl ssh console -a co-artelonga
#   3. Inside the machine: rm -rf /data/* && tar xzf /tmp/data.tar.gz -C /data
#   4. flyctl machine restart -a co-artelonga
#
# Restore single universe:
#   tar xzf data-<date>.tar.gz universes/<key>/  (locally first to inspect)

set -euo pipefail

APP=${APP:-co-artelonga}
DEST=${DEST:-$HOME/co-backups}
DATE=$(date -u +%Y%m%d-%H%M%S)

mkdir -p "$DEST"

echo "[backup-local] app=$APP dest=$DEST date=$DATE"

# Tar the entire /data/ tree — captures meta.db, every per-universe data.db,
# all .md content files, /data/co/ (dev board), uat-reset.flag, etc.
#
# WAL-safe note: SQLite WAL mode makes the main DB file checkpoint-consistent
# at all times, with pending writes in `.db-wal`. Including all three (-wal,
# -shm, .db) in the tar gives a recoverable snapshot. We don't use sqlite3
# `.backup` because it isn't installed in the Fly runtime image — falling
# back to cp/tar is the realistic option without modifying the Docker image.
flyctl ssh console -a "$APP" -C "tar czf /tmp/data.tar.gz -C /data ."
flyctl sftp get -a "$APP" /tmp/data.tar.gz "$DEST/data-$DATE.tar.gz"
flyctl ssh console -a "$APP" -C "rm /tmp/data.tar.gz"
echo "[backup-local] /data/ snapshot: $DEST/data-$DATE.tar.gz ($(du -sh "$DEST/data-$DATE.tar.gz" | cut -f1))"

# 3. Drop a manifest alongside so future-you knows what version produced this snapshot.
APP_VERSION=$(curl -s "https://${APP}.fly.dev/api/health" | python3 -c "import sys,json; print(json.load(sys.stdin).get('version','?'))" 2>/dev/null || echo "?")
cat > "$DEST/manifest-$DATE.txt" <<EOF
app:        $APP
date_utc:   $DATE
app_version: $APP_VERSION
host:       $(hostname)
EOF
echo "[backup-local] manifest: $DEST/manifest-$DATE.txt"

echo
echo "[backup-local] DONE. Files:"
ls -lh "$DEST/" | grep -E "$DATE"
echo
echo "[backup-local] NEXT: configure S3/R2 (CO-143) and re-run scripts/backup-prod.sh"
echo "[backup-local] for off-host durability. Local snapshots only protect against Fly volume loss"
echo "[backup-local] — not your laptop being lost / wiped / stolen."
