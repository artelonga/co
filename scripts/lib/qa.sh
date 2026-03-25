#!/usr/bin/env bash
#
# lib/qa.sh — QA check registry + runner
#

[[ -n "${__CO_LIB_QA_LOADED:-}" ]] && return
__CO_LIB_QA_LOADED=1

source_lib core

# ── Registry ──

# Parallel arrays for registered checks
_QA_NAMES=()
_QA_COMMANDS=()
_QA_BLOCKING=()

QA_LOG="/tmp/co-qa-latest.log"

# Register a QA check
# Usage: qa_register name command blocking
#   blocking: "true" or "false"
qa_register() {
    local name="$1" command="$2" blocking="${3:-true}"
    _QA_NAMES+=("$name")
    _QA_COMMANDS+=("$command")
    _QA_BLOCKING+=("$blocking")
}

# Remove a registered check
qa_unregister() {
    local name="$1"
    local i
    for i in "${!_QA_NAMES[@]}"; do
        if [[ "${_QA_NAMES[$i]}" == "$name" ]]; then
            unset '_QA_NAMES[$i]' '_QA_COMMANDS[$i]' '_QA_BLOCKING[$i]'
            # Re-index arrays
            _QA_NAMES=("${_QA_NAMES[@]}")
            _QA_COMMANDS=("${_QA_COMMANDS[@]}")
            _QA_BLOCKING=("${_QA_BLOCKING[@]}")
            return
        fi
    done
}

# ── Built-in Checks ──

qa_register "cargo_check"  "cargo check -p co-web"                       "true"
qa_register "cargo_clippy"  "cargo clippy -p co-web -- -D warnings"       "true"
qa_register "cargo_fmt"     "cargo fmt -p co-web -- --check"              "true"
qa_register "cargo_test"    "cargo test -p co-web"                        "true"

# ── Runner ──

# Run all registered QA checks
# Usage: qa_run task_id [phase]
qa_run() {
    local task_id="$1"
    local phase="${2:-}"

    info "Running QA for $CO_PROJECT_KEY-$task_id${phase:+ (Phase $phase)}"
    echo "" > "$QA_LOG"

    local passed=true
    local i name cmd blocking

    for i in "${!_QA_NAMES[@]}"; do
        name="${_QA_NAMES[$i]}"
        cmd="${_QA_COMMANDS[$i]}"
        blocking="${_QA_BLOCKING[$i]}"

        info "  Check: $name"
        if eval "$cmd" >> "$QA_LOG" 2>&1; then
            echo "  ✓ $name" | tee -a "$QA_LOG"
        else
            echo "  ✗ $name FAILED" | tee -a "$QA_LOG"
            if [[ "$blocking" == "true" ]]; then
                passed=false
            else
                warn "  ($name failure non-blocking)"
            fi
        fi
    done

    # Playwright — conditional on phase and availability
    if [[ -f "co-web/package.json" ]] && command -v npx &>/dev/null; then
        info "  Check: playwright"
        if (cd co-web && npx playwright test --reporter=list --pass-with-no-tests) >> "$QA_LOG" 2>&1; then
            echo "  ✓ playwright tests" | tee -a "$QA_LOG"
        else
            echo "  ✗ playwright FAILED" | tee -a "$QA_LOG"
            if [[ "$phase" != "A" ]]; then
                passed=false
            else
                warn "  (playwright failure non-blocking during Phase A setup)"
            fi
        fi
    fi

    echo ""

    if $passed; then
        info "QA PASSED for $CO_PROJECT_KEY-$task_id"
        return 0
    else
        warn "QA FAILED for $CO_PROJECT_KEY-$task_id — see $QA_LOG"
        return 1
    fi
}

# Display formatted QA results
qa_results() {
    local task_id="$1"
    if [[ -f "$QA_LOG" ]]; then
        echo "## QA Results"
        echo ""
        grep -E '^\s+[✓✗]' "$QA_LOG" || echo "(no QA results — run: co-pipeline qa $CO_PROJECT_KEY-$task_id)"
        echo ""
    fi
}

# Check if last QA run passed
qa_passed() {
    [[ -f "$QA_LOG" ]] && ! grep -q '✗' "$QA_LOG"
}
