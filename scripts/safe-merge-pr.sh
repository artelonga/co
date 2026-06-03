#!/usr/bin/env bash
# Safe PR merge: poll until GitHub computes mergeable status before --delete-branch.
# After a successful merge: auto-creates the task archive and prunes the worktree.
#
# Usage: safe-merge-pr.sh <repo> <pr-number>
# Example: safe-merge-pr.sh artelonga/co 99
#
# Motivation: gh pr merge --squash --delete-branch fails with "Pull Request is not
# mergeable" when GitHub returns mergeable=UNKNOWN (transient state-computation lag),
# but the branch deletion side-effect still runs — leaving the PR CLOSED without merge.
# This wrapper polls until the state is determinable, then merges safely.
#
# Post-merge lifecycle (CO-301):
#   1. git pull origin main  — bring the squash-merge commit local
#   2. archive-task.sh       — write docs/task-archive/<TASK-ID>.json
#   3. commit + push archive — git-tracked artifact on main
#   4. prune-worktrees.sh    — remove the just-archived worktree
#
# Exit codes:
#   0  squash-merged (+ archived + pruned) successfully
#   1  PR is conflicting/dirty — user must resolve
#   1  state stayed UNKNOWN for 30s — PR left open, no action taken

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel)"

REPO="${1:-}"
PR="${2:-}"
if [[ -z "$REPO" || -z "$PR" ]]; then
  echo "Usage: $0 <repo> <pr-number>" >&2
  echo "Example: $0 artelonga/co 99" >&2
  exit 1
fi

# Capture branch name before merge (for task ID extraction)
PR_BRANCH=$(gh pr view -R "$REPO" "$PR" --json headRefName --jq '.headRefName' 2>/dev/null || true)

_do_post_merge() {
  local branch="$PR_BRANCH"
  local task_id
  task_id=$(echo "$branch" | grep -oE '[A-Z]+-[0-9]+' | head -1 || true)

  cd "$REPO_ROOT"

  echo "  Pulling main..." >&2
  git pull origin main --ff-only --quiet 2>/dev/null || git pull origin main --quiet

  if [[ -z "$task_id" ]]; then
    echo "  Warning: could not extract task ID from branch '$branch'; skipping archive" >&2
    return
  fi

  echo "  Archiving $task_id..." >&2
  if bash "$SCRIPT_DIR/archive-task.sh" "$task_id"; then
    archive_file="docs/task-archive/${task_id}.json"
    if git diff --name-only HEAD -- "$archive_file" | grep -q . 2>/dev/null \
       || ! git ls-files --error-unmatch "$archive_file" 2>/dev/null; then
      git add "$archive_file"
      git add "docs/task-archive/.gitkeep" 2>/dev/null || true
      git commit -m "chore(archive): $task_id — task archive

Co-Authored-By: Claude <noreply@anthropic.com>"
      git push origin main
      echo "  Archive committed and pushed." >&2
    else
      echo "  Archive unchanged; no commit needed." >&2
    fi
  else
    echo "  Warning: archive-task.sh failed for $task_id; skipping prune" >&2
    return
  fi

  echo "  Pruning worktrees..." >&2
  bash "$SCRIPT_DIR/prune-worktrees.sh" --apply
}

for attempt in 1 2 3 4 5 6; do
  STATE=$(gh pr view -R "$REPO" "$PR" --json mergeable,mergeStateStatus \
    --jq '"\(.mergeable):\(.mergeStateStatus)"')
  case "$STATE" in
    MERGEABLE:CLEAN|MERGEABLE:UNSTABLE)
      echo "  state=$STATE — merging..."
      gh pr merge -R "$REPO" "$PR" --squash --delete-branch
      _do_post_merge
      exit 0
      ;;
    CONFLICTING:*|*:DIRTY)
      echo "ERROR: PR #$PR is conflicting/dirty: $STATE" >&2
      echo "Resolve conflicts on the branch and re-push before merging." >&2
      exit 1
      ;;
    UNKNOWN:*|*:UNKNOWN)
      echo "  attempt $attempt/6: state=$STATE; waiting 5s..."
      sleep 5
      ;;
    *)
      echo "ERROR: unrecognized merge state: $STATE" >&2
      exit 1
      ;;
  esac
done

echo "ERROR: state stayed UNKNOWN for 30s; refusing to merge PR #$PR" >&2
echo "PR is left open. Re-run once GitHub finishes computing the mergeable check." >&2
exit 1
