#!/usr/bin/env bash
# CO-108: Back up Co universe sources to an external HD as co-compatible archives.
#
# Usage:
#   bash scripts/backup-to-disk.sh <mount-path> [--from local|uat|prod]
#   bash scripts/backup-to-disk.sh <mount-path> --verify <YYYY-MM-DD>
#
# Examples:
#   bash scripts/backup-to-disk.sh /Volumes/Backup --from local
#   bash scripts/backup-to-disk.sh /Volumes/Backup --from prod
#   bash scripts/backup-to-disk.sh /Volumes/Backup --verify 2026-04-30
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Parse arguments ───────────────────────────────────────────────────────────

TARGET_DIR="${1:?usage: backup-to-disk.sh <mount-path> [--from local|uat|prod]}"
shift

MODE="local"
VERIFY_DATE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --from)
            MODE="${2:?--from requires: local|uat|prod}"
            shift 2
            ;;
        --verify)
            VERIFY_DATE="${2:?--verify requires YYYY-MM-DD}"
            shift 2
            ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

# ── Verify mode ───────────────────────────────────────────────────────────────

if [[ -n "$VERIFY_DATE" ]]; then
    VERIFY_DIR="$TARGET_DIR/co-archive/$VERIFY_DATE"
    if [[ ! -d "$VERIFY_DIR" ]]; then
        echo "ERROR: archive dir not found: $VERIFY_DIR" >&2
        exit 1
    fi
    GLOBAL_MANIFEST="$VERIFY_DIR/manifest.json"
    if [[ ! -f "$GLOBAL_MANIFEST" ]]; then
        echo "ERROR: manifest.json not found: $GLOBAL_MANIFEST" >&2
        exit 1
    fi
    echo "Verifying archives in $VERIFY_DIR ..."
    python3 "$SCRIPT_DIR/_archive-verify.py" "$VERIFY_DIR" "$GLOBAL_MANIFEST"
    exit $?
fi

# ── Validate mode ─────────────────────────────────────────────────────────────

case "$MODE" in
    local|uat|prod) ;;
    *) echo "Unknown mode: $MODE (expected: local|uat|prod)" >&2; exit 1 ;;
esac

# ── Prepare output directory ──────────────────────────────────────────────────

DATE=$(date -u +%Y-%m-%d)
OUT="$TARGET_DIR/co-archive/$DATE"
mkdir -p "$OUT"

echo "Co archive — mode=$MODE — date=$DATE"
echo "Output: $OUT"
echo

# ── Universe sources (key:path pairs) ────────────────────────────────────────
# Format: "key:path" — bash 3.2 compatible (no associative arrays on macOS)

UNIVERSE_MAP=(
    "quilomboaraucaria:/Users/artelonga/projects/quilomboaraucaria"
    "qa:/Users/artelonga/projects/quilomboaraucaria"
    "artelonga:/Users/artelonga/projects/ArteLonga"
    "rfq:/Users/artelonga/projects/rfq-gateway"
    "mbya:/Users/artelonga/projects/mbya"
)

# ── Archive each universe ─────────────────────────────────────────────────────

ARCHIVED=0
FAILED=0

for entry in "${UNIVERSE_MAP[@]}"; do
    key="${entry%%:*}"
    src="${entry#*:}"
    if [[ ! -d "$src" ]]; then
        echo "[$key] skip — source $src not found"
        continue
    fi
    echo "[$key] archiving from $src ..."
    if bash "$SCRIPT_DIR/_archive-one.sh" "$key" "$src" "$OUT" "$MODE"; then
        ARCHIVED=$((ARCHIVED + 1))
        echo "[$key] done"
    else
        FAILED=$((FAILED + 1))
        echo "[$key] FAILED" >&2
    fi
    echo
done

if [[ $ARCHIVED -eq 0 ]]; then
    echo "ERROR: no sources archived. Check that your universe repos exist." >&2
    exit 1
fi

# ── Global manifest ───────────────────────────────────────────────────────────

python3 "$SCRIPT_DIR/_archive-manifest.py" "$OUT" "$MODE" "$DATE"

# ── Summary ───────────────────────────────────────────────────────────────────

echo "Archive complete: $OUT"
echo
if ls "$OUT"/*.tar.zst 2>/dev/null | head -1 > /dev/null; then
    du -sh "$OUT"/*.tar.zst
fi
echo "Total: $ARCHIVED archived, $FAILED failed"

if [[ $FAILED -gt 0 ]]; then
    exit 1
fi
