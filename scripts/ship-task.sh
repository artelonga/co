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
#   3  auto-resolve reduced branch to origin/main (empty commit dropped by rebase)

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

# CO-440 safety net: if the agent did the work but left it UNCOMMITTED (staged
# or unstaged — Fable sometimes `git add`s without committing), commit it here so
# a stage-but-don't-commit run still ships instead of dying "nothing to ship".
# The work is already isolated in this worktree, so `git add -A` is scoped to it.
if [[ -n "$(git status --porcelain)" ]]; then
  CC="chore(co-auto):"; TITLE="$TASK_ID"
  for space_dir in work/*/; do
    sp="${space_dir}${TASK_ID}.md"
    if [[ -f "$sp" ]]; then
      cc_raw=$(grep -m1 '^conventional_commit:' "$sp" | sed 's/^conventional_commit: *//; s/"//g')
      [[ -n "$cc_raw" ]] && CC="$cc_raw"
      t_raw=$(grep -m1 '^title:' "$sp" | sed 's/^title: *//; s/"//g')
      [[ -n "$t_raw" ]] && TITLE="$t_raw"
      break
    fi
  done
  echo "  ⚠ CO-440 safety net: agent left uncommitted work — committing it ($CC)"
  git add -A
  git commit --no-verify -m "${CC} ${TASK_ID} — ${TITLE} (auto-committed by co-auto)" >/dev/null 2>&1 \
    || echo "  ⚠ auto-commit produced nothing"
fi

# Verify HEAD is ahead of origin/main
git fetch origin main --quiet
AHEAD=$(git rev-list --count origin/main..HEAD)
if [[ "$AHEAD" -eq 0 ]]; then
  echo "ERROR: Nothing to ship — no commits ahead of origin/main." >&2
  exit 1
fi
echo "Commits:  $AHEAD ahead of origin/main"

# === Pre-rebase status sync: set task spec to 'done' on this branch ===
# Root cause of repeated rebase conflicts: each new task's spec file (work/<space>/<TASK>.md)
# may have status=in_progress on the worktree branch while main has status=done from prior
# sync. Fix at source: stamp 'done' on the branch BEFORE rebase so main's state matches.
SPEC_LOCAL=""
for space_dir in work/*/; do
  candidate="${space_dir}${TASK_ID}.md"
  if [[ -f "$candidate" ]]; then
    SPEC_LOCAL="$candidate"
    break
  fi
done
if [[ -n "$SPEC_LOCAL" ]]; then
  current_status=$(grep -m1 '^status:' "$SPEC_LOCAL" | sed 's/^status: *//')
  if [[ "$current_status" != "done" ]]; then
    echo "  patching $SPEC_LOCAL status: $current_status → done"
    # macOS sed compat
    if [[ "$OSTYPE" == "darwin"* ]]; then
      sed -i '' 's/^status:.*$/status: done/' "$SPEC_LOCAL"
    else
      sed -i 's/^status:.*$/status: done/' "$SPEC_LOCAL"
    fi
    git add "$SPEC_LOCAL"
    # Amend the last commit (the task's commit) to bundle status: done
    git commit --amend --no-edit --no-verify > /dev/null 2>&1 || git commit -m "chore: mark $TASK_ID done" --no-verify > /dev/null 2>&1
  fi
fi

# Rebase on origin/main, auto-resolving known-safe conflict patterns:
#   - work/<space>/X.md  (spec status sync)  → ours (this branch's, now 'done')
#   - Cargo.lock         (build state)       → theirs (regenerate)
# CHANGELOG.md and Cargo.toml are never touched by agents (CO-258); if they appear
# in a conflict, treat as unhandled and require manual resolution.
echo "Rebasing on origin/main (auto-resolving metadata conflicts)..."
git rebase origin/main 2>&1 | tail -3 || true
while [[ -n "$(git diff --name-only --diff-filter=U 2>/dev/null)" ]]; do
  echo "  resolving conflicts:"
  while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    if [[ "$f" == work/*/*.md ]]; then
      echo "    $f → ours (spec)"
      git checkout --ours -- "$f"
    elif [[ "$f" == "Cargo.lock" ]]; then
      echo "    $f → theirs (regenerable)"
      git checkout --theirs -- "$f"
    else
      echo "    $f → no auto-resolve; aborting" >&2
      git rebase --abort 2>&1 | head -1 >&2
      echo "ERROR: Unhandled conflict in $f. Resolve manually." >&2
      exit 2
    fi
    git add "$f"
  done < <(git diff --name-only --diff-filter=U)
  # Continue
  if ! git -c core.editor=true rebase --continue 2>&1 | tail -3; then
    # If --continue fails, exit
    echo "ERROR: rebase --continue failed; resolve manually." >&2
    exit 2
  fi
done

# Post-rebase safety check: detect when auto-resolve dropped the task commit entirely.
# Git silently drops empty commits during rebase, leaving HEAD == origin/main.
BRANCH_HEAD=$(git rev-parse HEAD)
MAIN_HEAD=$(git rev-parse origin/main)
if [[ "$BRANCH_HEAD" == "$MAIN_HEAD" ]]; then
    echo "ERROR: After auto-resolve, branch is identical to origin/main." >&2
    echo "       The task's commit was dropped as empty. This is the rebase-empty bug." >&2
    echo "       Manual recovery: reset the branch, cherry-pick the original commit," >&2
    echo "       and resolve conflicts preserving the task's intent." >&2
    echo "       Original commit SHA (from reflog):" >&2
    git reflog --pretty='%h %s' | grep -m1 "commit:" >&2
    exit 3
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
