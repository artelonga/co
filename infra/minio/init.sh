#!/usr/bin/env bash
# Bootstrap MinIO: create buckets used by the CO lakehouse.
# Run once after `docker compose up`.
set -euo pipefail

ENDPOINT="${S3_ENDPOINT:-http://localhost:9000}"
KEY_ID="${AWS_ACCESS_KEY_ID:-co-admin}"
SECRET="${AWS_SECRET_ACCESS_KEY:-co-secret-local}"

echo "Configuring mc alias..."
mc alias set co-local "$ENDPOINT" "$KEY_ID" "$SECRET" --api S3v4

echo "Creating buckets..."
mc mb --ignore-existing co-local/co-lakehouse
mc mb --ignore-existing co-local/co-backups

echo "Setting lakehouse policy (private)..."
mc anonymous set none co-local/co-lakehouse

echo "Done. Buckets:"
mc ls co-local
