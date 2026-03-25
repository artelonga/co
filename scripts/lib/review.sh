#!/usr/bin/env bash
#
# lib/review.sh — Review display, approve/reject
#

[[ -n "${__CO_LIB_REVIEW_LOADED:-}" ]] && return
__CO_LIB_REVIEW_LOADED=1

source_lib core
source_lib board
source_lib qa

# ── Display ──

# Show review summary: diff + acceptance criteria + QA results
review_show() {
    local project_key="$1" task_id="$2"
    local task_json
    task_json=$(task_fetch "$project_key" "$task_id")

    local task_title_str task_desc task_status_str
    task_title_str=$(echo "$task_json" | jq -r '.title')
    task_desc=$(echo "$task_json" | jq -r '.description')
    task_status_str=$(echo "$task_json" | jq -r '.status')

    echo "╔══════════════════════════════════════════════════════════╗"
    echo "║  REVIEW: $project_key-$task_id"
    echo "║  $task_title_str"
    echo "║  Status: $task_status_str"
    echo "╠══════════════════════════════════════════════════════════╣"
    echo ""

    # Show acceptance criteria
    echo "## Acceptance Criteria"
    echo ""
    echo "$task_desc" | grep -E '^\- \[' || echo "(no checkboxes found)"
    echo ""

    # Show QA results
    qa_results "$task_id"

    # Show git diff summary
    echo "## Changes (uncommitted)"
    echo ""
    git diff --stat 2>/dev/null || echo "(no changes)"
    echo ""

    # Show auto-execution log summary if exists
    if [[ -f "/tmp/co-auto-$task_id.log" ]]; then
        echo "## Execution Summary (last 20 lines)"
        echo ""
        tail -20 "/tmp/co-auto-$task_id.log"
        echo ""
    fi

    echo "╠══════════════════════════════════════════════════════════╣"
    echo "║  Actions:                                               ║"
    echo "║    co-pipeline approve $project_key-$task_id             "
    echo "║    co-pipeline reject $project_key-$task_id              "
    echo "╚══════════════════════════════════════════════════════════╝"
}

# ── Actions ──

# Approve task: mark done, check epic completion, show next
review_approve() {
    local project_key="$1" task_id="$2"
    task_set_status "$project_key" "$task_id" "done"
    check_epic_completion "$project_key" "$task_id"
    info "Task $project_key-$task_id → done"

    # Show what's next
    local next_id
    next_id=$(pick_next "$project_key")
    if [[ -n "$next_id" ]]; then
        info "Next: $next_id"
        info "Run: co-pipeline run $next_id"
    else
        info "All tasks complete!"
    fi
}

# Reject task: send back to in_progress
review_reject() {
    local project_key="$1" task_id="$2"
    task_set_status "$project_key" "$task_id" "in_progress"
    info "Task $project_key-$task_id → in_progress (rejected, needs rework)"
    info "Run: co-pipeline run $project_key-$task_id"
}
