#!/usr/bin/env bash
# Entrypoint for the co-backup-cron Fly app.
# All config comes from Fly secrets:
#   FLY_API_TOKEN, AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY
set -euo pipefail

: "${FLY_API_TOKEN:?FLY_API_TOKEN secret not set}"
: "${AWS_ACCESS_KEY_ID:?AWS_ACCESS_KEY_ID secret not set}"
: "${AWS_SECRET_ACCESS_KEY:?AWS_SECRET_ACCESS_KEY secret not set}"

exec /scripts/backup-prod.sh
