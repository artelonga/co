#!/usr/bin/env bash
#
# lib/session.sh — Claude Code session launcher
#

[[ -n "${__CO_LIB_SESSION_LOADED:-}" ]] && return
__CO_LIB_SESSION_LOADED=1

source_lib core
source_lib board
source_lib context

# ── Configuration ──

CO_MAX_TURNS="${CO_MAX_TURNS:-50}"
CO_ALLOWED_TOOLS="${CO_ALLOWED_TOOLS:-Bash,Read,Edit,Write,Glob,Grep}"

# ── Session Launcher ──

# Launch interactive Claude session
# Usage: session_interactive project_key task_id [flags...]
session_interactive() {
    local project_key="$1" task_id="$2"
    shift 2

    context_build_session "$project_key" "$task_id"
    local prompt_file
    prompt_file=$(context_write_file)

    # Mark as in_progress if todo
    local current_status
    current_status=$(task_status "$project_key" "$task_id")
    if [[ "$current_status" == "todo" ]]; then
        info "Marking $project_key-$task_id as in_progress..."
        task_set_status "$project_key" "$task_id" "in_progress"
    fi

    local task_title_str
    task_title_str=$(task_title "$project_key" "$task_id")
    local session_name="task-${project_key}-${task_id}"

    info "Starting Claude session: $session_name"
    info "Task: $task_title_str"
    echo ""

    # Parse flags
    local -a claude_flags=()
    local arg
    for arg in "$@"; do
        case "$arg" in
            --continue|-c) claude_flags+=("--resume" "$session_name") ;;
            --plan)        claude_flags+=("--permission-mode" "plan") ;;
            *)             claude_flags+=("$arg") ;;
        esac
    done

    # Launch Claude
    if [[ ${#claude_flags[@]} -gt 0 ]]; then
        claude --append-system-prompt-file "$prompt_file" "${claude_flags[@]}"
    else
        claude --append-system-prompt-file "$prompt_file"
    fi
    local exit_code=$?

    session_sync
    return $exit_code
}

# Launch non-interactive auto session
# Usage: session_auto project_key task_id
session_auto() {
    local project_key="$1" task_id="$2"

    context_build_auto "$project_key" "$task_id"
    local prompt
    prompt=$(context_get)

    local task_title_str
    task_title_str=$(task_title "$project_key" "$task_id")

    info "Auto-executing $project_key-$task_id: $task_title_str"
    task_set_status "$project_key" "$task_id" "in_progress"

    # Write prompt to temp file
    local prompt_file
    prompt_file=$(mktemp /tmp/co-auto-XXXXXX.md)
    echo "$prompt" > "$prompt_file"
    co_register_cleanup "$prompt_file"

    # Run Claude non-interactively
    claude -p "$(cat "$prompt_file")" \
        --allowedTools "$CO_ALLOWED_TOOLS" \
        --max-turns "$CO_MAX_TURNS" \
        2>&1 | tee "/tmp/co-auto-$task_id.log"

    local exit_code=${PIPESTATUS[0]}

    session_sync
    return $exit_code
}

# Sync session logs
session_sync() {
    if [[ -x "$CO_SCRIPT_DIR/co-session-sync" ]]; then
        info "Syncing session logs..."
        "$CO_SCRIPT_DIR/co-session-sync" --latest --push 2>/dev/null || true
    fi
}
