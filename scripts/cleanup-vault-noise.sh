#!/usr/bin/env bash
# Delete vault entries whose paths match noise patterns (.claude/, .obsidian/,
# etc.) — left over from earlier seed runs that didn't exclude them.
#
# Usage:
#   bash scripts/cleanup-vault-noise.sh                    # dry-run on prod
#   bash scripts/cleanup-vault-noise.sh --execute          # actually delete on prod
#   bash scripts/cleanup-vault-noise.sh --to uat --execute # UAT
set -euo pipefail

DEPLOYMENT="prod"
EXECUTE=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --to) DEPLOYMENT="$2"; shift 2 ;;
        --execute) EXECUTE=1; shift ;;
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
    echo "no '$TOKEN_NAME' token in keychain"
    exit 1
fi

# Patterns to delete (substring match in entry path)
NOISE_PATTERNS='\.claude/|\.obsidian/|\.cache/|seed-co/|node_modules/|/\.venv/|/__pycache__/|/target/|/dist/|/build/|/\.next/|/\.svelte-kit/'

echo "Mode: $([[ $EXECUTE == 1 ]] && echo EXECUTE || echo DRY-RUN)"
echo "Target: $URL"
echo "Patterns: $NOISE_PATTERNS"
echo

for slug in artelonga rfq qa-dev quilomboaraucaria; do
    echo "[$slug]"
    paths=$(curl -s -H "Authorization: Bearer $TOKEN" "$URL/api/v1/universes/$slug/vault/" | \
        python3 -c "import sys,json; d=json.load(sys.stdin); [print(e['path']) for e in (d if isinstance(d,list) else [])]" 2>/dev/null || true)
    [[ -z "$paths" ]] && { echo "  (no vault listing — skipping)"; continue; }

    matched=$(echo "$paths" | grep -E "$NOISE_PATTERNS" || true)
    if [[ -z "$matched" ]]; then
        echo "  no noise to delete"
        continue
    fi
    n=$(echo "$matched" | wc -l | tr -d ' ')
    echo "  $n path(s) match noise patterns:"
    echo "$matched" | head -5 | sed 's/^/    /'
    [[ $n -gt 5 ]] && echo "    ... and $((n-5)) more"

    if [[ "$EXECUTE" != "1" ]]; then
        echo "  (dry-run — pass --execute to actually delete)"
        continue
    fi

    deleted=0
    while IFS= read -r path; do
        [[ -z "$path" ]] && continue
        encoded=$(python3 -c "import sys,urllib.parse; print('/'.join(urllib.parse.quote(s,safe='') for s in sys.argv[1].split('/')))" "$path")
        code=$(curl -s -o /dev/null -w '%{http_code}' \
            -H "Authorization: Bearer $TOKEN" -X DELETE "$URL/api/v1/universes/$slug/vault/$encoded")
        case "$code" in
            200|204) deleted=$((deleted+1));;
            *) echo "    HTTP $code: $path" ;;
        esac
        sleep 1.1
    done <<< "$matched"
    echo "  deleted $deleted/$n"
done
