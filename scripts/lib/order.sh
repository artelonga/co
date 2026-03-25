#!/usr/bin/env bash
#
# lib/order.sh — Phase ordering (data-driven from pipeline.conf)
# Compatible with bash 3.2+ (no associative arrays)
#

[[ -n "${__CO_LIB_ORDER_LOADED:-}" ]] && return
__CO_LIB_ORDER_LOADED=1

source_lib core
source_lib board

# ── State ──
# Parallel arrays to simulate key-value mapping (bash 3.2 compatible)

_ORDER_PHASE_NAMES=()
_ORDER_PHASE_TASK_LISTS=()   # CSV strings, one per phase
_ORDER_ALL_TASKS=()

# ── Loading ──

# Load phase definitions from config file
# Format: PHASE_NAME=id1,id2,id3,...
order_load() {
    local config_file="${1:-$CO_SCRIPT_DIR/pipeline.conf}"
    [[ -f "$config_file" ]] || die "Pipeline config not found: $config_file"

    _ORDER_PHASE_NAMES=()
    _ORDER_PHASE_TASK_LISTS=()
    _ORDER_ALL_TASKS=()

    local line phase_name task_ids
    while IFS= read -r line; do
        # Skip comments and empty lines
        [[ -z "$line" || "$line" =~ ^[[:space:]]*# ]] && continue

        if [[ "$line" =~ ^([A-Za-z0-9_]+)=(.+)$ ]]; then
            phase_name="${BASH_REMATCH[1]}"
            task_ids="${BASH_REMATCH[2]}"

            _ORDER_PHASE_NAMES+=("$phase_name")
            _ORDER_PHASE_TASK_LISTS+=("$task_ids")

            # Append to all tasks
            IFS=',' read -ra ids <<< "$task_ids"
            _ORDER_ALL_TASKS+=("${ids[@]}")
        fi
    done < "$config_file"
}

# ── Internal ──

# Get task list for a phase by index
_order_get_tasks_by_index() {
    local idx="$1"
    echo "${_ORDER_PHASE_TASK_LISTS[$idx]}"
}

# Find phase index by name, echo index or return 1
_order_find_phase() {
    local name="$1" i
    for i in "${!_ORDER_PHASE_NAMES[@]}"; do
        if [[ "${_ORDER_PHASE_NAMES[$i]}" == "$name" ]]; then
            echo "$i"
            return 0
        fi
    done
    return 1
}

# ── Queries ──

# List phase names
order_phases() {
    printf '%s\n' "${_ORDER_PHASE_NAMES[@]}"
}

# Task IDs for a given phase
order_tasks() {
    local phase="$1"
    local idx
    idx=$(_order_find_phase "$phase") || return
    local tasks
    tasks=$(_order_get_tasks_by_index "$idx")
    IFS=',' read -ra ids <<< "$tasks"
    printf '%s\n' "${ids[@]}"
}

# All tasks in execution order
order_all_tasks() {
    printf '%s\n' "${_ORDER_ALL_TASKS[@]}"
}

# Which phase does a task belong to?
order_phase_for_task() {
    local task_id="$1"
    local i tasks
    for i in "${!_ORDER_PHASE_NAMES[@]}"; do
        tasks="${_ORDER_PHASE_TASK_LISTS[$i]}"
        IFS=',' read -ra ids <<< "$tasks"
        local id
        for id in "${ids[@]}"; do
            [[ "$id" == "$task_id" ]] && echo "${_ORDER_PHASE_NAMES[$i]}" && return
        done
    done
    echo "?"
}

# Next todo task in execution order
order_next_task() {
    local id status
    for id in "${_ORDER_ALL_TASKS[@]}"; do
        status=$(task_status "$CO_PROJECT_KEY" "$id" 2>/dev/null || echo "unknown")
        if [[ "$status" == "todo" ]]; then
            echo "$id"
            return
        fi
    done
    echo ""
}

# ── Display ──

# Print execution order table
order_show() {
    echo ""
    echo "  Execution Order (subtasks only — epics auto-complete)"
    echo ""

    local seq=1 i tasks
    for i in "${!_ORDER_PHASE_NAMES[@]}"; do
        echo "  ${_ORDER_PHASE_NAMES[$i]}"
        tasks="${_ORDER_PHASE_TASK_LISTS[$i]}"
        IFS=',' read -ra ids <<< "$tasks"
        local id title
        for id in "${ids[@]}"; do
            title=$(task_title "$CO_PROJECT_KEY" "$id" 2>/dev/null || echo "?")
            printf "    %2d. %s-%-3d %s\n" "$seq" "$CO_PROJECT_KEY" "$id" "$title"
            seq=$((seq+1))
        done
        echo ""
    done
    echo "  Total: $((seq-1)) tasks"
    echo ""
}

# Print status dashboard with icons
order_show_status() {
    echo ""
    echo "╔══════════════════════════════════════════════════════════╗"
    echo "║         Pipeline Status — Project $CO_PROJECT_KEY              ║"
    echo "╚══════════════════════════════════════════════════════════╝"
    echo ""

    local done_count=0 in_progress=0 in_review=0 todo_count=0

    local i tasks
    for i in "${!_ORDER_PHASE_NAMES[@]}"; do
        echo "  ── ${_ORDER_PHASE_NAMES[$i]} ──"
        tasks="${_ORDER_PHASE_TASK_LISTS[$i]}"
        IFS=',' read -ra ids <<< "$tasks"
        local id
        for id in "${ids[@]}"; do
            local status title icon
            status=$(task_status "$CO_PROJECT_KEY" "$id" 2>/dev/null || echo "?")
            title=$(task_title "$CO_PROJECT_KEY" "$id" 2>/dev/null || echo "?")

            case "$status" in
                done)        icon="✓"; done_count=$((done_count+1)) ;;
                in_review)   icon="◎"; in_review=$((in_review+1)) ;;
                in_progress) icon="▶"; in_progress=$((in_progress+1)) ;;
                todo)        icon="○"; todo_count=$((todo_count+1)) ;;
                *)           icon="?" ;;
            esac

            printf "    %s  %-6s  %-13s  %s\n" "$icon" "$CO_PROJECT_KEY-$id" "[$status]" "$title"
        done
        echo ""
    done

    local total=$((done_count + in_progress + in_review + todo_count))
    echo "  ── Summary ──"
    echo "    Done: $done_count / $total"
    echo "    In Review: $in_review"
    echo "    In Progress: $in_progress"
    echo "    Todo: $todo_count"
    echo ""
}
