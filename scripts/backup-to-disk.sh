#!/usr/bin/env bash
# CO-108: Universe archive format — one .tar.zst per source repo → external HD.
# CO-109: mbya corpus included as the 4th (5th, with qa alias) source.
#
# Usage:
#   bash scripts/backup-to-disk.sh /Volumes/Backup [--from local|uat|prod]
#
# Output:
#   <TARGET_DIR>/co-archive/<YYYY-MM-DD>/
#       manifest.json            — global run manifest
#       artelonga.tar.zst
#       quilomboaraucaria.tar.zst
#       rfq.tar.zst
#       qa.tar.zst
#       mbya.tar.zst             — CO-109 stress-test corpus (≤25 MB compressed)
#
# Requires: zstd, python3, sqlite3 (for --from prod/uat also: flyctl)
set -euo pipefail

TARGET_DIR="${1:?usage: backup-to-disk.sh <mount-path> [--from local|uat|prod]}"
SOURCE="local"
if [[ "${2:-}" == "--from" && -n "${3:-}" ]]; then
    SOURCE="$3"
elif [[ "${2:-}" == --from=* ]]; then
    SOURCE="${2#--from=}"
fi

DATE=$(date -u +%Y-%m-%d)
OUT="$TARGET_DIR/co-archive/$DATE"
mkdir -p "$OUT"

# Universe keys and their corresponding source paths (parallel arrays).
# Add new universes here; use the same index in both arrays.
UNIVERSE_KEYS=(
    quilomboaraucaria
    qa
    artelonga
    rfq
    mbya
)
UNIVERSE_PATHS=(
    "/Users/artelonga/projects/quilomboaraucaria"
    "/Users/artelonga/projects/quilomboaraucaria"
    "/Users/artelonga/projects/ArteLonga"
    "/Users/artelonga/projects/rfq-gateway"
    "/Users/artelonga/projects/mbya/content"
)

archive_one() {
    local key="$1" src="$2"
    local out_file="$OUT/${key}.tar.zst"
    local work
    work=$(mktemp -d)
    # shellcheck disable=SC2064
    trap "rm -rf $work" RETURN

    local entry_dir="$work/$key/entries"
    mkdir -p "$entry_dir"

    echo "  archiving $key from $src ..."

    local entry_count=0
    if [[ -d "$src" ]]; then
        while IFS= read -r f; do
            rel="${f#$src/}"
            dir="$entry_dir/$(dirname "$rel")"
            mkdir -p "$dir"
            cp "$f" "$entry_dir/$rel"
            entry_count=$((entry_count + 1))
        done < <(find "$src" -type f -name "*.md" \
            -not -path "*/.git/*" -not -path "*/.jj/*" \
            -not -path "*/node_modules/*" -not -path "*/target/*")
    fi

    # manifest.json
    python3 - "$key" "$src" "$entry_count" "$DATE" > "$work/$key/manifest.json" <<'PYEOF'
import sys, json
key, src, count, date = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
print(json.dumps({
    "format_version": 1,
    "universe_key": key,
    "source_repo": src,
    "archived_at": date + "T00:00:00Z",
    "entry_count": count,
}, indent=2))
PYEOF

    # README
    cat > "$work/$key/README.md" <<EOREADME
# Archive: $key

- **Source:** $src
- **Date:** $DATE
- **Entries:** $entry_count markdown files

## Restore

    tar -I zstd -xf ${key}.tar.zst
    # Then: drop entries/ into /data/universes/${key}/ and co.db into /data/
EOREADME

    # Pack with zstd level 19
    ( cd "$work" && tar cf - "$key" | zstd -19 -q -o "$out_file" )

    local size
    size=$(du -sh "$out_file" | cut -f1)
    echo "  → $out_file ($size, $entry_count entries)"
}

echo "[backup] source=$SOURCE date=$DATE target=$OUT"
echo

i=0
while [[ $i -lt ${#UNIVERSE_KEYS[@]} ]]; do
    key="${UNIVERSE_KEYS[$i]}"
    src="${UNIVERSE_PATHS[$i]}"
    i=$((i + 1))
    if [[ ! -d "$src" ]]; then
        echo "  skip $key — source $src missing"
        continue
    fi
    archive_one "$key" "$src"
done

# Global manifest with SHA-256 hashes
python3 - "$OUT" "$DATE" > "$OUT/manifest.json" <<'PYEOF'
import sys, json, os, glob, hashlib
out, date = sys.argv[1], sys.argv[2]
archives = {}
for f in sorted(glob.glob(os.path.join(out, "*.tar.zst"))):
    key = os.path.basename(f).replace(".tar.zst", "")
    sha256 = hashlib.sha256(open(f, "rb").read()).hexdigest()
    size = os.path.getsize(f)
    archives[key] = {"file": os.path.basename(f), "sha256": sha256, "bytes": size}
print(json.dumps({"archived_at": date + "T00:00:00Z", "archives": archives}, indent=2))
PYEOF

echo
echo "Archive complete: $OUT"
du -sh "$OUT"/*.tar.zst 2>/dev/null | sort -h
echo
echo "Global manifest written: $OUT/manifest.json"
