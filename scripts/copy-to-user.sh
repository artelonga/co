#!/bin/bash
# Copy co auto + game project to another local user
#
# Usage:
#   sudo ./scripts/copy-to-user.sh artelonga
#
# Requires: run as root (sudo) or the source user

set -e

TARGET_USER="${1:?Usage: $0 <username>}"
TARGET_HOME="/Users/$TARGET_USER"

if [ ! -d "$TARGET_HOME" ]; then
    echo "Error: User home not found: $TARGET_HOME"
    exit 1
fi

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

echo -e "${BOLD}${CYAN}Copying co auto + game to user: $TARGET_USER${NC}"
echo ""

# ── Paths ────────────────────────────────────────────
SRC_CO="/Users/yuri/projects/co"
SRC_GAME="/Users/yuri/projects/game"
SRC_UNIVERSE="/Users/yuri/projects/universe"
SRC_CO_BIN="/Users/yuri/.cargo/bin/co"

DST_PROJECTS="$TARGET_HOME/projects"
DST_CO="$DST_PROJECTS/co"
DST_GAME="$DST_PROJECTS/game"
DST_UNIVERSE="$DST_PROJECTS/universe"

# ── Create directories ───────────────────────────────
echo "Creating directories..."
mkdir -p "$DST_PROJECTS"
mkdir -p "$TARGET_HOME/.co/runs"
mkdir -p "$TARGET_HOME/.local/bin"

# ── Copy co binary ───────────────────────────────────
echo -e "Copying co binary..."
cp "$SRC_CO_BIN" "$TARGET_HOME/.local/bin/co"
echo -e "${GREEN}✓${NC} co binary → $TARGET_HOME/.local/bin/co"

# ── Copy co workspace (minimal: data + CLAUDE.md + .co marker) ──
echo "Copying co workspace..."
mkdir -p "$DST_CO/data"
mkdir -p "$DST_CO/.co"

# Task data (GP project)
cp -r "$SRC_CO/data/gp" "$DST_CO/data/gp"
echo -e "${GREEN}✓${NC} GP tasks ($(ls $SRC_CO/data/gp/*.md | wc -l | tr -d ' ') files)"

# CLAUDE.md (conventions for context injection)
cp "$SRC_CO/CLAUDE.md" "$DST_CO/CLAUDE.md" 2>/dev/null || true
echo -e "${GREEN}✓${NC} CLAUDE.md"

# .co marker
touch "$DST_CO/.co/marker"
echo -e "${GREEN}✓${NC} .co workspace marker"

# Scripts
mkdir -p "$DST_CO/scripts"
cp "$SRC_CO/scripts/setup.sh" "$DST_CO/scripts/" 2>/dev/null || true
cp "$SRC_CO/scripts/copy-to-user.sh" "$DST_CO/scripts/" 2>/dev/null || true

# ── Copy game repo (exclude target/ and .godot/ cache) ──
echo "Copying game repo..."
rsync -a \
    --exclude='target/' \
    --exclude='.godot/' \
    --exclude='releases/' \
    "$SRC_GAME/" "$DST_GAME/"
echo -e "${GREEN}✓${NC} game repo"

# ── Copy universe crate (if exists) ──────────────────
if [ -d "$SRC_UNIVERSE" ]; then
    echo "Copying universe crate..."
    rsync -a \
        --exclude='target/' \
        "$SRC_UNIVERSE/" "$DST_UNIVERSE/"
    echo -e "${GREEN}✓${NC} universe crate"
fi

# ── Fix ownership ────────────────────────────────────
echo "Setting ownership..."
chown -R "$TARGET_USER:staff" "$DST_PROJECTS"
chown -R "$TARGET_USER:staff" "$TARGET_HOME/.co"
chown -R "$TARGET_USER:staff" "$TARGET_HOME/.local/bin/co"
echo -e "${GREEN}✓${NC} Ownership set to $TARGET_USER"

# ── Shell profile ────────────────────────────────────
SHELL_RC="$TARGET_HOME/.zshrc"
if [ ! -f "$SHELL_RC" ]; then
    SHELL_RC="$TARGET_HOME/.bashrc"
fi

# Add to PATH and set CO_WORKSPACE
if [ -f "$SHELL_RC" ]; then
    if ! grep -q "CO_WORKSPACE" "$SHELL_RC" 2>/dev/null; then
        cat >> "$SHELL_RC" << 'PROFILE'

# CO auto — task execution pipeline
export CO_WORKSPACE="$HOME/projects/co"
export GAME_REPO="$HOME/projects/game"
export PATH="$HOME/.local/bin:$PATH"
PROFILE
        chown "$TARGET_USER:staff" "$SHELL_RC"
        echo -e "${GREEN}✓${NC} Updated $SHELL_RC with CO_WORKSPACE + PATH"
    else
        echo -e "${GREEN}✓${NC} Shell profile already configured"
    fi
else
    # Create .zshrc if neither exists
    cat > "$TARGET_HOME/.zshrc" << 'PROFILE'
# CO auto — task execution pipeline
export CO_WORKSPACE="$HOME/projects/co"
export GAME_REPO="$HOME/projects/game"
export PATH="$HOME/.local/bin:$PATH"
PROFILE
    chown "$TARGET_USER:staff" "$TARGET_HOME/.zshrc"
    echo -e "${GREEN}✓${NC} Created $TARGET_HOME/.zshrc"
fi

# ── Summary ──────────────────────────────────────────
echo ""
echo -e "${BOLD}${CYAN}Done. What was copied:${NC}"
echo ""
echo "  Repos:"
echo "    $DST_CO         (co workspace — tasks + CLAUDE.md)"
echo "    $DST_GAME       (game — Rust server + Godot + SvelteKit)"
[ -d "$DST_UNIVERSE" ] && echo "    $DST_UNIVERSE    (universe crate)"
echo ""
echo "  Binary:"
echo "    $TARGET_HOME/.local/bin/co"
echo ""
echo "  Config:"
echo "    CO_WORKSPACE=$DST_CO"
echo "    GAME_REPO=$DST_GAME"
echo "    ~/.co/runs/     (auto run tracking)"
echo ""
echo -e "${BOLD}Usage (as $TARGET_USER):${NC}"
echo ""
echo "  # Dry run — see next task"
echo "  co auto --space gp --dry-run"
echo ""
echo "  # Execute next task on game repo"
echo "  co auto --space gp --workdir \$GAME_REPO"
echo ""
echo "  # Cycle through tasks"
echo "  co auto --space gp --cycle --max-tasks 3 --workdir \$GAME_REPO"
echo ""
echo "  # With explicit paths (no env vars needed)"
echo "  co auto --space gp --workspace $DST_CO --workdir $DST_GAME"
echo ""
echo -e "${BOLD}Prerequisites for $TARGET_USER:${NC}"
echo "  - Claude Code CLI installed and authenticated (claude --version)"
echo "  - Rust toolchain if building from source (cargo build)"
