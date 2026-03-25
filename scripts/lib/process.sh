#!/usr/bin/env bash
#
# lib/process.sh — Canonical process: task in → output out
#
# A process always takes a task and returns an output.
# It wraps the full lifecycle: preflight → execute → qa → record.
#
# Process record at ~/.co/runs/{timestamp}_{task-key}/
#   process.yaml   — structured input/output/status
#   session.id      — claude session UUID
#   qa.log          — QA results
#   context.md      — the prompt that was sent
#   output.log      — execution output (auto mode only)
#

[[ -n "${__CO_LIB_PROCESS_LOADED:-}" ]] && return
__CO_LIB_PROCESS_LOADED=1

source_lib core
source_lib board
source_lib context
source_lib session
source_lib qa
source_lib runner

# ── Configuration ──

CO_PROCESS_DIR="${CO_RUNS_DIR}"

# ── Preflight ──

# Check all prerequisites before starting a process
# Returns 0 if ready, 1 with diagnostics if not
process_preflight() {
    local project_key="$1" task_id="$2"
    local ready=true

    info "Preflight checks for $project_key-$task_id..."

    # 1. Required commands
    local cmd
    for cmd in curl jq claude git; do
        if ! command -v "$cmd" &>/dev/null; then
            warn "  Missing command: $cmd"
            ready=false
        fi
    done

    # 2. Board availability
    local board_ok=false
    if curl -sf "$CO_BOARD_API/health" > /dev/null 2>&1; then
        board_ok=true
    elif curl -sf "${CO_BOARD_API%/api}/api/health" > /dev/null 2>&1; then
        board_ok=true
    fi

    if $board_ok; then
        echo "  ✓ Board API reachable" >&2
    else
        warn "  Board API not reachable at $CO_BOARD_API"
        local cache_file="$CO_PROCESS_DIR/.task-cache/$project_key-$task_id.json"
        if [[ -f "$cache_file" ]]; then
            echo "  ✓ Cached task data available (offline mode)" >&2
        else
            warn "  No cached task data — board required for first run"
            ready=false
        fi
    fi

    # 3. Task fetchable (if board is up)
    # Run in subshell because task_fetch calls die() on failure
    if $board_ok; then
        if (task_fetch "$project_key" "$task_id" > /dev/null 2>&1); then
            echo "  ✓ Task $project_key-$task_id exists" >&2
        else
            warn "  Task $project_key-$task_id not found on board"
            local cache_file="$CO_PROCESS_DIR/.task-cache/$project_key-$task_id.json"
            if [[ -f "$cache_file" ]]; then
                echo "  ✓ Falling back to cached task data" >&2
            else
                ready=false
            fi
        fi
    fi

    # 4. Port conflict check
    if lsof -i :3000 -sTCP:LISTEN > /dev/null 2>&1; then
        local pid_on_port
        pid_on_port=$(lsof -ti :3000 -sTCP:LISTEN 2>/dev/null || true)
        echo "  ⚠ Port 3000 in use (PID: $pid_on_port) — may conflict with tests" >&2
    fi

    # 5. Node dependencies
    if [[ -f "co-web/package.json" ]] && [[ ! -d "co-web/node_modules" ]]; then
        warn "  co-web/node_modules missing — run: cd co-web && npm install"
    fi

    if $ready; then
        info "Preflight passed"
        return 0
    else
        warn "Preflight failed — fix issues above"
        return 1
    fi
}

# ── Task Cache ──

_process_cache_task() {
    local project_key="$1" task_id="$2" task_json="$3"
    local cache_dir="$CO_PROCESS_DIR/.task-cache"
    mkdir -p "$cache_dir"
    # Write compact JSON (single line) — safe for shell variable round-tripping
    printf '%s\n' "$task_json" | jq -c '.' > "$cache_dir/$project_key-$task_id.json" 2>/dev/null \
        || printf '%s\n' "$task_json" > "$cache_dir/$project_key-$task_id.json"
}

# Fetch task with cache fallback
# Returns compact (single-line) JSON safe for shell variables
process_fetch_task() {
    local project_key="$1" task_id="$2"
    local task_json
    task_json=$(task_fetch "$project_key" "$task_id" 2>/dev/null || true)

    if [[ -n "$task_json" ]]; then
        _process_cache_task "$project_key" "$task_id" "$task_json"
        printf '%s\n' "$task_json"
    else
        local cache_file="$CO_PROCESS_DIR/.task-cache/$project_key-$task_id.json"
        if [[ -f "$cache_file" ]]; then
            warn "Using cached task data (board offline)"
            # Read and compact — ensures single-line JSON for shell safety
            jq -c '.' "$cache_file" 2>/dev/null || cat "$cache_file"
        else
            die "Task $project_key-$task_id not available (board offline, no cache)"
        fi
    fi
}

# ── Process Record ──

_process_create_record() {
    local task_id="$1" mode="$2"
    local timestamp
    timestamp=$(date -u +%Y%m%dT%H%M%SZ)
    local process_dir="$CO_PROCESS_DIR/${timestamp}_${CO_PROJECT_KEY}-${task_id}"

    mkdir -p "$process_dir"

    cat > "$process_dir/process.yaml" <<EOF
task: ${CO_PROJECT_KEY}-${task_id}
mode: $mode
status: running
start: $(date -u +%Y-%m-%dT%H:%M:%SZ)
end:
session_id:
qa_passed:
EOF

    echo "$process_dir"
}

_process_update() {
    local process_dir="$1" field="$2" value="$3"
    local file="$process_dir/process.yaml"
    [[ -f "$file" ]] || return
    sed -i '' "s|^${field}:.*|${field}: ${value}|" "$file" 2>/dev/null || true
}

_process_finish() {
    local process_dir="$1" final_status="$2"
    _process_update "$process_dir" "status" "$final_status"
    _process_update "$process_dir" "end" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}

# ── Session ID Capture ──
# Detect the Claude project directory dynamically, then find the latest transcript.

_process_claude_project_dir() {
    # Derive from the git root of the current working directory
    local git_root
    git_root=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
    if [[ -n "$git_root" ]]; then
        # Claude encodes the path: /Users/yuri/projects/co → -Users-yuri-projects-co
        local encoded
        encoded=$(echo "$git_root" | sed 's|/|-|g; s|^-||')
        local candidate="$HOME/.claude/projects/-${encoded}"
        if [[ -d "$candidate" ]]; then
            echo "$candidate"
            return
        fi
    fi
    # Fallback: find any project dir with recent jsonl files
    ls -1dt "$HOME"/.claude/projects/*/  2>/dev/null | head -1
}

_process_capture_session_id() {
    local process_dir="$1"
    local project_dir
    project_dir=$(_process_claude_project_dir)
    [[ -z "$project_dir" ]] && return

    local latest_session
    latest_session=$(ls -t "$project_dir"/*.jsonl 2>/dev/null | head -1 | xargs basename 2>/dev/null | sed 's/.jsonl//')
    if [[ -n "$latest_session" ]]; then
        echo "$latest_session" > "$process_dir/session.id"
        _process_update "$process_dir" "session_id" "$latest_session"
    fi
}

# ── Sync ──

process_sync() {
    if [[ -x "$CO_SCRIPT_DIR/co-session-sync" ]]; then
        info "Syncing session logs..."
        "$CO_SCRIPT_DIR/co-session-sync" --latest --push 2>/dev/null || true
    fi
}

# ── Summary ──

_process_summary() {
    local process_dir="$1"
    local pfile="$process_dir/process.yaml"
    [[ -f "$pfile" ]] || return

    echo ""
    echo "╔══════════════════════════════════════════════════════════╗"
    echo "║  Process Complete                                       ║"
    echo "╠══════════════════════════════════════════════════════════╣"

    local p_task p_mode p_status p_start p_end p_session p_qa
    p_task=$(grep '^task:' "$pfile" | cut -d' ' -f2-)
    p_mode=$(grep '^mode:' "$pfile" | cut -d' ' -f2-)
    p_status=$(grep '^status:' "$pfile" | cut -d' ' -f2-)
    p_start=$(grep '^start:' "$pfile" | cut -d' ' -f2-)
    p_end=$(grep '^end:' "$pfile" | cut -d' ' -f2-)
    p_session=$(grep '^session_id:' "$pfile" | cut -d' ' -f2-)
    p_qa=$(grep '^qa_passed:' "$pfile" | cut -d' ' -f2-)

    printf "║  Task:       %-42s ║\n" "$p_task"
    printf "║  Mode:       %-42s ║\n" "$p_mode"
    printf "║  Status:     %-42s ║\n" "$p_status"
    printf "║  Start:      %-42s ║\n" "$p_start"
    printf "║  End:        %-42s ║\n" "$p_end"
    if [[ -n "$p_session" ]]; then
        printf "║  Session:    %-42s ║\n" "${p_session:0:36}..."
    fi
    if [[ -n "$p_qa" ]]; then
        printf "║  QA:         %-42s ║\n" "$p_qa"
    fi
    printf "║  Record:     %-42s ║\n" "$(basename "$process_dir")"
    echo "╚══════════════════════════════════════════════════════════╝"
    echo ""
}

# ── Main Process Functions ──

# Run a process interactively
# Usage: process_interactive project_key task_id [flags...]
process_interactive() {
    local project_key="$1" task_id="$2"
    shift 2

    # Preflight
    process_preflight "$project_key" "$task_id" || return 1

    # Create process record
    local process_dir
    process_dir=$(_process_create_record "$task_id" "interactive")

    # Fetch and cache task (single fetch, reused everywhere)
    local task_json
    task_json=$(process_fetch_task "$project_key" "$task_id")

    # Build context — pass task_json to avoid re-fetching
    context_build_session "$project_key" "$task_id" "$task_json"
    local prompt_file
    prompt_file=$(context_write_file)
    cp "$prompt_file" "$process_dir/context.md"

    # Mark as in_progress
    local current_status_str
    current_status_str=$(printf '%s\n' "$task_json" | jq -r '.status')
    if [[ "$current_status_str" == "todo" ]]; then
        task_set_status "$project_key" "$task_id" "in_progress" 2>/dev/null || true
    fi

    local task_title_str
    task_title_str=$(printf '%s\n' "$task_json" | jq -r '.title')
    local session_name="task-${project_key}-${task_id}"

    info "Process: $project_key-$task_id — $task_title_str"
    info "Record:  $(basename "$process_dir")"
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

    # Launch Claude — NO tee for interactive mode (preserves TUI)
    if [[ ${#claude_flags[@]} -gt 0 ]]; then
        claude --append-system-prompt-file "$prompt_file" "${claude_flags[@]}"
    else
        claude --append-system-prompt-file "$prompt_file"
    fi
    local exit_code=$?

    # Post-session
    _process_capture_session_id "$process_dir"
    process_sync

    if [[ $exit_code -eq 0 ]]; then
        _process_finish "$process_dir" "completed"
    else
        _process_finish "$process_dir" "failed"
    fi

    _process_summary "$process_dir"
    return $exit_code
}

# Run a process automatically (non-interactive)
# Usage: process_auto project_key task_id
process_auto() {
    local project_key="$1" task_id="$2"

    # Preflight
    process_preflight "$project_key" "$task_id" || return 1

    # Create process record
    local process_dir
    process_dir=$(_process_create_record "$task_id" "auto")

    # Fetch and cache task (single fetch)
    local task_json
    task_json=$(process_fetch_task "$project_key" "$task_id")

    # Build context — pass task_json to avoid re-fetching
    context_build_auto "$project_key" "$task_id" "$task_json"
    local prompt
    prompt=$(context_get)
    local prompt_file
    prompt_file=$(mktemp /tmp/co-process-XXXXXX.md)
    echo "$prompt" > "$prompt_file"
    co_register_cleanup "$prompt_file"
    cp "$prompt_file" "$process_dir/context.md"

    local task_title_str
    task_title_str=$(printf '%s\n' "$task_json" | jq -r '.title')

    info "Process auto: $project_key-$task_id — $task_title_str"
    info "Record:       $(basename "$process_dir")"
    task_set_status "$project_key" "$task_id" "in_progress" 2>/dev/null || true

    # Run Claude non-interactively (tee is fine here — no TUI)
    claude -p "$(cat "$prompt_file")" \
        --allowedTools "$CO_ALLOWED_TOOLS" \
        --max-turns "$CO_MAX_TURNS" \
        2>&1 | tee "$process_dir/output.log"

    local exit_code=${PIPESTATUS[0]}

    # Post-session
    _process_capture_session_id "$process_dir"

    # QA
    info "Running QA..."
    if qa_run "$task_id" 2>&1 | tee "$process_dir/qa.log"; then
        _process_update "$process_dir" "qa_passed" "true"
        task_set_status "$project_key" "$task_id" "in_review" 2>/dev/null || true
        _process_finish "$process_dir" "success"
    else
        _process_update "$process_dir" "qa_passed" "false"
        _process_finish "$process_dir" "qa_failed"
    fi

    process_sync
    _process_summary "$process_dir"
    return $exit_code
}

# ── Process Queries ──

process_list() {
    local limit="${1:-10}"
    echo "  Recent Processes:"
    echo ""
    printf "  %-38s  %-10s  %-12s  %-12s  %s\n" "ID" "TASK" "STATUS" "MODE" "SESSION"
    printf "  %-38s  %-10s  %-12s  %-12s  %s\n" "──" "────" "──────" "────" "───────"

    ls -1d "$CO_PROCESS_DIR"/*_*/ 2>/dev/null | sort -r | head -"$limit" | while read -r dir; do
        local id
        id=$(basename "$dir")
        local pfile="$dir/process.yaml"
        if [[ -f "$pfile" ]]; then
            local p_task p_status p_mode p_session
            p_task=$(grep '^task:' "$pfile" | cut -d' ' -f2-)
            p_status=$(grep '^status:' "$pfile" | cut -d' ' -f2-)
            p_mode=$(grep '^mode:' "$pfile" | cut -d' ' -f2-)
            p_session=$(grep '^session_id:' "$pfile" | cut -d' ' -f2-)
            printf "  %-38s  %-10s  %-12s  %-12s  %s\n" "$id" "$p_task" "[$p_status]" "$p_mode" "${p_session:0:12}"
        fi
    done
}

process_show() {
    local process_id="$1"
    local process_dir="$CO_PROCESS_DIR/$process_id"

    if [[ ! -d "$process_dir" ]]; then
        process_dir=$(ls -1d "$CO_PROCESS_DIR"/*"$process_id"* 2>/dev/null | head -1)
        [[ -z "$process_dir" ]] && die "Process not found: $process_id"
    fi

    echo "── Process: $(basename "$process_dir") ──"
    echo ""

    if [[ -f "$process_dir/process.yaml" ]]; then
        cat "$process_dir/process.yaml"
        echo ""
    fi

    echo "Artifacts:"
    ls -lh "$process_dir/" 2>/dev/null | grep -v '^total' | grep -v '^\.' || echo "  (none)"
}
