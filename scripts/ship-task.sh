#!/usr/bin/env bash
# Ship a completed co-auto task: rebase its worktree on origin/main, push, open PR.
#
# Usage:
#   scripts/ship-task.sh <TASK-ID> [--draft]
#
# Examples:
#   scripts/ship-task.sh CO-215
#   scripts/ship-task.sh RFQ-17 --draft
#
# Finds the worktree across all known repos, reads conventional-commit prefix + title
# from the task spec, pushes the branch, opens the PR via `gh`.
#
# Exit codes:
#   0  PR opened
#   1  invalid args / worktree not found / nothing to ship
#   2  rebase conflict (user must resolve manually)

set -u

TASK_ID="${1:-}"
if [[ -z "$TASK_ID" ]]; then
  echo "Usage: $0 <TASK-ID> [--draft]" >&2
  echo "Example: $0 CO-215" >&2
  exit 1
fi

shift
DRAFT_FLAG=""
[[ "${1:-}" == "--draft" ]] && DRAFT_FLAG="--draft"

# Repos to search. Order matters — refactor worktrees first since that's where co-auto runs.
REPOS=(
  "/Users/artelonga/projects/co"
  "/Users/artelonga/projects/rfq-gateway-refactor"
  "/Users/artelonga/projects/rfq-gateway"
  "/Users/artelonga/projects/ArteLonga-refactor"
  "/Users/artelonga/projects/ArteLonga"
  "/Users/artelonga/projects/yggdrasil-refactor"
  "/Users/artelonga/projects/yggdrasil"
  "/Users/artelonga/projects/quilomboaraucaria-refactor"
  "/Users/artelonga/projects/quilomboaraucaria"
  "/Users/artelonga/projects/quilombo-blog-refactor"
  "/Users/artelonga/projects/quilombo-blog"
)

# Find the worktree
WT=""
for r in "${REPOS[@]}"; do
  candidate="$r/.worktrees/$TASK_ID"
  if [[ -d "$candidate" ]]; then
    WT="$candidate"
    break
  fi
done

if [[ -z "$WT" ]]; then
  echo "ERROR: No worktree found for $TASK_ID under ${#REPOS[@]} repos' .worktrees/" >&2
  exit 1
fi

echo "Worktree: $WT"
cd "$WT" || exit 1

BRANCH=$(git branch --show-current)
echo "Branch:   $BRANCH"

# Resolve GH repo from origin URL
ORIGIN=$(git remote get-url origin)
GH_REPO=$(echo "$ORIGIN" | sed -E 's|^git@github.com:([^/]+/[^/]+)\.git$|\1|; s|^https://github.com/([^/]+/[^/]+)\.git$|\1|; s|^https://github.com/([^/]+/[^/]+)$|\1|')
echo "GH repo:  $GH_REPO"

# Verify HEAD is ahead of origin/main
git fetch origin main --quiet
AHEAD=$(git rev-list --count origin/main..HEAD)
if [[ "$AHEAD" -eq 0 ]]; then
  echo "ERROR: Nothing to ship — no commits ahead of origin/main." >&2
  exit 1
fi
echo "Commits:  $AHEAD ahead of origin/main"

# Rebase on origin/main
echo "Rebasing on origin/main..."
if ! git rebase origin/main; then
  echo
  echo "ERROR: Rebase failed (conflicts). Steps to resolve:" >&2
  echo "  cd $WT" >&2
  echo "  # resolve conflicts" >&2
  echo "  git add <files>" >&2
  echo "  git rebase --continue" >&2
  echo "  $0 $TASK_ID  # rerun" >&2
  exit 2
fi

# Read task title + conventional-commit prefix from spec file
TASK_TITLE="$TASK_ID"
PREFIX="chore"
SPEC=""
for space in co rfq artelonga yggdrasil qb; do
  for r in "${REPOS[@]}"; do
    f="$r/work/$space/$TASK_ID.md"
    if [[ -f "$f" ]]; then
      SPEC="$f"
      break 2
    fi
  done
done
if [[ -n "$SPEC" ]]; then
  TITLE_LINE=$(grep -m1 '^title:' "$SPEC" || true)
  if [[ -n "$TITLE_LINE" ]]; then
    TASK_TITLE=$(echo "$TITLE_LINE" | sed -E 's/^title: *"?//; s/"$//')
    TASK_TITLE="$TASK_ID — $TASK_TITLE"
  fi
  CC=$(grep -m1 '^conventional_commit:' "$SPEC" | sed 's/^conventional_commit: *"//; s/"$//' || true)
  [[ -n "$CC" ]] && PREFIX="$CC"
fi

# Push branch
echo "Pushing $BRANCH to origin..."
git push -u origin "$BRANCH" 2>&1 | tail -2

# Open PR
PR_TITLE="$PREFIX $TASK_TITLE"
PR_BODY="Shipped via \`scripts/ship-task.sh\` after co-auto completion.

See \`work/<space>/$TASK_ID.md\` for the full spec, acceptance criteria, and blast-radius notes."

PR_URL=$(gh pr create -R "$GH_REPO" --base main --head "$BRANCH" --title "$PR_TITLE" --body "$PR_BODY" $DRAFT_FLAG 2>&1)
echo "PR: $PR_URL"
