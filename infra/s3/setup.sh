#!/usr/bin/env bash
# One-time S3 bucket setup for CO backups (CO-104).
# Run once per environment. Idempotent.
#
# Requires: aws CLI, configured with an account that has S3 admin rights.
# After this runs, create a dedicated IAM user (see below) for the cron app.
set -euo pipefail

BUCKET=${BUCKET:-artelonga-co-backups}
REGION=${AWS_DEFAULT_REGION:-us-east-1}

echo "Setting up s3://$BUCKET in $REGION..."

# 1. Create bucket (idempotent).
if [ "$REGION" = "us-east-1" ]; then
  aws s3api create-bucket --bucket "$BUCKET" --region "$REGION"
else
  aws s3api create-bucket --bucket "$BUCKET" --region "$REGION" \
    --create-bucket-configuration LocationConstraint="$REGION"
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
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
aws s3api put-bucket-lifecycle-configuration \
  --bucket "$BUCKET" \
  --lifecycle-configuration "file://$SCRIPT_DIR/lifecycle.json"

echo "Bucket s3://$BUCKET configured."
echo ""
echo "Next: create a dedicated IAM user with PutObject/GetObject on this bucket only."
echo "  Policy template:"
echo '  {"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":["s3:PutObject","s3:GetObject","s3:ListBucket"],"Resource":["arn:aws:s3:::'"$BUCKET"'","arn:aws:s3:::'"$BUCKET"'/*"]}]}'
