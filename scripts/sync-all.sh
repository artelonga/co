#!/usr/bin/env bash
# CO auto-sync: weekly upload of 4 admin-content repos to prod.
#
# Pulls each repo, refreshes the admin session via macOS keychain, runs
# bulk-upload-binary.py for each universe. Logs everything to ~/.co/sync.log.
#
# Triggered weekly by ~/Library/LaunchAgents/com.artelonga.co-sync.plist.
# Run manually: bash ~/projects/co/scripts/sync-all.sh
#
# Keychain prep (one-time):
#   security add-generic-password -a yuri@artelonga.com.br \
#                                 -s co-prod-admin -w '<password>' -U
#
# To uninstall the schedule:
#   launchctl unload ~/Library/LaunchAgents/com.artelonga.co-sync.plist
#   rm ~/Library/LaunchAgents/com.artelonga.co-sync.plist

set -uo pipefail

BASE="https://co-artelonga.fly.dev"
EMAIL="yuri@artelonga.com.br"
COOKIE_FILE="$HOME/.co/cookie.txt"
LOG="$HOME/.co/sync.log"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)"
UPLOADER="$SCRIPT_DIR/bulk-upload-binary.py"

# Order matters: pairs of (slug, local-repo-path).
REPOS=(
    "quilomboaraucaria|$HOME/projects/quilomboaraucaria"
    "artelonga|$HOME/projects/ArteLonga"
    "rfq|$HOME/projects/rfq-gateway"
    "co|$HOME/projects/co"
)

mkdir -p "$HOME/.co"

log() {
    printf '%s  %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >> "$LOG"
}

log "=== sync-all start ==="

# 1. Fetch password from keychain.
PASSWORD=$(security find-generic-password -a "$EMAIL" -s co-prod-admin -w 2>/dev/null || true)
if [ -z "$PASSWORD" ]; then
    log "FATAL: no keychain entry for service=co-prod-admin account=$EMAIL"
    log "Run: security add-generic-password -a $EMAIL -s co-prod-admin -w '<password>' -U"
    exit 1
fi

# 2. Login → fresh cookie.
HTTP_CODE=$(curl -sS -m 30 -c "$COOKIE_FILE" \
    -X POST "$BASE/api/v1/auth/password-login" \
    -H 'Content-Type: application/json' \
    --data-binary @<(printf '{"email":"%s","password":"%s"}' "$EMAIL" "$PASSWORD") \
    -o /dev/null -w '%{http_code}')
if [ "$HTTP_CODE" != "200" ]; then
    log "FATAL: password-login returned HTTP $HTTP_CODE"
    exit 1
fi
log "login ok (HTTP 200)"

# 3. The bulk-upload script reads from /tmp/c.txt — symlink for compatibility.
ln -sf "$COOKIE_FILE" /tmp/c.txt

# 4. Pull + upload each repo.
for entry in "${REPOS[@]}"; do
    SLUG="${entry%%|*}"
    REPO="${entry#*|}"
    log "--- $SLUG ($REPO) ---"
    if [ ! -d "$REPO" ]; then
        log "  skip: $REPO does not exist"
        continue
    fi
    if [ -d "$REPO/.git" ]; then
        if (cd "$REPO" && git pull --ff-only --quiet 2>>"$LOG"); then
            log "  git pull ok"
        else
            log "  git pull FAILED (continuing with local state)"
        fi
    else
        log "  no .git — skipping pull"
    fi
    if python3 "$UPLOADER" "$SLUG" "$REPO" "$BASE" >>"$LOG" 2>&1; then
        log "  upload ok"
    else
        log "  upload FAILED (rc=$?)"
    fi
done

log "=== sync-all done ==="
