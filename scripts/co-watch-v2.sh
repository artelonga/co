#!/usr/bin/env bash
# CO-151 v2 watcher supervisor.
#
# Spawns one `co-agent-watch` per universe and keeps them all running.
# Replaces scripts/co-watch.py (v1 JSON/REST poll). Wire format is now
# protobuf SyncDelta + zstd over a persistent WebSocket; latency drops
# from 4–8s (poll) to <1s (FSEvents push).

set -uo pipefail

BASE="https://co-artelonga.fly.dev"
WS_BASE="wss://co-artelonga.fly.dev/api/v1/sync/ws"
EMAIL="yuri@artelonga.com.br"
COOKIE_FILE="$HOME/.co/cookie.txt"
LOG="$HOME/.co/watch-v2.log"
WATCH_BIN="$HOME/.cargo/bin/co-agent-watch"

# (slug, watch_dir) — single watcher per universe.
#
# Topologia is split into 4 universes (concepts + 3 language planes) per
# its README — each is its own CO universe key matching a sub-directory.
# mbya is the Arandu Mbyá Guarani lexicon project (separate from topologia/
# guarani-mbya/ which is a shallow cross-language anchor layer above it).
REPOS=(
    "quilomboaraucaria|$HOME/projects/quilomboaraucaria"
    "artelonga|$HOME/projects/ArteLonga"
    "rfq|$HOME/projects/rfq-gateway"
    "co|$HOME/projects/co"
    "mbya|$HOME/projects/mbya"
    "concepts|$HOME/projects/topologia/concepts"
    "guarani-mbya|$HOME/projects/topologia/guarani-mbya"
    "portuguese|$HOME/projects/topologia/portuguese"
    "yoruba|$HOME/projects/topologia/yoruba"
)

mkdir -p "$HOME/.co"

log() {
    printf '%s  %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >> "$LOG"
}

cleanup() {
    log "shutdown — killing child watchers"
    pkill -P $$ 2>/dev/null || true
}
trap cleanup EXIT INT TERM

refresh_cookie() {
    local password
    password=$(security find-generic-password -a "$EMAIL" -s co-prod-admin -w 2>/dev/null || true)
    if [ -z "$password" ]; then
        log "FATAL: no keychain entry"
        return 1
    fi
    local code
    code=$(curl -sS -m 30 -c "$COOKIE_FILE" \
        -X POST "$BASE/api/v1/auth/password-login" \
        -H 'Content-Type: application/json' \
        --data-binary @<(printf '{"email":"%s","password":"%s"}' "$EMAIL" "$password") \
        -o /dev/null -w '%{http_code}')
    if [ "$code" != "200" ]; then
        log "login failed: HTTP $code"
        return 1
    fi
    log "cookie refreshed via keychain"
    return 0
}

extract_session() {
    grep "session" "$COOKIE_FILE" 2>/dev/null | tail -1 | awk '{print $NF}'
}

if [ ! -x "$WATCH_BIN" ]; then
    log "FATAL: $WATCH_BIN not found — run: cargo install --path co-agent --bin co-agent-watch"
    exit 1
fi

if [ ! -f "$COOKIE_FILE" ] || [ -z "$(extract_session)" ]; then
    refresh_cookie || exit 1
fi

SESSION=$(extract_session)
if [ -z "$SESSION" ]; then
    log "FATAL: cookie file has no session line"
    exit 1
fi

ln -sf "$COOKIE_FILE" /tmp/c.txt

log "=== co-watch-v2 start (4 universes) ==="

for entry in "${REPOS[@]}"; do
    SLUG="${entry%%|*}"
    DIR="${entry#*|}"
    if [ ! -d "$DIR" ]; then
        log "[$SLUG] skip: $DIR does not exist"
        continue
    fi
    (
        while true; do
            log "[$SLUG] starting watcher"
            "$WATCH_BIN" --universe "$SLUG" --server-url "$WS_BASE" \
                --token "$SESSION" --watch "$DIR" >> "$LOG" 2>&1
            rc=$?
            log "[$SLUG] watcher exited rc=$rc; restart in 5s"
            sleep 5
            refresh_cookie && SESSION=$(extract_session) || true
        done
    ) &
done

# Wait for any child to exit; launchd restarts us if needed.
wait
