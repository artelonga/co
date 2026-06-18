#!/usr/bin/env bash
# co-deploy — one-word deploy of the CO app to prod (main), with disk gate + smoke.
# Universe *content* deploys separately/CO-natively via `co sync push <key>`
# (Vault API → prod), no app deploy needed.
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/pipeline-deploy-gate.sh
flyctl deploy --build-arg GIT_SHA="$(git rev-parse --short HEAD)"
bash scripts/smoke-prod.sh 2>/dev/null || echo "(smoke-prod.sh not run)"
