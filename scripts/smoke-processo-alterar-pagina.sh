#!/usr/bin/env bash
# End-to-end smoke for the alterar-pagina-na-web process — exercises all 7
# chain steps against a target universe.
#
# Usage:
#   ADMIN_EMAIL=yuri@artelonga.com.br ADMIN_PASSWORD=<...> \
#     UNIVERSE=yuri PAGE=projetos/index.md FIELD=Titulo \
#     bash scripts/smoke-processo-alterar-pagina.sh
#
# Defaults:
#   BASE_URL    https://co-artelonga.fly.dev
#   UNIVERSE    yuri
#   PAGE        projetos/index.md   (must exist in the target universe)
#   FIELD       Titulo
#   NEW_VALUE   "Smoke test $(date +%H:%M:%S)"
#
# Prereqs (the page must already exist with a frontmatter object):
#   - The target universe has a page at PAGE with a frontmatter field FIELD
#   - The admin user has ReadWrite access (owner / admin member)

set -euo pipefail

BASE_URL="${BASE_URL:-https://co-artelonga.fly.dev}"
UNIVERSE="${UNIVERSE:-yuri}"
PAGE="${PAGE:-projetos/index.md}"
FIELD="${FIELD:-Titulo}"
NEW_VALUE="${NEW_VALUE:-Smoke test $(date +%H:%M:%S)}"
ADMIN_EMAIL="${ADMIN_EMAIL:?Set ADMIN_EMAIL}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:?Set ADMIN_PASSWORD}"

JAR=$(mktemp)
trap "rm -f $JAR" EXIT

echo "[smoke] base=$BASE_URL universe=$UNIVERSE page=$PAGE field=$FIELD"

# Step 0: Login (collect session cookie).
echo
echo "[smoke] Step 0 — password-login as $ADMIN_EMAIL"
curl -sc "$JAR" -X POST -H 'Content-Type: application/json' \
    -d "{\"email\":\"$ADMIN_EMAIL\",\"password\":\"$ADMIN_PASSWORD\"}" \
    "$BASE_URL/api/v1/auth/password-login" \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print('  user_id={} expires_at={}'.format(d.get('user_id','?'), d.get('expires_at','?')))"

# Step 1+2+3: Trigger / Source / Review.
echo
echo "[smoke] Steps 1–3 — POST /preview"
PREVIEW=$(curl -sb "$JAR" -X POST -H 'Content-Type: application/json' \
    -d "{\"universe\":\"$UNIVERSE\",\"page_path\":\"$PAGE\",\"field\":\"$FIELD\",\"new_value\":\"$NEW_VALUE\"}" \
    "$BASE_URL/api/v1/processos/alterar-pagina-na-web/preview")
echo "$PREVIEW" | python3 -m json.tool
RUN_ID=$(echo "$PREVIEW" | python3 -c 'import sys,json; print(json.load(sys.stdin)["run_id"])')
PROPOSED=$(echo "$PREVIEW" | python3 -c 'import sys,json; print(json.load(sys.stdin)["proposed_version"])')
echo "[smoke] run_id=$RUN_ID proposed=$PROPOSED"

# Step 4+5+6: Approval / Sink / Telemetry.
echo
echo "[smoke] Steps 4–6 — POST /approve/$RUN_ID"
APPROVE=$(curl -sb "$JAR" -X POST \
    "$BASE_URL/api/v1/processos/alterar-pagina-na-web/approve/$RUN_ID")
echo "$APPROVE" | python3 -m json.tool
COMPLETED_VERSION=$(echo "$APPROVE" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("to_version","?"))')
echo "[smoke] universe is now at v$COMPLETED_VERSION"

# Verify the change landed: read the page, check field.
echo
echo "[smoke] Verify — GET universe content_version + entry frontmatter"
curl -sb "$JAR" "$BASE_URL/api/v1/universes/$UNIVERSE" \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print('  universe content_version={}'.format(d.get('content_version','?')))"
curl -sb "$JAR" "$BASE_URL/api/v1/universes/$UNIVERSE/entries/$PAGE" \
    | python3 -c "import sys,json; d=json.load(sys.stdin); fm=d.get('frontmatter',{}); print(f'  entry $FIELD={fm.get(\"$FIELD\",\"?\")!r}')"

# Step 7: Rollback.
echo
echo "[smoke] Step 7 — POST /revert (target_version=prior)"
REVERT=$(curl -sb "$JAR" -X POST -H 'Content-Type: application/json' \
    -d "{\"universe\":\"$UNIVERSE\",\"target_version\":\"prior\"}" \
    "$BASE_URL/api/v1/processos/alterar-pagina-na-web/revert")
echo "$REVERT" | python3 -m json.tool

# Verify revert landed.
echo
echo "[smoke] Verify revert — universe content_version + entry frontmatter"
curl -sb "$JAR" "$BASE_URL/api/v1/universes/$UNIVERSE" \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print('  universe content_version={}'.format(d.get('content_version','?')))"
curl -sb "$JAR" "$BASE_URL/api/v1/universes/$UNIVERSE/entries/$PAGE" \
    | python3 -c "import sys,json; d=json.load(sys.stdin); fm=d.get('frontmatter',{}); print(f'  entry $FIELD={fm.get(\"$FIELD\",\"?\")!r}')"

# List runs.
echo
echo "[smoke] GET /runs — universe history"
curl -sb "$JAR" "$BASE_URL/api/v1/processos/alterar-pagina-na-web/runs?universe=$UNIVERSE&limit=5" \
    | python3 -m json.tool | head -40

# Verify CHANGELOG.md exists in universe.
echo
echo "[smoke] Verify <universe>/CHANGELOG.md (read first 25 lines)"
CHLOG=$(curl -sb "$JAR" "$BASE_URL/api/v1/universes/$UNIVERSE/entries/CHANGELOG.md" 2>/dev/null \
    | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("body","(no CHANGELOG.md found)"))' 2>/dev/null \
    || echo "(CHANGELOG.md not yet ingested into entries — may be filesystem-only until next walk)")
echo "$CHLOG" | head -25

echo
echo "[smoke] DONE — all 7 steps exercised."
