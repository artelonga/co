#!/usr/bin/env bash
# One-time S3 bucket setup for CO backups (CO-104).
# Run once per environment. Idempotent.
#
# Works with AWS S3 and S3-compatible backends (Garage, MinIO, Cloudflare R2):
#   BUCKET=artelonga-co-backups ./infra/s3/setup.sh              # AWS S3
#   AWS_ENDPOINT_URL=http://localhost:9000 BUCKET=co-backups \   # local MinIO/Garage
#     AWS_ACCESS_KEY_ID=minioadmin AWS_SECRET_ACCESS_KEY=minioadmin \
#     ./infra/s3/setup.sh
#
# For Garage: public-access-block and SSE-S3 are skipped (not supported).
# Lifecycle policy is applied only when running against AWS.
set -euo pipefail

BUCKET=${BUCKET:-artelonga-co-backups}
REGION=${AWS_DEFAULT_REGION:-us-east-1}
ENDPOINT=${AWS_ENDPOINT_URL:-}
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ -n "$ENDPOINT" ]; then
  echo "Setting up s3://$BUCKET on $ENDPOINT (S3-compatible backend)..."

  # MinIO/Garage: just create the bucket — path-style is handled by endpoint URL.
  aws s3api create-bucket --bucket "$BUCKET" 2>/dev/null || true
  echo "Bucket s3://$BUCKET created (or already exists)."
  echo "Note: public-access-block, SSE, and lifecycle are AWS-only features."
  echo "      Configure encryption + retention via your backend's UI/config."
  exit 0
fi

echo "Setting up s3://$BUCKET in $REGION (AWS)..."

# 1. Create bucket (idempotent).
if [ "$REGION" = "us-east-1" ]; then
  aws s3api create-bucket --bucket "$BUCKET" --region "$REGION" 2>/dev/null || true
else
  aws s3api create-bucket --bucket "$BUCKET" --region "$REGION" \
    --create-bucket-configuration LocationConstraint="$REGION" 2>/dev/null || true
fi

# 2. Block public access.
aws s3api put-public-access-block \
  --bucket "$BUCKET" \
  --public-access-block-configuration \
    "BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true"

# 3. Enable SSE-S3 encryption at rest.
aws s3api put-bucket-encryption \
  --bucket "$BUCKET" \
  --server-side-encryption-configuration '{
    "Rules": [{
      "ApplyServerSideEncryptionByDefault": {"SSEAlgorithm": "AES256"},
      "BucketKeyEnabled": true
    }]
  }'

# 4. Apply lifecycle policy (transition to IA after 30 days, delete after 365).
aws s3api put-bucket-lifecycle-configuration \
  --bucket "$BUCKET" \
  --lifecycle-configuration "file://$SCRIPT_DIR/lifecycle.json"

echo "Bucket s3://$BUCKET configured."
echo ""
echo "Next: create a dedicated IAM user with PutObject/GetObject on this bucket only."
echo '  {"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":["s3:PutObject","s3:GetObject","s3:ListBucket"],"Resource":["arn:aws:s3:::'"$BUCKET"'","arn:aws:s3:::'"$BUCKET"'/*"]}]}'
