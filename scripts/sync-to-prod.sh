#!/usr/bin/env bash
# sync-to-prod.sh — push local universe content to the CO production server.
#
# Usage:
#   export TOKEN="$(curl -s -X POST https://co.artelonga.com.br/api/v1/auth/password-login \
#     -H 'Content-Type: application/json' \
#     -d '{"email":"yuri@artelonga.com.br","password":"<pw>"}' | python3 -c \
#     "import sys,json; r=json.load(sys.stdin); print(r.get('session',''))")"
#
#   # Sync CO tasks (161 files):
#   bash scripts/sync-to-prod.sh co work/co
#
#   # Sync mbya refs:
#   bash scripts/sync-to-prod.sh mbya /path/to/mbya/refs  refs
#
#   # Reindex after sync:
#   curl -X POST https://co.artelonga.com.br/api/v1/universes/<slug>/reindex \
#     -H "Authorization: Bearer $TOKEN"

set -euo pipefail

API="https://co.artelonga.com.br/api/v1/universes"
UNIVERSE="${1:?Usage: $0 <universe-slug> <local-dir> [<remote-prefix>]}"
LOCAL_DIR="${2:?}"
REMOTE_PREFIX="${3:-}"   # optional: prepend this to all vault paths (e.g. "work/co")
TOKEN="${TOKEN:?Set TOKEN env var to your session JWT}"

ok=0; fail=0

find "$LOCAL_DIR" -name "*.md" | sort | while read -r file; do
    # Compute vault path relative to LOCAL_DIR
    rel="${file#$LOCAL_DIR/}"
    vault_path="${REMOTE_PREFIX:+$REMOTE_PREFIX/}${rel}"

    http_code=$(curl -s -o /dev/null -w "%{http_code}" \
        -X PUT "${API}/${UNIVERSE}/vault/${vault_path}" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: text/markdown" \
        --data-binary @"$file")

    if [[ "$http_code" =~ ^2 ]]; then
        echo "  ✓ $vault_path"
        ((ok++)) || true
    else
        echo "  ✗ $vault_path — HTTP $http_code"
        ((fail++)) || true
    fi
done

echo ""
echo "Done. OK=$ok  FAIL=$fail"
echo "Run reindex: curl -X POST ${API}/${UNIVERSE}/reindex -H 'Authorization: Bearer \$TOKEN'"
