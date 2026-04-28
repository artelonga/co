#!/usr/bin/env bash
# CO-92 — upload the 4 sample timeline events from work/timeline-samples/
# to a target universe via the Vault API. Auth via co-token (keychain).
#
# Usage:
#   bash scripts/seed-timeline-events.sh                           # → co-dev on prod
#   bash scripts/seed-timeline-events.sh --to uat --universe co-dev
#   bash scripts/seed-timeline-events.sh --universe co-experience  # public-subscribable on prod
set -euo pipefail

DEPLOYMENT="prod"
UNIVERSE="co-dev"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --to) DEPLOYMENT="$2"; shift 2 ;;
        --universe) UNIVERSE="$2"; shift 2 ;;
        *) echo "unknown arg: $1"; exit 1 ;;
    esac
done

case "$DEPLOYMENT" in
    prod) URL="https://co-artelonga.fly.dev"; TOKEN_NAME="prod" ;;
    uat)  URL="https://co-artelonga-uat.fly.dev"; TOKEN_NAME="uat" ;;
    *)    echo "unknown deployment: $DEPLOYMENT (try: prod | uat)"; exit 1 ;;
esac

if ! command -v co-token >/dev/null; then
    echo "co-token not on PATH. cargo install --path dev/co-token"
    exit 1
fi

AUTH=""
TOKEN=$(co-token get "$TOKEN_NAME" 2>/dev/null || true)
if [[ -n "$TOKEN" ]]; then
    AUTH="Authorization: Bearer $TOKEN"
elif [[ "$DEPLOYMENT" == "uat" ]]; then
    # UAT has hardcoded uat-login; fetch a session cookie instead
    COOKIES=$(mktemp)
    trap 'rm -f "$COOKIES"' EXIT
    curl -sc "$COOKIES" -X POST "$URL/api/v1/auth/uat-login" \
        -H 'Content-Type: application/json' \
        --data '{"email":"yuri@uat.local","password":"uat"}' >/dev/null
    SESSION=$(awk '/\tsession\t/ {print $7}' "$COOKIES")
    AUTH="Cookie: session=$SESSION"
else
    echo "no '$TOKEN_NAME' token. Run: bash scripts/seed-prod-universes.sh --bootstrap"
    exit 1
fi

ROOT="/Users/artelonga/projects/co/work/timeline-samples"
echo "Uploading 4 sample events to $URL/$UNIVERSE/..."

for f in earth-formation homo-sapiens-emergence now 300k-years-from-now; do
    file="$ROOT/$f.md"
    [[ -f "$file" ]] || { echo "  missing: $file"; continue; }
    code=$(curl -s -o /tmp/seed-resp -w '%{http_code}' \
        -H "$AUTH" -X PUT "$URL/api/v1/universes/$UNIVERSE/vault/events/$f.md" \
        -H 'Content-Type: text/markdown' \
        --data-binary "@$file")
    case "$code" in
        200|201) echo "  ✓ events/$f.md" ;;
        *)       echo "  ✗ events/$f.md HTTP $code: $(cat /tmp/seed-resp | head -c 200)" ;;
    esac
    sleep 0.1
done

echo
echo "View the timeline:"
echo "  $URL/shared/timeline.html?u=$UNIVERSE"
