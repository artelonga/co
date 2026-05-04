#!/usr/bin/env bash
# consolidate-topologia.sh — merge topologia sub-directories into one CO universe.
#
# Option C: all content under alphabetical top-level folders in one 'topologia' universe:
#   topologia/
#     concepts/       (from ~/projects/topologia/concepts/)
#     guarani-mbya/   (from ~/projects/topologia/guarani-mbya/)
#     languages/      (from ~/projects/topologia/languages/)
#     portuguese/     (from ~/projects/topologia/portuguese/)
#     yoruba/         (from ~/projects/topologia/yoruba/)
#     docs/           (from ~/projects/topologia/docs/)
#
# Usage:
#   export TOKEN="<your-jwt>"
#   bash scripts/consolidate-topologia.sh

set -euo pipefail

API="https://co.artelonga.com.br/api/v1/universes"
UNIVERSE="topologia"
TOPOLOGIA_ROOT="${HOME}/projects/topologia"
TOKEN="${TOKEN:?Set TOKEN env var}"

# Folders to sync (alphabetical, as requested)
FOLDERS=(concepts guarani-mbya languages portuguese yoruba docs)

echo "=== Consolidating topologia → $UNIVERSE ==="
echo ""

ok=0; fail=0

for folder in "${FOLDERS[@]}"; do
    local_dir="${TOPOLOGIA_ROOT}/${folder}"
    if [[ ! -d "$local_dir" ]]; then
        echo "  Skipping $folder (not found at $local_dir)"
        continue
    fi

    echo "--- Syncing ${folder}/ ---"
    find "$local_dir" -name "*.md" | sort | while read -r file; do
        rel="${file#$local_dir/}"
        vault_path="${folder}/${rel}"

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
done

echo ""
echo "=== Done. OK=$ok  FAIL=$fail ==="
echo ""
echo "Reindexing ${UNIVERSE}..."
curl -s -X POST "${API}/${UNIVERSE}/reindex" \
    -H "Authorization: Bearer $TOKEN" \
    | python3 -c "import sys,json; r=json.load(sys.stdin); print(f'  Indexed: {r[\"indexed\"]}, Errors: {len(r[\"errors\"])}')"

echo ""
echo "Apply template (adds CLAUDE.md, docs/api.md, type check)..."
curl -s -X POST "${API}/${UNIVERSE}/apply-template" \
    -H "Authorization: Bearer $TOKEN" \
    | python3 -c "import sys,json; r=json.load(sys.stdin); print(f'  Created: {r[\"created\"]}, Type errors: {len(r[\"type_errors\"])}')"
