#!/usr/bin/env bash
# prune-worktrees.sh [--apply]
# Audits all git worktrees and safely removes ones whose PR is merged + archived.
#
# Usage:
#   scripts/prune-worktrees.sh          # dry-run: shows what would be removed
#   scripts/prune-worktrees.sh --apply  # actually remove safe worktrees
#
# Safety rules — a worktree is prunable only when ALL are true:
#   1. Its branch appears in the merged PR list (GitHub)
#   2. docs/task-archive/<TASK-ID>.json exists (archive was created first)
#   3. The worktree has no uncommitted changes (git status --porcelain is empty)
#   4. The worktree has no commits ahead of main (unpushed work)
#
# Locked worktrees (agent worktrees) are always skipped.
#
# Exit codes:
#   0  audit/prune complete

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

APPLY=false
[[ "${1:-}" == "--apply" ]] && APPLY=true

REPO="artelonga/co"

# ── Fetch merged PR branch names (one-time) ───────────────────────────────────

echo "→ Fetching merged PR list from $REPO..." >&2
MERGED_BRANCHES=$(gh pr list -R "$REPO" --state merged --limit 300 \
  --json headRefName --jq '.[].headRefName')

is_merged() { echo "$MERGED_BRANCHES" | grep -qxF "${1:-}"; }

task_id_from_branch() {
  echo "${1:-}" | grep -oE '[A-Z]+-[0-9]+' | head -1 || true
}

archive_exists() {
  local tid="${1:-}"
  [[ -n "$tid" && -f "docs/task-archive/${tid}.json" ]]
}

# ── Walk worktrees ────────────────────────────────────────────────────────────

prune_count=0
skip_count=0
total=0

echo "" >&2

while IFS= read -r line; do
  # git worktree list format: "<path>  <sha>  [<branch>]" or "(bare)" or "locked"
  # Locked worktrees have "locked" at the end
  if [[ "$line" == *" locked"* ]]; then
    wt_path=$(echo "$line" | awk '{print $1}')
    echo "  skip (locked agent worktree): $wt_path" >&2
    skip_count=$((skip_count + 1))
    continue
  fi

  if [[ "$line" =~ ^([^[:space:]]+)[[:space:]]+[0-9a-f]+[[:space:]]+\[([^\]]+)\] ]]; then
    wt_path="${BASH_REMATCH[1]}"
    wt_branch="${BASH_REMATCH[2]}"
  else
    continue
  fi

  # Skip main worktree
  [[ "$wt_path" == "$REPO_ROOT" ]] && continue

  total=$((total + 1))
  task_id=$(task_id_from_branch "$wt_branch")

  echo "  worktree: $(basename "$wt_path")  branch=$wt_branch" >&2

  # Rule 1: branch must be merged
  if ! is_merged "$wt_branch"; then
    echo "    ↳ keep: branch not yet merged" >&2
    skip_count=$((skip_count + 1))
    continue
  fi

  # Rule 2: archive must exist
  if ! archive_exists "$task_id"; then
    if [[ -n "$task_id" ]]; then
      echo "    ↳ keep: no archive for $task_id (run: scripts/archive-task.sh $task_id)" >&2
    else
      echo "    ↳ keep: cannot extract task ID from branch, skipping" >&2
    fi
    skip_count=$((skip_count + 1))
    continue
  fi

  # Rule 3: no uncommitted changes
  if [[ -d "$wt_path" ]]; then
    dirty=$(git -C "$wt_path" status --porcelain 2>/dev/null || true)
    if [[ -n "$dirty" ]]; then
      echo "    ↳ keep: has uncommitted changes" >&2
      skip_count=$((skip_count + 1))
      continue
    fi

    # Rule 4: no unpushed commits ahead of main
    unpushed=$(git -C "$wt_path" log "origin/main..HEAD" --oneline 2>/dev/null | head -1 || true)
    if [[ -n "$unpushed" ]]; then
      echo "    ↳ keep: has unpushed commits" >&2
      skip_count=$((skip_count + 1))
      continue
    fi
  fi

  echo "    ↳ safe to prune ✓" >&2
  prune_count=$((prune_count + 1))

  if $APPLY; then
    echo "    Removing: $wt_path" >&2
    git worktree remove --force "$wt_path" 2>/dev/null \
      || rm -rf "$wt_path"
  fi

done < <(git worktree list)

if $APPLY; then
  git worktree prune 2>/dev/null || true
fi

echo "" >&2
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" >&2
echo "  Scanned:        $total worktrees" >&2
echo "  Safe to prune:  $prune_count" >&2
echo "  Kept/skipped:   $skip_count" >&2
if ! $APPLY && [[ $prune_count -gt 0 ]]; then
  echo "" >&2
  echo "  Dry run — pass --apply to remove worktrees" >&2
fi
