#!/usr/bin/env bash
# Phase C launcher — kick off co-auto cycle mode per repo.
#
# Model: one long-running refactor branch per repo, ALL tasks commit to it,
# ONE PR per repo at the end (opened manually after all tasks done).
#
# Prerequisite: each repo's audit PR is MERGED into main (so the work/<space>/
# task specs are available to co-auto).
#
# Usage:
#     # Verify audit PRs are merged first:
#     bash scripts/launch-phase-c.sh --check
#
#     # Dry-run (show commands, don't execute):
#     bash scripts/launch-phase-c.sh --dry-run
#
#     # Launch one repo only (smoke test recommended):
#     bash scripts/launch-phase-c.sh --only co
#
#     # Launch all 5 in parallel (background):
#     bash scripts/launch-phase-c.sh --all --background
#
# Co-auto flags used:
#   --workdir <repo>         repo path
#   --space <name>           work space (co | rfq | qb | artelonga | yggdrasil)
#   --cycle                  cycle through pending tasks on a single branch
#   --max-tasks <N>          total tasks in this batch (sets the cap)
#   --branch <name>          long-running branch (one per repo)
#   --headless               no interactive prompts
#
# After all tasks complete, manually open ONE PR per repo:
#     gh -R <repo> pr create --base main --head refactor/architecture-audit-2026-05-18 \
#       --title "refactor(architecture): epic-aggregate (CO-215..226 / RFQ-16..23 / ...)"

set -euo pipefail

BRANCH="refactor/architecture-audit-2026-05-18"

# repo:space:max-tasks
REPOS=(
  "co:/Users/artelonga/projects/co:co:12"
  "rfq-gateway:/Users/artelonga/projects/rfq-gateway:rfq:8"
  "ArteLonga:/Users/artelonga/projects/ArteLonga:artelonga:10"
  "yggdrasil:/Users/artelonga/projects/yggdrasil:yggdrasil:9"
  "quilombo-blog:/Users/artelonga/projects/quilombo-blog:qb:12"
)

# Tasks to skip per repo (e.g. blocked on external gates):
# rfq: skip 14, 15 (Hedix DNS gate)
SKIP_TASKS_RFQ="14 15"

# Wave 1 priority pick per repo (for --only smoke test):
WAVE1_CO=215
WAVE1_RFQ=17
WAVE1_AL=51
WAVE1_YG=38
WAVE1_QB=2

action="${1:---help}"
only=""
background=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)       action="check" ;;
    --dry-run)     action="dry" ;;
    --only)        only="$2"; shift ;;
    --all)         action="all" ;;
    --background)  background="--background" ;;
    --help|-h)     action="help" ;;
  esac
  shift
done

print_help() {
  cat <<EOF
Phase C launcher for the cross-repo architecture refactor.

Modes:
  --check               Verify each repo's audit PR is merged (main has work/<space>/ specs)
  --dry-run             Show the commands that WOULD run, but don't execute
  --only <repo>         Launch one repo (smoke test). Repo names:
                        co | rfq-gateway | ArteLonga | yggdrasil | quilombo-blog
  --all                 Launch all 5 repos
  --background          Pair with --all or --only: run via 'co-auto ... &' (logs to /tmp/<repo>-co-auto.log)
  --help                Show this message

The "single PR per repo" model: each repo gets a long-running branch
'${BRANCH}'. Co-auto runs in --cycle mode, commits each task to this
branch, no per-task PRs. When you're done, open ONE PR per repo manually.
EOF
}

check_repo() {
  local name="$1" path="$2" space="$3"
  echo -n "[$name] "
  # Does main have the spec files we expect?
  local f
  case "$space" in
    co)         f="work/co/CO-215.md" ;;
    rfq)        f="work/rfq/RFQ-17.md" ;;
    artelonga)  f="work/artelonga/AL-51.md" ;;
    yggdrasil)  f="work/yggdrasil/YG-38.md" ;;
    qb)         f="work/qb/QB-2.md" ;;
  esac
  if git -C "$path" show "origin/main:$f" > /dev/null 2>&1; then
    echo "audit PR MERGED — $f present on origin/main ✓"
    return 0
  else
    echo "audit PR NOT MERGED — $f missing on origin/main ✗"
    return 1
  fi
}

launch_repo() {
  local name="$1" path="$2" space="$3" max="$4" dry="$5"
  echo "[$name] launching co-auto cycle mode"
  echo "  path:    $path"
  echo "  space:   $space"
  echo "  branch:  $BRANCH"
  echo "  max:     $max tasks"
  if [[ "$space" == "rfq" ]]; then
    echo "  skip:    RFQ-14, RFQ-15 (Hedix DNS gate)"
  fi
  local cmd="co-auto --workdir $path --space $space --branch $BRANCH --cycle --max-tasks $max --headless"
  if [[ "$dry" == "1" ]]; then
    echo "  DRY-RUN: $cmd"
    return 0
  fi
  echo "  RUN:     $cmd"
  if [[ -n "$background" ]]; then
    nohup $cmd > "/tmp/${name}-co-auto.log" 2>&1 &
    echo "  PID:     $!"
    echo "  LOG:     /tmp/${name}-co-auto.log"
  else
    $cmd
  fi
}

case "$action" in
  help) print_help; exit 0 ;;
  check)
    fail=0
    for entry in "${REPOS[@]}"; do
      IFS=':' read -r name path space _ <<< "$entry"
      check_repo "$name" "$path" "$space" || fail=$((fail+1))
    done
    [[ $fail -eq 0 ]] && echo "All audit PRs merged. Ready for Phase C." || \
      echo "WARNING: $fail audit PR(s) not yet merged. Phase C will fail for those repos."
    exit $fail
    ;;
  dry)
    for entry in "${REPOS[@]}"; do
      IFS=':' read -r name path space max <<< "$entry"
      launch_repo "$name" "$path" "$space" "$max" 1
    done
    ;;
  all)
    for entry in "${REPOS[@]}"; do
      IFS=':' read -r name path space max <<< "$entry"
      launch_repo "$name" "$path" "$space" "$max" 0
    done
    ;;
  *)
    if [[ -n "$only" ]]; then
      for entry in "${REPOS[@]}"; do
        IFS=':' read -r name path space max <<< "$entry"
        if [[ "$name" == "$only" ]]; then
          launch_repo "$name" "$path" "$space" "$max" 0
          exit 0
        fi
      done
      echo "Unknown repo: $only" >&2
      exit 1
    fi
    print_help
    ;;
esac
