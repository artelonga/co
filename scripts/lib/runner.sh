#!/usr/bin/env bash
#
# lib/runner.sh — Run tracking (PST AutoRunTracker pattern)
#

[[ -n "${__CO_LIB_RUNNER_LOADED:-}" ]] && return
__CO_LIB_RUNNER_LOADED=1

source_lib core

# ── Run Lifecycle ──

# Start a new run, returns run_id
# Usage: run_start task_id mode
run_start() {
    local task_id="$1" mode="$2"
    local timestamp
    timestamp=$(date -u +%Y%m%dT%H%M%SZ)
    local run_id="${timestamp}_${CO_PROJECT_KEY}-${task_id}"
    local run_dir="$CO_RUNS_DIR/$run_id"

    mkdir -p "$run_dir"

    cat > "$run_dir/run.yaml" <<EOF
task: ${CO_PROJECT_KEY}-${task_id}
mode: $mode
status: running
start: $(date -u +%Y-%m-%dT%H:%M:%SZ)
end:
phases: []
artifacts: []
EOF

    echo "$run_id"
}

# Record a phase transition
# Usage: run_phase run_id phase_name
run_phase() {
    local run_id="$1" phase_name="$2"
    local run_file="$CO_RUNS_DIR/$run_id/run.yaml"
    [[ -f "$run_file" ]] || return

    local timestamp
    timestamp=$(date -u +%H:%M:%S)

    # Append phase to the phases list
    sed -i '' "s|^phases: \[\]|phases:|" "$run_file" 2>/dev/null || true
    echo "  - $phase_name $timestamp" >> "$run_file"
}

# Record an artifact
# Usage: run_artifact run_id type ref
run_artifact() {
    local run_id="$1" type="$2" ref="$3"
    local run_file="$CO_RUNS_DIR/$run_id/run.yaml"
    [[ -f "$run_file" ]] || return

    sed -i '' "s|^artifacts: \[\]|artifacts:|" "$run_file" 2>/dev/null || true
    echo "  - $type $ref" >> "$run_file"
}

# Finalize a run
# Usage: run_finish run_id status
run_finish() {
    local run_id="$1" status="$2"
    local run_file="$CO_RUNS_DIR/$run_id/run.yaml"
    [[ -f "$run_file" ]] || return

    local end_time
    end_time=$(date -u +%Y-%m-%dT%H:%M:%SZ)

    sed -i '' "s|^status: running|status: $status|" "$run_file" 2>/dev/null || true
    sed -i '' "s|^end:$|end: $end_time|" "$run_file" 2>/dev/null || true
}

# ── Queries ──

# Print run summary
run_report() {
    local run_id="$1"
    local run_file="$CO_RUNS_DIR/$run_id/run.yaml"
    [[ -f "$run_file" ]] || { warn "Run not found: $run_id"; return 1; }

    echo "── Run: $run_id ──"
    cat "$run_file"
    echo ""
}

# Most recent run_id
run_latest() {
    # Directories are named with timestamps, so sorting gives chronological order
    ls -1d "$CO_RUNS_DIR"/*/ 2>/dev/null | sort | tail -1 | xargs basename 2>/dev/null
}

# List recent runs
run_list() {
    local limit="${1:-10}"
    ls -1d "$CO_RUNS_DIR"/*/ 2>/dev/null | sort -r | head -"$limit" | while read -r dir; do
        local id
        id=$(basename "$dir")
        local run_file="$dir/run.yaml"
        if [[ -f "$run_file" ]]; then
            local r_task r_status
            r_task=$(grep '^task:' "$run_file" | awk '{print $2}')
            r_status=$(grep '^status:' "$run_file" | awk '{print $2}')
            printf "  %-40s  %-20s  %s\n" "$id" "$r_task" "$r_status"
        fi
    done
}
