#!/usr/bin/env bash
# CO-123: Smoke test for the icebergS3() table function against Cloudflare R2.
#
# Verifies Phase 3 readiness: ClickHouse can read an Iceberg table stored on R2
# via the icebergS3() table function (stable in ClickHouse 24.x).
#
# Prerequisites:
#   1. Create an empty Iceberg table on R2 (run co-export.py once with R2 endpoint):
#        S3_ENDPOINT=https://<ACCOUNT_ID>.r2.cloudflarestorage.com \
#        S3_BUCKET=co-lakehouse \
#        AWS_ACCESS_KEY_ID=<R2_ACCESS_KEY> \
#        AWS_SECRET_ACCESS_KEY=<R2_SECRET_KEY> \
#        python3 scripts/co-export.py
#
#   2. Open a proxy to co-clickhouse:
#        flyctl proxy 8123:8123 -a co-clickhouse
#
#   3. Export required env vars, then run this script:
#        CH_PASSWORD=<pass> R2_ACCOUNT_ID=<id> R2_ACCESS_KEY=<key> \
#        R2_SECRET_KEY=<secret> bash infra/clickhouse/iceberg-smoke-test.sh

set -euo pipefail

: "${CH_HOST:=127.0.0.1}"
: "${CH_PORT:=8123}"
: "${CH_USER:=co_admin}"
: "${CH_PASSWORD:?CH_PASSWORD not set}"
: "${R2_ACCOUNT_ID:?R2_ACCOUNT_ID not set}"
: "${R2_ACCESS_KEY:?R2_ACCESS_KEY not set}"
: "${R2_SECRET_KEY:?R2_SECRET_KEY not set}"
: "${R2_BUCKET:=co-lakehouse}"

R2_ENDPOINT="https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com"
TABLE_PATH="${R2_ENDPOINT}/${R2_BUCKET}/co/universes/"

echo "[iceberg-smoke] ClickHouse: ${CH_HOST}:${CH_PORT}"
echo "[iceberg-smoke] Iceberg table: ${TABLE_PATH}"

QUERY="SELECT count() AS row_count FROM icebergS3('${TABLE_PATH}', '${R2_ACCESS_KEY}', '${R2_SECRET_KEY}')"

ENCODED=$(python3 -c "import urllib.parse, sys; print(urllib.parse.quote(sys.argv[1]))" "$QUERY")

RESULT=$(curl -sf -u "${CH_USER}:${CH_PASSWORD}" \
    "http://${CH_HOST}:${CH_PORT}/?query=${ENCODED}")

echo "[iceberg-smoke] row_count = ${RESULT}"

if [[ "$RESULT" =~ ^[0-9]+$ ]]; then
    echo "[iceberg-smoke] PASS — icebergS3() function is operational (Phase 3 ready)"
    exit 0
else
    echo "[iceberg-smoke] FAIL — unexpected response: ${RESULT}"
    exit 1
fi
