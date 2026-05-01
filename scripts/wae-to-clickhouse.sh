#!/usr/bin/env bash
# CO-123: Daily WAE → ClickHouse export.
#
# Queries the Cloudflare Analytics Engine SQL API for yesterday's co_telemetry
# events and bulk-inserts them into co_analytics.wae_events via ClickHouse HTTP.
#
# WAE column mapping (co-wae-proxy data layout):
#   index1   → universe_id
#   blob1    → event_type
#   blob2    → user_kind
#   blob3    → flag_key
#   blob4    → variant
#   double1  → duration_ms
#   double2  → status
#
# Usage:
#   CF_ACCOUNT_ID=<id> CF_API_TOKEN=<token> CH_PASSWORD=<pass> bash scripts/wae-to-clickhouse.sh
#
# Env (all required unless noted):
#   CF_ACCOUNT_ID   Cloudflare account ID
#   CF_API_TOKEN    CF API token with Analytics Engine read permission
#   CH_HOST         ClickHouse hostname (default: co-clickhouse.internal)
#   CH_PORT         ClickHouse HTTP port (default: 8123)
#   CH_USER         ClickHouse user (default: co_admin)
#   CH_PASSWORD     ClickHouse password
#   EXPORT_DATE     YYYY-MM-DD to export (default: yesterday)

set -euo pipefail

: "${CF_ACCOUNT_ID:?CF_ACCOUNT_ID not set}"
: "${CF_API_TOKEN:?CF_API_TOKEN not set}"
: "${CH_HOST:=co-clickhouse.internal}"
: "${CH_PORT:=8123}"
: "${CH_USER:=co_admin}"
: "${CH_PASSWORD:?CH_PASSWORD not set}"

# Resolve target date — portable between GNU date and BSD date.
if [[ -n "${EXPORT_DATE:-}" ]]; then
    TARGET_DATE="$EXPORT_DATE"
elif date -u -d "yesterday" "+%Y-%m-%d" &>/dev/null; then
    TARGET_DATE=$(date -u -d "yesterday" "+%Y-%m-%d")
else
    TARGET_DATE=$(date -u -v-1d "+%Y-%m-%d")
fi

echo "[wae-export] date=${TARGET_DATE} ch=${CH_HOST}:${CH_PORT}"

WAE_SQL="SELECT
    timestamp,
    index1          AS universe_id,
    blob1           AS event_type,
    blob2           AS user_kind,
    blob3           AS flag_key,
    blob4           AS variant,
    double1         AS duration_ms,
    double2         AS status
FROM co_telemetry
WHERE timestamp >= toDateTime('${TARGET_DATE} 00:00:00')
  AND timestamp <  toDateTime('${TARGET_DATE} 00:00:00') + INTERVAL 1 DAY
LIMIT 500000"

echo "[wae-export] Fetching from Cloudflare Analytics Engine..."
RAW=$(curl -sf \
    "https://api.cloudflare.com/client/v4/accounts/${CF_ACCOUNT_ID}/analytics_engine/sql" \
    -H "Authorization: Bearer ${CF_API_TOKEN}" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "sql=${WAE_SQL}")

ROW_COUNT=$(python3 - <<'PYEOF'
import json, sys

raw = sys.stdin.read()
data = json.loads(raw)

if not data.get("success"):
    errors = data.get("errors", [])
    print(f"WAE API error: {errors}", file=sys.stderr)
    sys.exit(1)

rows = data.get("data", [])
if not rows:
    print(0)
    sys.exit(0)

# Emit TabSeparated rows for ClickHouse INSERT.
for row in rows:
    fields = [
        row.get("timestamp", ""),
        row.get("universe_id", ""),
        row.get("event_type", ""),
        row.get("user_kind", ""),
        row.get("flag_key", ""),
        row.get("variant", ""),
        str(row.get("duration_ms", 0) or 0),
        str(row.get("status", 0) or 0),
    ]
    print("\t".join(str(f) for f in fields), file=sys.stdout)

print(len(rows), file=sys.stderr)
PYEOF
<<< "$RAW")

echo "[wae-export] rows fetched: ${ROW_COUNT}"

if [[ "$ROW_COUNT" -eq 0 ]]; then
    echo "[wae-export] Nothing to insert."
    exit 0
fi

echo "[wae-export] Inserting into ClickHouse..."

python3 - <<'PYEOF' <<< "$RAW" | curl -sf \
    -u "${CH_USER}:${CH_PASSWORD}" \
    "http://${CH_HOST}:${CH_PORT}/?query=INSERT%20INTO%20co_analytics.wae_events%20FORMAT%20TabSeparated" \
    --data-binary @-
import json, sys

raw = sys.stdin.read()
data = json.loads(raw)
rows = data.get("data", [])

for row in rows:
    fields = [
        row.get("timestamp", ""),
        row.get("universe_id", ""),
        row.get("event_type", ""),
        row.get("user_kind", ""),
        row.get("flag_key", ""),
        row.get("variant", ""),
        str(row.get("duration_ms", 0) or 0),
        str(row.get("status", 0) or 0),
    ]
    print("\t".join(str(f) for f in fields))
PYEOF

echo "[wae-export] Done — ${TARGET_DATE}"
