#!/usr/bin/env bash
# CO-119: Restore drill — provision scratch Fly app, restore latest backup,
# smoke-check, log result, tear down.
#
# Usage: ./tools/restore-drill.sh [<snapshot-date YYYYMMDD>]
# Default: yesterday's date.
#
# Requires: flyctl, curl, jq
# Set DRY_RUN=1 to skip Fly provisioning (verifies only local backup availability).
set -euo pipefail

# ---------- Config ----------
DATE=${1:-$(date -u +%Y%m%d -d 'yesterday' 2>/dev/null || date -u -v-1d +%Y%m%d)}
DRY_RUN=${DRY_RUN:-0}
DRILL_APP="co-drill-$(date -u +%Y%m%d-%H%M%S)"
LOG_FILE="tools/restore-drill.log"
SECONDS=0

# ---------- Cleanup trap ----------
cleanup() {
  local exit_code=$?
  if [ "$DRY_RUN" = "0" ] && flyctl apps list 2>/dev/null | grep -q "$DRILL_APP"; then
    echo "Tearing down $DRILL_APP..."
    flyctl apps destroy "$DRILL_APP" --yes 2>/dev/null || true
  fi
  exit $exit_code
}
trap cleanup EXIT

# ---------- Locate backup ----------
BACKUP_FILE=$(ls backups/*"${DATE}"*.db 2>/dev/null | sort | tail -1 || true)
if [ -z "$BACKUP_FILE" ]; then
  # Try yesterday's backup with any timestamp
  BACKUP_FILE=$(ls backups/co-"${DATE}"*.db 2>/dev/null | sort | tail -1 || \
                ls backups/*.db 2>/dev/null | sort | tail -1 || true)
fi

if [ -z "$BACKUP_FILE" ]; then
  echo "WARN: No backup found for $DATE — drill will test fresh-deploy only (no restore step)"
  RESTORE_STEP=0
else
  echo "Found backup: $BACKUP_FILE"
  RESTORE_STEP=1
fi

# ---------- Dry-run early exit ----------
if [ "$DRY_RUN" = "1" ]; then
  echo "DRY_RUN=1: skipping Fly provisioning. Backup check: ${BACKUP_FILE:-NONE}"
  echo "OK  dry_run=1 date=$DATE backup=${BACKUP_FILE:-none} time=${SECONDS}s" \
    | tee -a "$LOG_FILE"
  exit 0
fi

# ---------- Provision scratch app ----------
echo "Creating scratch app: $DRILL_APP"
flyctl apps create "$DRILL_APP" --org personal
flyctl secrets set JWT_SECRET=drill-secret -a "$DRILL_APP"
flyctl deploy --config fly.uat.toml -a "$DRILL_APP" --wait-timeout 300

echo "Waiting for $DRILL_APP to start..."
sleep 15

# ---------- Restore (if backup available) ----------
if [ "$RESTORE_STEP" = "1" ]; then
  echo "Restoring $BACKUP_FILE into $DRILL_APP..."
  ./scripts/restore.sh "$BACKUP_FILE" "$DRILL_APP"
  sleep 20
fi

# ---------- Smoke checks ----------
BASE="https://${DRILL_APP}.fly.dev"

HEALTH=$(curl -sf "$BASE/api/health" | jq -r '.status' 2>/dev/null || echo "FAIL")
UNIV_COUNT=$(curl -sf "$BASE/api/v1/universes" 2>/dev/null | jq 'length' 2>/dev/null || echo 0)
TEMPLATE=$(curl -sf "$BASE/api/v1/universes/template" 2>/dev/null | jq -r '.key' 2>/dev/null || echo "")

# ---------- Evaluate ----------
if [ "$HEALTH" = "ok" ] && [ "$UNIV_COUNT" -ge 1 ] && [ "$TEMPLATE" = "template" ]; then
  STATUS="OK"
else
  STATUS="FAIL"
fi

RESULT="${STATUS}  drill=${DRILL_APP}  date=${DATE}  restore=${RESTORE_STEP}  \
health=${HEALTH}  univ=${UNIV_COUNT}  template=${TEMPLATE:-missing}  time=${SECONDS}s"

echo "$RESULT" | tee -a "$LOG_FILE"

[ "$STATUS" = "OK" ] && exit 0 || exit 1
