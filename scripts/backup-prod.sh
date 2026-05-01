#!/usr/bin/env bash
# CO-104: Daily S3 snapshot of SQLite + universes/.
# Runs unattended (cron). No interactive prompts.
#
# Usage: APP=co-artelonga BUCKET=artelonga-co-backups ./scripts/backup-prod.sh
#
# Requires: flyctl, aws CLI, sqlite3 on the Fly machine
# S3 key layout:
#   co.db/<YYYYMMDD-HHMMSS>.db
#   universes/<YYYYMMDD-HHMMSS>.tar.gz
set -euo pipefail

APP=${APP:-co-artelonga}
BUCKET=${BUCKET:-artelonga-co-backups}
DATE=$(date -u +%Y%m%d-%H%M%S)
WORK=$(mktemp -d)
trap "rm -rf $WORK" EXIT

echo "[backup] app=$APP bucket=$BUCKET date=$DATE"

# 1. Snapshot SQLite via the official .backup command (atomic, no lock).
flyctl ssh console -a "$APP" -C "sqlite3 /data/co.db '.backup /tmp/co.db.bak'"
flyctl sftp get -a "$APP" /tmp/co.db.bak "$WORK/co-$DATE.db"
flyctl ssh console -a "$APP" -C "rm /tmp/co.db.bak"
echo "[backup] db snapshot: ok ($(du -sh "$WORK/co-$DATE.db" | cut -f1))"

# 2. Tar the universes directory.
flyctl ssh console -a "$APP" -C "tar czf /tmp/universes.tar.gz -C /data universes"
flyctl sftp get -a "$APP" /tmp/universes.tar.gz "$WORK/universes-$DATE.tar.gz"
flyctl ssh console -a "$APP" -C "rm /tmp/universes.tar.gz"
echo "[backup] universes snapshot: ok ($(du -sh "$WORK/universes-$DATE.tar.gz" | cut -f1))"

# 3. Upload both artifacts to S3. PUT is idempotent — re-running same date is safe.
aws s3 cp "$WORK/co-$DATE.db" "s3://$BUCKET/co.db/$DATE.db"
aws s3 cp "$WORK/universes-$DATE.tar.gz" "s3://$BUCKET/universes/$DATE.tar.gz"

echo "[backup] uploaded to s3://$BUCKET — done (date=$DATE)"
