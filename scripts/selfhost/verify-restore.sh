#!/usr/bin/env bash
#
# verify-restore.sh — prove the Litestream backup of meta.db actually restores.
#
# "Don't trust a backup you haven't restored." This restores the LATEST Litestream
# replica of meta.db to a TEMP file and sanity-checks it. It NEVER touches the live
# DB and is safe to run anytime — purely read-only toward the live data and bucket.
#
# What it does:
#   1. Restore the latest meta.db replica → a mktemp path (never the live DB path).
#   2. Run `PRAGMA integrity_check` (must equal 'ok').
#   3. COUNT(*) a known table to confirm real content restored (>= 0 rows).
#   4. Report PASS/FAIL; clean up the temp file (unless --keep). Exit non-zero on FAIL.
#
# NOTE (CO-77): this verifies meta.db (the GLOBAL DB Litestream covers). The
# per-universe universes/<key>/data.db files are backed up by the companion
# tar/rsync snapshot (see litestream.yml) — verify those by extracting your
# latest archive and running `sqlite3 <db> 'PRAGMA integrity_check'` on a sample.
#
# Integrity check uses `sqlite3`. co-web links SQLite statically (rusqlite) but
# ships no standalone CLI, so install the sqlite3 CLI if it is missing.
#
# Usage:
#   scripts/selfhost/verify-restore.sh [--config FILE] [--keep]
#
# Env:
#   CO_WEB_DATA  local data dir   (default: ~/.co/data)
#                used only to derive the meta.db path Litestream is configured for.
#
set -euo pipefail

# ── Resolve repo root (this script lives in scripts/selfhost/) ──────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

DATA="${CO_WEB_DATA:-$HOME/.co/data}"
DB_PATH="$DATA/meta.db"
CONFIG="$SCRIPT_DIR/litestream.yml"
KEEP=0

# Tables that always exist in meta.db (CO-77 global DB). `universes` is the
# primary content table; `schema_version` is the always-present fallback.
KNOWN_TABLE_PRIMARY="universes"
KNOWN_TABLE_FALLBACK="schema_version"

usage() {
  cat <<EOF
verify-restore.sh — restore the latest Litestream replica of meta.db to a temp
file and sanity-check it (read-only; never touches the live DB).

Usage:
  scripts/selfhost/verify-restore.sh [--config FILE] [--keep]

Options:
  --config FILE   Litestream config (default: $CONFIG)
  --keep          Keep the temp restored DB for inspection (prints its path).
  -h, --help      Show this help.

Env:
  CO_WEB_DATA     data dir used to derive the meta.db path (default: ~/.co/data)

Exit status: 0 = PASS, non-zero = FAIL (or no replica / missing prerequisites).
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --config)   CONFIG="${2:-}"; shift 2 ;;
    --config=*) CONFIG="${1#*=}"; shift ;;
    --keep)     KEEP=1; shift ;;
    -h|--help)  usage; exit 0 ;;
    *) echo "verify-restore.sh: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

say()  { echo "verify-restore: $*"; }
err()  { echo "verify-restore: ERROR: $*" >&2; }

# ── Preflight ─────────────────────────────────────────────────────────────────
if ! command -v litestream >/dev/null 2>&1; then
  err "the 'litestream' CLI is not installed or not on PATH."
  echo "  Install it: brew install litestream" >&2
  exit 1
fi
if ! command -v sqlite3 >/dev/null 2>&1; then
  err "the 'sqlite3' CLI is not installed or not on PATH."
  echo "  Install it: brew install sqlite (macOS already ships one in /usr/bin)." >&2
  exit 1
fi
if [ ! -f "$CONFIG" ]; then
  err "litestream config not found: $CONFIG"
  echo "  Pass --config FILE, or create it (see scripts/selfhost/litestream.yml)." >&2
  exit 1
fi

# ── Restore the latest replica to a temp file ─────────────────────────────────
TMPDIR_X="$(mktemp -d "${TMPDIR:-/tmp}/co-verify.XXXXXX")"
TMP_DB="$TMPDIR_X/restored-meta.db"
cleanup() {
  if [ "$KEEP" -eq 1 ]; then
    say "keeping restored DB at: $TMP_DB (remove it yourself when done)"
  else
    rm -rf "$TMPDIR_X"
  fi
}
trap cleanup EXIT

say "config: $CONFIG"
say "configured live DB path: $DB_PATH (NOT touched)"
say "restoring latest replica → $TMP_DB ..."

# `-if-replica-exists` makes a fresh/empty bucket a clean no-op rather than an
# error, so we can give a precise "no replica yet" message.
RESTORE_OUT="$(litestream restore -config "$CONFIG" -if-replica-exists -o "$TMP_DB" "$DB_PATH" 2>&1)" \
  && RESTORE_RC=0 || RESTORE_RC=$?
[ -n "$RESTORE_OUT" ] && echo "$RESTORE_OUT" | sed 's/^/  litestream: /'

if [ "$RESTORE_RC" -ne 0 ]; then
  err "litestream restore failed (exit $RESTORE_RC)."
  exit 1
fi

if [ ! -f "$TMP_DB" ]; then
  err "no replica found — nothing was restored."
  echo "  Has 'litestream replicate -config $CONFIG' run yet (fresh bucket)?" >&2
  echo "  Until at least one snapshot exists, there is nothing to verify." >&2
  exit 1
fi
say "restored: $(wc -c < "$TMP_DB" | tr -d ' ') bytes."

# ── Sanity check: integrity_check + COUNT(*) on a known table ─────────────────
PASS=0

integ="$(sqlite3 "$TMP_DB" 'PRAGMA integrity_check;' 2>/dev/null || true)"
if [ "$integ" != "ok" ]; then
  err "integrity_check returned: '${integ:-<empty>}' (expected 'ok')"
else
  say "integrity_check: ok"
  table="$KNOWN_TABLE_PRIMARY"
  count="$(sqlite3 "$TMP_DB" "SELECT COUNT(*) FROM $table;" 2>/dev/null || true)"
  if [ -z "$count" ]; then
    table="$KNOWN_TABLE_FALLBACK"
    count="$(sqlite3 "$TMP_DB" "SELECT COUNT(*) FROM $table;" 2>/dev/null || true)"
  fi
  if [ -z "$count" ]; then
    err "could not COUNT(*) on '$KNOWN_TABLE_PRIMARY' or '$KNOWN_TABLE_FALLBACK' — schema unexpected."
  else
    say "table '$table' row count: $count"
    PASS=1
  fi
fi

echo
if [ "$PASS" -eq 1 ]; then
  say "RESULT: PASS — the latest Litestream replica of meta.db restores to a healthy DB."
  exit 0
else
  err "RESULT: FAIL — the restored replica did not pass the sanity check."
  exit 1
fi
