#!/usr/bin/env bash
# CO-108: Archive one Co universe into a .tar.zst bundle.
# Internal helper — called by backup-to-disk.sh.
#
# Usage: _archive-one.sh <key> <source_path> <out_dir> <mode:local|uat|prod>
# Outputs: <out_dir>/<key>.tar.zst + <out_dir>/<key>.manifest.json (sidecar)
set -euo pipefail

KEY="$1"
SRC="$2"
OUT_DIR="$3"
MODE="$4"

WORK=$(mktemp -d)
trap "rm -rf $WORK" EXIT

BUNDLE_DIR="$WORK/$KEY"
ENTRIES_DIR="$BUNDLE_DIR/entries"
mkdir -p "$ENTRIES_DIR"

ARCHIVED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)

# ── 1. Collect markdown files ─────────────────────────────────────────────────

echo "  [entries] scanning $SRC ..."
(
    cd "$SRC"
    find . -type f -name '*.md' \
        -not -path './.git/*' \
        -not -path './.jj/*' \
        -not -path './.claude/*' \
        -not -path './.obsidian/*' \
        -not -path './.cache/*' \
        -not -path './node_modules/*' \
        -not -path './target/*' \
        -not -path './build/*' \
        -not -path './dist/*' \
) | sed 's|^\./||' | while IFS= read -r rel; do
    dst="$ENTRIES_DIR/$rel"
    mkdir -p "$(dirname "$dst")"
    cp "$SRC/$rel" "$dst"
done

ENTRY_COUNT=$(find "$ENTRIES_DIR" -name '*.md' 2>/dev/null | wc -l | tr -d ' ')
echo "  [entries] $ENTRY_COUNT markdown files"

# ── 2. Source commit info ─────────────────────────────────────────────────────

SOURCE_COMMIT=""
SOURCE_JJ_COMMIT=""

if [[ -d "$SRC/.git" ]]; then
    SOURCE_COMMIT=$(git -C "$SRC" rev-parse HEAD 2>/dev/null) || SOURCE_COMMIT=""
    # Exclude placeholder "HEAD" (empty repo with no commits)
    [[ "$SOURCE_COMMIT" == "HEAD" ]] && SOURCE_COMMIT=""
fi
if [[ -d "$SRC/.jj" ]]; then
    SOURCE_JJ_COMMIT=$(jj -R "$SRC" log -r @ --no-graph -T 'commit_id' 2>/dev/null \
        | head -1) || SOURCE_JJ_COMMIT=""
fi

# ── 3. Warn if local has un-synced changes (best-effort) ─────────────────────

if command -v co &>/dev/null && [[ "$MODE" != "local" ]]; then
    co sync status --universe "$KEY" 2>/dev/null \
        | grep -q 'un-pushed\|diverged' \
        && echo "  [warn] local has un-synced changes — archiving $MODE state" \
        || true
fi

# ── 4. Obtain co.db ──────────────────────────────────────────────────────────

case "$MODE" in
    local)
        echo "  [co.db] creating synthetic db (local mode) ..."
        # Empty SQLite — server runs all migrations on restore.
        sqlite3 "$BUNDLE_DIR/co.db" ""

        # seed.sql: processed once by co-web on startup, then deleted.
        UNIVERSE_NAME="$KEY"
        NOW="$ARCHIVED_AT"
        cat > "$BUNDLE_DIR/seed.sql" << SEEDEOF
-- CO-108 archive restore seed — generated $NOW
-- Consumed once by co-web on startup, then auto-deleted.
INSERT OR IGNORE INTO users
    (id, email, display_name, tier, created_at)
    VALUES ('archive-owner', 'yuri@artelonga.com.br', 'Yuri', 'admin', '$NOW');
INSERT OR IGNORE INTO universes
    (key, name, description, owner_id, created_at, is_template, is_public, content_count)
    VALUES ('$KEY', '$UNIVERSE_NAME',
            'Restored from local archive — source: $SRC',
            'archive-owner', '$NOW', 0, 0, $ENTRY_COUNT);
SEEDEOF
        echo "  [co.db] seed.sql written ($ENTRY_COUNT entries registered)"
        ;;

    uat|prod)
        APP="co-artelonga"
        [[ "$MODE" == "uat" ]] && APP="${APP}-uat"
        echo "  [co.db] extracting from $APP ..."

        # Wake the machine
        curl -s --max-time 30 "https://${APP}.fly.dev/api/health" > /dev/null 2>&1 || true
        sleep 1

        # Prefer meta.db (post-CO-77), fall back to co.db
        DB_TMP="$WORK/remote.db"
        if flyctl ssh console -a "$APP" -C "test -f /data/meta.db" 2>/dev/null; then
            flyctl ssh console -a "$APP" -C "cat /data/meta.db" > "$DB_TMP" 2>/dev/null
        else
            flyctl ssh console -a "$APP" -C "cat /data/co.db" > "$DB_TMP" 2>/dev/null
        fi

        # Extract rows for this universe only into a portable co.db
        python3 - "$DB_TMP" "$BUNDLE_DIR/co.db" "$KEY" << 'PYEOF'
import sqlite3, sys

src_path, dst_path, key = sys.argv[1], sys.argv[2], sys.argv[3]
src = sqlite3.connect(src_path)
dst = sqlite3.connect(dst_path)
dst.execute("PRAGMA journal_mode=WAL")

# Copy table schemas
tables = src.execute(
    "SELECT name, sql FROM sqlite_master "
    "WHERE type='table' AND sql IS NOT NULL "
    "ORDER BY rowid"
).fetchall()
for tname, tsql in tables:
    try:
        dst.execute(tsql)
    except Exception:
        pass

# Recreate indexes
indexes = src.execute(
    "SELECT sql FROM sqlite_master WHERE type='index' AND sql IS NOT NULL"
).fetchall()
for (isql,) in indexes:
    try:
        dst.execute(isql)
    except Exception:
        pass

# schema_version — copy all rows
for row in src.execute("SELECT version FROM schema_version").fetchall():
    try:
        dst.execute("INSERT OR IGNORE INTO schema_version VALUES (?)", row)
    except Exception:
        pass

# Rows in universes table
for row in src.execute("SELECT * FROM universes WHERE key=?", (key,)).fetchall():
    cols = len(row)
    ph = ','.join(['?'] * cols)
    try:
        dst.execute(f"INSERT OR IGNORE INTO universes VALUES ({ph})", row)
    except Exception:
        pass

# Owner user
try:
    stmt = src.execute(
        "SELECT owner_id FROM universes WHERE key=?", (key,)
    ).fetchone()
    if stmt:
        owner_id = stmt[0]
        user = src.execute(
            "SELECT * FROM users WHERE id=?", (owner_id,)
        ).fetchone()
        if user:
            cols = len(user)
            ph = ','.join(['?'] * cols)
            dst.execute(f"INSERT OR IGNORE INTO users VALUES ({ph})", user)
except Exception:
    pass

# universe_members
try:
    for row in src.execute(
        "SELECT * FROM universe_members WHERE universe_key=?", (key,)
    ).fetchall():
        cols = len(row)
        ph = ','.join(['?'] * cols)
        dst.execute(f"INSERT OR IGNORE INTO universe_members VALUES ({ph})", row)
except Exception:
    pass

# entries (legacy meta.db entries if they still exist)
try:
    for row in src.execute(
        "SELECT path, universe_key, entry_type, title, frontmatter_json, "
        "body, body_hash, created_at, updated_at "
        "FROM entries WHERE universe_key=?", (key,)
    ).fetchall():
        dst.execute(
            "INSERT OR IGNORE INTO entries "
            "(path, universe_key, entry_type, title, frontmatter_json, "
            "body, body_hash, created_at, updated_at) "
            "VALUES (?,?,?,?,?,?,?,?,?)",
            row
        )
except Exception:
    pass

dst.commit()
src.close()
dst.close()
print("  ok")
PYEOF

        # Also grab per-universe data.db (entries index) and merge into co.db
        echo "  [co.db] merging per-universe entries ..."
        DATA_DB_TMP="$WORK/data.db"
        flyctl ssh console -a "$APP" \
            -C "python3 -c \"
import xxhash, struct
k='$KEY'
h=0
for b in k.encode():
    h = h*1000003 ^ b
print(hex(h & 0xffffffffffffffff))
\"" 2>/dev/null || true
        # Fetch per-universe data.db via sftp-compatible tar
        HASH_B7_B6=$(python3 -c "
import xxhash; k='$KEY'.encode()
h = 0
for b in range(0, len(k)):
    byte = k[b]
    h ^= byte * (256 ** (7 - b % 8))
# Use xxhash3_64 equivalent approximation
# Use the actual xxh3_64 via ctypes if available, else skip
try:
    import ctypes
    lib = ctypes.CDLL('libxxhash.so', use_errno=True)
    lib.XXH3_64bits.restype = ctypes.c_uint64
    lib.XXH3_64bits.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    h = lib.XXH3_64bits(k, len(k))
    b7 = (h >> 56) & 0xff
    b6 = (h >> 48) & 0xff
    print(f'{b7:02x}/{b6:02x}')
except Exception:
    print('NA/NA')
" 2>/dev/null || echo "NA/NA")
        if [[ "$HASH_B7_B6" != "NA/NA" ]]; then
            flyctl ssh console -a "$APP" \
                -C "[ -f /data/universes/$HASH_B7_B6/$KEY/data.db ] && \
                    cat /data/universes/$HASH_B7_B6/$KEY/data.db || true" \
                > "$DATA_DB_TMP" 2>/dev/null || true
        fi
        if [[ -f "$DATA_DB_TMP" ]] && [[ -s "$DATA_DB_TMP" ]]; then
            python3 - "$DATA_DB_TMP" "$BUNDLE_DIR/co.db" "$KEY" << 'PYEOF'
import sqlite3, sys

data_path, meta_path, key = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    data = sqlite3.connect(data_path)
    meta = sqlite3.connect(meta_path)
    # Ensure entries table exists in meta db
    meta.execute("""CREATE TABLE IF NOT EXISTS entries (
        path TEXT NOT NULL, universe_key TEXT NOT NULL,
        entry_type TEXT NOT NULL, title TEXT,
        frontmatter_json TEXT NOT NULL DEFAULT '{}',
        body TEXT NOT NULL DEFAULT '', body_hash TEXT NOT NULL DEFAULT '',
        created_at TEXT, updated_at TEXT,
        PRIMARY KEY (universe_key, path))""")
    for row in data.execute(
        "SELECT path, universe_key, entry_type, title, frontmatter_json, "
        "body, body_hash, created_at, updated_at FROM entries WHERE universe_key=?",
        (key,)
    ).fetchall():
        meta.execute(
            "INSERT OR REPLACE INTO entries "
            "(path, universe_key, entry_type, title, frontmatter_json, "
            "body, body_hash, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)",
            row
        )
    meta.commit()
    count = meta.execute(
        "SELECT COUNT(*) FROM entries WHERE universe_key=?", (key,)
    ).fetchone()[0]
    print(f"  merged {count} entries from data.db")
    data.close()
    meta.close()
except Exception as e:
    print(f"  skip data.db merge: {e}")
PYEOF
        fi
        ;;
esac

# ── 5. README.md ──────────────────────────────────────────────────────────────

cat > "$BUNDLE_DIR/README.md" << READEOF
# Co Archive — $KEY

| Field | Value |
|-------|-------|
| Universe key | \`$KEY\` |
| Source path | \`$SRC\` |
| Source commit | \`${SOURCE_COMMIT:-n/a}\` |
| jj commit | \`${SOURCE_JJ_COMMIT:-n/a}\` |
| Mode | \`$MODE\` |
| Archived at | \`$ARCHIVED_AT\` |
| Entry count | $ENTRY_COUNT |

## Restore

\`\`\`bash
bash scripts/restore-from-disk.sh \\
    $(basename "$OUT_DIR")/$KEY.tar.zst \\
    /tmp/restored-co

# Boot co-web against the restored data dir:
DATA_DIR=/tmp/restored-co/data cargo run -p co-web
# → universe accessible at http://localhost:3000/co/$KEY
\`\`\`

## Contents

\`\`\`
$KEY/
├── manifest.json      — universe metadata + archive provenance
├── co.db              — SQLite: universe row (+ entries for prod/uat mode)
├── seed.sql           — one-shot seed for local mode (auto-deleted on restore)
├── entries/           — full markdown source tree ($ENTRY_COUNT files)
└── README.md          — this file
\`\`\`
READEOF

# ── 6. manifest.json (no sha256 yet — computed after pack) ────────────────────

python3 - "$BUNDLE_DIR/manifest.json" \
    "$KEY" "$SRC" "$ENTRY_COUNT" "$ARCHIVED_AT" \
    "$SOURCE_COMMIT" "$SOURCE_JJ_COMMIT" "$MODE" << 'PYEOF'
import json, sys

manifest_path = sys.argv[1]
key = sys.argv[2]
src = sys.argv[3]
entry_count = int(sys.argv[4])
archived_at = sys.argv[5]
commit = sys.argv[6] or None
jj_commit = sys.argv[7] or None
mode = sys.argv[8]

deployment = None
if mode == 'prod':
    deployment = 'https://co-artelonga.fly.dev'
elif mode == 'uat':
    deployment = 'https://co-artelonga-uat.fly.dev'

manifest = {
    "format_version": 1,
    "universe_key": key,
    "name": key,
    "description": "",
    "owner_email": "yuri@artelonga.com.br",
    "visibility": "private",
    "theme_preset": "scholarly-light",
    "layout": "board",
    "parent_key": None,
    "source_repo": src,
    "source_commit": commit,
    "source_jj_commit": jj_commit,
    "deployment_snapshot": deployment,
    "archived_at": archived_at,
    "entry_count": entry_count,
    "tar_zst_sha256": None,
    "co_db_schema_version": 26,
}

with open(manifest_path, 'w') as f:
    json.dump(manifest, f, indent=2)
PYEOF

# ── 7. Pack into .tar.zst ─────────────────────────────────────────────────────

OUT_FILE="$OUT_DIR/${KEY}.tar.zst"
echo "  [pack] compressing at zstd level 19 → $(basename "$OUT_FILE") ..."
COPYFILE_DISABLE=1 tar cf - -C "$WORK" "$KEY" | zstd -19 -q -o "$OUT_FILE"

SIZE=$(du -sh "$OUT_FILE" | awk '{print $1}')
echo "  [pack] $SIZE compressed"

# ── 8. SHA256 + sidecar manifest ─────────────────────────────────────────────

SHA256=$(shasum -a 256 "$OUT_FILE" | awk '{print $1}')
echo "  [sha256] $SHA256"

python3 - "$OUT_DIR/${KEY}.manifest.json" \
    "$KEY" "$SRC" "$ENTRY_COUNT" "$ARCHIVED_AT" \
    "$SOURCE_COMMIT" "$SOURCE_JJ_COMMIT" "$MODE" \
    "$SHA256" "$OUT_FILE" << 'PYEOF'
import json, sys, os

manifest_path = sys.argv[1]
key = sys.argv[2]
src = sys.argv[3]
entry_count = int(sys.argv[4])
archived_at = sys.argv[5]
commit = sys.argv[6] or None
jj_commit = sys.argv[7] or None
mode = sys.argv[8]
sha256 = sys.argv[9]
archive_path = sys.argv[10]

deployment = None
if mode == 'prod':
    deployment = 'https://co-artelonga.fly.dev'
elif mode == 'uat':
    deployment = 'https://co-artelonga-uat.fly.dev'

manifest = {
    "format_version": 1,
    "universe_key": key,
    "name": key,
    "description": "",
    "owner_email": "yuri@artelonga.com.br",
    "visibility": "private",
    "theme_preset": "scholarly-light",
    "layout": "board",
    "parent_key": None,
    "source_repo": src,
    "source_commit": commit,
    "source_jj_commit": jj_commit,
    "deployment_snapshot": deployment,
    "archived_at": archived_at,
    "entry_count": entry_count,
    "tar_zst_sha256": sha256,
    "co_db_schema_version": 26,
    "archive_size_bytes": os.path.getsize(archive_path),
    "filename": os.path.basename(archive_path),
}

with open(manifest_path, 'w') as f:
    json.dump(manifest, f, indent=2)
PYEOF
