#!/usr/bin/env bash
# compact-meta-db.sh — reclaim the ~18 GB of dead `bridge.*` rows from prod meta.db.
#
# WHY: the EDA bridge flooded `event_log` (38M rows / ~18 GB) before the 3.15.3
# fix stopped the bleeding. Those rows are < 30 days old, so the retention task
# won't delete them yet, and SQLite won't shrink the file in-place. A full VACUUM
# on the box is impossible (needs ~19 GB temp on a volume with ~6 GB free) and the
# runtime image has no sqlite3. So we compact OFF-box: download → prune + VACUUM
# locally → upload the small result → swap.
#
# ⚠️ REQUIRES DOWNTIME on co-artelonga (machine stopped during the swap, ~download
#    time + a minute). Run it when a short outage is acceptable. NOT autonomous —
#    you must run this yourself (it stops a prod machine and replaces its DB).
#
# Safety gates: keeps the original prod copy locally, verifies the compacted DB
# passes integrity_check AND retains the critical tables before uploading.
set -euo pipefail

APP=co-artelonga
WORK="${WORK:-$HOME/projects/co/.meta-compact-$(date -u +%Y%m%dT%H%M%SZ)}"
ORIG="$WORK/meta-prod-orig.db"
COMPACT="$WORK/meta-compact.db"
BRIDGE_TYPES="'bridge.event_sent','bridge.event_received'"

mkdir -p "$WORK"
echo "==> Work dir: $WORK"

command -v sqlite3 >/dev/null || { echo "FATAL: sqlite3 not installed locally"; exit 1; }

echo "==> [1/6] Stopping $APP (downtime begins) ..."
flyctl machine stop -a "$APP"

echo "==> [2/6] Downloading /data/meta.db (this is ~19 GB; be patient) ..."
flyctl ssh sftp get /data/meta.db "$ORIG" -a "$APP"
ls -lh "$ORIG"

echo "==> [3/6] Pruning bridge rows + VACUUM locally ..."
cp "$ORIG" "$COMPACT"
sqlite3 "$COMPACT" "DELETE FROM event_log WHERE event_type IN ($BRIDGE_TYPES); VACUUM;"
echo "    before: $(du -h "$ORIG" | cut -f1)   after: $(du -h "$COMPACT" | cut -f1)"

echo "==> [4/6] Verifying compacted DB integrity + critical tables ..."
INTEG=$(sqlite3 "$COMPACT" "PRAGMA integrity_check;")
[ "$INTEG" = "ok" ] || { echo "FATAL: integrity_check failed: $INTEG"; echo "Original kept at $ORIG; start the machine with: flyctl machine start -a $APP"; exit 1; }
for t in universes users schema_version telemetry_events event_log release_notes; do
  sqlite3 "$COMPACT" "SELECT 1 FROM $t LIMIT 1;" >/dev/null 2>&1 \
    || { echo "WARN: table $t empty or missing (continuing — verify it should be empty)"; }
done
echo "    integrity_check: ok"
echo "    schema_version : $(sqlite3 "$COMPACT" 'SELECT MAX(version) FROM schema_version;')"
echo "    universes      : $(sqlite3 "$COMPACT" 'SELECT COUNT(*) FROM universes;')"
echo "    event_log rows : $(sqlite3 "$COMPACT" 'SELECT COUNT(*) FROM event_log;')"

read -r -p "==> Upload compacted DB and replace prod /data/meta.db? [y/N] " ans
[ "$ans" = "y" ] || { echo "Aborted. Machine still STOPPED — start it with: flyctl machine start -a $APP"; exit 1; }

echo "==> [5/6] Uploading compacted DB to /data/meta.db ..."
# Remove WAL/SHM so the new DB isn't reconciled against stale write-ahead pages.
flyctl ssh console -a "$APP" -C "sh -c 'rm -f /data/meta.db-wal /data/meta.db-shm'"
flyctl ssh sftp shell -a "$APP" <<EOF
put $COMPACT /data/meta.db
EOF

echo "==> [6/6] Starting $APP + smoke ..."
flyctl machine start -a "$APP"
sleep 8
curl -fsS https://co-artelonga.fly.dev/api/health && echo
echo "Done. Original prod DB preserved at: $ORIG"
echo "If anything looks wrong: flyctl machine stop, sftp put $ORIG /data/meta.db, machine start."
