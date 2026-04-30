#!/usr/bin/env bash
# smoke-lib.sh — shared helpers for smoke-{prod,uat}.sh
# Source this file; do not execute directly.
# Requires: BASE_URL exported before sourcing.

SMOKE_FAILURES=0

_pass() { printf '✓ [%s] %s\n' "$1" "$2"; }
_fail() {
    printf '✗ [%s] %s  ← FAIL\n' "$1" "$2"
    SMOKE_FAILURES=$((SMOKE_FAILURES + 1))
}

# check_status NUM PATH EXPECTED_CODE [DETAIL]
# Verifies the HTTP status code returned by GET PATH.
check_status() {
    local num="$1" path="$2" expected="$3" detail="${4:-}"
    local status
    status=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 "${BASE_URL}${path}" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" == "$expected" ]]; then
        local msg="${path} → ${status}"
        [[ -n "$detail" ]] && msg+=" (${detail})"
        _pass "$num" "$msg"
    else
        _fail "$num" "${path} expected HTTP ${expected}, got ${status}"
    fi
}

# check_json_field NUM PATH PYTHON3_EXPR EXPECTED [LABEL]
# GETs PATH, parses JSON body, evaluates PYTHON3_EXPR (d = parsed dict),
# and compares the printed result to EXPECTED.
check_json_field() {
    local num="$1" path="$2" expr="$3" expected="$4" label="${5:-${expr}}"
    local url="${BASE_URL}${path}"
    local tmpfile status actual
    tmpfile=$(mktemp)
    status=$(curl -s -o "$tmpfile" -w "%{http_code}" --max-time 10 "$url" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" != "200" ]]; then
        _fail "$num" "${path} expected HTTP 200, got ${status}"
        rm -f "$tmpfile"
        return 0
    fi
    actual=$(python3 -c "import json; d=json.load(open('$tmpfile')); print($expr)" 2>/dev/null) || true
    actual="${actual:-PARSE_ERROR}"
    rm -f "$tmpfile"
    if [[ "$actual" == "$expected" ]]; then
        _pass "$num" "${path} ${label}=${actual}"
    else
        _fail "$num" "${path} expected ${label}=${expected}, got ${actual}"
    fi
}

# check_count NUM PATH PYTHON3_EXPR EXPECTED [LABEL]
# GETs PATH, parses JSON body, evaluates PYTHON3_EXPR (d = parsed dict),
# and compares the numeric result exactly to EXPECTED.
check_count() {
    local num="$1" path="$2" expr="$3" expected="$4" label="${5:-count}"
    local url="${BASE_URL}${path}"
    local tmpfile status actual
    tmpfile=$(mktemp)
    status=$(curl -s -o "$tmpfile" -w "%{http_code}" --max-time 10 "$url" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" != "200" ]]; then
        _fail "$num" "${path} expected HTTP 200, got ${status}"
        rm -f "$tmpfile"
        return 0
    fi
    actual=$(python3 -c "import json; d=json.load(open('$tmpfile')); print($expr)" 2>/dev/null) || true
    actual="${actual:-PARSE_ERROR}"
    rm -f "$tmpfile"
    if [[ "$actual" == "$expected" ]]; then
        _pass "$num" "${path} ${label} = ${actual}"
    else
        _fail "$num" "${path} expected ${label} ${expected}, got ${actual}"
    fi
}

# check_count_gte NUM PATH PYTHON3_EXPR MINIMUM [LABEL]
# Like check_count but passes when result >= MINIMUM.
check_count_gte() {
    local num="$1" path="$2" expr="$3" minimum="$4" label="${5:-count}"
    local url="${BASE_URL}${path}"
    local tmpfile status actual
    tmpfile=$(mktemp)
    status=$(curl -s -o "$tmpfile" -w "%{http_code}" --max-time 10 "$url" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" != "200" ]]; then
        _fail "$num" "${path} expected HTTP 200, got ${status}"
        rm -f "$tmpfile"
        return 0
    fi
    actual=$(python3 -c "import json; d=json.load(open('$tmpfile')); print($expr)" 2>/dev/null) || true
    actual="${actual:-PARSE_ERROR}"
    rm -f "$tmpfile"
    if [[ "$actual" =~ ^[0-9]+$ ]] && (( actual >= minimum )); then
        _pass "$num" "${path} ${label} = ${actual} (≥${minimum} ✓)"
    else
        _fail "$num" "${path} expected ${label} >= ${minimum}, got ${actual}"
    fi
}

# check_post_status NUM PATH JSON_BODY EXPECTED_CODE [DETAIL]
# POSTs JSON_BODY to PATH and verifies the HTTP status code.
check_post_status() {
    local num="$1" path="$2" body="$3" expected="$4" detail="${5:-}"
    local status
    status=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 \
        -X POST -H 'Content-Type: application/json' -d "$body" "${BASE_URL}${path}" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" == "$expected" ]]; then
        local msg="${path} POST → ${status}"
        [[ -n "$detail" ]] && msg+=" (${detail})"
        _pass "$num" "$msg"
    else
        _fail "$num" "${path} POST expected HTTP ${expected}, got ${status}"
    fi
}

# smoke_summary — print footer and exit 0 (all pass) or 1 (any failure).
smoke_summary() {
    echo ""
    if [[ "$SMOKE_FAILURES" -eq 0 ]]; then
        echo "--- ALL CHECKS PASSED ---"
        exit 0
    else
        echo "--- FAILED (${SMOKE_FAILURES} check(s) failed) ---"
        exit 1
    fi
}
