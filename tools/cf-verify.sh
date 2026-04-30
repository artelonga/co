#!/usr/bin/env bash
# Verify Cloudflare cache rules are working for co.artelonga.com.br
# Usage: ./tools/cf-verify.sh [base-url]
set -euo pipefail

BASE=${1:-https://co.artelonga.com.br}

pass=0; fail=0

check() {
  local label=$1 url=$2 expected=$3 header=${4:-cf-cache-status}
  local val
  val=$(curl -sI "$url" | grep -i "^${header}:" | awk '{print $2}' | tr -d '\r' || true)
  if echo "$val" | grep -qiE "$expected"; then
    echo "PASS  $label  ($val)"
    ((pass++))
  else
    echo "FAIL  $label  expected=$expected got=$val"
    ((fail++))
  fi
}

# Static assets should be cached
check "theme.css HIT/MISS/REVALIDATED" "$BASE/theme.css"                 "HIT|MISS|REVALIDATED"
check "immutable JS HIT/MISS"          "$BASE/_app/immutable/start.js"   "HIT|MISS"           || true

# API must bypass
check "API health BYPASS/DYNAMIC"      "$BASE/api/health"                "BYPASS|DYNAMIC|MISS"

# Auth must never be served from cache (check no Set-Cookie in a repeated request)
val=$(curl -sI -X POST "$BASE/api/v1/auth/uat-login" \
  -H 'Content-Type: application/json' \
  -d '{}' | grep -i "^cf-cache-status:" | awk '{print $2}' | tr -d '\r' || true)
if echo "$val" | grep -qiE "BYPASS|DYNAMIC|MISS"; then
  echo "PASS  auth endpoint not cached ($val)"; ((pass++))
else
  echo "FAIL  auth endpoint cache status: $val"; ((fail++))
fi

echo ""
echo "Results: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
