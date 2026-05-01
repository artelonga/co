#!/usr/bin/env bash
# CO-104: Restore a CO backup into a target Fly app.
#
# Usage:
#   restore.sh <local-file.db> <target-app>
#   restore.sh <YYYYMMDD-HHMMSS> <target-app>
#   restore.sh <YYYYMMDD-HHMMSS> co-artelonga --yes-i-want-to-overwrite-prod
#
# If the first argument is an existing file path, only the SQLite DB is restored
# (local-file mode; used by restore-drill.sh). If it looks like a backup date,
# both co.db and universes/ are pulled from S3 (requires BUCKET + aws CLI).
#
# Works with any S3-compatible backend — set AWS_ENDPOINT_URL for Garage/MinIO/R2.
#
# The target must never default to production — pass the explicit flag to
# override this protection.
set -euo pipefail

BACKUP_ARG=${1:-}
TARGET=${2:-}
PROD_CONFIRM=${3:-}
BUCKET=${BUCKET:-artelonga-co-backups}
WORK=$(mktemp -d)
trap "rm -rf $WORK" EXIT

if [ -z "$BACKUP_ARG" ] || [ -z "$TARGET" ]; then
  echo "Usage: $0 <date-YYYYMMDD-HHMMSS | local-file.db> <target-app> [--yes-i-want-to-overwrite-prod]" >&2
  exit 1
fi

# Safety guard: refuse to overwrite production without an explicit flag.
if [ "$TARGET" = "co-artelonga" ] && [ "$PROD_CONFIRM" != "--yes-i-want-to-overwrite-prod" ]; then
  echo "ERROR: target is the production app (co-artelonga)." >&2
  echo "       Pass --yes-i-want-to-overwrite-prod as the third argument to confirm." >&2
  exit 1
fi

# ── Resolve backup artifacts ──────────────────────────────────────────────────

if [ -f "$BACKUP_ARG" ]; then
  # Local file mode — used by restore-drill.sh. Only SQLite is restored.
  DB_FILE="$BACKUP_ARG"
  UNIVERSES_FILE=""
  echo "[restore] local file: $DB_FILE → $TARGET"
else
  # Date mode — pull both artifacts from S3.
  DATE="$BACKUP_ARG"
  DB_FILE="$WORK/co.db"
  UNIVERSES_FILE="$WORK/universes.tar.gz"

  if ! command -v aws &>/dev/null; then
    echo "ERROR: aws CLI not found. Install it or set CO_BACKUP_BUCKET and use local files." >&2
    exit 1
  fi

  echo "[restore] pulling s3://$BUCKET/co.db/$DATE.db..."
  aws s3 cp "s3://$BUCKET/co.db/$DATE.db" "$DB_FILE"

  echo "[restore] pulling s3://$BUCKET/universes/$DATE.tar.gz..."
  aws s3 cp "s3://$BUCKET/universes/$DATE.tar.gz" "$UNIVERSES_FILE"

  echo "[restore] s3 → $TARGET (date=$DATE)"
fi

# ── Restore SQLite ────────────────────────────────────────────────────────────

# flyctl -C runs exec (not a shell), so wrap compound commands in 'sh -c "..."'.
# flyctl sftp put refuses to overwrite existing files, so upload to /tmp and move.
# CO-77: database lives at meta.db (falls back to co.db for pre-CO-77 targets).
flyctl sftp put -a "$TARGET" "$DB_FILE" /tmp/co-restore.db
flyctl ssh console -a "$TARGET" \
  -C "sh -c 'DB=/data/meta.db; [ -f /data/co.db ] && DB=/data/co.db; rm -f \"\$DB\" \"\$DB-wal\" \"\$DB-shm\" && mv /tmp/co-restore.db \"\$DB\"'"
echo "[restore] db: ok"

# ── Restore universes/ (S3 mode only) ────────────────────────────────────────

if [ -n "${UNIVERSES_FILE:-}" ] && [ -f "$UNIVERSES_FILE" ]; then
  flyctl sftp put -a "$TARGET" "$UNIVERSES_FILE" /tmp/universes.tar.gz
  flyctl ssh console -a "$TARGET" \
    -C "sh -c 'rm -rf /data/universes && tar xzf /tmp/universes.tar.gz -C /data && rm /tmp/universes.tar.gz'"
  echo "[restore] universes/: ok"
fi

# ── Restart ───────────────────────────────────────────────────────────────────

# flyctl machine restart requires a machine ID in non-interactive mode.
MACHINE_ID=$(flyctl machine list -a "$TARGET" --json 2>/dev/null \
  | python3 -c "import sys,json; m=json.load(sys.stdin); print(m[0]['id'])" 2>/dev/null \
  || echo "")
if [ -n "$MACHINE_ID" ]; then
  flyctl machine restart "$MACHINE_ID" -a "$TARGET"
else
  flyctl apps restart "$TARGET"
fi

echo "[restore] done. Allow ~30 s for restart, then verify:"
echo "  curl -s https://${TARGET}.fly.dev/api/health"
