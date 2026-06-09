#!/usr/bin/env bash
# ci-preflight.sh — run the exact CI gates locally before pushing, so the
# recurring red causes (fmt, clippy, openapi drift) fail here, not in CI.
#
# Usage:
#   scripts/ci-preflight.sh            # fast gates (fmt, clippy, build, openapi-if-changed)
#   scripts/ci-preflight.sh --test     # also run cargo test --workspace (slow)
#
# Wire as a pre-push hook (optional):
#   ln -sf ../../scripts/ci-preflight.sh .git/hooks/pre-push
#
# Mirrors .github/workflows/ci.yml (Build/Test/Clippy/Format) and
# .github/workflows/openapi-check.yml (drift on *_routes.rs / openapi.yaml).

set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

fail=0
step() {
  local label="$1"; shift
  echo "→ $label"
  if "$@"; then echo "  ✓ $label"; else echo "  ✗ $label FAILED"; fail=1; fi
}

step "cargo fmt --check"          cargo fmt --all -- --check
step "cargo clippy -D warnings"   cargo clippy --workspace -- -D warnings
step "cargo build --workspace"    cargo build --workspace

if [[ "${1:-}" == "--test" ]]; then
  step "cargo test --workspace"   cargo test --workspace
fi

# OpenAPI drift check only when route/spec files changed vs main (mirrors the
# openapi-check.yml path filter). Catches feedback_coauto_catalog_drift early.
changed="$(git diff --name-only origin/main...HEAD 2>/dev/null || git diff --name-only HEAD)"
if echo "$changed" | grep -qE 'co-web/src/.*_routes\.rs|co-web/(openapi.*\.yaml|scripts/generate-openapi\.ts)|docs/architecture/api-catalog\.md'; then
  step "openapi:check (routes changed)" bash -c 'cd co-web && npm run openapi:check'
else
  echo "→ openapi:check — skipped (no route/spec changes)"
fi

if [[ $fail -eq 0 ]]; then
  echo "✓ preflight clean — safe to push"
else
  echo "✗ preflight FAILED — fix the above before pushing (these are CI gates)"
  exit 1
fi
