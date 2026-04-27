#!/usr/bin/env bash
# CO-67 prod seed — create artelonga + rfq universes on prod and bulk-upload
# their content from local repos. Idempotent: re-runs are safe (vault PUTs
# are upserts; universe creates 409 on dupe).
#
# Usage: bash scripts/seed-prod-universes.sh YOUR_PASSWORD
#
# Deliberately skips quilomboaraucaria — prod already has 161 entries from a
# prior migration; we don't want the local 88-file copy clobbering them.
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

echo "[1/4] login as $EMAIL ..."
LOGIN_BODY=$(curl -sc "$COOKIES" -X POST "$PROD/api/v1/auth/password-login" \
    -H 'Content-Type: application/json' \
    --data "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")
if ! echo "$LOGIN_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); assert 'user_id' in d" 2>/dev/null; then
    echo "  login failed: $LOGIN_BODY"
    exit 1
fi
echo "  ok"

echo "[2/4] create universes (idempotent — 409 on existing is fine) ..."
create() {
    local key="$1" name="$2" desc="$3"
    local code
    code=$(curl -s -o /tmp/seed-resp.json -w '%{http_code}' \
        -b "$COOKIES" -X POST "$PROD/api/v1/universes" \
        -H 'Content-Type: application/json' \
        --data "{\"key\":\"$key\",\"name\":\"$name\",\"description\":\"$desc\"}")
    case "$code" in
        201) echo "  ✓ $key created" ;;
        409) echo "  • $key already exists (skipping create)" ;;
        *)   echo "  ✗ $key HTTP $code: $(cat /tmp/seed-resp.json)"; exit 1 ;;
    esac
}
create artelonga "ArteLonga" "Rede de marcas e empreendedores"
create rfq       "RFQ"        "Quote engine for prediction market making"

echo "[3/4] bulk upload content via Vault API (throttled 100ms/file) ..."
BEARER=""  # extract API token? not needed: use cookie auth via vault routes
upload_dir() {
    local universe="$1" root="$2"
    if [[ ! -d "$root" ]]; then
        echo "  • $universe: source $root not found, skipping"
        return
    fi
    local count=0 ok=0 fail=0
    while IFS= read -r -d '' file; do
        local rel="${file#$root/}"
        # url-encode each path segment but keep slashes
        local encoded
        encoded=$(python3 -c "import sys, urllib.parse; print('/'.join(urllib.parse.quote(seg, safe='') for seg in sys.argv[1].split('/')))" "$rel")
        local code
        code=$(curl -s -o /dev/null -w '%{http_code}' \
            -b "$COOKIES" -X PUT "$PROD/api/v1/universes/$universe/vault/$encoded" \
            -H 'Content-Type: text/markdown' \
            --data-binary "@$file")
        count=$((count + 1))
        if [[ "$code" == "200" || "$code" == "201" ]]; then
            ok=$((ok + 1))
        else
            fail=$((fail + 1))
            [[ "$fail" -le 3 ]] && echo "    HTTP $code: $rel"
        fi
        sleep 0.1
    done < <(find "$root" -type f -name '*.md' \
        -not -path '*/node_modules/*' -not -path '*/.git/*' \
        -not -path '*/target/*' -not -path '*/build/*' \
        -not -path '*/dist/*' -not -path '*/.next/*' \
        -not -path '*/.svelte-kit/*' -print0)
    echo "  $universe: $ok ok, $fail fail (of $count)"
}
upload_dir artelonga /Users/artelonga/projects/ArteLonga
upload_dir rfq       /Users/artelonga/projects/rfq-gateway

echo "[4/4] verify content counts ..."
for slug in artelonga quilomboaraucaria rfq; do
    count=$(curl -s -b "$COOKIES" "$PROD/api/v1/universes/$slug" | \
        python3 -c "import sys,json; print(json.load(sys.stdin).get('content_count','?'))" 2>/dev/null)
    echo "  $slug: count=$count"
done

echo
echo "Done. To replicate to UAT, trigger a reset (mirror auto-runs):"
echo "  flyctl ssh console -a co-artelonga-uat -C 'touch /data/uat-reset.flag'"
echo "  flyctl machine restart 287e357f66e5d8 -a co-artelonga-uat"
