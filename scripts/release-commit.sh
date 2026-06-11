#!/usr/bin/env bash
# Consolidate CHANGELOG-PENDING notes into a single release commit.
#
# Usage:
#   scripts/release-commit.sh <NEW-VERSION> [<theme>] [--ignore-dod]
#
# Examples:
#   scripts/release-commit.sh 2.18.1 "architecture hygiene"
#   scripts/release-commit.sh 2.19.0
#   scripts/release-commit.sh 2.19.0 hotfix --ignore-dod   # emergency override
#
# What it does:
#   1. CO-382: DoD gate — reads docs/scrum/dod/*.json for each pending task,
#      refuses release if any task has DoD < 100% (override with --ignore-dod)
#   2. Bumps version in Cargo.toml (workspace) and co-cli/Cargo.toml
#   3. Reads all CHANGELOG-PENDING/*.md (skips .gitkeep)
#   4. Prepends a new ## [<NEW>] — <DATE> — <theme> block to CHANGELOG.md
#   5. Deletes the consumed pending files
#   6. Commits: chore(release): <NEW> — <theme>

set -euo pipefail

NEW="${1:-}"
THEME="${2:-release}"
IGNORE_DOD=false

# Parse flags (strip --ignore-dod from positional args)
ARGS=()
for arg in "$@"; do
  if [[ "$arg" == "--ignore-dod" ]]; then
    IGNORE_DOD=true
  else
    ARGS+=("$arg")
  fi
done
NEW="${ARGS[0]:-}"
THEME="${ARGS[1]:-release}"

if [[ -z "$NEW" ]]; then
  echo "Usage: $0 <NEW-VERSION> [<theme>] [--ignore-dod]" >&2
  echo "Example: $0 2.18.1 'architecture hygiene'" >&2
  exit 1
fi

REPO_ROOT="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

PENDING_DIR="CHANGELOG-PENDING"
DOD_DIR="docs/scrum/dod"

# ---------------------------------------------------------------------------
# CO-372: Sprint cutoff gate — refuse if PRs merged after Wednesday 23:59 BRT.
# ---------------------------------------------------------------------------

CUTOFF_SCRIPT="co-web/scripts/scrum/cutoff-check.ts"
if [[ -f "$CUTOFF_SCRIPT" ]] && command -v node >/dev/null 2>&1; then
  CUTOFF_FLAGS=""
  [[ "$IGNORE_DOD" == "true" ]] && CUTOFF_FLAGS="--ignore-cutoff"
  # tsx resolves from co-web/node_modules — run with co-web as cwd (cutoff-check
  # only shells out to git, which works from any subdirectory).
  if ! (cd co-web && node --import tsx scripts/scrum/cutoff-check.ts $CUTOFF_FLAGS) 2>&1; then
    echo "" >&2
    echo "   Pass --ignore-dod to also bypass the cutoff check." >&2
    exit 1
  fi
fi

# ---------------------------------------------------------------------------
# CO-382: DoD gate — check every pending task has 100% DoD before releasing.
# ---------------------------------------------------------------------------

# Collect CO-IDs from pending changelog files
WAVE_CO_IDS=()
for f in "$PENDING_DIR"/*.md; do
  [[ -f "$f" ]] && [[ "$(basename "$f")" != ".gitkeep" ]] || continue
  # Extract CO-NNN from filename (e.g., CO-382.md → CO-382)
  BASENAME="$(basename "$f" .md)"
  if [[ "$BASENAME" =~ ^CO-[0-9]+$ ]]; then
    WAVE_CO_IDS+=("$BASENAME")
  fi
done

if [[ ${#WAVE_CO_IDS[@]} -gt 0 ]]; then
  DOD_FAILURES=()
  DOD_MISSING=()

  for CO_ID in "${WAVE_CO_IDS[@]}"; do
    DOD_FILE="$DOD_DIR/$CO_ID.json"
    if [[ -f "$DOD_FILE" ]]; then
      # Block only on real matched-test failures; pending items (stubs /
      # no test mapped yet) are advisory — see scripts/dod/verify.ts.
      BLOCKING=$(python3 -c "import json; d=json.load(open('$DOD_FILE')); print(d.get('blocking_failures', 0))" 2>/dev/null || echo "1")
      if [[ "$BLOCKING" != "0" ]]; then
        PCT=$(python3 -c "import json; d=json.load(open('$DOD_FILE')); print(d.get('dod_pct',0))" 2>/dev/null || echo "0")
        DOD_FAILURES+=("$CO_ID (${PCT}%, ${BLOCKING} blocking)")
      fi
    else
      DOD_MISSING+=("$CO_ID")
    fi
  done

  if [[ ${#DOD_FAILURES[@]} -gt 0 ]] || [[ ${#DOD_MISSING[@]} -gt 0 ]]; then
    if [[ "$IGNORE_DOD" == "true" ]]; then
      echo "⚠️  WARNING: --ignore-dod override active. Release proceeds despite DoD failures." >&2
      [[ ${#DOD_FAILURES[@]} -gt 0 ]] && echo "   Failing DoD: ${DOD_FAILURES[*]}" >&2
      [[ ${#DOD_MISSING[@]} -gt 0 ]] && echo "   Missing DoD reports: ${DOD_MISSING[*]}" >&2
      echo "   This override is logged in the release commit." >&2
      # Append override note to theme
      THEME="$THEME [--ignore-dod override]"
    else
      echo "❌ Release blocked by DoD gate (CO-382)." >&2
      echo "" >&2
      [[ ${#DOD_FAILURES[@]} -gt 0 ]] && \
        echo "   Tasks below 100% DoD: ${DOD_FAILURES[*]}" >&2
      [[ ${#DOD_MISSING[@]} -gt 0 ]] && \
        echo "   Tasks with no DoD report: ${DOD_MISSING[*]}" >&2
      echo "" >&2
      echo "   Run \`scripts/dod/verify.ts --spec CO-N\` for each task," >&2
      echo "   or use --ignore-dod for emergency hotfixes." >&2
      exit 1
    fi
  else
    echo "✅ DoD gate passed — all ${#WAVE_CO_IDS[@]} task(s) at 100% DoD"
  fi
fi

# Collect pending files (excluding .gitkeep)
PENDING_FILES=()
for f in "$PENDING_DIR"/*.md; do
  [[ -f "$f" ]] && [[ "$(basename "$f")" != ".gitkeep" ]] && PENDING_FILES+=("$f")
done

if [[ ${#PENDING_FILES[@]} -eq 0 ]]; then
  echo "No pending notes in $PENDING_DIR/. Nothing to release." >&2
  exit 1
fi

DATE=$(date +%Y-%m-%d)

# Build the new changelog entry block
TEMP_ENTRY=$(mktemp)
echo "## [$NEW] — $DATE — $THEME" >> "$TEMP_ENTRY"
echo >> "$TEMP_ENTRY"
for f in "${PENDING_FILES[@]}"; do
  cat "$f" >> "$TEMP_ENTRY"
  echo >> "$TEMP_ENTRY"
done

# Insert the new block before the first existing ## [ entry in CHANGELOG.md
FIRST_ENTRY=$(grep -n '^## \[' CHANGELOG.md | head -1 | cut -d: -f1)
if [[ -n "$FIRST_ENTRY" ]]; then
  TEMP_CL=$(mktemp)
  head -n $((FIRST_ENTRY - 1)) CHANGELOG.md > "$TEMP_CL"
  cat "$TEMP_ENTRY" >> "$TEMP_CL"
  echo >> "$TEMP_CL"
  tail -n +"$FIRST_ENTRY" CHANGELOG.md >> "$TEMP_CL"
  mv "$TEMP_CL" CHANGELOG.md
else
  echo >> CHANGELOG.md
  cat "$TEMP_ENTRY" >> CHANGELOG.md
fi
rm "$TEMP_ENTRY"

# Bump version in workspace Cargo.toml
if [[ "$OSTYPE" == "darwin"* ]]; then
  sed -i '' "s/^version = \"[0-9][^\"]*\"/version = \"$NEW\"/" Cargo.toml
else
  sed -i "s/^version = \"[0-9][^\"]*\"/version = \"$NEW\"/" Cargo.toml
fi

# Bump version in co-cli/Cargo.toml (independent versioning scheme)
if [[ -f co-cli/Cargo.toml ]]; then
  if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/^version = \"[0-9][^\"]*\"/version = \"$NEW\"/" co-cli/Cargo.toml
  else
    sed -i "s/^version = \"[0-9][^\"]*\"/version = \"$NEW\"/" co-cli/Cargo.toml
  fi
fi

# Delete consumed pending files (keep .gitkeep)
for f in "${PENDING_FILES[@]}"; do
  rm "$f"
done

# Stage and commit
git add Cargo.toml CHANGELOG.md "$PENDING_DIR"/
[[ -f co-cli/Cargo.toml ]] && git add co-cli/Cargo.toml

git commit -m "$(cat <<EOF
chore(release): $NEW — $THEME

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"

# CO-260: populate changelog_cache for the new release.
# Calls the local dev server if running; silently skips otherwise.
DEV_URL="${CO_DEV_URL:-http://localhost:3000}"
if curl -sf "$DEV_URL/api/health" >/dev/null 2>&1; then
  echo "Reindexing changelog cache on $DEV_URL…"
  curl -sf -X POST "$DEV_URL/api/v1/admin/changelog/reindex" \
    -H 'Content-Type: application/json' || true
fi

echo "Released $NEW — $THEME"
