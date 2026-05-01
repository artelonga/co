#!/usr/bin/env bash
# CO-104: Daily S3 snapshot of SQLite + universes/.
# Runs unattended (cron). No interactive prompts.
#
# Usage: APP=co-artelonga BUCKET=artelonga-co-backups ./scripts/backup-prod.sh
#
# Works with any S3-compatible backend (AWS, Garage, MinIO, Cloudflare R2):
#   AWS_ENDPOINT_URL=http://localhost:9000  # MinIO / Garage
#   AWS_ENDPOINT_URL=https://...r2.cloudflarestorage.com  # Cloudflare R2
#   (unset = AWS default)
#
# Requires: flyctl (authenticated), aws CLI
# S3 key layout:
#   co.db/<YYYYMMDD-HHMMSS>.db
#   universes/<YYYYMMDD-HHMMSS>.tar.gz
set -euo pipefail

APP=${APP:-co-artelonga}
BUCKET=${BUCKET:-artelonga-co-backups}
DATE=$(date -u +%Y%m%d-%H%M%S)
WORK=$(mktemp -d)
trap "rm -rf $WORK" EXIT

# aws CLI honours AWS_ENDPOINT_URL automatically for S3-compatible backends.
S3="aws s3"

echo "[backup] app=$APP bucket=$BUCKET date=$DATE endpoint=${AWS_ENDPOINT_URL:-aws}"

# flyctl -C runs exec (not a shell), so wrap compound commands in 'sh -c "..."'.

# 1. Snapshot the main database.
# CO-77: renamed co.db → meta.db; falls back to co.db for pre-CO-77 targets.
# sqlite3 .backup is the preferred atomic hot backup; falls back to cp.
flyctl ssh console -a "$APP" -C \
  "sh -c 'DB=/data/meta.db; [ -f \"\$DB\" ] || DB=/data/co.db; command -v sqlite3 >/dev/null 2>&1 && sqlite3 \"\$DB\" \".backup /tmp/meta.db.bak\" || cp \"\$DB\" /tmp/meta.db.bak'"
flyctl sftp get -a "$APP" /tmp/meta.db.bak "$WORK/co-$DATE.db"
flyctl ssh console -a "$APP" -C "sh -c 'rm -f /tmp/meta.db.bak'"
echo "[backup] db snapshot: ok ($(du -sh "$WORK/co-$DATE.db" | cut -f1))"

# 2. Tar the universes directory (includes per-universe data.db files).
flyctl ssh console -a "$APP" -C "sh -c 'tar czf /tmp/universes.tar.gz -C /data universes'"
flyctl sftp get -a "$APP" /tmp/universes.tar.gz "$WORK/universes-$DATE.tar.gz"
flyctl ssh console -a "$APP" -C "sh -c 'rm /tmp/universes.tar.gz'"
echo "[backup] universes snapshot: ok ($(du -sh "$WORK/universes-$DATE.tar.gz" | cut -f1))"

# 3. Upload both artifacts to S3. PUT is idempotent — re-running same date is safe.
$S3 cp "$WORK/co-$DATE.db" "s3://$BUCKET/co.db/$DATE.db"
$S3 cp "$WORK/universes-$DATE.tar.gz" "s3://$BUCKET/universes/$DATE.tar.gz"

echo "[backup] uploaded to s3://$BUCKET — done (date=$DATE)"
