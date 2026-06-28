#!/usr/bin/env bash
#
# run.sh — self-host run script for co-web (macOS / Rust, NO Docker, NO Fly).
#
# This is the PARALLEL deploy path to Fly (CO-497 invariant: SAME code/binary —
# self-host just runs `target/release/co-web` directly on your own machine
# instead of inside a Fly machine). Boot sequence:
#   1. resolve a persistent DATA dir (default ~/.co/data)
#   2. build co-web (cargo build --release -p co-web) unless --no-build
#   3. require JWT_SECRET (this script NEVER invents one — see note below)
#   4. pick the bind host:
#        - network self-host (default): CO_WEB_HOST=0.0.0.0 + CO_ENV=prod
#          (reachable on the LAN / by a Cloudflare Tunnel / reverse proxy)
#        - --localhost: CO_WEB_HOST=127.0.0.1 (loopback only, LAN-safe)
#   5. exec the co-web binary, then wait for GET /api/health to go green.
#
# JWT_SECRET MUST be provided via the environment (HS256 session signing key).
# This script will NOT invent one: co-web ships an insecure dev fallback
# ("dev-secret-change-me") and CO-469 makes the server PANIC at boot in any
# non-local env if that fallback is in effect — otherwise every session token
# is forgeable. A self-host run is a prod env, so an unset secret is fatal here.
#
# Usage:
#   JWT_SECRET=... scripts/selfhost/run.sh [--no-build] [--port N] [--localhost]
#
# Env:
#   JWT_SECRET    (required) HS256 session signing key — generate once:
#                            openssl rand -base64 48   (keep it STABLE; rotating
#                            it invalidates every live session)
#   CO_WEB_DATA   persistent data dir   (default: ~/.co/data)
#   CO_WEB_PORT   listen port           (default: 8742, or --port)
#   CO_SEED_ADMIN_EMAIL          (optional) seeds/gates the admin user
#   CO_SEED_ADMIN_PASSWORD_HASH  (optional) argon2 hash for password-login
#
# Flags:
#   --no-build    skip the cargo build (binary must already exist)
#   --port N      override CO_WEB_PORT
#   --localhost   bind 127.0.0.1 only (loopback) instead of 0.0.0.0
#
set -euo pipefail

# ── Resolve repo root (this script lives in scripts/selfhost/) ──────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Flags ───────────────────────────────────────────────────────────────────
NO_BUILD=0
LOCALHOST=0
PORT_OVERRIDE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build)  NO_BUILD=1; shift ;;
    --localhost) LOCALHOST=1; shift ;;
    --port)      PORT_OVERRIDE="${2:-}"; shift 2 ;;
    --port=*)    PORT_OVERRIDE="${1#*=}"; shift ;;
    -h|--help)   sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "run.sh: unknown argument: $1" >&2; exit 2 ;;
  esac
done

# ── Required secret ───────────────────────────────────────────────────────────
if [ -z "${JWT_SECRET:-}" ] || [ "${JWT_SECRET}" = "dev-secret-change-me" ]; then
  echo "ERROR: JWT_SECRET is not set (or is the insecure dev fallback)." >&2
  echo "  This is the HS256 session signing key and must be a stable, unique" >&2
  echo "  secret. co-web (CO-469) PANICS at boot in a prod env without it." >&2
  echo "  Generate one once and keep it (do NOT regenerate per run, or all" >&2
  echo "  sessions are invalidated):" >&2
  echo "      export JWT_SECRET=\$(openssl rand -base64 48)" >&2
  echo "  Then re-run, or put it in the launchd plist / a 0600 sourced env file." >&2
  exit 1
fi

# ── Data dir layout (CO-77: meta.db + universes/<key>/data.db) ────────────────
DATA="${CO_WEB_DATA:-$HOME/.co/data}"
mkdir -p "$DATA/universes"

# ── Port + bind host ──────────────────────────────────────────────────────────
PORT="${PORT_OVERRIDE:-${CO_WEB_PORT:-8742}}"
if [ "$LOCALHOST" -eq 1 ]; then
  BIND_HOST="127.0.0.1"
  # Loopback-only: keep prod safety semantics but never reachable off-box.
  export CO_ENV="${CO_ENV:-prod}"
else
  BIND_HOST="0.0.0.0"
  export CO_ENV="prod"
fi
export CO_WEB_HOST="$BIND_HOST"
export CO_WEB_PORT="$PORT"
export CO_WEB_DATA="$DATA"

echo "co-web self-host"
echo "  repo:   $REPO_ROOT"
echo "  data:   $DATA"
echo "  meta:   $DATA/meta.db"
echo "  host:   $BIND_HOST"
echo "  port:   $PORT"
echo "  env:    $CO_ENV"

# ── Build ─────────────────────────────────────────────────────────────────────
BIN="$REPO_ROOT/target/release/co-web"
if [ "$NO_BUILD" -eq 0 ]; then
  echo "Building co-web (cargo build --release -p co-web)..."
  ( cd "$REPO_ROOT" && cargo build --release -p co-web )
else
  echo "Skipping build (--no-build)."
  if [ ! -x "$BIN" ]; then
    echo "ERROR: --no-build given but $BIN does not exist." >&2
    echo "  Run once without --no-build first." >&2
    exit 1
  fi
fi
[ -x "$BIN" ] || { echo "ERROR: co-web binary not found at $BIN after build." >&2; exit 1; }

# ── Health-wait in a backgrounded subshell, then exec the server ──────────────
BASE="http://127.0.0.1:$PORT"
(
  for _ in $(seq 1 60); do
    sleep 1
    if curl -s --max-time 2 "$BASE/api/health" >/dev/null 2>&1; then
      echo "✓ co-web healthy: $BASE/api/health"
      exit 0
    fi
  done
  echo "⚠️  co-web did not report healthy on $BASE within 60s (check logs)." >&2
) &

echo "Starting co-web on $BIND_HOST:$PORT ..."
cd "$REPO_ROOT"
exec "$BIN"
