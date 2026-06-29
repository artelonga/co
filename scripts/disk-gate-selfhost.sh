#!/usr/bin/env bash
# disk-gate-selfhost.sh — CO-537: local disk pre-flight for a self-hosted / M4 prod.
#
# The self-host analogue of the CO-446 Fly disk gate in pipeline-deploy-gate.sh —
# but PURE LOCAL `df` (NO flyctl, no SSH). A CO upgrade that adds a migration
# writes a `schema_version` row at boot; on a near-full data volume that write
# fails with SQLITE_FULL and the server crash-loops (2026-06-11 + 2026-06-13
# outages). Run this BEFORE pulling + restarting co-web on the box.
#
# Usage:
#   bash scripts/disk-gate-selfhost.sh [DATA_DIR]
#   DATA_DIR=/Volumes/co bash scripts/disk-gate-selfhost.sh
#
# Default DATA_DIR: $CO_WEB_DATA, else ~/.co/data.
#
# Thresholds (mirror CO-446):
#   > BLOCK_PCT (default 85)  → exit 1 (refuse the upgrade), actionable message.
#   > WARN_PCT  (default 75)  → warn, exit 0.
#   otherwise                 → pass, exit 0.
#
# Env:
#   BLOCK_PCT   block threshold % full (default 85)
#   WARN_PCT    warn  threshold % full (default 75)
set -uo pipefail

DATA_DIR="${1:-${DATA_DIR:-${CO_WEB_DATA:-$HOME/.co/data}}}"
BLOCK_PCT="${BLOCK_PCT:-85}"
WARN_PCT="${WARN_PCT:-75}"

# If the data dir doesn't exist yet (first boot), check its nearest existing
# parent so the gate still reports the volume that will hold the data.
probe="$DATA_DIR"
while [[ -n "$probe" && ! -e "$probe" ]]; do
    parent="$(dirname "$probe")"
    [[ "$parent" == "$probe" ]] && break
    probe="$parent"
done
if [[ ! -e "$probe" ]]; then
    echo "✗ disk-gate: cannot resolve a real path for '$DATA_DIR' to df" >&2
    exit 1
fi

# `df -P` = POSIX one-line-per-fs output; capacity is column 5 as "NN%".
df_out="$(df -P "$probe" 2>/dev/null)" || {
    echo "✗ disk-gate: df failed for '$probe'" >&2
    exit 1
}
used_pct="$(echo "$df_out" | awk 'END{gsub(/%/,"",$5); print $5}')"
avail_h="$(echo "$df_out" | awk 'END{print $4}')"   # 1K-blocks available

if ! [[ "$used_pct" =~ ^[0-9]+$ ]]; then
    echo "✗ disk-gate: could not parse df capacity for '$probe':" >&2
    echo "$df_out" >&2
    exit 1
fi

# avail is in 1K blocks; render a rough human figure (MB/GB) for the message.
avail_human="${avail_h}K"
if [[ "$avail_h" =~ ^[0-9]+$ ]]; then
    if (( avail_h >= 1048576 )); then
        avail_human="$(awk "BEGIN{printf \"%.1fGB\", $avail_h/1048576}")"
    elif (( avail_h >= 1024 )); then
        avail_human="$(awk "BEGIN{printf \"%.0fMB\", $avail_h/1024}")"
    fi
fi

printf 'disk-gate (CO-537): %s\n' "$DATA_DIR"
printf '  volume: %s — %s%% full, %s free (block >%s%%, warn >%s%%)\n' \
    "$probe" "$used_pct" "$avail_human" "$BLOCK_PCT" "$WARN_PCT"

if (( used_pct > BLOCK_PCT )); then
    echo "✗ [disk] FAIL — ${used_pct}% full (> ${BLOCK_PCT}%)." >&2
    echo "    A migration write at boot can hit SQLITE_FULL and crash-loop co-web." >&2
    echo "    Free space or move \$CO_WEB_DATA to a larger volume BEFORE upgrading:" >&2
    echo "      • prune old backups / Litestream temp / generated artifacts;" >&2
    echo "      • or relocate ${DATA_DIR} to a bigger disk and re-point CO_WEB_DATA;" >&2
    echo "      • on an external volume, extend it, then restart co-web." >&2
    echo "    Runbook: docs/OPERATIONS.md → 'Disk-full recovery'." >&2
    exit 1
elif (( used_pct > WARN_PCT )); then
    echo "⚠ [disk] WARN — ${used_pct}% full (> ${WARN_PCT}%). Free space soon; an" >&2
    echo "    upgrade that adds a migration needs headroom to write schema_version." >&2
    exit 0
else
    echo "✓ [disk] pass — ${used_pct}% full, under the ${WARN_PCT}% warn line."
    exit 0
fi
