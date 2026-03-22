#!/bin/bash
# CO Auto — Portable Setup Script
# Installs the co binary and configures for any user.
#
# Usage:
#   curl -sSL <url>/setup.sh | bash
#   # or
#   git clone <repo> co && cd co && ./scripts/setup.sh
#
# Requirements:
#   - Rust toolchain (rustup)
#   - Claude Code CLI (claude) installed and authenticated
#   - Git

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

echo -e "${BOLD}${CYAN}CO Auto — Setup${NC}"
echo ""

# ── Check prerequisites ──────────────────────────────

check_cmd() {
    if ! command -v "$1" &> /dev/null; then
        echo -e "${RED}✗ $1 not found. Please install it first.${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓${NC} $1"
}

echo "Checking prerequisites..."
check_cmd rustc
check_cmd cargo
check_cmd git
check_cmd claude

# ── Determine paths ──────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo ""
echo -e "CO workspace: ${CYAN}${CO_ROOT}${NC}"

# ── Build and install ────────────────────────────────

echo ""
echo "Building co binary (release)..."
cd "$CO_ROOT"
cargo install --path co-cli --quiet

CO_BIN="$(which co)"
echo -e "${GREEN}✓${NC} Installed: ${CO_BIN}"

# ── Configure workspace ──────────────────────────────

# Create .co marker if missing
if [ ! -d "$CO_ROOT/.co" ]; then
    mkdir -p "$CO_ROOT/.co"
    echo -e "${GREEN}✓${NC} Created .co workspace marker"
fi

# Create ~/.co directory for run tracking
mkdir -p "$HOME/.co/runs"
echo -e "${GREEN}✓${NC} Created ~/.co/runs for auto tracking"

# ── Shell environment ────────────────────────────────

echo ""
echo -e "${BOLD}Shell configuration:${NC}"
echo ""
echo "Add to your shell profile (~/.zshrc or ~/.bashrc):"
echo ""
echo -e "  ${YELLOW}# CO workspace (enables 'co auto' from any directory)${NC}"
echo -e "  ${YELLOW}export CO_WORKSPACE=\"${CO_ROOT}\"${NC}"
echo ""

# Auto-detect shell and offer to append
SHELL_RC=""
if [ -f "$HOME/.zshrc" ]; then
    SHELL_RC="$HOME/.zshrc"
elif [ -f "$HOME/.bashrc" ]; then
    SHELL_RC="$HOME/.bashrc"
fi

if [ -n "$SHELL_RC" ]; then
    if grep -q "CO_WORKSPACE" "$SHELL_RC" 2>/dev/null; then
        echo -e "${GREEN}✓${NC} CO_WORKSPACE already in $SHELL_RC"
    else
        read -p "Add CO_WORKSPACE to $SHELL_RC? [y/N] " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            echo "" >> "$SHELL_RC"
            echo "# CO workspace (enables 'co auto' from any directory)" >> "$SHELL_RC"
            echo "export CO_WORKSPACE=\"${CO_ROOT}\"" >> "$SHELL_RC"
            echo -e "${GREEN}✓${NC} Added to $SHELL_RC (source it or open a new terminal)"
        fi
    fi
fi

# ── Verify ───────────────────────────────────────────

echo ""
echo -e "${BOLD}Verification:${NC}"
CO_WORKSPACE="$CO_ROOT" co auto --space gp --dry-run 2>/dev/null && echo -e "${GREEN}✓${NC} co auto works" || echo -e "${RED}✗${NC} verification failed"

# ── Print usage ──────────────────────────────────────

echo ""
echo -e "${BOLD}${CYAN}Usage:${NC}"
echo ""
echo "  # From any directory (with CO_WORKSPACE set):"
echo "  co auto --space gp --dry-run"
echo ""
echo "  # Explicit paths (no env var needed):"
echo "  co auto --space gp --workspace $CO_ROOT --workdir /path/to/target/repo --dry-run"
echo ""
echo "  # Execute next task on a target repo:"
echo "  co auto --space gp --workdir /path/to/game --model opus"
echo ""
echo "  # Cycle through multiple tasks:"
echo "  co auto --space gp --cycle --max-tasks 3"
echo ""
echo "  # With Claude Code agent teams (parallel):"
echo "  co auto --space gp --teams"
echo ""
echo -e "${BOLD}Repos referenced in GP tasks:${NC}"
echo "  game    — /path/to/game     (base app: Rust server + Godot + SvelteKit)"
echo "  co      — $CO_ROOT          (tasks/notes: CLI + web)"
echo "  universe — /path/to/universe (universe crate: multi-universe platform)"
echo ""
echo -e "${GREEN}Setup complete.${NC}"
