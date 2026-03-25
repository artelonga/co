#!/usr/bin/env bash
#
# lib/core.sh — Shared foundations for co-pipeline and co-session
#

[[ -n "${__CO_LIB_CORE_LOADED:-}" ]] && return
__CO_LIB_CORE_LOADED=1

# ── Constants ──

CO_BOARD_API="${CO_BOARD_API:-http://localhost:3000/api}"
CO_PROJECT_KEY="${CO_PROJECT:-UX}"
CO_RUNS_DIR="${CO_RUNS_DIR:-$HOME/.co/runs}"
CO_SCRIPT_DIR="${CO_SCRIPT_DIR:-$(cd "$(dirname "${BASH_SOURCE[1]:-${BASH_SOURCE[0]}}")" && pwd)}"

# ── Logging ──

die()  { echo "error: $*" >&2; exit 1; }
info() { echo "→ $*" >&2; }
warn() { echo "⚠ $*" >&2; }

# ── Utilities ──

# Source a library module idempotently
source_lib() {
    local name="$1"
    local lib_path="$CO_SCRIPT_DIR/lib/${name}.sh"
    [[ -f "$lib_path" ]] || die "Library not found: $lib_path"
    # shellcheck disable=SC1090
    source "$lib_path"
}

# Assert required commands exist
require_cmd() {
    local cmd
    for cmd in "$@"; do
        command -v "$cmd" &>/dev/null || die "Required command not found: $cmd"
    done
}

# Parse task key (e.g., "UX-51" → sets CO_PROJECT_KEY=UX, CO_TASK_ID=51)
parse_task_key() {
    local key="$1"
    if [[ "$key" =~ ^([A-Za-z]+)-([0-9]+)$ ]]; then
        CO_PROJECT_KEY="${BASH_REMATCH[1]}"
        CO_TASK_ID="${BASH_REMATCH[2]}"
    elif [[ "$key" =~ ^[0-9]+$ ]]; then
        CO_TASK_ID="$key"
    else
        die "Invalid task key: $key (expected format: UX-51 or 51)"
    fi
}

# Trap handler for temp files
_CO_CLEANUP_FILES=()

co_register_cleanup() {
    _CO_CLEANUP_FILES+=("$@")
}

co_cleanup() {
    if [[ ${#_CO_CLEANUP_FILES[@]} -gt 0 ]]; then
        local f
        for f in "${_CO_CLEANUP_FILES[@]}"; do
            rm -f "$f" 2>/dev/null || true
        done
    fi
}

trap co_cleanup EXIT
