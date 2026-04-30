#!/usr/bin/env bash
# Restore a CO SQLite backup into a target Fly app.
# Usage: ./scripts/restore.sh <backup-file-or-date> <target-app>
#
# If <backup-file-or-date> looks like a date (YYYYMMDD), picks the most
# recent backup matching that date under backups/. Otherwise treats it as
# a direct path to a .db file.
set -euo pipefail

BACKUP_ARG=${1:-}
TARGET_APP=${2:-}

if [ -z "$BACKUP_ARG" ] || [ -z "$TARGET_APP" ]; then
  echo "Usage: $0 <backup-file-or-date> <target-app>" >&2
  exit 1
fi

# Resolve backup file
if [ -f "$BACKUP_ARG" ]; then
  BACKUP_FILE="$BACKUP_ARG"
else
  # Treat as date prefix — find most recent matching file
  BACKUP_FILE=$(ls backups/*"${BACKUP_ARG}"*.db 2>/dev/null | sort | tail -1 || true)
  if [ -z "$BACKUP_FILE" ]; then
    # Try S3 if AWS CLI available and CO_BACKUP_BUCKET is set
    if [ -n "${CO_BACKUP_BUCKET:-}" ] && command -v aws &>/dev/null; then
      BACKUP_FILE=$(mktemp /tmp/co-restore-XXXXXX.db)
      aws s3 cp "s3://${CO_BACKUP_BUCKET}/co-${BACKUP_ARG}.db" "$BACKUP_FILE"
    else
      echo "No backup found for date '$BACKUP_ARG' in backups/ and CO_BACKUP_BUCKET not set" >&2
      exit 1
    fi
  fi
fi

echo "Restoring $BACKUP_FILE → $TARGET_APP"

# Copy backup file to the Fly machine
flyctl sftp shell -a "$TARGET_APP" <<EOF
put $BACKUP_FILE /data/co.db
EOF

# Restart so the server picks up the new DB
flyctl machine restart -a "$TARGET_APP"

echo "Restore complete. Allow 30s for restart, then verify:"
echo "  curl -s https://${TARGET_APP}.fly.dev/api/health"
