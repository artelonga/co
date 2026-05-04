#!/usr/bin/env bash
# upload-assets.sh — upload binary files (PDFs, images) to a CO universe as assets,
# then update any reference card (.md) that has `file: <filename>` with the blob_sha256.
#
# Usage:
#   export TOKEN="<api-token from co.artelonga.com.br/co/settings/sync>"
#   bash scripts/upload-assets.sh mbya ~/projects/mbya/refs

set -euo pipefail

UNIVERSE="${1:?Usage: $0 <universe-slug> <local-dir> [extensions]}"
LOCAL_DIR="${2:?}"
EXTENSIONS="${3:-pdf,png,jpg,jpeg,gif,webp,mp4,webm,mp3,ogg}"
API="https://co.artelonga.com.br/api/v1/universes/${UNIVERSE}"
TOKEN="${TOKEN:?Set TOKEN env var from co.artelonga.com.br/co/settings/sync}"

echo "=== Uploading binary assets: ${UNIVERSE} ==="
echo "    Source: ${LOCAL_DIR}"
echo ""

IFS=',' read -ra EXTS <<< "$EXTENSIONS"

ok=0; fail=0; skip=0

for ext in "${EXTS[@]}"; do
    while IFS= read -r -d '' file; do
        filename=$(basename "$file")

        # Upload the binary file as an asset
        resp=$(curl -s -X POST "${API}/assets?filename=${filename}" \
            -H "Authorization: Bearer ${TOKEN}" \
            -H "Content-Type: application/octet-stream" \
            --data-binary @"$file")

        sha256=$(echo "$resp" | python3 -c "import sys,json; r=json.load(sys.stdin); print(r.get('sha256',''))" 2>/dev/null || true)

        if [[ -z "$sha256" ]]; then
            echo "  ✗ ${filename} — $(echo "$resp" | python3 -c "import sys,json; r=json.load(sys.stdin); print(r.get('message_en','upload failed'))" 2>/dev/null || echo 'upload failed')"
            ((fail++)) || true
            continue
        fi

        echo "  ✓ ${filename} → sha256:${sha256:0:12}…"
        ((ok++)) || true

        # Find any reference card in the same directory that references this file
        card="${file%.$ext}.md"
        # Also check if the card exists with underscores/spaces replaced
        rel_dir=$(dirname "${file#$LOCAL_DIR/}")

        if [[ -f "$card" ]]; then
            # Check if the card has `file: <filename>` and is missing blob_sha256
            if grep -q "^file: ${filename}$" "$card" 2>/dev/null; then
                if ! grep -q "^blob_sha256:" "$card" 2>/dev/null; then
                    # Inject blob_sha256 after the file: line
                    sed -i.bak "s|^file: ${filename}$|file: ${filename}\nblob_sha256: ${sha256}|" "$card"
                    rm -f "${card}.bak"
                    echo "    → Updated ${card##*/} with blob_sha256"

                    # Push updated card to server
                    vault_path="${rel_dir:+$rel_dir/}$(basename "$card")"
                    http_code=$(curl -s -o /dev/null -w "%{http_code}" \
                        -X PUT "${API}/vault/${vault_path}" \
                        -H "Authorization: Bearer ${TOKEN}" \
                        -H "Content-Type: text/markdown" \
                        --data-binary @"$card")
                    [[ "$http_code" =~ ^2 ]] && echo "    → Synced ${vault_path}" || echo "    ! Failed to sync ${vault_path}"
                else
                    ((skip++)) || true
                fi
            fi
        fi
    done < <(find "$LOCAL_DIR" -name "*.${ext}" -print0 2>/dev/null | sort -z)
done

echo ""
echo "=== Done: ${ok} uploaded, ${fail} failed, ${skip} already had sha256 ==="
echo ""
echo "Reindexing ${UNIVERSE}…"
curl -s -X POST "${API}/reindex" \
    -H "Authorization: Bearer ${TOKEN}" \
    | python3 -c "import sys,json; r=json.load(sys.stdin); print(f'  {r[\"indexed\"]} entries indexed')" 2>/dev/null || true
