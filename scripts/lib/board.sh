#!/usr/bin/env bash
#
# lib/board.sh — Board API client
#

[[ -n "${__CO_LIB_BOARD_LOADED:-}" ]] && return
__CO_LIB_BOARD_LOADED=1

source_lib core

# ── Internal ──

_board_task_url() {
    echo "$CO_BOARD_API/projects/$1/tasks/$2"
}

_board_tasks_url() {
    echo "$CO_BOARD_API/projects/$1/tasks"
}

# ── Public API ──

# Fetch full task JSON
task_fetch() {
    local project_key="$1" task_id="$2"
    curl -sf "$(_board_task_url "$project_key" "$task_id")" \
        || die "Task $project_key-$task_id not found (is the board running?)"
}

# Get task status string
task_status() {
    local project_key="$1" task_id="$2"
    curl -sf "$(_board_task_url "$project_key" "$task_id")" | jq -r '.status'
}

# Get task title string
task_title() {
    local project_key="$1" task_id="$2"
    curl -sf "$(_board_task_url "$project_key" "$task_id")" | jq -r '.title'
}

# Update task status
task_set_status() {
    local project_key="$1" task_id="$2" status="$3"
    curl -sf -X PUT "$(_board_task_url "$project_key" "$task_id")" \
        -H "Content-Type: application/json" \
        -d "{\"status\":\"$status\"}" > /dev/null
}

# List all tasks formatted
task_list() {
    local project_key="$1"
    curl -sf "$(_board_tasks_url "$project_key")?archived=false&limit=500" \
        | jq -r '.[] | "\(.key)\t[\(.priority)]\t[\(.status)]\t\(.title)"' \
        | column -t -s $'\t'
}

# Fetch parent task JSON (returns "{}" if not found)
task_parent() {
    local project_key="$1" task_json="$2"
    local parent_id
    parent_id=$(printf '%s\n' "$task_json" | jq -r '.parent // empty')
    if [[ -n "$parent_id" ]]; then
        curl -sf "$(_board_task_url "$project_key" "$parent_id")" 2>/dev/null || echo "{}"
    else
        echo "{}"
    fi
}

# Fetch sibling tasks (tasks sharing the same parent)
task_siblings() {
    local project_key="$1" parent_id="$2"
    curl -sf "$(_board_tasks_url "$project_key")?archived=false&limit=500" \
        | jq --argjson pid "$parent_id" '[.[] | select(.parent == $pid)]'
}

# Fetch children (subtasks of a parent)
task_children() {
    local project_key="$1" parent_id="$2"
    curl -sf "$(_board_tasks_url "$project_key")?archived=false&limit=500" \
        | jq --argjson pid "$parent_id" '[.[] | select(.parent == $pid)]'
}

# Check if all siblings are done → mark parent epic as done
check_epic_completion() {
    local project_key="$1" task_id="$2"
    local parent_id
    parent_id=$(curl -sf "$(_board_task_url "$project_key" "$task_id")" | jq -r '.parent // empty')
    [[ -z "$parent_id" ]] && return

    local all_done
    all_done=$(curl -sf "$(_board_tasks_url "$project_key")?limit=500" \
        | jq --argjson pid "$parent_id" '
            [.[] | select(.parent == $pid)] |
            all(.status == "done")
        ')

    if [[ "$all_done" == "true" ]]; then
        info "All subtasks of $project_key-$parent_id complete — marking epic as done"
        task_set_status "$project_key" "$parent_id" "done"
    fi
}

# Pick next todo task by priority
pick_next() {
    local project_key="${1:-$CO_PROJECT_KEY}"
    curl -sf "$(_board_tasks_url "$project_key")?archived=false&limit=500" \
        | jq -r '
            [.[] | select(.status == "todo")] |
            sort_by(
                (if .priority == "critical" then 0
                 elif .priority == "high" then 1
                 elif .priority == "medium" then 2
                 else 3 end),
                (if .parent then 0 else 1 end),
                .id
            ) | .[0] // empty | .key
        '
}
