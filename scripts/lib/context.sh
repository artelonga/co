#!/usr/bin/env bash
#
# lib/context.sh — Prompt/context builder (PST ContextBuilder pattern)
#

[[ -n "${__CO_LIB_CONTEXT_LOADED:-}" ]] && return
__CO_LIB_CONTEXT_LOADED=1

source_lib core
source_lib board

# ── State ──

_CO_CONTEXT=""

# ── Core Operations ──

context_reset() {
    _CO_CONTEXT=""
}

# Append a section to the context
_context_append() {
    _CO_CONTEXT+="$1"$'\n'
}

# ── Section Builders ──

# Add task heading + description
context_add_task() {
    local task_json="$1"
    local task_key task_title task_desc task_priority task_status_str task_labels
    task_key=$(printf '%s\n' "$task_json" | jq -r '.key')
    task_title=$(printf '%s\n' "$task_json" | jq -r '.title')
    task_desc=$(printf '%s\n' "$task_json" | jq -r '.description')
    task_priority=$(printf '%s\n' "$task_json" | jq -r '.priority')
    task_status_str=$(printf '%s\n' "$task_json" | jq -r '.status')
    task_labels=$(printf '%s\n' "$task_json" | jq -r '.labels | join(", ")')

    _context_append "# Task: $task_key — $task_title

**Priority:** $task_priority | **Status:** $task_status_str | **Labels:** $task_labels

## Description

$task_desc"
}

# Add parent user story context
context_add_parent() {
    local parent_json="$1"
    [[ "$parent_json" == "{}" ]] && return

    local parent_key parent_title parent_desc
    parent_key=$(printf '%s\n' "$parent_json" | jq -r '.key // empty')
    [[ -z "$parent_key" ]] && return

    parent_title=$(printf '%s\n' "$parent_json" | jq -r '.title')
    parent_desc=$(printf '%s\n' "$parent_json" | jq -r '.description')

    _context_append "
## Parent User Story: $parent_key — $parent_title

$parent_desc"
}

# Add sibling tasks list
context_add_siblings() {
    local siblings_json="$1"
    [[ "$siblings_json" == "[]" || -z "$siblings_json" ]] && return

    _context_append "
## Sibling Tasks (same epic)

$(printf '%s\n' "$siblings_json" | jq -r '.[] | "- [\(.status)] \(.key): \(.title)"')"
}

# Add subtasks for epics
context_add_children() {
    local project_key="$1" task_id="$2"
    local children
    children=$(task_children "$project_key" "$task_id")
    [[ "$children" == "[]" ]] && return

    _context_append "
## Subtasks

$(printf '%s\n' "$children" | jq -r '.[] | "- [\(.status)] \(.key): \(.title)\n  \(.description)\n"')"
}

# Add TDD instructions block
# Accepts task_json directly — no re-fetch
context_add_instructions_from() {
    local task_json="$1"
    local task_key proj_key task_id
    task_key=$(printf '%s\n' "$task_json" | jq -r '.key')
    proj_key=$(printf '%s\n' "$task_json" | jq -r '.project_key')
    task_id=$(printf '%s\n' "$task_json" | jq -r '.id')

    _context_append "
## Instructions

1. This is task **$task_key**. Focus exclusively on its scope.
2. Follow TDD: write failing test first, then implement, then refactor.
3. Run \`cargo test -p co-web\`, \`cargo clippy -p co-web -- -D warnings\`, \`cargo fmt -p co-web\` before finishing.
4. When done, mark the task as complete by running:
   \`curl -s -X PUT $CO_BOARD_API/projects/$proj_key/tasks/$task_id -H 'Content-Type: application/json' -d '{\"status\":\"done\"}'\`
5. Summarize what was done and what the next task should be."
}

# Add arbitrary custom section
context_add_custom() {
    local title="$1" body="$2"
    _context_append "
## $title

$body"
}

# ── High-Level Builders ──
# All accept optional pre-fetched task_json to avoid re-fetching.

# Build full interactive session context
# Usage: context_build_session project_key task_id [task_json]
context_build_session() {
    local project_key="$1" task_id="$2"
    local task_json="${3:-}"

    context_reset

    if [[ -z "$task_json" ]]; then
        task_json=$(task_fetch "$project_key" "$task_id")
    fi

    local parent_id
    parent_id=$(printf '%s\n' "$task_json" | jq -r '.parent // empty')

    local parent_json="{}"
    local siblings_json="[]"

    if [[ -n "$parent_id" ]]; then
        parent_json=$(task_parent "$project_key" "$task_json")
        siblings_json=$(task_siblings "$project_key" "$parent_id")
    fi

    context_add_task "$task_json"

    if [[ -n "$parent_id" ]]; then
        context_add_parent "$parent_json"
        context_add_siblings "$siblings_json"
    else
        # Epic — show subtasks
        local tid
        tid=$(printf '%s\n' "$task_json" | jq -r '.id')
        context_add_children "$project_key" "$tid"
    fi

    context_add_instructions_from "$task_json"
}

# Build terser auto-mode context
# Usage: context_build_auto project_key task_id [task_json]
context_build_auto() {
    local project_key="$1" task_id="$2"
    local task_json="${3:-}"

    context_reset

    if [[ -z "$task_json" ]]; then
        task_json=$(task_fetch "$project_key" "$task_id")
    fi

    local task_title task_desc parent_id
    task_title=$(printf '%s\n' "$task_json" | jq -r '.title')
    task_desc=$(printf '%s\n' "$task_json" | jq -r '.description')
    parent_id=$(printf '%s\n' "$task_json" | jq -r '.parent // empty')

    _context_append "Implement the following task. Work autonomously — do not ask questions, make reasonable decisions.

# Task: $project_key-$task_id — $task_title

$task_desc"

    if [[ -n "$parent_id" ]]; then
        local parent_json parent_title parent_desc
        parent_json=$(task_parent "$project_key" "$task_json")
        if [[ "$parent_json" != "{}" ]]; then
            parent_title=$(printf '%s\n' "$parent_json" | jq -r '.title // empty')
            parent_desc=$(printf '%s\n' "$parent_json" | jq -r '.description // empty')
            _context_append "
## Parent User Story: $project_key-$parent_id — $parent_title

$parent_desc"
        fi
    fi

    _context_append "
## Instructions
1. Implement the task following the acceptance criteria checkboxes.
2. Follow TDD where applicable.
3. Run cargo test, cargo clippy -- -D warnings, cargo fmt before finishing.
4. Summarize what was done."
}

# ── Output ──

# Return accumulated context string
context_get() {
    echo "$_CO_CONTEXT"
}

# Write context to temp file, echo path
context_write_file() {
    local prompt_file
    prompt_file=$(mktemp /tmp/co-context-XXXXXX.md)
    echo "$_CO_CONTEXT" > "$prompt_file"
    co_register_cleanup "$prompt_file"
    echo "$prompt_file"
}
