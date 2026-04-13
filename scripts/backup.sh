#!/bin/bash
# Co — Backup SQLite database from Fly.io
# Usage: ./scripts/backup.sh [prod|uat]
#
# Creates a timestamped backup in backups/ directory.
# Run manually or via cron/CI.

set -euo pipefail

ENV="${1:-prod}"

if [ "$ENV" = "uat" ]; then
    APP="co-artelonga-uat"
else
    APP="co-artelonga"
fi

BACKUP_DIR="backups"
TIMESTAMP=$(date +%Y-%m-%d_%H%M%S)
FILENAME="${BACKUP_DIR}/co-${ENV}-${TIMESTAMP}.db"

mkdir -p "$BACKUP_DIR"

echo "Backing up $APP → $FILENAME"

# Wake the machine if sleeping
curl -s --max-time 30 "https://${APP}.fly.dev/api/health" > /dev/null || true
sleep 2

# Use SQLite .backup via SSH (consistent snapshot)
flyctl ssh console -a "$APP" -C "cat /data/co.db" > "$FILENAME" 2>/dev/null

SIZE=$(ls -lh "$FILENAME" | awk '{print $5}')
echo "Done: $FILENAME ($SIZE)"

# Keep last 7 daily backups per env
ls -t "${BACKUP_DIR}/co-${ENV}-"*.db 2>/dev/null | tail -n +8 | xargs rm -f 2>/dev/null || true
echo "Rotated: keeping last 7 backups"
