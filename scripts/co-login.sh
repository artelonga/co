#!/usr/bin/env bash
# co-login — magic-code login against a local co-web; saves the session cookie.
# Local dev only: with no RESEND key the server's LogMailProvider prints the code
# to the co-local log, so this reads it from there automatically.
#   Usage: scripts/co-login.sh [email] [port] [logfile]
#   Defaults: yuri@artelonga.com.br  :${CO_WEB_PORT:-3000}  /tmp/co-local.log
set -euo pipefail
EMAIL="${1:-yuri@artelonga.com.br}"
PORT="${2:-${CO_WEB_PORT:-3000}}"
LOG="${3:-/tmp/co-local.log}"
B="http://localhost:$PORT"; J='Content-Type: application/json'
COOKIE="$HOME/.co/local-cookies.txt"; mkdir -p "$HOME/.co"
curl -s -X POST "$B/api/v1/auth/login" -H "$J" -d "{\"email\":\"$EMAIL\"}" >/dev/null
sleep 1
CODE=""
if [ -f "$LOG" ]; then CODE=$(grep -iA4 '\[MAIL\]' "$LOG" | grep -oE '\b[0-9]{6}\b' | tail -1); fi
if [ -z "$CODE" ]; then read -r -p "6-digit code (see co-local log [MAIL]): " CODE; fi
curl -s -c "$COOKIE" -X POST "$B/api/v1/auth/verify" -H "$J" \
  -d "{\"email\":\"$EMAIL\",\"code\":\"$CODE\"}" && echo
echo "Session → $COOKIE   e.g.:  curl -b $COOKIE $B/api/v1/me/universes"
