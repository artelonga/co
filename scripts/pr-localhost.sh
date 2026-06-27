#!/usr/bin/env bash
#
# pr-localhost.sh — boot an ISOLATED localhost deployment of co-web for a PR (or
# branch). The CO half of the A/B-live pair; mirrors yggdrasil/scripts/pr-localhost.sh
# so the two can run SIDE BY SIDE without colliding.
#
# Each PR runs in its own git worktree, on a deterministic port, with its own
# data dir (meta.db + per-universe content) — several PRs at once, no clobber.
# Review a PR for real in the browser, or compare conflicting PRs, before any
# Fly deploy.
#
# Usage:
#   scripts/pr-localhost.sh <pr-number|branch>              # build + run (backgrounded, returns)
#   scripts/pr-localhost.sh <pr-number|branch> --build-only
#   scripts/pr-localhost.sh <pr-number|branch> --port 9123
#   scripts/pr-localhost.sh <pr-number|branch> --with-bot   # also boot the whatsapp-bot bridge
#   scripts/pr-localhost.sh <pr-number|branch> --stop        # kill proc + remove worktree + temp data
#   scripts/pr-localhost.sh --list                           # active per-PR deployments + ports
#
# Notes:
# - PORT BAND 8900-9699  =  8900 + (pr_number | hash(branch)) % 800.  This band is
#   DISJOINT from yggdrasil's 8100-8899, so CO (A) and yggdrasil (B) coexist.
#   CO's own default 8742 sits inside yg's band and is deliberately NOT used here.
# - Unlike yggdrasil's script (which foregrounds `cargo run`), this BACKGROUNDS
#   co-web and returns once /api/health is green — so you can launch several and
#   manage them with --list / --stop, and so ab-localhost.sh can chain A then B.
# - Dev JWT secret default (JWT_SECRET); override by exporting before running.
# - Worktrees live in .worktrees/<slug>; data in .worktrees/_data/<slug>.
# - CARGO_TARGET_DIR is SHARED (target/) → fast incremental builds; a binary
#   already loaded in memory survives another PR's rebuild.
set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

DATA_BASE="$REPO_ROOT/.worktrees/_data"
PORT_LO=8900            # CO band start (disjoint from yggdrasil's 8100-8899)
PORT_SPAN=800          # → 8900-9699

die()   { echo "error: $*" >&2; exit 1; }
usage() { sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-1}"; }

# ── --list ───────────────────────────────────────────────────────────────────
if [ "${1:-}" = "--list" ]; then
  echo "co-web per-PR deployments (worktrees):"
  git worktree list | grep -F "/.worktrees/" | grep -v "/_data/" || echo "  (none)"
  echo
  echo "tracked processes:"
  shopt -s nullglob
  found=0
  for pf in "$DATA_BASE"/*/co-web.pid; do
    slug="$(basename "$(dirname "$pf")")"
    pid="$(cat "$pf" 2>/dev/null)"
    port="$(cat "$(dirname "$pf")/co-web.port" 2>/dev/null || echo '?')"
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      echo "  ✅ $slug  pid=$pid  http://localhost:$port"
      found=1
    else
      echo "  ⚰️  $slug  (stale pid file; port $port) — run --stop to clean"
    fi
  done
  [ "$found" = 0 ] && echo "  (no live co-web processes)"
  shopt -u nullglob
  echo
  echo "listeners in CO band $PORT_LO-$((PORT_LO+PORT_SPAN-1)):"
  lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null | awk '/:(89|9[0-6])[0-9][0-9] /{print "  "$1" "$9}' || true
  exit 0
fi

[ $# -ge 1 ] || usage
REF="$1"; shift || true

STOP=0; BUILD_ONLY=0; WITH_BOT=0; PORT_OVERRIDE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --stop)       STOP=1 ;;
    --build-only) BUILD_ONLY=1 ;;
    --with-bot)   WITH_BOT=1 ;;
    --port)       PORT_OVERRIDE="${2:-}"; shift ;;
    -h|--help)    usage 0 ;;
    *) die "unknown flag: $1" ;;
  esac
  shift
done

# ── resolve REF → branch + slug + port key ───────────────────────────────────
# A bare integer is treated as a PR number IF gh is available; otherwise (and for
# any non-numeric arg) the arg is taken literally as a branch name.
if [[ "$REF" =~ ^[0-9]+$ ]] && command -v gh >/dev/null 2>&1 \
     && BRANCH="$(gh pr view "$REF" --json headRefName -q .headRefName 2>/dev/null)" \
     && [ -n "$BRANCH" ]; then
  SLUG="pr-$REF"
  PORT_KEY="$REF"
else
  BRANCH="$REF"
  SLUG="branch-$(printf '%s' "$REF" | tr '/ ' '--' | tr -cd 'a-zA-Z0-9-')"
  PORT_KEY="$(printf '%s' "$REF" | cksum | cut -d' ' -f1)"
fi

WT="$REPO_ROOT/.worktrees/$SLUG"
DATA="$DATA_BASE/$SLUG"
PORT="${PORT_OVERRIDE:-$((PORT_LO + PORT_KEY % PORT_SPAN))}"
PIDFILE="$DATA/co-web.pid"
BOT_PIDFILE="$DATA/bot.pid"

# ── --stop ───────────────────────────────────────────────────────────────────
if [ "$STOP" = 1 ]; then
  for pf in "$BOT_PIDFILE" "$PIDFILE"; do
    [ -f "$pf" ] || continue
    pid="$(cat "$pf" 2>/dev/null)"
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      echo "killing $(basename "$pf" .pid) (pid $pid)…"; kill "$pid" 2>/dev/null || true
    fi
  done
  # belt-and-suspenders: anything still listening on the computed port
  if stray="$(lsof -nP -tiTCP:"$PORT" -sTCP:LISTEN 2>/dev/null)"; then
    [ -n "$stray" ] && { echo "killing stray listener on :$PORT (pid $stray)…"; kill $stray 2>/dev/null || true; }
  fi
  if [ -d "$WT" ]; then
    echo "removing worktree ${WT}…"; git worktree remove --force "$WT" 2>/dev/null || rm -rf "$WT"
  fi
  if [ -d "$DATA" ]; then
    echo "removing temp data ${DATA}…"; rm -rf "$DATA"
  fi
  echo "✓ stopped + cleaned ($SLUG)"
  exit 0
fi

# ── fetch + worktree (detached on the branch HEAD) ───────────────────────────
# The branch may live only on origin, only locally, or both. Prefer origin when
# we can fetch it (fresh PR head); fall back to the local ref (local-only branch).
echo "→ PR/branch: $BRANCH   slug: $SLUG   port: $PORT"
if git fetch -q origin "$BRANCH" 2>/dev/null; then
  REFSPEC="origin/$BRANCH"
else
  REFSPEC="$BRANCH"
fi
git rev-parse --verify -q "${REFSPEC}^{commit}" >/dev/null \
  || die "cannot resolve branch ref '$REFSPEC' (not on origin and no local branch '$BRANCH')"

if [ -d "$WT" ]; then
  echo "→ refreshing existing worktree…"
  git -C "$WT" reset -q --hard "$REFSPEC"
else
  echo "→ creating worktree at ${WT}…"
  git worktree add -q --detach "$WT" "$REFSPEC" || die "git worktree add failed"
fi

mkdir -p "$DATA"

# ── isolated env ─────────────────────────────────────────────────────────────
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
WEB="$CARGO_TARGET_DIR/debug/co-web"

echo "→ cargo build -p co-web (shared target, incremental)…"
( cd "$WT" && cargo build -q -p co-web ) || die "build failed on $BRANCH"
[ -x "$WEB" ] || die "co-web binary not found at $WEB after build"

if [ "$BUILD_ONLY" = 1 ]; then
  echo "✓ build OK ($BRANCH). Run with:  scripts/pr-localhost.sh $REF  (port $PORT)"
  exit 0
fi

# ── boot co-web (backgrounded) + wait for health ─────────────────────────────
# If something is already serving this port from a previous run, reuse-by-replace.
if old="$(cat "$PIDFILE" 2>/dev/null)"; then
  [ -n "$old" ] && kill -0 "$old" 2>/dev/null && { echo "→ replacing previous co-web (pid $old)…"; kill "$old" 2>/dev/null || true; sleep 1; }
fi

echo "→ booting co-web on :$PORT (CO_ENV=local)…"
# Detach hard: the subshell closes the inherited fds (so a `… | tail` pipeline
# doesn't hang on the long-lived child) then exec's co-web, so $! is co-web's pid.
(
  cd "$WT" || exit 1
  exec </dev/null >"$DATA/co-web.log" 2>&1
  export CO_WEB_DATA="$DATA" CO_WEB_PORT="$PORT" CO_ENV="local" \
         JWT_SECRET="${JWT_SECRET:-dev-localhost-secret}"
  exec "$WEB"
) &
echo $! > "$PIDFILE"
echo "$PORT" > "$DATA/co-web.port"

BASE="http://localhost:$PORT"
for _ in $(seq 1 60); do
  curl -s --max-time 2 "$BASE/api/health" >/dev/null 2>&1 && break
  sleep 1
done
if ! curl -s --max-time 3 "$BASE/api/health" >/dev/null 2>&1; then
  echo "✗ co-web never became healthy on :$PORT" >&2
  tail -20 "$DATA/co-web.log" >&2 2>/dev/null || true
  exit 2
fi
echo "✓ co-web UP   $BASE/api/health   →   $BASE"

# ── optional: whatsapp-bot bridge pointed at THIS co-web ─────────────────────
if [ "$WITH_BOT" = 1 ]; then
  BOT_DIR="/Users/artelonga/projects/tools/whatsapp-bot"
  BOT_PORT=$((PORT + 1000))   # CO 8900-9699 → bot 9900-10699, no collision
  if [ -d "$BOT_DIR" ]; then
    echo "→ booting whatsapp-bot bridge on :$BOT_PORT (CO_V1_URL=$BASE)…"
    (
      cd "$BOT_DIR" || exit 1
      exec </dev/null >"$DATA/bot.log" 2>&1
      export BOT_PORT="$BOT_PORT" BOT_HOST="127.0.0.1" CO_V1_URL="$BASE" \
             CO_BOT_IDENTITY_STORE_PATH="$DATA/bot-identity.json"
      exec python3 -m bridge.main
    ) &
    echo $! > "$BOT_PIDFILE"
    for _ in $(seq 1 30); do
      curl -s --max-time 2 "http://localhost:$BOT_PORT/health" >/dev/null 2>&1 && break
      sleep 0.5
    done
    if curl -s --max-time 3 "http://localhost:$BOT_PORT/health" >/dev/null 2>&1; then
      echo "✓ bot bridge UP   http://localhost:$BOT_PORT/health"
    else
      echo "⚠️  bot bridge did not become healthy (see $DATA/bot.log) — continuing"
    fi
  else
    echo "⚠️  --with-bot: $BOT_DIR not found — skipping bot"
  fi
fi

echo
echo "  live: $BASE   (logs: $DATA/co-web.log)"
echo "  stop: scripts/pr-localhost.sh $REF --stop"
exit 0
