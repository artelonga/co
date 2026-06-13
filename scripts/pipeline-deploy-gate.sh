#!/usr/bin/env bash
#
# pipeline-deploy-gate.sh — CO-88d pre-prod-deploy gate.
#
# Blocks a production deploy unless the latest UAT pipeline report is:
#   1. present and younger than 24h,
#   2. free of failed matrix cells (no decode/round-trip errors),
#   3. free of regressions beyond 20% vs the previous run.
#
# On a passing gate (and when invoked with `--smoke`), runs the prod read-only
# smoke (Path D) and appends its report alongside the UAT one.
#
# Usage:
#   scripts/pipeline-deploy-gate.sh                 # gate the latest report
#   scripts/pipeline-deploy-gate.sh --baseline P    # also check regressions vs P
#   scripts/pipeline-deploy-gate.sh --smoke         # gate, then run prod smoke
#
# Exit non-zero (and refuse the deploy) when the gate fails.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPORTS_DIR="$REPO_ROOT/dev/reports"
PROD_BASE="${PROD_BASE:-https://co-artelonga.fly.dev}"
MAX_AGE_HOURS="${MAX_AGE_HOURS:-24}"
MAX_REGRESSION_PCT="${MAX_REGRESSION_PCT:-20}"

baseline=""
run_smoke=false
while [ $# -gt 0 ]; do
  case "$1" in
    --baseline) baseline="$2"; shift 2 ;;
    --smoke) run_smoke=true; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

report="$(ls -t "$REPORTS_DIR"/co-pipeline-report-*.yaml 2>/dev/null | head -1 || true)"
if [ -z "$report" ]; then
  echo "GATE FAIL: no pipeline report found in $REPORTS_DIR" >&2
  echo "Run: cargo run -p co-pipeline -- run --paths local,uat --uat-base \$UAT_BASE" >&2
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
echo "GATE: pass — UAT pipeline report is green, fresh, and within tolerance."

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
