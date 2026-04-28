#!/usr/bin/env bash
# CO-67 prod seed — create artelonga + rfq universes on prod and bulk-upload
# their content from local repos.
#
# Auth model:
#   - Universe CREATE / PUT requires JWT (require_auth — JWT-only middleware).
#     So the bootstrap branch logs in with the password, does the full seed,
#     and stores a long-lived API token for FUTURE re-uploads.
#   - Vault PUT/GET (the actual content upload path) accepts API tokens.
#     So normal runs use the token from the OS keychain — no password.
#
# Usage:
#   bash scripts/seed-prod-universes.sh --bootstrap        # one-time full seed; prompts for password
#   bash scripts/seed-prod-universes.sh                    # re-upload content via stored token (no password)
set -euo pipefail

PROD="https://co-artelonga.fly.dev"
EMAIL="yuri@artelonga.com.br"
TOKEN_NAME="prod"

# ---------- helpers ----------

upload_dir_with_auth() {
    # Args: universe, root_dir, auth_header_value (e.g. "Authorization: Bearer ...")
    local universe="$1" root="$2" auth="$3"
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
            -H "$auth" -X PUT "$PROD/api/v1/universes/$universe/vault/$encoded" \
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

verify_counts_with_auth() {
    local auth="$1"
    for slug in artelonga quilomboaraucaria rfq; do
        count=$(curl -s -H "$auth" "$PROD/api/v1/universes/$slug" | \
            python3 -c "import sys,json; print(json.load(sys.stdin).get('content_count','?'))" 2>/dev/null)
        echo "  $slug: count=$count"
    done
}

# ===========================================================================
# Bootstrap branch — login once, create universes, upload content, store token
# ===========================================================================
if [[ "${1:-}" == "--bootstrap" ]]; then
    PASSWORD="${2:-}"
    if [[ -z "$PASSWORD" ]]; then
        read -rs -p "Password: " PASSWORD
        echo
    fi
    COOKIES=$(mktemp)
    trap 'rm -f "$COOKIES"' EXIT

    echo "[bootstrap 1/5] login as $EMAIL ..."
    LOGIN=$(curl -sc "$COOKIES" -X POST "$PROD/api/v1/auth/password-login" \
        -H 'Content-Type: application/json' \
        --data "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")
    if ! echo "$LOGIN" | python3 -c "import sys,json; d=json.load(sys.stdin); assert 'user_id' in d" 2>/dev/null; then
        echo "  login failed: $LOGIN"
        exit 1
    fi
    echo "  ok"

    echo "[bootstrap 2/5] create universes (idempotent — 409 on existing is fine) ..."
    create_with_cookie() {
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
    create_with_cookie artelonga "ArteLonga" "Rede de marcas e empreendedores"
    create_with_cookie rfq       "RFQ"        "Quote engine for prediction market making"

    echo "[bootstrap 3/5] bulk upload content (throttled 100ms/file) ..."
    SESSION=$(awk '/\tsession\t/ {print $7}' "$COOKIES")
    COOKIE_AUTH="Cookie: session=$SESSION"
    upload_dir_with_auth artelonga /Users/artelonga/projects/ArteLonga "$COOKIE_AUTH"
    upload_dir_with_auth rfq       /Users/artelonga/projects/rfq-gateway "$COOKIE_AUTH"

    echo "[bootstrap 4/5] generate long-lived API token for re-uploads ..."
    TBODY=$(curl -sb "$COOKIES" -X POST "$PROD/api/v1/auth/token" \
        -H 'Content-Type: application/json' \
        --data "{\"name\":\"yuri-cli\"}")
    TOKEN=$(echo "$TBODY" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null || true)
    if [[ -z "$TOKEN" ]]; then
        echo "  token generation failed: $TBODY"
        exit 1
    fi
    echo "  generated (${#TOKEN} bytes)"

    echo "[bootstrap 5/5] store in OS keychain via co-token ..."
    if ! command -v co-token >/dev/null; then
        echo "  co-token not on PATH. Install: cargo install --path dev/co-token"
        exit 1
    fi
    printf '%s' "$TOKEN" | co-token set "$TOKEN_NAME"

    echo
    echo "Verify counts:"
    verify_counts_with_auth "$COOKIE_AUTH"
    echo
    echo "Done. Future runs (re-uploads only — universes already exist):"
    echo "  bash scripts/seed-prod-universes.sh"
    exit 0
fi

# ===========================================================================
# Normal run — re-upload content via stored API token (vault routes only)
# ===========================================================================
if ! command -v co-token >/dev/null; then
    echo "co-token not on PATH. Install: cargo install --path dev/co-token"
    echo "First-time setup: bash scripts/seed-prod-universes.sh --bootstrap"
    exit 1
fi
TOKEN=$(co-token get "$TOKEN_NAME" 2>/dev/null || true)
if [[ -z "$TOKEN" ]]; then
    echo "No '$TOKEN_NAME' token in keychain."
    echo "Run once: bash scripts/seed-prod-universes.sh --bootstrap"
    exit 1
fi
TOKEN_AUTH="Authorization: Bearer $TOKEN"

echo "[1/2] verify token (vault listing on a known universe) ..."
CODE=$(curl -s -o /dev/null -w '%{http_code}' -H "$TOKEN_AUTH" "$PROD/api/v1/universes/artelonga/vault/")
if [[ "$CODE" != "200" ]]; then
    echo "  token rejected by vault listing (HTTP $CODE)"
    echo "  re-bootstrap: bash scripts/seed-prod-universes.sh --bootstrap"
    exit 1
fi
echo "  ok"

echo "[2/2] re-upload content via vault (universes must already exist) ..."
upload_dir_with_auth artelonga /Users/artelonga/projects/ArteLonga "$TOKEN_AUTH"
upload_dir_with_auth rfq       /Users/artelonga/projects/rfq-gateway "$TOKEN_AUTH"

echo
echo "Verify counts:"
verify_counts_with_auth "$TOKEN_AUTH"

echo
echo "Done. To replicate to UAT, trigger a reset (mirror auto-runs):"
echo "  flyctl ssh console -a co-artelonga-uat -C 'touch /data/uat-reset.flag'"
echo "  flyctl machine restart 287e357f66e5d8 -a co-artelonga-uat"
