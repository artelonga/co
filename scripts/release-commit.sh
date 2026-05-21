#!/usr/bin/env bash
# Consolidate CHANGELOG-PENDING notes into a single release commit.
#
# Usage:
#   scripts/release-commit.sh <NEW-VERSION> [<theme>]
#
# Examples:
#   scripts/release-commit.sh 2.18.1 "architecture hygiene"
#   scripts/release-commit.sh 2.19.0
#
# What it does:
#   1. Bumps version in Cargo.toml (workspace) and co-cli/Cargo.toml
#   2. Reads all CHANGELOG-PENDING/*.md (skips .gitkeep)
#   3. Prepends a new ## [<NEW>] — <DATE> — <theme> block to CHANGELOG.md
#   4. Deletes the consumed pending files
#   5. Commits: chore(release): <NEW> — <theme>

set -euo pipefail

NEW="${1:-}"
THEME="${2:-release}"

if [[ -z "$NEW" ]]; then
  echo "Usage: $0 <NEW-VERSION> [<theme>]" >&2
  echo "Example: $0 2.18.1 'architecture hygiene'" >&2
  exit 1
fi

REPO_ROOT="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

PENDING_DIR="CHANGELOG-PENDING"

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

echo "Released $NEW — $THEME"
