#!/usr/bin/env bash
# CO-67 prod seed — create artelonga + rfq universes on prod and bulk-upload
# their content from local repos.
#
# Auth: uses an API token stored encrypted in the OS keychain via co-token.
# First run: pass --bootstrap PASSWORD to log in once and stash a long-lived
# token (named "prod") in the keychain. Subsequent runs need no args.
#
# Usage:
#   bash scripts/seed-prod-universes.sh                  # uses keychain token "prod"
#   bash scripts/seed-prod-universes.sh --bootstrap PWD  # one-time setup
set -euo pipefail

PROD="https://co-artelonga.fly.dev"
EMAIL="yuri@artelonga.com.br"
TOKEN_NAME="prod"

# --- Bootstrap branch: log in once, generate + store a long-lived API token ---
if [[ "${1:-}" == "--bootstrap" ]]; then
    PASSWORD="${2:-}"
    if [[ -z "$PASSWORD" ]]; then
        read -rs -p "Password: " PASSWORD
        echo
    fi
    COOKIES=$(mktemp)
    trap 'rm -f "$COOKIES"' EXIT

    echo "[bootstrap 1/3] login as $EMAIL ..."
    LOGIN=$(curl -sc "$COOKIES" -X POST "$PROD/api/v1/auth/password-login" \
        -H 'Content-Type: application/json' \
        --data "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")
    if ! echo "$LOGIN" | python3 -c "import sys,json; d=json.load(sys.stdin); assert 'user_id' in d" 2>/dev/null; then
        echo "  login failed: $LOGIN"
        exit 1
    fi
    echo "  ok"

    echo "[bootstrap 2/3] generate API token ..."
    TBODY=$(curl -sb "$COOKIES" -X POST "$PROD/api/v1/auth/token" \
        -H 'Content-Type: application/json' \
        --data "{\"name\":\"yuri-cli\"}")
    TOKEN=$(echo "$TBODY" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null || true)
    if [[ -z "$TOKEN" ]]; then
        echo "  token generation failed: $TBODY"
        exit 1
    fi
    echo "  generated (${#TOKEN} bytes)"

    echo "[bootstrap 3/3] store in OS keychain via co-token ..."
    if ! command -v co-token >/dev/null; then
        echo "  co-token not on PATH. Install: cargo install --path dev/co-token"
        exit 1
    fi
    printf '%s' "$TOKEN" | co-token set "$TOKEN_NAME"
    echo
    echo "Done bootstrapping. Future runs: bash scripts/seed-prod-universes.sh"
    exit 0
fi

# --- Normal run: use the stored token ---
if ! command -v co-token >/dev/null; then
    echo "co-token not on PATH. Install: cargo install --path dev/co-token"
    echo "Or run with --bootstrap PASSWORD to set it up."
    exit 1
fi
TOKEN=$(co-token get "$TOKEN_NAME" 2>/dev/null || true)
if [[ -z "$TOKEN" ]]; then
    echo "No '$TOKEN_NAME' token in keychain. Run once with:"
    echo "  bash scripts/seed-prod-universes.sh --bootstrap"
    exit 1
fi
AUTH_HEADER="Authorization: Bearer $TOKEN"

echo "[1/3] verify token works ..."
ME=$(curl -s -w '\nHTTP %{http_code}' -H "$AUTH_HEADER" "$PROD/api/v1/auth/me")
if ! echo "$ME" | head -1 | python3 -c "import sys,json; d=json.load(sys.stdin); assert 'email' in d" 2>/dev/null; then
    echo "  token rejected: $ME"
    echo "  re-bootstrap: bash scripts/seed-prod-universes.sh --bootstrap"
    exit 1
fi
echo "  ok"

echo "[2/3] create universes (idempotent — 409 on existing is fine) ..."
create() {
    local key="$1" name="$2" desc="$3"
    local code
    code=$(curl -s -o /tmp/seed-resp.json -w '%{http_code}' \
        -H "$AUTH_HEADER" -X POST "$PROD/api/v1/universes" \
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

echo "[3/3] bulk upload content (throttled 100ms/file) ..."
upload_dir() {
    local universe="$1" root="$2"
    if [[ ! -d "$root" ]]; then
        echo "  • $universe: source $root not found, skipping"
        return
    fi
    local count=0 ok=0 fail=0
    while IFS= read -r -d '' file; do
        local rel="${file#$root/}"
        local encoded
        encoded=$(python3 -c "import sys, urllib.parse; print('/'.join(urllib.parse.quote(seg, safe='') for seg in sys.argv[1].split('/')))" "$rel")
        local code
        code=$(curl -s -o /dev/null -w '%{http_code}' \
            -H "$AUTH_HEADER" -X PUT "$PROD/api/v1/universes/$universe/vault/$encoded" \
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

echo
echo "Verify counts:"
for slug in artelonga quilomboaraucaria rfq; do
    count=$(curl -s -H "$AUTH_HEADER" "$PROD/api/v1/universes/$slug" | \
        python3 -c "import sys,json; print(json.load(sys.stdin).get('content_count','?'))" 2>/dev/null)
    echo "  $slug: count=$count"
done

echo
echo "Done. To replicate to UAT, trigger a reset (mirror auto-runs):"
echo "  flyctl ssh console -a co-artelonga-uat -C 'touch /data/uat-reset.flag'"
echo "  flyctl machine restart 287e357f66e5d8 -a co-artelonga-uat"
