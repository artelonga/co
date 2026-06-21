#!/usr/bin/env bash
# compute-dashboard — local compute-utilization snapshot for the co-auto fleet
# and this machine. Answers "how much compute am I burning right now?"
#
# Sections:
#   FLEET   — running co-auto --cycle streams + the claude workers they spawn
#             (per-process %CPU, RSS, elapsed, and the CO-task each is on)
#   MACHINE — CPU cores + load average, physical memory, disk free/used
#   DISK    — the build-artifact footprint that fills the disk (target/, worktrees)
#   WORK    — open PRs the fleet has produced
#
# Usage:
#   scripts/compute-dashboard.sh           one snapshot to stdout
#   scripts/compute-dashboard.sh --watch   refresh every 5s (Ctrl-C to stop)
#   scripts/compute-dashboard.sh --html out.html   write an HTML snapshot
#
# macOS (darwin) only — uses ps/sysctl/vm_stat/df/du.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROJECTS_DIR="${CO_LOCAL_REPOS_DIR:-$HOME/projects}"

human_mb() { awk -v b="$1" 'BEGIN{printf "%.0f", b/1024}'; }  # KB → MB

snapshot() {
  local now; now="$(date '+%Y-%m-%d %H:%M:%S')"
  echo "════════════════════════════════════════════════════════════════"
  echo " CO compute utilization · $now"
  echo "════════════════════════════════════════════════════════════════"

  # ── FLEET ──────────────────────────────────────────────────────────
  echo
  echo "▶ FLEET (co-auto)"
  local proclist ncycle nworker
  proclist="$(ps -axo pid,pcpu,rss,etime,command 2>/dev/null \
    | grep -E 'co-auto( |$)|co-auto-CO-' | grep -v grep || true)"
  # cycle drivers = the rust `co-auto --cycle` binaries; workers = spawned claude `co-auto-CO-N`
  ncycle="$(printf '%s\n' "$proclist" | grep -c -- '--cycle' || true)"
  nworker="$(printf '%s\n' "$proclist" | grep -c 'co-auto-CO-' || true)"
  echo "  cycle drivers: ${ncycle:-0}   active workers: ${nworker:-0}"
  if [[ -n "$proclist" ]]; then
    printf "  %-7s %5s %8s %9s  %s\n" PID %CPU RSS-MB ELAPSED TASK
    # all co-auto + spawned claude workers
    printf '%s\n' "$proclist" \
      | while read -r pid pcpu rss etime rest; do
          local task
          task="$(printf '%s' "$rest" | grep -oE 'CO-[0-9]+' | head -1)"
          [[ -z "$task" ]] && task="$(printf '%s' "$rest" | grep -oE -- '--cycle[^ ]*' | head -1)"
          [[ -z "$task" ]] && task="(driver)"
          printf "  %-7s %5s %8s %9s  %s\n" "$pid" "$pcpu" "$(human_mb "$rss")" "$etime" "$task"
        done
  else
    echo "  (no co-auto processes running)"
  fi

  # ── MACHINE ────────────────────────────────────────────────────────
  echo
  echo "▶ MACHINE"
  local cores load memline diskline
  cores="$(sysctl -n hw.ncpu 2>/dev/null)"
  load="$(sysctl -n vm.loadavg 2>/dev/null | tr -d '{}')"
  echo "  cpu cores:$cores   load avg:$load"
  memline="$(top -l 1 -n 0 2>/dev/null | grep -i PhysMem || true)"
  echo "  ${memline:-mem: n/a}"
  diskline="$(df -h /System/Volumes/Data 2>/dev/null | tail -1 \
    | awk '{print "disk /Data: "$4" free, "$3" used ("$5")"}')"
  echo "  ${diskline:-disk: n/a}"

  # ── DISK FOOTPRINT (build artifacts) ───────────────────────────────
  echo
  echo "▶ DISK FOOTPRINT (reclaimable build artifacts)"
  for d in "$REPO_ROOT/target" "$REPO_ROOT/.worktrees" "$PROJECTS_DIR/yggdrasil/target"; do
    if [[ -d "$d" ]]; then
      printf "  %6s  %s\n" "$(du -sh "$d" 2>/dev/null | cut -f1)" "${d/#$HOME/~}"
    fi
  done
  local wtt
  wtt="$(find "$REPO_ROOT/.worktrees" -maxdepth 2 -type d -name target 2>/dev/null | wc -l | tr -d ' ')"
  echo "  worktree target/ dirs: ${wtt:-0}  (prune: find .worktrees -maxdepth 2 -name target -prune -exec rm -rf {} +)"

  # ── WORK (fleet output) ────────────────────────────────────────────
  echo
  echo "▶ WORK (open PRs)"
  if command -v gh >/dev/null 2>&1; then
    gh pr list -R artelonga/co --state open --json number,title,mergeable \
      --jq '.[]|"  #\(.number) [\(.mergeable)] \(.title[:58])"' 2>/dev/null \
      || echo "  (gh unavailable)"
  fi
  echo
}

# HTML snapshot (wraps the text snapshot in <pre>)
if [[ "${1:-}" == "--html" ]]; then
  out="${2:-$REPO_ROOT/playground/compute-dashboard.html}"
  {
    echo "<!doctype html><meta charset=utf-8><title>CO compute</title>"
    echo "<meta http-equiv=refresh content=10>"
    echo "<style>body{background:#0d1117;color:#c9d1d9;font:13px ui-monospace,Menlo,monospace;padding:16px}pre{margin:0}</style>"
    echo "<pre>"
    snapshot | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g'
    echo "</pre>"
  } > "$out"
  echo "wrote $out"
  exit 0
fi

if [[ "${1:-}" == "--watch" ]]; then
  while true; do clear; snapshot; sleep 5; done
else
  snapshot
fi
