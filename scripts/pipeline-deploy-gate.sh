#!/usr/bin/env bash
#
# pipeline-deploy-gate.sh — CO-88d pre-prod-deploy gate.
#
# Blocks a production deploy unless the latest pipeline report is:
#   1. present and younger than 24h,
#   2. free of failed matrix cells (no decode/round-trip errors),
#   3. free of regressions beyond 20% vs the previous run.
#
# NOTE: UAT was decommissioned (the co-artelonga-uat Fly app no longer exists).
# The report is produced from the local path (`--paths local`), optionally with a
# prod read-only smoke (`--paths local,prod`). There is no UAT path anymore.
#
# CO-446: also checks prod `/data` free space before a deploy. A deploy that
# adds a migration writes a `schema_version` row at boot; on a near-full volume
# that write fails with SQLITE_FULL and the server crash-loops (2026-06-11 +
# 2026-06-13 outages). The gate blocks when `/data` is > DISK_MAX_PCT% full.
#
# On a passing gate (and when invoked with `--smoke`), runs the prod read-only
# smoke (Path D) and appends its report alongside the local one.
#
# Usage:
#   scripts/pipeline-deploy-gate.sh                 # gate report + prod disk
#   scripts/pipeline-deploy-gate.sh --baseline P    # also check regressions vs P
#   scripts/pipeline-deploy-gate.sh --smoke         # gate, then run prod smoke
#   scripts/pipeline-deploy-gate.sh --no-disk       # skip the disk check
#
# Env:
#   DISK_MAX_PCT   threshold % full that blocks the deploy (default 85)
#   PROD_APP       Fly app whose /data volume is checked (default co-artelonga)
#   ALLOW_FULL_DISK=1   warn instead of block (use only with a planned extend)
#
# Exit non-zero (and refuse the deploy) when the gate fails.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPORTS_DIR="$REPO_ROOT/dev/reports"
PROD_BASE="${PROD_BASE:-https://co-artelonga.fly.dev}"
PROD_APP="${PROD_APP:-co-artelonga}"
MAX_AGE_HOURS="${MAX_AGE_HOURS:-24}"
MAX_REGRESSION_PCT="${MAX_REGRESSION_PCT:-20}"
DISK_MAX_PCT="${DISK_MAX_PCT:-85}"

baseline=""
run_smoke=false
check_disk=true
while [ $# -gt 0 ]; do
  case "$1" in
    --baseline) baseline="$2"; shift 2 ;;
    --smoke) run_smoke=true; shift ;;
    --no-disk) check_disk=false; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

# CO-446: pre-deploy /data free-space gate. Reads `df -P /data` on the prod
# machine over `flyctl ssh` and blocks when usage exceeds DISK_MAX_PCT.
disk_gate() {
  if ! command -v flyctl >/dev/null 2>&1; then
    echo "DISK GATE: flyctl not found — skipping prod disk check" >&2
    return 0
  fi
  echo "Checking prod /data free space on $PROD_APP (block at ${DISK_MAX_PCT}% full) ..."
  local df_out used_pct
  if ! df_out="$(flyctl ssh console -a "$PROD_APP" -C "df -P /data" 2>/dev/null)"; then
    echo "DISK GATE: could not reach $PROD_APP via flyctl ssh — skipping (verify manually)" >&2
    return 0
  fi
  # Last line, 5th column is "NN%". Strip the percent sign.
  used_pct="$(echo "$df_out" | awk 'END{gsub(/%/,"",$5); print $5}')"
  if ! [[ "$used_pct" =~ ^[0-9]+$ ]]; then
    echo "DISK GATE: could not parse df output — skipping (verify manually):" >&2
    echo "$df_out" >&2
    return 0
  fi
  echo "  /data is ${used_pct}% full."
  if [ "$used_pct" -gt "$DISK_MAX_PCT" ]; then
    if [ "${ALLOW_FULL_DISK:-0}" = "1" ]; then
      echo "DISK GATE: WARN — /data ${used_pct}% > ${DISK_MAX_PCT}%, proceeding (ALLOW_FULL_DISK=1)." >&2
      return 0
    fi
    echo "DISK GATE FAIL: /data is ${used_pct}% full (> ${DISK_MAX_PCT}%)." >&2
    echo "  A migration write at boot can hit SQLITE_FULL and crash-loop the site." >&2
    echo "  Extend first, then redeploy. Runbook: docs/OPERATIONS.md → 'Disk-full recovery'." >&2
    echo "    flyctl volumes list -a $PROD_APP" >&2
    echo "    flyctl volumes extend <vol-id> -s <new-GB> -a $PROD_APP" >&2
    echo "    flyctl machine stop <id> -a $PROD_APP && flyctl machine start <id> -a $PROD_APP" >&2
    echo "  (machine stop/start — a plain restart does NOT resize the filesystem)" >&2
    return 1
  fi
  echo "DISK GATE: pass — /data ${used_pct}% full, under the ${DISK_MAX_PCT}% limit."
}

if [ "$check_disk" = true ]; then
  disk_gate || exit 1
fi

report="$(ls -t "$REPORTS_DIR"/co-pipeline-report-*.yaml 2>/dev/null | head -1 || true)"
if [ -z "$report" ]; then
  echo "GATE FAIL: no pipeline report found in $REPORTS_DIR" >&2
  # UAT was decommissioned (no co-artelonga-uat app), so the report is produced
  # from the local path (optionally a prod read-only smoke). Do NOT ask for a UAT
  # report that can never be generated.
  echo "Run: cargo run -p co-pipeline -- run --paths local --out dev/reports" >&2
  echo "  (or add a prod smoke: --paths local,prod --prod-base $PROD_BASE)" >&2
  exit 1
fi
echo "Gating on: $report"

gate_args=(gate --report "$report" --max-age-hours "$MAX_AGE_HOURS" --max-regression-pct "$MAX_REGRESSION_PCT")
if [ -n "$baseline" ]; then
  gate_args+=(--baseline "$baseline")
fi

if ! cargo run -q -p co-pipeline -- "${gate_args[@]}"; then
  echo "GATE FAIL: pipeline gate blocked the deploy" >&2
  exit 1
fi
echo "GATE: pass — pipeline report is green, fresh, and within tolerance."

if [ "$run_smoke" = true ]; then
  echo "Running prod read-only smoke (Path D) against $PROD_BASE ..."
  cargo run -q -p co-pipeline -- run \
    --corpus-root "$REPO_ROOT/.." \
    --paths prod \
    --prod-base "$PROD_BASE" \
    --out "$REPORTS_DIR" \
    --date "prod-smoke-$(date -u +%Y-%m-%d)"
  echo "Prod smoke appended to $REPORTS_DIR."
fi
