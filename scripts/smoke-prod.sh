#!/usr/bin/env bash
# smoke-prod.sh — post-deploy regression smoke test for production.
#
# Usage:
#   bash scripts/smoke-prod.sh
#   BASE_URL=https://co-artelonga.fly.dev bash scripts/smoke-prod.sh
#
# Exits 0 on full pass; exits 1 on any failure.
# Output is grep-friendly: each line starts with ✓ or ✗.
set -uo pipefail

BASE_URL="${BASE_URL:-https://co.artelonga.com.br}"

# Pinned event counts — update here whenever seed JSON changes (same commit).
EXPECTED_TEMPO_EVENTS=21
EXPECTED_HUMANITY_EVENTS=26
EXPECTED_UNIVERSO_EVENTS=28
EXPECTED_SW_CACHE_NAME='co-v5-offline'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./smoke-lib.sh
source "${SCRIPT_DIR}/smoke-lib.sh"

printf 'Smoke test: %s\n\n' "$BASE_URL"

# ── [01] Health ──────────────────────────────────────────────────────────────
{
    tmpfile=$(mktemp)
    status=$(curl -s -o "$tmpfile" -w "%{http_code}" --max-time 10 "${BASE_URL}/api/health" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" == "200" ]]; then
        api_status=$(python3 -c "import json; d=json.load(open('$tmpfile')); print(d.get('status',''))" 2>/dev/null) || true
        api_version=$(python3 -c "import json; d=json.load(open('$tmpfile')); print(d.get('version','?'))" 2>/dev/null) || true
        if [[ "${api_status:-}" == "ok" ]]; then
            _pass "01" "/api/health → 200 (version ${api_version:-?})"
        else
            _fail "01" "/api/health status=${api_status:-empty}, expected ok"
        fi
    else
        _fail "01" "/api/health expected HTTP 200, got ${status}"
    fi
    rm -f "$tmpfile"
}

# ── [02] Health-deep ─────────────────────────────────────────────────────────
{
    tmpfile=$(mktemp)
    status=$(curl -s -o "$tmpfile" -w "%{http_code}" --max-time 10 "${BASE_URL}/api/health/deep" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" == "200" ]]; then
        db=$(python3 -c "import json; d=json.load(open('$tmpfile')); print(d.get('db','?'))" 2>/dev/null) || true
        disk=$(python3 -c "import json; d=json.load(open('$tmpfile')); print(d.get('disk','?'))" 2>/dev/null) || true
        if [[ "${db:-?}" == "ok" && "${disk:-?}" == "ok" ]]; then
            _pass "02" "/api/health/deep → 200 (db=ok disk=ok)"
        else
            _fail "02" "/api/health/deep db=${db:-?} disk=${disk:-?}, expected both ok"
        fi
    else
        _fail "02" "/api/health/deep expected HTTP 200, got ${status}"
    fi
    rm -f "$tmpfile"
}

# ── [03] Template universe present ───────────────────────────────────────────
{
    tmpfile=$(mktemp)
    status=$(curl -s -o "$tmpfile" -w "%{http_code}" --max-time 10 "${BASE_URL}/api/v1/universes/template" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" == "200" ]]; then
        name=$(python3 -c "import json; d=json.load(open('$tmpfile')); print(d.get('name',''))" 2>/dev/null) || true
        if [[ -n "${name:-}" ]]; then
            _pass "03" "/api/v1/universes/template → 200 (name=${name})"
        else
            _fail "03" "/api/v1/universes/template name field missing or empty"
        fi
    else
        _fail "03" "/api/v1/universes/template expected HTTP 200, got ${status}"
    fi
    rm -f "$tmpfile"
}

# ── [04] Timeline trio present and shaped ────────────────────────────────────
for universe_key in tempo humanity universo; do
    case "$universe_key" in
        tempo)    expected_events=$EXPECTED_TEMPO_EVENTS ;;
        humanity) expected_events=$EXPECTED_HUMANITY_EVENTS ;;
        universo) expected_events=$EXPECTED_UNIVERSO_EVENTS ;;
    esac

    # Visibility check
    check_json_field "04" "/api/v1/universes/${universe_key}" \
        "d.get('visibility','')" "public-static" "${universe_key}.visibility"

    # Event count check — counts pinned at top of script; update here when seed changes
    check_count "04" "/api/v1/universes/${universe_key}/entries?type=event" \
        "d['total']" "$expected_events" "${universe_key} events"
done

# ── [05] Themes endpoint live ─────────────────────────────────────────────────
{
    tmpfile=$(mktemp)
    meta=$(curl -s -o "$tmpfile" -w "%{http_code}|%{content_type}" --max-time 10 \
        "${BASE_URL}/api/v1/themes/modern" 2>/dev/null) || true
    status="${meta%%|*}"
    status="${status:-000}"
    ct="${meta#*|}"
    body=$(cat "$tmpfile" 2>/dev/null) || true
    rm -f "$tmpfile"

    if [[ "$status" != "200" ]]; then
        _fail "05" "/api/v1/themes/modern expected HTTP 200, got ${status}"
    elif [[ "$ct" != *"text/css"* ]]; then
        _fail "05" "/api/v1/themes/modern expected content-type text/css, got ${ct}"
    elif ! printf '%s' "$body" | grep -qF -- '--accent: #6366f1'; then
        _fail "05" "/api/v1/themes/modern body missing '--accent: #6366f1'"
    else
        _pass "05" "/api/v1/themes/modern → 200 text/css (--accent:#6366f1 ✓)"
    fi
}

# ── [06] Static assets reachable ─────────────────────────────────────────────
for asset_spec in '/:text/html' '/app.js:application/javascript' '/shared/timeline.html:'; do
    asset_path="${asset_spec%%:*}"
    expected_ct="${asset_spec#*:}"

    meta=$(curl -s -o /dev/null -w "%{http_code}|%{content_type}" --max-time 10 \
        "${BASE_URL}${asset_path}" 2>/dev/null) || true
    status="${meta%%|*}"
    status="${status:-000}"
    ct="${meta#*|}"

    if [[ "$status" != "200" ]]; then
        _fail "06" "${asset_path} expected HTTP 200, got ${status}"
    elif [[ -n "$expected_ct" && "$ct" != *"$expected_ct"* ]]; then
        _fail "06" "${asset_path} expected content-type ${expected_ct}, got ${ct}"
    else
        if [[ -n "$expected_ct" ]]; then
            _pass "06" "${asset_path} → 200 ${ct}"
        else
            _pass "06" "${asset_path} → 200"
        fi
    fi
done

# ── [07] Service worker is the network-first one ─────────────────────────────
{
    body=$(curl -s --max-time 10 "${BASE_URL}/sw.js" 2>/dev/null) || true
    if printf '%s' "$body" | grep -qF "$EXPECTED_SW_CACHE_NAME"; then
        _pass "07" "/sw.js contains '${EXPECTED_SW_CACHE_NAME}'"
    else
        _fail "07" "/sw.js expected to contain '${EXPECTED_SW_CACHE_NAME}'"
    fi
}

# ── [08] Login endpoint reachable (bogus credentials → 401, not 5xx) ─────────
check_post_status "08" "/api/v1/auth/password-login" \
    '{"email":"smoke-test@example.com","password":"definitely-wrong"}' \
    "401" "auth reachable"

# ── [09] Conteúdo view loads ──────────────────────────────────────────────────
check_count_gte "09" "/api/v1/universes/template/entries" \
    "d['total']" "14" "template entries"

# ── [10] Static favicon present ──────────────────────────────────────────────
check_status "10" "/shared/favicon.svg" "200"

# ── [11] Public universes reachable anonymously (CO-142 Phase A) ─────────────
for pub_key in template quilomboaraucaria co tempo humanity universo; do
    check_status "11" "/api/v1/universes/${pub_key}" "200" "${pub_key} public"
done

# ── [12] template content_count reflects actual entries (CO-142 Phase B) ─────
check_count_gte "12" "/api/v1/universes/template" \
    "d.get('content_count', d.get('contentCount', 0))" "6" "template content_count"

smoke_summary
