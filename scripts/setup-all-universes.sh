#!/usr/bin/env bash
# setup-all-universes.sh — one-time setup: configure co-sync for all universes
# and push initial content to the CO server.
#
# Prerequisites:
#   1. co-sync installed:  cargo install --path co-agent --bin co-sync
#   2. Login done:         co-sync init --email yuri@artelonga.com.br
#
# Usage:
#   bash scripts/setup-all-universes.sh

set -euo pipefail

CONFIG="${HOME}/.co/sync.toml"

if [[ ! -f "$CONFIG" ]]; then
    echo "❌ ~/.co/sync.toml not found. Run: co-sync init --email yuri@artelonga.com.br"
    exit 1
fi

TOKEN=$(python3 -c "
import sys, re
txt = open('$CONFIG').read()
m = re.search(r'token\s*=\s*\"([^\"]+)\"', txt)
print(m.group(1) if m else '')
")

if [[ -z "$TOKEN" ]]; then
    echo "❌ No token in config. Run: co-sync init --email yuri@artelonga.com.br"
    exit 1
fi

API="https://co.artelonga.com.br/api/v1/universes"

echo "=== CO Universe Setup ==="
echo ""

# --- Register universe mappings in co-sync ---
echo "▶ Configuring co-sync mappings..."

for slug_and_path in \
    "co:/Users/artelonga/projects/co/work/co" \
    "artelonga:/Users/artelonga/projects/ArteLonga" \
    "quilomboaraucaria:/Users/artelonga/projects/quilomboaraucaria" \
    "topologia:/Users/artelonga/projects/topologia" \
    "rfq:/Users/artelonga/projects/rfq-gateway"
do
    slug="${slug_and_path%%:*}"
    path="${slug_and_path##*:}"
    if [[ -d "$path" ]]; then
        co-sync add --slug "$slug" --local "$path" 2>/dev/null \
            && echo "  ✓ Added $slug → $path" \
            || echo "  · $slug already configured"
    else
        echo "  ⚠ $path not found — skipping $slug"
    fi
done

echo ""
echo "▶ Syncing content to server..."
echo ""

# --- Push initial content for each universe ---
push_universe() {
    local slug="$1"
    local local_dir="$2"
    local prefix="${3:-}"

    if [[ ! -d "$local_dir" ]]; then
        echo "  ⚠ $local_dir not found — skipping"
        return
    fi

    echo "--- Syncing $slug ---"
    local ok=0 fail=0
    while IFS= read -r -d '' file; do
        local rel="${file#$local_dir/}"
        local vault_path="${prefix:+$prefix/}${rel}"
        local http_code
        http_code=$(curl -s -o /dev/null -w "%{http_code}" \
            -X PUT "${API}/${slug}/vault/${vault_path}" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: text/markdown" \
            --data-binary @"$file")
        if [[ "$http_code" =~ ^2 ]]; then
            ((ok++)) || true
        else
            echo "  ✗ $vault_path (HTTP $http_code)"
            ((fail++)) || true
        fi
    done < <(find "$local_dir" -name "*.md" -print0 | sort -z)
    echo "  ✓ $ok files synced, $fail failed"

    echo "  Reindexing $slug..."
    curl -s -X POST "${API}/${slug}/reindex" \
        -H "Authorization: Bearer $TOKEN" \
        | python3 -c "import sys,json; r=json.load(sys.stdin); print(f'  ✓ {r[\"indexed\"]} entries indexed, {len(r[\"errors\"])} errors')" \
        2>/dev/null || echo "  (reindex response not JSON — check server logs)"
    echo ""
}

# CO development tasks
push_universe "co" "/Users/artelonga/projects/co/work/co"

# ArteLonga
push_universe "artelonga" "/Users/artelonga/projects/ArteLonga"

# Quilombo Araucária
push_universe "quilomboaraucaria" "/Users/artelonga/projects/quilomboaraucaria"

# Topologia language universe (consolidate all sub-planes)
echo "--- Syncing topologia (consolidated language universe) ---"
TOPO_ROOT="/Users/artelonga/projects/topologia"
TOPO_OK=0; TOPO_FAIL=0
for folder in concepts guarani-mbya languages portuguese yoruba docs; do
    sub_dir="${TOPO_ROOT}/${folder}"
    [[ -d "$sub_dir" ]] || continue
    while IFS= read -r -d '' file; do
        rel="${file#$sub_dir/}"
        vault_path="${folder}/${rel}"
        http_code=$(curl -s -o /dev/null -w "%{http_code}" \
            -X PUT "${API}/topologia/vault/${vault_path}" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: text/markdown" \
            --data-binary @"$file")
        if [[ "$http_code" =~ ^2 ]]; then
            ((TOPO_OK++)) || true
        else
            echo "  ✗ $vault_path (HTTP $http_code)"
            ((TOPO_FAIL++)) || true
        fi
    done < <(find "$sub_dir" -name "*.md" -print0 | sort -z)
done
echo "  ✓ $TOPO_OK files synced, $TOPO_FAIL failed"
curl -s -X POST "${API}/topologia/reindex" \
    -H "Authorization: Bearer $TOKEN" \
    | python3 -c "import sys,json; r=json.load(sys.stdin); print(f'  ✓ {r[\"indexed\"]} entries indexed')" \
    2>/dev/null || true
echo ""

# RFQ (only docs/ and root .md files — skip Rust source)
echo "--- Syncing rfq (docs only) ---"
RFQ_ROOT="/Users/artelonga/projects/rfq-gateway"
RFQ_OK=0; RFQ_FAIL=0
for f in "${RFQ_ROOT}"/*.md "${RFQ_ROOT}"/docs/*.md; do
    [[ -f "$f" ]] || continue
    rel="${f#$RFQ_ROOT/}"
    http_code=$(curl -s -o /dev/null -w "%{http_code}" \
        -X PUT "${API}/rfq/vault/${rel}" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: text/markdown" \
        --data-binary @"$f")
    if [[ "$http_code" =~ ^2 ]]; then
        ((RFQ_OK++)) || true
    else
        echo "  ✗ $rel (HTTP $http_code)"
        ((RFQ_FAIL++)) || true
    fi
done
echo "  ✓ $RFQ_OK files synced, $RFQ_FAIL failed"
curl -s -X POST "${API}/rfq/reindex" \
    -H "Authorization: Bearer $TOKEN" \
    | python3 -c "import sys,json; r=json.load(sys.stdin); print(f'  ✓ {r[\"indexed\"]} entries indexed')" \
    2>/dev/null || true
echo ""

# --- Apply template to all ---
echo "▶ Applying template scaffold to all owned universes..."
curl -s -X POST "${API}/apply-template-all" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"hub_universe":"co"}' \
    | python3 -c "
import sys,json
r=json.load(sys.stdin)
for u in r.get('results',[]):
    errs = u['type_error_count']
    created = ','.join(u['created']) or '—'
    print(f'  {u[\"slug\"]}: {u[\"content_count\"]} entries, created [{created}], {errs} type errors')
print()
if r.get('hub_entry'):
    print(f'  Hub entry: {r[\"hub_entry\"]}')
" 2>/dev/null || true

echo ""
echo "▶ Installing co-sync launchd agent..."
co-sync install

echo ""
echo "=== Done ==="
echo ""
echo "co-sync status:"
co-sync status
