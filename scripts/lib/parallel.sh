#!/usr/bin/env bash
#
# lib/parallel.sh — Worktree-based parallel execution
# Compatible with bash 3.2+ (no associative arrays)
#

[[ -n "${__CO_LIB_PARALLEL_LOADED:-}" ]] && return
__CO_LIB_PARALLEL_LOADED=1

source_lib core
source_lib board
source_lib session
source_lib qa
source_lib runner

# ── Configuration ──

CO_PARALLEL_MAX="${CO_PARALLEL_MAX:-3}"
CO_WORKTREE_DIR="${CO_WORKTREE_DIR:-.worktrees}"

# ── Job Tracking (parallel arrays, bash 3.2 compatible) ──

_PARALLEL_TASK_IDS=()
_PARALLEL_PIDS=()
_PARALLEL_WORKTREE_PATHS=()

_parallel_find_task() {
    local task_id="$1" i
    for i in "${!_PARALLEL_TASK_IDS[@]}"; do
        if [[ "${_PARALLEL_TASK_IDS[$i]}" == "$task_id" ]]; then
            echo "$i"
            return 0
        fi
    done
    return 1
}

# ── Worktree Management ──

# Create a worktree for a task
parallel_create_worktree() {
    local task_id="$1"
    local branch_name="auto/task-${task_id}"
    local worktree_path="$CO_WORKTREE_DIR/task-${task_id}"

    # Create worktree directory
    mkdir -p "$CO_WORKTREE_DIR"

    # Create branch and worktree
    git worktree add "$worktree_path" -b "$branch_name" 2>/dev/null \
        || git worktree add "$worktree_path" "$branch_name" 2>/dev/null \
        || die "Failed to create worktree for task $task_id"

    _PARALLEL_TASK_IDS+=("$task_id")
    _PARALLEL_PIDS+=("")
    _PARALLEL_WORKTREE_PATHS+=("$worktree_path")

    info "Created worktree: $worktree_path (branch: $branch_name)"
    echo "$worktree_path"
}

# Remove a worktree and clean up
parallel_remove_worktree() {
    local task_id="$1"
    local worktree_path="$CO_WORKTREE_DIR/task-${task_id}"
    local branch_name="auto/task-${task_id}"

    if [[ -d "$worktree_path" ]]; then
        git worktree remove "$worktree_path" --force 2>/dev/null || true
        git branch -D "$branch_name" 2>/dev/null || true
        info "Removed worktree: $worktree_path"
    fi
}

# List active worktrees
parallel_list_worktrees() {
    git worktree list 2>/dev/null | grep "$CO_WORKTREE_DIR" || echo "(no parallel worktrees)"
}

# ── Job Management ──

# Spawn an auto session in a worktree (background)
parallel_spawn() {
    local task_id="$1" mode="${2:-auto}"
    local worktree_path
    worktree_path=$(parallel_create_worktree "$task_id")
    local idx
    idx=$(_parallel_find_task "$task_id")

    (
        cd "$worktree_path" || exit 1

        # Run auto session in the worktree
        local run_id
        run_id=$(run_start "$task_id" "parallel-$mode")
        run_phase "$run_id" "execute"

        session_auto "$CO_PROJECT_KEY" "$task_id"
        local exit_code=$?

        run_phase "$run_id" "qa"
        if qa_run "$task_id"; then
            run_finish "$run_id" "success"
            task_set_status "$CO_PROJECT_KEY" "$task_id" "in_review"
        else
            run_finish "$run_id" "qa_failed"
        fi

        exit $exit_code
    ) &

    local pid=$!
    _PARALLEL_PIDS[$idx]=$pid
    info "Spawned task $CO_PROJECT_KEY-$task_id (PID: $pid)"
}

# Wait for a specific job
parallel_wait() {
    local task_id="$1"
    local idx
    idx=$(_parallel_find_task "$task_id") || { warn "No running job for task $task_id"; return 1; }
    local pid="${_PARALLEL_PIDS[$idx]}"
    [[ -z "$pid" ]] && { warn "No PID for task $task_id"; return 1; }

    wait "$pid"
    local exit_code=$?
    _PARALLEL_PIDS[$idx]=""
    return $exit_code
}

# Wait for all jobs, handle SIGINT gracefully
parallel_wait_all() {
    local _parallel_interrupted=false

    trap '_parallel_interrupted=true' INT

    local i pid task_id
    for i in "${!_PARALLEL_TASK_IDS[@]}"; do
        if $_parallel_interrupted; then
            warn "Interrupted — killing remaining jobs"
            for pid in "${_PARALLEL_PIDS[@]}"; do
                [[ -n "$pid" ]] && kill "$pid" 2>/dev/null || true
            done
            break
        fi

        pid="${_PARALLEL_PIDS[$i]}"
        task_id="${_PARALLEL_TASK_IDS[$i]}"
        [[ -z "$pid" ]] && continue

        info "Waiting for task $CO_PROJECT_KEY-$task_id (PID: $pid)..."
        wait "$pid" 2>/dev/null || true
        _PARALLEL_PIDS[$i]=""
    done

    trap - INT
}

# Show status of parallel jobs
parallel_status() {
    echo "  Parallel Jobs:"
    if [[ ${#_PARALLEL_TASK_IDS[@]} -eq 0 ]]; then
        echo "    (none running)"
        return
    fi

    local i task_id pid
    for i in "${!_PARALLEL_TASK_IDS[@]}"; do
        task_id="${_PARALLEL_TASK_IDS[$i]}"
        pid="${_PARALLEL_PIDS[$i]}"
        [[ -z "$pid" ]] && continue

        if kill -0 "$pid" 2>/dev/null; then
            printf "    ▶  %s-%-3s  PID %-6s  running\n" "$CO_PROJECT_KEY" "$task_id" "$pid"
        else
            printf "    ■  %s-%-3s  PID %-6s  completed\n" "$CO_PROJECT_KEY" "$task_id" "$pid"
        fi
    done
}

# ── Merge ──

# Merge a worktree branch back to the current branch
parallel_merge() {
    local task_id="$1"
    local branch_name="auto/task-${task_id}"

    info "Merging $branch_name..."
    if git merge "$branch_name" --no-edit 2>/dev/null; then
        info "Merged $branch_name successfully"
    else
        warn "Merge conflict for $branch_name — manual resolution needed"
        return 1
    fi
}

# ── Batch Execution ──

# Execute tasks with controlled parallelism
# Usage: parallel_batch max_concurrent task_ids...
parallel_batch() {
    local max_concurrent="$1"
    shift
    local task_ids=("$@")

    local running=0
    local id

    for id in "${task_ids[@]}"; do
        # Wait if at max concurrency
        while [[ $running -ge $max_concurrent ]]; do
            wait -n 2>/dev/null || true
            running=$((running - 1))
        done

        parallel_spawn "$id"
        running=$((running + 1))
    done

    # Wait for all remaining
    parallel_wait_all

    # Merge results
    info "Merging completed tasks..."
    for id in "${task_ids[@]}"; do
        if [[ "$(task_status "$CO_PROJECT_KEY" "$id")" == "in_review" ]]; then
            parallel_merge "$id"
            qa_run "$id" || warn "Post-merge QA failed for $CO_PROJECT_KEY-$id"
        fi
        parallel_remove_worktree "$id"
    done
}
