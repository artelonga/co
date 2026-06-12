#!/usr/bin/env bash
#
# uptime-probe.sh — external health probe for CO (CO-407)
#
# Probes a CO deployment's /api/health endpoint from OUTSIDE the prod machine
# (run by a GitHub Actions cron). Requires N *consecutive* failures within a
# single run before declaring "down", so a single transient blip never raises a
# false alarm. Tracks a tiny JSON state file across runs (persisted via the GH
# Actions cache) so it can emit a `health.recovered` notice with the downtime
# duration when the endpoint comes back.
#
# It is deliberately dependency-light: bash + curl + (optionally) jq. If jq is
# absent it falls back to a grep/sed state parser.
#
# Usage:
#   uptime-probe.sh <name> <url> <state-file>
#     name        Human label, e.g. "prod" or "staging".
#     url         Health endpoint, e.g. https://co.artelonga.com.br/api/health
#     state-file  Path to a JSON file persisted across runs.
#
# Environment:
#   CHECKS         Consecutive in-run checks required (default 3).
#   CHECK_INTERVAL Seconds between in-run checks (default 20).
#   CURL_TIMEOUT   Per-request timeout in seconds (default 10).
#   PRIORITY       "high" (push immediately) or "digest" (low priority, no push).
#                  Defaults to "high".
#   RESEND_API_KEY If set, alerts are sent by e-mail via the Resend HTTP API.
#   ALERT_FROM     From address for Resend (default onboarding@resend.dev).
#   ALERT_TO       Recipient for Resend (default yuri@artelonga.com.br).
#   NTFY_URL       If set, alerts are POSTed to this ntfy topic URL.
#
# Exit status: 0 always (a failed probe is a notification, not a CI failure —
# we never want the alerting job itself to go red and get muted).

set -uo pipefail

NAME="${1:?usage: uptime-probe.sh <name> <url> <state-file>}"
URL="${2:?missing url}"
STATE_FILE="${3:?missing state-file}"

CHECKS="${CHECKS:-3}"
CHECK_INTERVAL="${CHECK_INTERVAL:-20}"
CURL_TIMEOUT="${CURL_TIMEOUT:-10}"
PRIORITY="${PRIORITY:-high}"
ALERT_FROM="${ALERT_FROM:-onboarding@resend.dev}"
ALERT_TO="${ALERT_TO:-yuri@artelonga.com.br}"

now_iso() { date -u +"%Y-%m-%dT%H:%M:%SZ"; }
now_epoch() { date -u +%s; }

log() { echo "[$(now_iso)] $NAME: $*"; }

# --- one HTTP probe -> 0 healthy, 1 unhealthy -------------------------------
probe_once() {
  local code
  code=$(curl -fsS -o /dev/null -m "$CURL_TIMEOUT" -w "%{http_code}" "$URL" 2>/dev/null) || return 1
  [ "$code" = "200" ]
}

# --- N consecutive probes ---------------------------------------------------
# Returns 0 if healthy on any check (we only need one good response to be "up").
# Returns 1 only if ALL $CHECKS checks fail consecutively (true outage).
is_down() {
  local i
  for ((i = 1; i <= CHECKS; i++)); do
    if probe_once; then
      log "check $i/$CHECKS OK"
      return 1 # at least one healthy -> not down
    fi
    log "check $i/$CHECKS FAILED"
    [ "$i" -lt "$CHECKS" ] && sleep "$CHECK_INTERVAL"
  done
  return 0 # all checks failed -> down
}

# --- state helpers ----------------------------------------------------------
# State file shape: {"status":"up"|"down","down_since":"<iso>","down_since_epoch":<n>}
read_down_since_epoch() {
  [ -f "$STATE_FILE" ] || { echo ""; return; }
  if command -v jq >/dev/null 2>&1; then
    jq -r 'if .status == "down" then (.down_since_epoch // empty) else empty end' \
      "$STATE_FILE" 2>/dev/null
  else
    grep -q '"status":[[:space:]]*"down"' "$STATE_FILE" 2>/dev/null || { echo ""; return; }
    grep -o '"down_since_epoch":[[:space:]]*[0-9]*' "$STATE_FILE" 2>/dev/null \
      | grep -o '[0-9]*' | head -1
  fi
}

write_state() {
  local status="$1" since_iso="$2" since_epoch="$3"
  printf '{"status":"%s","down_since":"%s","down_since_epoch":%s}\n' \
    "$status" "$since_iso" "${since_epoch:-0}" >"$STATE_FILE"
}

fmt_duration() {
  local s=$1 h m
  h=$((s / 3600))
  m=$(((s % 3600) / 60))
  s=$((s % 60))
  printf '%dh%02dm%02ds' "$h" "$m" "$s"
}

# --- notification channels --------------------------------------------------
notify() {
  local subject="$1" body="$2"

  # Always emit a GH Actions annotation so the run is self-documenting even with
  # no channel configured. ::error:: for high-priority down/up transitions.
  if [ "$PRIORITY" = "high" ]; then
    echo "::error title=${subject}::${body}"
  else
    echo "::warning title=${subject}::${body}"
  fi
  # Append to the job summary for the digest view.
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    {
      echo "### ${subject}"
      echo ""
      echo "${body}"
      echo ""
    } >>"$GITHUB_STEP_SUMMARY"
  fi

  # Digest-priority targets (staging) never push — annotation/summary only.
  if [ "$PRIORITY" != "high" ]; then
    log "digest-only: $subject"
    return
  fi

  if [ -n "${RESEND_API_KEY:-}" ]; then
    log "sending e-mail via Resend -> $ALERT_TO"
    local payload
    payload=$(printf '{"from":"%s","to":["%s"],"subject":"%s","text":"%s"}' \
      "$ALERT_FROM" "$ALERT_TO" "${subject//\"/\\\"}" "${body//\"/\\\"}")
    curl -fsS -X POST "https://api.resend.com/emails" \
      -H "Authorization: Bearer ${RESEND_API_KEY}" \
      -H "Content-Type: application/json" \
      -d "$payload" >/dev/null \
      && log "Resend e-mail sent" \
      || log "Resend send FAILED (alert still annotated)"
  fi

  if [ -n "${NTFY_URL:-}" ]; then
    log "pushing to ntfy"
    curl -fsS -X POST "$NTFY_URL" \
      -H "Title: ${subject}" \
      -H "Priority: high" \
      -H "Tags: warning" \
      -d "$body" >/dev/null \
      && log "ntfy push sent" \
      || log "ntfy push FAILED (alert still annotated)"
  fi

  if [ -z "${RESEND_API_KEY:-}" ] && [ -z "${NTFY_URL:-}" ]; then
    log "no RESEND_API_KEY / NTFY_URL configured — alert is annotation-only"
  fi
}

# --- main state machine -----------------------------------------------------
main() {
  local prev_down_epoch ts_iso ts_epoch
  prev_down_epoch=$(read_down_since_epoch)
  ts_iso=$(now_iso)
  ts_epoch=$(now_epoch)

  if is_down; then
    if [ -n "$prev_down_epoch" ]; then
      # Already known down — keep the original down_since (still in outage).
      local elapsed
      elapsed=$(fmt_duration $((ts_epoch - prev_down_epoch)))
      log "STILL DOWN (~$elapsed so far)"
      # Leave the state file unchanged so down_since is preserved. If the cache
      # was lost between runs the file may be absent; recreate it as down-now.
      [ -f "$STATE_FILE" ] || write_state down "$ts_iso" "$ts_epoch"
    else
      # New outage.
      log "DOWN — $CHECKS consecutive failures"
      write_state down "$ts_iso" "$ts_epoch"
      notify "health.down — ${NAME} is DOWN" \
        "CO ${NAME} health probe failed ${CHECKS} consecutive checks. URL: ${URL}. Detected at ${ts_iso}."
    fi
  else
    if [ -n "$prev_down_epoch" ]; then
      # Recovery transition.
      local dur
      dur=$(fmt_duration $((ts_epoch - prev_down_epoch)))
      log "RECOVERED after $dur"
      write_state up "" ""
      notify "health.recovered — ${NAME} is back UP" \
        "CO ${NAME} recovered at ${ts_iso}. Downtime: ${dur}. URL: ${URL}."
    else
      log "healthy"
      write_state up "" ""
    fi
  fi
}

main
