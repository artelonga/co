#!/usr/bin/env bash
# Safe PR merge: poll until GitHub computes mergeable status before --delete-branch.
#
# Usage: safe-merge-pr.sh <repo> <pr-number>
# Example: safe-merge-pr.sh artelonga/co 99
#
# Motivation: gh pr merge --squash --delete-branch fails with "Pull Request is not
# mergeable" when GitHub returns mergeable=UNKNOWN (transient state-computation lag),
# but the branch deletion side-effect still runs — leaving the PR CLOSED without merge.
# This wrapper polls until the state is determinable, then merges safely.
#
# Exit codes:
#   0  squash-merged successfully
#   1  PR is conflicting/dirty — user must resolve
#   1  state stayed UNKNOWN for 30s — PR left open, no action taken

set -euo pipefail

REPO="${1:-}"
PR="${2:-}"
if [[ -z "$REPO" || -z "$PR" ]]; then
  echo "Usage: $0 <repo> <pr-number>" >&2
  echo "Example: $0 artelonga/co 99" >&2
  exit 1
fi

for attempt in 1 2 3 4 5 6; do
  STATE=$(gh pr view -R "$REPO" "$PR" --json mergeable,mergeStateStatus \
    --jq '"\(.mergeable):\(.mergeStateStatus)"')
  case "$STATE" in
    MERGEABLE:CLEAN|MERGEABLE:UNSTABLE)
      echo "  state=$STATE — merging..."
      gh pr merge -R "$REPO" "$PR" --squash --delete-branch
      exit $?
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
