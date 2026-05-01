#!/usr/bin/env bash
# CO-108: Restore a co-archive (.tar.zst) into a target data directory.
#
# Usage:
#   bash scripts/restore-from-disk.sh <archive.tar.zst> <target_data_dir>
#
# Examples:
#   bash scripts/restore-from-disk.sh \
#       /Volumes/Backup/co-archive/2026-04-30/qa.tar.zst \
#       /tmp/restored-co
#
#   # Then boot co-web against the restored data:
#   DATA_DIR=/tmp/restored-co/data cargo run -p co-web
#
# What this script does:
#   1. Verifies the archive SHA256 against its sidecar manifest (if present)
#   2. Checks co_db_schema_version compatibility
#   3. Extracts the archive into <target_data_dir>/data/
#   4. Places co.db, seed.sql, and entries/ in the right locations
set -euo pipefail

ARCHIVE="${1:?usage: restore-from-disk.sh <archive.tar.zst> <target_data_dir>}"
TARGET="${2:?usage: restore-from-disk.sh <archive.tar.zst> <target_data_dir>}"

CURRENT_SCHEMA_VERSION=26

ARCHIVE_ABS=$(cd "$(dirname "$ARCHIVE")" && pwd)/$(basename "$ARCHIVE")
ARCHIVE_DIR=$(dirname "$ARCHIVE_ABS")
ARCHIVE_BASE=$(basename "$ARCHIVE_ABS" .tar.zst)

echo "Co archive restore"
echo "  archive: $ARCHIVE_ABS"
echo "  target:  $TARGET"
echo

# ── 1. Verify SHA256 (if sidecar manifest exists) ─────────────────────────────

SIDECAR="$ARCHIVE_DIR/${ARCHIVE_BASE}.manifest.json"
if [[ -f "$SIDECAR" ]]; then
    echo "[verify] checking SHA256 against sidecar manifest ..."
    EXPECTED_SHA=$(python3 -c "
import json, sys
with open('$SIDECAR') as f:
    m = json.load(f)
print(m.get('tar_zst_sha256', ''))
")
    if [[ -n "$EXPECTED_SHA" ]]; then
        ACTUAL_SHA=$(shasum -a 256 "$ARCHIVE_ABS" | awk '{print $1}')
        if [[ "$ACTUAL_SHA" == "$EXPECTED_SHA" ]]; then
            echo "[verify] SHA256 OK"
        else
            echo "[verify] SHA256 MISMATCH — archive may be corrupted" >&2
            echo "  expected: $EXPECTED_SHA" >&2
            echo "  actual:   $ACTUAL_SHA" >&2
            exit 1
        fi
    else
        echo "[verify] no sha256 in sidecar — skipping"
    fi
else
    echo "[verify] no sidecar manifest found — skipping SHA256 check"
fi

# ── 2. Extract archive to temp dir ───────────────────────────────────────────

WORK=$(mktemp -d)
trap "rm -rf $WORK" EXIT

echo "[extract] decompressing ..."
zstd -d -q "$ARCHIVE_ABS" -o "$WORK/archive.tar"
COPYFILE_DISABLE=1 tar xf "$WORK/archive.tar" -C "$WORK"
rm "$WORK/archive.tar"

# Detect universe key (first directory in the archive)
KEY=$(ls "$WORK" | head -1)
BUNDLE_DIR="$WORK/$KEY"

if [[ ! -d "$BUNDLE_DIR" ]]; then
    echo "ERROR: expected archive to contain a directory named after the universe key" >&2
    exit 1
fi
echo "[extract] universe key: $KEY"

# ── 3. Check schema version compatibility ────────────────────────────────────

if [[ -f "$BUNDLE_DIR/manifest.json" ]]; then
    ARCHIVE_SCHEMA=$(python3 -c "
import json, sys
with open('$BUNDLE_DIR/manifest.json') as f:
    m = json.load(f)
print(m.get('co_db_schema_version', 0))
")
    if [[ "$ARCHIVE_SCHEMA" -gt "$CURRENT_SCHEMA_VERSION" ]]; then
        echo "ERROR: archive was created with schema v$ARCHIVE_SCHEMA" >&2
        echo "       this binary expects schema v$CURRENT_SCHEMA_VERSION" >&2
        echo "       upgrade co-web before restoring." >&2
        exit 1
    fi
    echo "[schema] co_db_schema_version=$ARCHIVE_SCHEMA — compatible"
fi

# ── 4. Create data dir structure ─────────────────────────────────────────────

DATA_DIR="$TARGET/data"
UNIVERSE_ROOT="$DATA_DIR/universes/$KEY"
mkdir -p "$DATA_DIR"
mkdir -p "$UNIVERSE_ROOT"

# ── 5. Place co.db ────────────────────────────────────────────────────────────

if [[ -f "$BUNDLE_DIR/co.db" ]]; then
    cp "$BUNDLE_DIR/co.db" "$DATA_DIR/co.db"
    echo "[restore] co.db → $DATA_DIR/co.db"
else
    echo "WARN: no co.db in archive — server will start fresh"
fi

# ── 6. Place seed.sql (local mode) ───────────────────────────────────────────

if [[ -f "$BUNDLE_DIR/seed.sql" ]]; then
    cp "$BUNDLE_DIR/seed.sql" "$DATA_DIR/seed.sql"
    echo "[restore] seed.sql → $DATA_DIR/seed.sql (will run once on co-web startup)"
fi

# ── 7. Place markdown entries ────────────────────────────────────────────────

if [[ -d "$BUNDLE_DIR/entries" ]]; then
    cp -r "$BUNDLE_DIR/entries/." "$UNIVERSE_ROOT/"
    ENTRY_COUNT=$(find "$UNIVERSE_ROOT" -name '*.md' | wc -l | tr -d ' ')
    echo "[restore] entries/ → $UNIVERSE_ROOT/ ($ENTRY_COUNT markdown files)"
else
    echo "WARN: no entries/ in archive"
fi

# ── 8. Print manifest provenance ─────────────────────────────────────────────

if [[ -f "$BUNDLE_DIR/manifest.json" ]]; then
    echo
    echo "Archive provenance:"
    python3 -c "
import json
with open('$BUNDLE_DIR/manifest.json') as f:
    m = json.load(f)
print(f'  universe:   {m.get(\"universe_key\")}')
print(f'  source:     {m.get(\"source_repo\")}')
print(f'  commit:     {m.get(\"source_commit\") or \"n/a\"}')
print(f'  archived:   {m.get(\"archived_at\")}')
print(f'  entries:    {m.get(\"entry_count\")}')
print(f'  mode:       {m.get(\"deployment_snapshot\") or \"local\"}')
"
fi

# ── 9. Summary ────────────────────────────────────────────────────────────────

echo
echo "Restore complete: $TARGET"
echo
echo "To boot co-web against the restored data:"
echo "  DATA_DIR=$DATA_DIR cargo run -p co-web"
echo "  # → universe accessible at http://localhost:3000/co/$KEY"
echo
echo "Notes:"
echo "  - co-web renames co.db → meta.db on first startup (CO-77)"
if [[ -f "$DATA_DIR/seed.sql" ]]; then
    echo "  - seed.sql registers the universe on first boot, then is deleted"
fi
echo "  - The board may be empty if restored from a local-mode archive"
echo "    (no deployed entry index). Re-upload via: co sync push --universe $KEY"
