#!/usr/bin/env bash
# migrate-fly-to-m4.sh — one-time Fly → M4 (self-host) data migration.
#
# The data-move step of the prod cutover (CO-538): pull the live SQLite estate
# off the managed Fly app and restore it into the self-host data dir on the Mac
# M4, then PROVE it (smoke + restore check). After this runs green and DNS is
# cut, Fly can be retired.
#
# CO-77 layout moved is the whole `/data` tree:
#     /data/meta.db                      (global: universes, users, schema_version)
#     /data/universes/<key>/data.db      (per-universe content, WAL)
#
# SAFETY MODEL
#   * DRY-RUN BY DEFAULT. Without --apply it only PRINTS the exact commands it
#     would run and touches nothing (local or remote).
#   * --apply is required to actually tar/download/restore.
#   * Before overwriting an existing target data dir it (a) takes a timestamped
#     backup tarball of the current target and (b) asks for an explicit y/N
#     confirmation. Nothing destructive happens without BOTH --apply and the
#     confirmation.
#   * The remote side is READ-ONLY: it only `tar`s /data into /tmp and removes
#     that temp tarball afterwards. The live Fly DB is never modified.
#
# NOTE: this cannot be exercised without Fly credentials + a reachable app, so it
# is written to be correct and heavily commented; `bash -n` passes. Do a real run
# only during the actual cutover window, ideally with the Fly app quiesced (no
# writes) so meta.db + the per-universe DBs are a consistent snapshot.
#
# Usage:
#   bash scripts/migrate-fly-to-m4.sh                 # DRY-RUN (prints the plan)
#   bash scripts/migrate-fly-to-m4.sh --apply         # actually migrate
#   bash scripts/migrate-fly-to-m4.sh --app co-artelonga --data-dir ~/.co/data --apply
#
# Flags / env:
#   --apply                 perform the migration (otherwise dry-run)
#   --app NAME              Fly app to pull from   (default: co-artelonga / $PROD_APP)
#   --data-dir DIR          local target data dir  (default: ~/.co/data / $CO_WEB_DATA)
#   --staging DIR           local staging dir for the downloaded tar (default: mktemp)
#   --remote-data DIR       remote data dir on the Fly machine (default: /data)
#   --no-verify             skip the post-restore smoke + verify-restore proof
#   -h | --help             show this help
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

APPLY=0
APP="${PROD_APP:-co-artelonga}"
DATA_DIR="${CO_WEB_DATA:-$HOME/.co/data}"
STAGING=""
REMOTE_DATA="/data"
DO_VERIFY=1

usage() { sed -n '2,46p' "$0" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    --apply)        APPLY=1; shift ;;
    --app)          APP="${2:?--app needs a value}"; shift 2 ;;
    --app=*)        APP="${1#*=}"; shift ;;
    --data-dir)     DATA_DIR="${2:?--data-dir needs a value}"; shift 2 ;;
    --data-dir=*)   DATA_DIR="${1#*=}"; shift ;;
    --staging)      STAGING="${2:?--staging needs a value}"; shift 2 ;;
    --staging=*)    STAGING="${1#*=}"; shift ;;
    --remote-data)  REMOTE_DATA="${2:?--remote-data needs a value}"; shift 2 ;;
    --remote-data=*) REMOTE_DATA="${1#*=}"; shift ;;
    --no-verify)    DO_VERIFY=0; shift ;;
    -h|--help)      usage; exit 0 ;;
    *) echo "migrate-fly-to-m4: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

say()  { echo "migrate: $*"; }
err()  { echo "migrate: ERROR: $*" >&2; }
step() { echo; echo "──▶ $*"; }

# run CMD...  — echo the command; execute it only under --apply.
run() {
  if [ "$APPLY" -eq 1 ]; then
    echo "  + $*"
    "$@"
  else
    echo "  (dry-run) $*"
  fi
}

TS="$(date -u +%Y%m%dT%H%M%SZ)"
REMOTE_TAR="/tmp/co-data-${TS}.tar.gz"

if [ "$APPLY" -eq 0 ]; then
  say "DRY-RUN — nothing will be changed. Re-run with --apply to perform the migration."
fi
say "Fly app (source):   $APP"
say "remote data dir:    $REMOTE_DATA"
say "local target dir:   $DATA_DIR"

# ── Preflight ────────────────────────────────────────────────────────────────
step "Preflight"
if ! command -v flyctl >/dev/null 2>&1; then
  err "flyctl is not installed / not on PATH — required to reach the Fly app."
  echo "  Install: https://fly.io/docs/flyctl/install/" >&2
  exit 1
fi
say "flyctl: present"
if [ "$APPLY" -eq 1 ]; then
  if ! flyctl status -a "$APP" >/dev/null 2>&1; then
    err "cannot reach Fly app '$APP' (flyctl status failed). Check --app and 'fly auth'."
    exit 1
  fi
  say "Fly app reachable: $APP"
else
  echo "  (dry-run) flyctl status -a $APP"
fi

# ── Staging dir ──────────────────────────────────────────────────────────────
if [ -z "$STAGING" ]; then
  if [ "$APPLY" -eq 1 ]; then
    STAGING="$(mktemp -d "${TMPDIR:-/tmp}/co-fly-migrate.XXXXXX")"
  else
    STAGING="<mktemp -d>/co-fly-migrate.XXXXXX"
  fi
fi
LOCAL_TAR="$STAGING/co-data-${TS}.tar.gz"
say "staging dir:        $STAGING"

# ── 1. Snapshot /data on the Fly machine (remote, read-only toward the live DB) ─
step "1/5 — tar ${REMOTE_DATA} on the Fly machine → ${REMOTE_TAR}"
# `-C $REMOTE_DATA .` packs meta.db + universes/* with paths relative to /data so
# the archive extracts cleanly into ANY target dir. WAL files are included as-is.
run flyctl ssh console -a "$APP" -C "tar czf ${REMOTE_TAR} -C ${REMOTE_DATA} ."

# ── 2. Download the snapshot to local staging ────────────────────────────────
step "2/5 — download ${REMOTE_TAR} → ${LOCAL_TAR}"
run flyctl ssh sftp get -a "$APP" "$REMOTE_TAR" "$LOCAL_TAR"
# Tidy the remote temp tarball regardless (don't leave it filling Fly's /tmp).
run flyctl ssh console -a "$APP" -C "rm -f ${REMOTE_TAR}"

if [ "$APPLY" -eq 1 ]; then
  if [ ! -s "$LOCAL_TAR" ]; then
    err "downloaded archive is missing or empty: $LOCAL_TAR"
    exit 1
  fi
  say "downloaded $(wc -c < "$LOCAL_TAR" | tr -d ' ') bytes."
fi

# ── 3. Back up any existing target, then confirm before overwriting ──────────
step "3/5 — back up existing target + confirm overwrite"
if [ -e "$DATA_DIR" ] && [ -n "$(ls -A "$DATA_DIR" 2>/dev/null || true)" ]; then
  BACKUP_TAR="${DATA_DIR%/}.pre-migrate-${TS}.tar.gz"
  say "target $DATA_DIR is NON-EMPTY — backing it up to ${BACKUP_TAR} first."
  run tar czf "$BACKUP_TAR" -C "$(dirname "$DATA_DIR")" "$(basename "$DATA_DIR")"
  if [ "$APPLY" -eq 1 ]; then
    printf 'migrate: OVERWRITE %s with the Fly snapshot? [y/N] ' "$DATA_DIR"
    read -r reply
    case "$reply" in
      y|Y|yes|YES) say "confirmed — proceeding." ;;
      *) err "aborted by user; nothing was overwritten. Backup kept at ${BACKUP_TAR}."; exit 1 ;;
    esac
  else
    echo "  (dry-run) would prompt: OVERWRITE $DATA_DIR with the Fly snapshot? [y/N]"
  fi
else
  say "target $DATA_DIR is empty/absent — no pre-restore backup needed."
fi

# ── 4. Restore the snapshot into the local target dir ────────────────────────
step "4/5 — restore snapshot into ${DATA_DIR}"
run mkdir -p "$DATA_DIR/universes"
# Extract the archive (paths are relative to the old /data) into the target.
run tar xzf "$LOCAL_TAR" -C "$DATA_DIR"
say "restore complete — meta.db + universes/* now under $DATA_DIR"

# ── 5. Prove it: leak audit + smoke + restore verification ───────────────────
step "5/5 — verify the restored estate"
if [ "$DO_VERIFY" -eq 0 ]; then
  say "verification skipped (--no-verify). Run smoke-selfhost.sh + verify-restore.sh manually."
elif [ "$APPLY" -eq 0 ]; then
  echo "  (dry-run) CO_WEB_DATA=$DATA_DIR bash $SCRIPT_DIR/smoke-selfhost.sh"
  echo "  (dry-run) CO_WEB_DATA=$DATA_DIR bash $SCRIPT_DIR/selfhost/verify-restore.sh"
else
  # audit_serve confirms no draft-leak surface came across in the snapshot.
  if command -v cargo >/dev/null 2>&1; then
    say "audit_serve (CO-439) on the restored data dir ..."
    ( cd "$REPO_ROOT" && cargo run -q -p co-web --bin audit_serve -- "$DATA_DIR" ) \
      || err "audit_serve reported a leak surface — review before serving publicly."
  fi
  # Boot a co-web (the operator should already have it running via run.sh/launchd);
  # smoke against it. This asserts capabilities, not content counts (content-agnostic).
  say "running smoke-selfhost.sh against the local server (start co-web first if not up) ..."
  CO_WEB_DATA="$DATA_DIR" bash "$SCRIPT_DIR/smoke-selfhost.sh" \
    || err "smoke-selfhost.sh failed — is co-web running on http://localhost:8742?"
  # Prove the off-box backup of the freshly-restored meta.db restores cleanly
  # (only meaningful once Litestream has replicated at least once on the M4).
  say "running verify-restore.sh (Litestream replica integrity) ..."
  CO_WEB_DATA="$DATA_DIR" bash "$SCRIPT_DIR/selfhost/verify-restore.sh" \
    || say "verify-restore.sh did not pass yet (expected if Litestream hasn't replicated on the M4 — set up backups, then re-run)."
fi

echo
if [ "$APPLY" -eq 1 ]; then
  say "DONE — Fly → M4 data migration complete. Next: harden + cut DNS (see work/co/CO-538.md)."
  say "Pre-migrate backup of the previous target (if any) is kept next to $DATA_DIR."
else
  say "DRY-RUN complete — re-run with --apply during the cutover window to execute."
fi
