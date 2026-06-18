#!/usr/bin/env bash
# co-local — serve the ~/projects universe workspace on localhost.
#
# Every top-level folder under CO_LOCAL_REPOS_DIR with a `_universe.yaml`
# auto-registers as a universe (key = folder name) and is served at
# http://localhost:<port>/co/<key>. Drop/move a folder in → it's a universe.
# CRUD via the web editor or the Vault API / `co sync`.
set -euo pipefail
export CO_WEB_DATA="${CO_WEB_DATA:-$HOME/.co/local-data}"
export CO_WEB_PORT="${CO_WEB_PORT:-3000}"
export CO_LOCAL_REPOS_DIR="${CO_LOCAL_REPOS_DIR:-$HOME/projects}"
export JWT_SECRET="${JWT_SECRET:-dev-local-secret}"
# Owner login locally: with no RESEND key, LogMailProvider prints the magic-code
# to this server log; `scripts/co-login.sh` reads it. Log in as yuri by default.
export CO_SEED_ADMIN_EMAIL="${CO_SEED_ADMIN_EMAIL:-yuri@artelonga.com.br}"
mkdir -p "$CO_WEB_DATA"
cd "$(dirname "$0")/.."
echo "co-local · workspace=$CO_LOCAL_REPOS_DIR · http://localhost:$CO_WEB_PORT"
exec cargo run --release -p co-web
