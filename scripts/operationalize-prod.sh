#!/usr/bin/env bash
# Logs into prod, generates a UAT_PROD_TOKEN, sets the three Fly secrets on UAT,
# triggers a UAT reset. End-to-end CO-82 mirror activation.
#
# Usage: bash scripts/operationalize-prod.sh YOUR_PASSWORD
set -euo pipefail

PASSWORD="${1:-}"
if [[ -z "$PASSWORD" ]]; then
    echo "usage: $0 YOUR_PASSWORD"
    exit 1
fi

PROD="https://co-artelonga.fly.dev"
EMAIL="yuri@artelonga.com.br"
COOKIES=$(mktemp)
trap 'rm -f "$COOKIES"' EXIT

echo "[1/5] login as $EMAIL ..."
LOGIN_BODY=$(curl -sc "$COOKIES" -X POST "$PROD/api/v1/auth/password-login" \
    -H 'Content-Type: application/json' \
    --data "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")

if ! echo "$LOGIN_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); assert 'user_id' in d" 2>/dev/null; then
    echo "  login failed: $LOGIN_BODY"
    exit 1
fi
echo "  ok"

echo "[2/5] generate UAT_PROD_TOKEN ..."
TOKEN_BODY=$(curl -sb "$COOKIES" -X POST "$PROD/api/v1/auth/token" \
    -H 'Content-Type: application/json' \
    --data '{"name":"uat-mirror"}')
TOKEN=$(echo "$TOKEN_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null || true)
if [[ -z "$TOKEN" ]]; then
    echo "  token generation failed: $TOKEN_BODY"
    exit 1
fi
echo "  TOKEN=${TOKEN:0:8}…"

echo "[3/5] set Fly secrets on co-artelonga-uat ..."
flyctl secrets set \
    UAT_MIRROR_PROD=true \
    UAT_PROD_URL="$PROD" \
    UAT_PROD_TOKEN="$TOKEN" \
    -a co-artelonga-uat
echo "  ok"

echo "[4/5] trigger UAT reset flag ..."
flyctl ssh console -a co-artelonga-uat -C "touch /data/uat-reset.flag"
echo "  ok"

echo "[5/5] restart UAT machine ..."
flyctl machine restart -a co-artelonga-uat
echo "  ok"

echo
echo "Done. Watch the mirror with:"
echo "  flyctl logs -a co-artelonga-uat | grep -i 'UAT mirror'"
