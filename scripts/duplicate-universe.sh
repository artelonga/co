#!/usr/bin/env bash
# CO-95 Phase 1 — duplicate a universe via the API.
#
# Usage:
#   bash scripts/duplicate-universe.sh <source> <target> [--name "..."] [--desc "..."]
#   bash scripts/duplicate-universe.sh quilomboaraucaria quilombo-blog
#   bash scripts/duplicate-universe.sh quilomboaraucaria quilombo-blog \
#       --name "Quilombo Blog" --desc "Perf-test copy of quilomboaraucaria"
#
# Auth: keychain token (via co-token).
set -euo pipefail

SOURCE="${1:-}"
TARGET="${2:-}"
if [[ -z "$SOURCE" || -z "$TARGET" ]]; then
    echo "usage: $0 <source-key> <target-key> [--name '...'] [--desc '...']"
    exit 1
fi
shift 2

NAME="$TARGET"
DESC=""
DEPLOYMENT="prod"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --name) NAME="$2"; shift 2 ;;
        --desc|--description) DESC="$2"; shift 2 ;;
        --to) DEPLOYMENT="$2"; shift 2 ;;
        *) echo "unknown arg: $1"; exit 1 ;;
    esac
done

case "$DEPLOYMENT" in
    prod) URL="https://co-artelonga.fly.dev"; TOKEN_NAME="prod" ;;
    uat)  URL="https://co-artelonga-uat.fly.dev"; TOKEN_NAME="uat" ;;
    *)    echo "unknown deployment: $DEPLOYMENT"; exit 1 ;;
esac

TOKEN=$(co-token get "$TOKEN_NAME" 2>/dev/null || true)
if [[ -z "$TOKEN" ]]; then
    echo "no '$TOKEN_NAME' token in keychain. Run: bash scripts/seed-prod-universes.sh --bootstrap"
    exit 1
fi

echo "Duplicating $URL/$SOURCE → $URL/$TARGET"
echo "  name: $NAME"
echo "  desc: $DESC"

PAYLOAD=$(python3 -c "import json,sys; print(json.dumps({'key':sys.argv[1],'name':sys.argv[2],'description':sys.argv[3]}))" "$TARGET" "$NAME" "$DESC")

resp=$(curl -s -w '\n%{http_code}' -H "Authorization: Bearer $TOKEN" \
    -X POST "$URL/api/v1/universes/$SOURCE/duplicate" \
    -H 'Content-Type: application/json' \
    --data "$PAYLOAD")

body=$(echo "$resp" | head -n -1)
code=$(echo "$resp" | tail -n 1)

case "$code" in
    201) echo "  ✓ created — $URL/co/$TARGET" ;;
    409) echo "  ✗ HTTP 409 (target key already exists): $body" ;;
    403) echo "  ✗ HTTP 403 (not authorized for source): $body" ;;
    404) echo "  ✗ HTTP 404 (source not found): $body" ;;
    *)   echo "  ✗ HTTP $code: $body" ;;
esac
