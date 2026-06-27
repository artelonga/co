#!/usr/bin/env bash
#
# ab-localhost.sh — A/B-LIVE launcher: the same branch, two fronts, side by side.
#
#   A = CO          via co/scripts/pr-localhost.sh         → band 8900-9699
#   B = yggdrasil   via yggdrasil/scripts/pr-localhost.sh  → band 8100-8899
#
# Open both URLs and compare the experience live. A is always brought up; B is
# best-effort (if the yggdrasil repo / script / branch isn't present, we say so
# and continue with A alone — never fail the whole run).
#
# ── SHARED-SUBSTRATE CONTRACT ────────────────────────────────────────────────
# CO is the AUTHORITATIVE substrate. Both fronts are expected to render the SAME
# synthesized content; CO owns it, yggdrasil mirrors it.
#
#   • The substrate is CO's per-PR data dir:
#         co/.worktrees/_data/branch-<slug>/        (meta.db + per-universe data.db + op_log)
#     — exported to both processes as  CO_SHARED_DATA.
#   • The live read surface is CO's HTTP + event bus:
#         CO_V1_URL = http://localhost:<A-port>     (also exported to B's env)
#     yggdrasil subscribes to CO's event bus / pulls universes from this URL —
#     that live pickup is the yggdrasil agent's job; here we only fix the address
#     and print the contract so both halves agree on WHERE the content lives.
#
# Direction of truth:  CO writes → event bus → yggdrasil reads.  Never the reverse.
#
# Usage:
#   scripts/ab-localhost.sh <pr-number|branch>          # bring up A (+ B best-effort)
#   scripts/ab-localhost.sh <pr-number|branch> --stop    # stop both (delegates to each repo)
#
set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
CO_PR="$REPO_ROOT/scripts/pr-localhost.sh"
YG_ROOT="/Users/artelonga/projects/yggdrasil"
YG_PR="$YG_ROOT/scripts/pr-localhost.sh"

die() { echo "error: $*" >&2; exit 1; }
[ $# -ge 1 ] || die "usage: scripts/ab-localhost.sh <pr-number|branch> [--stop]"

REF="$1"; shift || true
STOP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --stop)    STOP=1 ;;
    -h|--help) sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown flag: $1" ;;
  esac
  shift
done

# ── deterministic ports (mirror each repo's own formula, so we can --port-pin) ─
# CO  : 8900 + key%800   (this repo's pr-localhost.sh)
# YG  : 8100 + key%800   (yggdrasil's pr-localhost.sh)
if [[ "$REF" =~ ^[0-9]+$ ]]; then
  KEY="$REF"
else
  KEY="$(printf '%s' "$REF" | cksum | cut -d' ' -f1)"
fi
A_PORT=$((8900 + KEY % 800))
B_PORT=$((8100 + KEY % 800))
A_URL="http://localhost:$A_PORT"
B_URL="http://localhost:$B_PORT"

# CO's per-branch data dir = the shared substrate location.
if [[ "$REF" =~ ^[0-9]+$ ]]; then SLUG="pr-$REF"; else
  SLUG="branch-$(printf '%s' "$REF" | tr '/ ' '--' | tr -cd 'a-zA-Z0-9-')"
fi
CO_SHARED_DATA="$REPO_ROOT/.worktrees/_data/$SLUG"
export CO_SHARED_DATA CO_V1_URL="$A_URL"

# ── --stop : delegate to each repo's own --stop ──────────────────────────────
if [ "$STOP" = 1 ]; then
  echo "== stopping A (CO) =="
  bash "$CO_PR" "$REF" --stop || true
  echo
  echo "== stopping B (yggdrasil) =="
  if [ -x "$YG_PR" ]; then
    ( cd "$YG_ROOT" && bash "$YG_PR" "$REF" --stop ) || true
  else
    echo "  (yggdrasil pr-localhost.sh not present — nothing to stop)"
  fi
  exit 0
fi

# ── print the contract up front ──────────────────────────────────────────────
cat <<EOF
┌─ A/B-live  ($REF) ──────────────────────────────────────────────
│ shared substrate (CO is authoritative):
│   CO_SHARED_DATA = $CO_SHARED_DATA
│   CO_V1_URL      = $A_URL   (event-bus + HTTP read surface for B)
│   truth flows:   CO writes → event bus → yggdrasil reads
└─────────────────────────────────────────────────────────────────
EOF
echo

# ── A = CO (backgrounds + returns) ───────────────────────────────────────────
echo "== A: CO  →  $A_URL =="
A_UP=0
if bash "$CO_PR" "$REF" --port "$A_PORT"; then
  curl -s --max-time 3 "$A_URL/api/health" >/dev/null 2>&1 && A_UP=1
fi
[ "$A_UP" = 1 ] || echo "⚠️  A (CO) did not come up healthy — see output above"
echo

# ── B = yggdrasil (best-effort; its script foregrounds → background it) ───────
echo "== B: yggdrasil  →  $B_URL =="
B_UP=0
if [ -x "$YG_PR" ]; then
  YG_LOG="${CO_SHARED_DATA}/yggdrasil-ab.log"
  mkdir -p "$CO_SHARED_DATA"
  echo "→ launching yggdrasil (deploys from ~/projects; log: $YG_LOG)…"
  # yggdrasil's script execs `cargo run` in the foreground → run detached.
  # Pass the shared substrate address through the environment for live pickup.
  ( cd "$YG_ROOT" && CO_V1_URL="$A_URL" CO_SHARED_DATA="$CO_SHARED_DATA" \
      nohup bash "$YG_PR" "$REF" --port "$B_PORT" > "$YG_LOG" 2>&1 & echo $! > "$CO_SHARED_DATA/yggdrasil-ab.pid" )
  for _ in $(seq 1 90); do
    curl -s --max-time 2 "$B_URL/" >/dev/null 2>&1 && { B_UP=1; break; }
    # process died early? stop waiting
    pid="$(cat "$CO_SHARED_DATA/yggdrasil-ab.pid" 2>/dev/null)"
    [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null && break
    sleep 1
  done
  if [ "$B_UP" = 1 ]; then
    echo "✓ yggdrasil UP on :$B_PORT"
  else
    echo "⚠️  yggdrasil did not come up (branch may not exist in yg, or build is slow)."
    echo "    last lines of $YG_LOG:"
    tail -6 "$YG_LOG" 2>/dev/null | sed 's/^/      /' || true
    echo "    → continuing with A only."
  fi
else
  echo "ℹ️  yggdrasil pr-localhost.sh not found at $YG_PR — A-only mode."
fi

# ── side-by-side summary ─────────────────────────────────────────────────────
echo
echo "──────────────────────────────────────────────"
printf '  A (CO)         %s   %s\n' "$([ "$A_UP" = 1 ] && echo '✅' || echo '❌')" "$A_URL"
printf '  B (yggdrasil)  %s   %s\n' "$([ "$B_UP" = 1 ] && echo '✅' || echo '— (not up)')" "$B_URL"
echo "──────────────────────────────────────────────"
if [ "$A_UP" = 1 ] && [ "$B_UP" = 1 ]; then
  echo "  open these to compare A/B live:  $A_URL   vs   $B_URL"
elif [ "$A_UP" = 1 ]; then
  echo "  open A live:  $A_URL   (B unavailable — A-only)"
fi
echo "  stop both:  scripts/ab-localhost.sh $REF --stop"
exit 0
