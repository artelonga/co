#!/usr/bin/env bash
# smoke-selfhost.sh — CO-536: content-agnostic capability smoke for a self-hosted
# (M4 / single-box) CO prod. This is the REPLACEMENT for the estate-pinned
# scripts/smoke-prod.sh: it asserts the platform *capabilities* (health, a clean
# serve-allowlist, an auth+CRUD round-trip) WITHOUT pinning any seed counts,
# tenant/universe names, or Fly URLs. A self-host box is content-agnostic — it may
# serve zero universes or a hundred — so the smoke proves the engine works, not
# that any particular content exists.
#
# Usage:
#   bash scripts/smoke-selfhost.sh [BASE_URL]
#   BASE_URL=https://co.example.com bash scripts/smoke-selfhost.sh
#
#   # Full round-trip (creates + deletes an EPHEMERAL universe) needs admin creds:
#   CO_SMOKE_EMAIL=you@example.com CO_SMOKE_PASSWORD='…' \
#     bash scripts/smoke-selfhost.sh http://localhost:8742
#
# Defaults:
#   BASE_URL      http://localhost:8742   (1st arg, or $BASE_URL)
#   CO_WEB_DATA   ~/.co/data              (data dir audit_serve inspects)
#
# Checks (all read-only EXCEPT [03], which is fully self-cleaning):
#   [01] GET /api/health → status ok AND version == workspace version (Cargo.toml)
#   [02] serve-allowlist clean (CO-439 audit_serve) — no servable-but-unindexed file
#   [03] capability round-trip (ONLY if CO_SMOKE_EMAIL/CO_SMOKE_PASSWORD set):
#        password-login → create ephemeral universe → add entry → GET it back →
#        delete the universe. Skipped (with a note, still exit 0) if no creds.
#   [04] telemetry/observability liveness (CO-538): the public analytics surface
#        (/api/v1/analytics/public/summary) answers — a 5xx is a HARD fail
#        (telemetry broken). When CO_WEB_DATA + sqlite3 are present, also asserts
#        the telemetry_events table EXISTS in meta.db and reports its row count
#        (a live prod should be > 0 and growing — proves events are recorded).
#        Soft-passes with a note when the surface is non-live or sqlite/DB absent.
#
# Exits 0 on full pass; exits 1 on any failed invariant.
# Output is grep-friendly: each line starts with ✓ or ✗ (matching smoke-prod.sh).
set -uo pipefail

# ── Resolve repo root (this script lives in scripts/) ───────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BASE_URL="${1:-${BASE_URL:-http://localhost:8742}}"
DATA_DIR="${CO_WEB_DATA:-$HOME/.co/data}"

# shellcheck source=./smoke-lib.sh
source "${SCRIPT_DIR}/smoke-lib.sh"

# Workspace version — the single source of truth co-web reports at /api/health.
# Parse the first `version = "…"` under [workspace.package] in Cargo.toml.
ws_version() {
    awk '/^\[workspace\.package\]/{f=1; next}
         f && /^version[[:space:]]*=/{gsub(/[",]/,""); print $3; exit}' \
        "$REPO_ROOT/Cargo.toml"
}

printf 'Self-host smoke: %s\n' "$BASE_URL"
printf '  data dir: %s\n\n' "$DATA_DIR"

# ── [01] Health → ok + version matches the workspace version ─────────────────
{
    expected_version="$(ws_version)"
    tmpfile=$(mktemp)
    status=$(curl -s -o "$tmpfile" -w "%{http_code}" --max-time 10 "${BASE_URL}/api/health" 2>/dev/null) || true
    status="${status:-000}"
    if [[ "$status" != "200" ]]; then
        _fail "01" "/api/health expected HTTP 200, got ${status}"
    else
        api_status=$(python3 -c "import json; print(json.load(open('$tmpfile')).get('status',''))" 2>/dev/null) || true
        api_version=$(python3 -c "import json; print(json.load(open('$tmpfile')).get('version',''))" 2>/dev/null) || true
        if [[ "${api_status:-}" != "ok" ]]; then
            _fail "01" "/api/health status=${api_status:-empty}, expected ok"
        elif [[ -n "$expected_version" && "${api_version:-}" != "$expected_version" ]]; then
            _fail "01" "/api/health version=${api_version:-empty}, expected ${expected_version} (Cargo.toml)"
        else
            _pass "01" "/api/health → 200 (status=ok version=${api_version:-?})"
        fi
    fi
    rm -f "$tmpfile"
}

# ── [02] Serve-allowlist clean (CO-439 audit_serve) ──────────────────────────
# Audits the LOCAL data dir for servable-but-unindexed files (the draft-leak
# surface the serve allowlist refuses). If the data dir isn't present locally
# (e.g. running this smoke from a different box against a remote BASE_URL), the
# disk audit isn't applicable here — skip with a clear note, don't fail.
{
    if [[ ! -f "$DATA_DIR/meta.db" ]]; then
        _pass "02" "audit_serve skipped — no $DATA_DIR/meta.db on this host (set CO_WEB_DATA to audit locally)"
    elif ! command -v cargo >/dev/null 2>&1; then
        _pass "02" "audit_serve skipped — cargo not on PATH (build + run audit_serve manually)"
    else
        if audit_out=$(cd "$REPO_ROOT" && cargo run -q -p co-web --bin audit_serve -- "$DATA_DIR" 2>&1); then
            _pass "02" "serve-allowlist clean — no servable-but-unindexed files (CO-439)"
        else
            _fail "02" "audit_serve found a leak surface (servable-but-unindexed files):"
            printf '%s\n' "$audit_out" | sed 's/^/      /'
        fi
    fi
}

# ── [03] Capability round-trip (opt-in via admin creds) ──────────────────────
# Proves the write path end-to-end on whatever content the box holds, by
# operating ONLY on an ephemeral universe it creates and then deletes. No seed
# content is touched. Skipped (still exit 0) when creds are absent.
{
    if [[ -z "${CO_SMOKE_EMAIL:-}" || -z "${CO_SMOKE_PASSWORD:-}" ]]; then
        _pass "03" "round-trip skipped — set CO_SMOKE_EMAIL + CO_SMOKE_PASSWORD to exercise auth+CRUD"
    else
        cookies=$(mktemp)
        eph_key="smoke-$(date +%s)-${RANDOM}"   # lowercase + digits + hyphen, ≤40 chars
        eph_created=0
        cleanup_rt() {
            # Best-effort teardown: always try to delete the ephemeral universe.
            # Runs on script exit (incl. smoke_summary's exit) so a mid-round-trip
            # failure never leaves the box littered with smoke-* universes.
            if [[ "$eph_created" -eq 1 ]]; then
                curl -s -o /dev/null -b "$cookies" -X DELETE --max-time 10 \
                    "${BASE_URL}/api/v1/universes/${eph_key}" 2>/dev/null || true
            fi
            rm -f "$cookies"
        }
        trap cleanup_rt EXIT

        rt_ok=1

        # (a) password-login → capture the session cookie.
        login_status=$(curl -s -o /dev/null -w "%{http_code}" -c "$cookies" --max-time 10 \
            -X POST -H 'Content-Type: application/json' \
            -d "$(python3 -c 'import json,os; print(json.dumps({"email":os.environ["CO_SMOKE_EMAIL"],"password":os.environ["CO_SMOKE_PASSWORD"]}))')" \
            "${BASE_URL}/api/v1/auth/password-login" 2>/dev/null) || true
        login_status="${login_status:-000}"
        if [[ "$login_status" != "200" ]]; then
            _fail "03" "password-login expected HTTP 200, got ${login_status} (check CO_SMOKE_EMAIL/PASSWORD)"
            rt_ok=0
        fi

        # (b) create the ephemeral universe.
        if [[ "$rt_ok" -eq 1 ]]; then
            create_status=$(curl -s -o /dev/null -w "%{http_code}" -b "$cookies" --max-time 10 \
                -X POST -H 'Content-Type: application/json' \
                -d "{\"key\":\"${eph_key}\",\"name\":\"smoke ephemeral\",\"visibility\":\"private\"}" \
                "${BASE_URL}/api/v1/universes" 2>/dev/null) || true
            create_status="${create_status:-000}"
            if [[ "$create_status" == "200" || "$create_status" == "201" ]]; then
                eph_created=1
            else
                _fail "03" "create universe expected HTTP 200/201, got ${create_status}"
                rt_ok=0
            fi
        fi

        # (c) add one entry.
        entry_path="smoke-note.md"
        if [[ "$rt_ok" -eq 1 ]]; then
            add_status=$(curl -s -o /dev/null -w "%{http_code}" -b "$cookies" --max-time 10 \
                -X POST -H 'Content-Type: application/json' \
                -d "{\"path\":\"${entry_path}\",\"frontmatter\":{\"title\":\"smoke\"},\"body\":\"round-trip\"}" \
                "${BASE_URL}/api/v1/universes/${eph_key}/entries" 2>/dev/null) || true
            add_status="${add_status:-000}"
            if [[ "$add_status" != "200" && "$add_status" != "201" ]]; then
                _fail "03" "add entry expected HTTP 200/201, got ${add_status}"
                rt_ok=0
            fi
        fi

        # (d) GET the entry back.
        if [[ "$rt_ok" -eq 1 ]]; then
            get_status=$(curl -s -o /dev/null -w "%{http_code}" -b "$cookies" --max-time 10 \
                "${BASE_URL}/api/v1/universes/${eph_key}/entries/${entry_path}" 2>/dev/null) || true
            get_status="${get_status:-000}"
            if [[ "$get_status" != "200" ]]; then
                _fail "03" "GET entry back expected HTTP 200, got ${get_status}"
                rt_ok=0
            fi
        fi

        # (e) delete the universe (also covered by the cleanup trap, but assert it here).
        if [[ "$rt_ok" -eq 1 ]]; then
            del_status=$(curl -s -o /dev/null -w "%{http_code}" -b "$cookies" --max-time 10 \
                -X DELETE "${BASE_URL}/api/v1/universes/${eph_key}" 2>/dev/null) || true
            del_status="${del_status:-000}"
            if [[ "$del_status" == "200" || "$del_status" == "204" ]]; then
                eph_created=0   # successfully deleted; nothing for the trap to do
                _pass "03" "round-trip OK — login → create → add → get → delete (${eph_key})"
            else
                _fail "03" "delete universe expected HTTP 200/204, got ${del_status}"
            fi
        fi
    fi
}

# ── [04] Telemetry / observability liveness (CO-538) ─────────────────────────
# Proves observability survived the cutover. (a) The public analytics/telemetry
# surface must answer — a 5xx means telemetry is broken (HARD fail); a non-live or
# remote box that doesn't expose it soft-passes. (b) When the local meta.db is
# reachable, the telemetry_events table must EXIST and we report its row count —
# a live prod should be > 0 and keep growing, which is the proof events are being
# recorded. Absent sqlite3/meta.db → soft-pass with a clear note (exit 0).
{
    tmpfile=$(mktemp)
    tel_status=$(curl -s -o "$tmpfile" -w "%{http_code}" --max-time 10 \
        "${BASE_URL}/api/v1/analytics/public/summary?days=1" 2>/dev/null) || true
    tel_status="${tel_status:-000}"
    if [[ "$tel_status" == "200" ]]; then
        ev_total=$(python3 -c "import json; print(json.load(open('$tmpfile')).get('events_total','?'))" 2>/dev/null) || true
        _pass "04" "/api/v1/analytics/public/summary → 200 (telemetry surface live, events_total=${ev_total:-?})"
    elif [[ "$tel_status" =~ ^5[0-9][0-9]$ ]]; then
        _fail "04" "/api/v1/analytics/public/summary returned ${tel_status} — telemetry surface is FAILING (5xx)"
    else
        _pass "04" "telemetry surface soft-skip — /api/v1/analytics/public/summary got ${tel_status} (not 5xx; not asserting on a non-live/remote box)"
    fi
    rm -f "$tmpfile"

    # (b) Local recording proof — telemetry_events table exists + row count.
    if [[ ! -f "$DATA_DIR/meta.db" ]]; then
        _pass "04" "telemetry_events row-count skipped — no $DATA_DIR/meta.db on this host (set CO_WEB_DATA to check locally)"
    elif ! command -v sqlite3 >/dev/null 2>&1; then
        _pass "04" "telemetry_events row-count skipped — sqlite3 not on PATH"
    else
        has_tbl=$(sqlite3 "$DATA_DIR/meta.db" \
            "SELECT name FROM sqlite_master WHERE type='table' AND name='telemetry_events'" 2>/dev/null) || true
        if [[ "$has_tbl" != "telemetry_events" ]]; then
            _fail "04" "telemetry_events table MISSING from $DATA_DIR/meta.db — telemetry is NOT being recorded"
        else
            ev_rows=$(sqlite3 "$DATA_DIR/meta.db" "SELECT count(*) FROM telemetry_events" 2>/dev/null) || true
            ev_rows="${ev_rows:-?}"
            if [[ "$ev_rows" =~ ^[0-9]+$ ]] && (( ev_rows > 0 )); then
                _pass "04" "telemetry_events present in meta.db — ${ev_rows} rows recorded (live prod should keep growing)"
            else
                _pass "04" "telemetry_events present in meta.db but ${ev_rows} rows (fresh box? expect > 0 on a live prod)"
            fi
        fi
    fi
}

smoke_summary
