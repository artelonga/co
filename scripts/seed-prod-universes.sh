#!/usr/bin/env bash
# CO-67 prod seed — create + upload local repos to prod, with delta detection
# via jujutsu (jj) and automated changelog generation.
#
# Auth model:
#   - Universe CREATE / PUT requires JWT (require_auth — JWT-only middleware).
#     So --bootstrap logs in with the password, does the full first seed,
#     and stores a long-lived API token for future re-uploads.
#   - Vault PUT/GET (the actual content upload path) accepts API tokens.
#     So normal runs use the token from the OS keychain — no password.
#
# Delta detection (jj):
#   - Each source repo is wrapped with jj (jj git init — non-destructive,
#     coexists with the git remote). jj snapshots the working copy on every
#     command, so file changes are tracked automatically.
#   - The script stores the last-uploaded jj commit ID in
#     ~/.co/seed-state/<universe>.commit. On the next run, only files that
#     differ between that baseline and `@` (current working copy) are PUT.
#   - First run with no baseline = full upload.
#
# Changelog:
#   - Between the baseline and `@`, `jj log -r '<baseline>..@'` lists every
#     commit you've made since the last successful upload. The script prints
#     this to stdout AND appends to ~/.co/seed-runs/<universe>-<ts>.md so you
#     have a per-upload diff history.
#
# Usage:
#   bash scripts/seed-prod-universes.sh --bootstrap                  # one-time full seed
#   bash scripts/seed-prod-universes.sh                              # delta re-upload
#   bash scripts/seed-prod-universes.sh --full                       # force full re-upload
#   bash scripts/seed-prod-universes.sh --no-changelog               # skip changelog generation
set -euo pipefail

PROD="https://co-artelonga.fly.dev"
EMAIL="yuri@artelonga.com.br"
TOKEN_NAME="prod"
STATE_DIR="$HOME/.co/seed-state"
RUNS_DIR="$HOME/.co/seed-runs"
mkdir -p "$STATE_DIR" "$RUNS_DIR"

FORCE_FULL=0
WANT_CHANGELOG=1
for arg in "$@"; do
    case "$arg" in
        --full) FORCE_FULL=1 ;;
        --no-changelog) WANT_CHANGELOG=0 ;;
    esac
done

# ---------- jj helpers ----------

ensure_jj_repo() {
    local root="$1"
    if [[ ! -d "$root/.jj" ]]; then
        # Send init noise to stderr so it doesn't pollute the captured commit_id.
        echo "  initializing jj wrapper over git in $root ..." >&2
        ( cd "$root" && jj git init --colocate ) 2>&1 | sed 's/^/    /' >&2
    fi
    # `jj snapshot` is implicit on every jj command; running anything refreshes the working copy.
    ( cd "$root" && jj log -r @ --no-graph -T 'commit_id' ) 2>/dev/null | head -1
}

# Returns newline-separated list of *.md paths changed between $1 (baseline)
# and `@` (current). If baseline is empty or invalid, returns ALL .md files.
changed_md_files() {
    local root="$1" baseline="$2"
    if [[ -z "$baseline" || "$FORCE_FULL" == "1" ]]; then
        ( cd "$root" && find . -type f -name '*.md' \
            -not -path '*/node_modules/*' -not -path '*/.git/*' -not -path '*/.jj/*' \
            -not -path '*/target/*' -not -path '*/build/*' \
            -not -path '*/dist/*' -not -path '*/.next/*' -not -path '*/.svelte-kit/*' \
            -not -path '*/.claude/*' -not -path '*/.obsidian/*' \
            -not -path '*/.cache/*' -not -path '*/.vercel/*' \
            -not -path '*/seed-co/*' -not -path '*/.venv/*' -not -path '*/__pycache__/*' \
            | sed 's|^./||' )
        return
    fi
    # Verify baseline exists in jj's history
    if ! ( cd "$root" && jj log -r "$baseline" --no-graph -T 'commit_id' >/dev/null 2>&1 ); then
        # Baseline lost (repo rewritten?); fall back to full upload
        ( cd "$root" && find . -type f -name '*.md' \
            -not -path './.git/*' -not -path './.jj/*' \
            -not -path './.claude/*' -not -path './.obsidian/*' \
            -not -path './.cache/*' -not -path './.vercel/*' \
            -not -path './seed-co/*' \
            | sed 's|^./||' )
        return
    fi
    # jj diff names paths from repo root; include both modified and added
    ( cd "$root" && jj diff --from "$baseline" --to @ --name-only 2>/dev/null \
        | grep -E '\.md$' | sort -u )
}

generate_changelog() {
    local root="$1" baseline="$2" current="$3" universe="$4"
    if [[ "$WANT_CHANGELOG" != "1" ]]; then return; fi
    local out="$RUNS_DIR/${universe}-$(date -u +%Y%m%dT%H%M%SZ).md"
    {
        echo "# Upload run — $universe — $(date -u +%FT%TZ)"
        echo
        echo "- baseline: ${baseline:-<none>}"
        echo "- current:  $current"
        echo "- source:   $root"
        echo
        if [[ -z "$baseline" ]]; then
            echo "## First upload (no commit history to summarize)"
        else
            echo "## Commits since last upload"
            echo
            ( cd "$root" && jj log -r "${baseline}..@" \
                --no-graph \
                -T 'change_id.short() ++ " " ++ author.timestamp().format("%Y-%m-%d %H:%M") ++ " — " ++ description.first_line() ++ "\n"' 2>/dev/null \
                || echo "(jj log failed)" )
        fi
    } > "$out"
    echo "  changelog → $out"
    echo "  --- preview ---"
    sed 's/^/  /' "$out"
    echo "  --- end preview ---"
}

# ---------- upload helpers ----------

upload_files_with_auth() {
    # Args: universe, root, auth-header, newline-separated relative file paths
    local universe="$1" root="$2" auth="$3" files="$4"
    if [[ -z "$files" ]]; then
        echo "  $universe: no files to upload (no changes since last run)"
        return
    fi
    local count=0 ok=0 fail=0
    while IFS= read -r rel; do
        [[ -z "$rel" ]] && continue
        local file="$root/$rel"
        [[ ! -f "$file" ]] && continue
        local encoded
        encoded=$(python3 -c "import sys, urllib.parse; print('/'.join(urllib.parse.quote(seg, safe='') for seg in sys.argv[1].split('/')))" "$rel")
        local code
        code=$(curl -s -o /dev/null -w '%{http_code}' \
            -H "$auth" -X PUT "$PROD/api/v1/universes/$universe/vault/$encoded" \
            -H 'Content-Type: text/markdown' \
            --data-binary "@$file")
        count=$((count + 1))
        if [[ "$code" == "200" || "$code" == "201" ]]; then
            ok=$((ok + 1))
        else
            fail=$((fail + 1))
            [[ "$fail" -le 3 ]] && echo "    HTTP $code: $rel"
        fi
        sleep 0.1
    done <<< "$files"
    echo "  $universe: $ok ok, $fail fail (of $count)"
}

verify_counts_with_auth() {
    local auth="$1"
    for slug in artelonga quilomboaraucaria rfq qa-dev mbya; do
        count=$(curl -s -H "$auth" "$PROD/api/v1/universes/$slug" | \
            python3 -c "import sys,json; print(json.load(sys.stdin).get('content_count','?'))" 2>/dev/null)
        echo "  $slug: count=$count"
    done
}

upload_universe_delta() {
    local universe="$1" root="$2" auth="$3"
    if [[ ! -d "$root" ]]; then
        echo "  • $universe: source $root not found, skipping"
        return
    fi
    local state_file="$STATE_DIR/${universe}.commit"
    local baseline
    baseline=$(cat "$state_file" 2>/dev/null || echo "")
    local current
    current=$(ensure_jj_repo "$root")
    local files
    files=$(changed_md_files "$root" "$baseline")
    local n
    n=$(echo "$files" | grep -c '\.md$' || true)
    echo "  $universe: $n file(s) to upload (baseline=${baseline:0:8} current=${current:0:8})"
    upload_files_with_auth "$universe" "$root" "$auth" "$files"
    generate_changelog "$root" "$baseline" "$current" "$universe"
    # Save new baseline only if we tried at least one file (or it was a no-op delta)
    echo "$current" > "$state_file"
}

# ===========================================================================
# Bootstrap branch
# ===========================================================================
if [[ "${1:-}" == "--bootstrap" ]]; then
    PASSWORD="${2:-}"
    if [[ -z "$PASSWORD" ]]; then
        read -rs -p "Password: " PASSWORD
        echo
    fi
    COOKIES=$(mktemp)
    trap 'rm -f "$COOKIES"' EXIT

    echo "[bootstrap 1/5] login as $EMAIL ..."
    LOGIN=$(curl -sc "$COOKIES" -X POST "$PROD/api/v1/auth/password-login" \
        -H 'Content-Type: application/json' \
        --data "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")
    if ! echo "$LOGIN" | python3 -c "import sys,json; d=json.load(sys.stdin); assert 'user_id' in d" 2>/dev/null; then
        echo "  login failed: $LOGIN"
        exit 1
    fi
    echo "  ok"

    echo "[bootstrap 2/5] create universes (idempotent — 409 is fine) ..."
    create_with_cookie() {
        local key="$1" name="$2" desc="$3"
        local code
        code=$(curl -s -o /tmp/seed-resp.json -w '%{http_code}' \
            -b "$COOKIES" -X POST "$PROD/api/v1/universes" \
            -H 'Content-Type: application/json' \
            --data "{\"key\":\"$key\",\"name\":\"$name\",\"description\":\"$desc\"}")
        case "$code" in
            201) echo "  ✓ $key created" ;;
            409) echo "  • $key already exists" ;;
            *)   echo "  ✗ $key HTTP $code: $(cat /tmp/seed-resp.json)"; exit 1 ;;
        esac
    }
    # Same pattern as co (public) ↔ co-dev (raw working files):
    #   quilomboaraucaria = public-facing content (currently 70 entries from migration)
    #   qa-dev            = raw notes / working files (yuri's local Obsidian-style vault)
    # Form/presentation will be split later; for now everything raw goes to qa-dev.
    create_with_cookie artelonga "ArteLonga" "Rede de marcas e empreendedores"
    create_with_cookie rfq       "RFQ"        "Quote engine for prediction market making"
    create_with_cookie qa-dev    "QA Dev"     "Raw working files for Quilombo Araucária — content to be split into form/data later"
    create_with_cookie mbya      "Mbya Guarani" "Léxico Mbya Guarani — entradas extraídas de GNDicLex e GNDicInt (uso acadêmico)."

    echo "[bootstrap 3/5] full upload (jj snapshots baseline for delta runs) ..."
    SESSION=$(awk '/\tsession\t/ {print $7}' "$COOKIES")
    COOKIE_AUTH="Cookie: session=$SESSION"
    upload_universe_delta artelonga /Users/artelonga/projects/ArteLonga       "$COOKIE_AUTH"
    upload_universe_delta rfq       /Users/artelonga/projects/rfq-gateway     "$COOKIE_AUTH"
    upload_universe_delta qa-dev    /Users/artelonga/projects/quilomboaraucaria "$COOKIE_AUTH"
    upload_universe_delta mbya      /Users/artelonga/projects/mbya/content     "$COOKIE_AUTH"

    echo "[bootstrap 4/5] generate long-lived API token for re-uploads ..."
    TBODY=$(curl -sb "$COOKIES" -X POST "$PROD/api/v1/auth/token" \
        -H 'Content-Type: application/json' \
        --data "{\"name\":\"yuri-cli\"}")
    TOKEN=$(echo "$TBODY" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null || true)
    if [[ -z "$TOKEN" ]]; then
        echo "  token generation failed: $TBODY"
        exit 1
    fi
    echo "  generated (${#TOKEN} bytes)"

    echo "[bootstrap 5/5] store in OS keychain ..."
    if ! command -v co-token >/dev/null; then
        echo "  co-token not on PATH. Install: cargo install --path dev/co-token"
        exit 1
    fi
    printf '%s' "$TOKEN" | co-token set "$TOKEN_NAME"

    echo
    echo "Verify counts:"
    verify_counts_with_auth "$COOKIE_AUTH"
    echo
    echo "Done. Future runs (delta only): bash scripts/seed-prod-universes.sh"
    exit 0
fi

# ===========================================================================
# Normal run — delta upload via stored API token
# ===========================================================================
if ! command -v co-token >/dev/null; then
    echo "co-token not on PATH. Install: cargo install --path dev/co-token"
    exit 1
fi
TOKEN=$(co-token get "$TOKEN_NAME" 2>/dev/null || true)
if [[ -z "$TOKEN" ]]; then
    echo "No '$TOKEN_NAME' token in keychain."
    echo "Run once: bash scripts/seed-prod-universes.sh --bootstrap"
    exit 1
fi
TOKEN_AUTH="Authorization: Bearer $TOKEN"

echo "[1/3] verify token (vault listing on artelonga) ..."
CODE=$(curl -s -o /dev/null -w '%{http_code}' -H "$TOKEN_AUTH" "$PROD/api/v1/universes/artelonga/vault/")
case "$CODE" in
    200) echo "  ok" ;;
    404) echo "  artelonga universe doesn't exist on prod yet — run --bootstrap first"; exit 1 ;;
    *)   echo "  token rejected (HTTP $CODE) — re-bootstrap"; exit 1 ;;
esac

echo "[2/3] delta upload (jj diff against baseline) ..."
upload_universe_delta artelonga /Users/artelonga/projects/ArteLonga       "$TOKEN_AUTH"
upload_universe_delta rfq       /Users/artelonga/projects/rfq-gateway     "$TOKEN_AUTH"
upload_universe_delta qa-dev    /Users/artelonga/projects/quilomboaraucaria "$TOKEN_AUTH"
upload_universe_delta mbya      /Users/artelonga/projects/mbya/content     "$TOKEN_AUTH"

echo "[3/3] verify counts ..."
verify_counts_with_auth "$TOKEN_AUTH"

echo
echo "Changelog snippets stored under: $RUNS_DIR/"
echo "Baselines stored under: $STATE_DIR/"


# --- CO-141 / CO-109 universe registration ---
# Run this section to register mbya and topologia in prod.
# Usage: CO_ADMIN_TOKEN=<jwt> bash scripts/seed-prod-universes.sh register [prod|uat]

if [ "${1:-}" = "register" ]; then
  ENV="${2:-prod}"
  case "$ENV" in
    uat)  BASE="https://co-artelonga-uat.fly.dev" ;;
    prod) BASE="https://co.artelonga.com.br" ;;
  esac

  if [ -z "${CO_ADMIN_TOKEN:-}" ]; then
    echo "Set CO_ADMIN_TOKEN before running." >&2; exit 1
  fi

  create_universe() {
    local key="$1" name="$2" desc="$3" vis="${4:-public-subscribable}"
    local status
    status=$(curl -s -o /tmp/co-seed-resp.json -w "%{http_code}" \
      -X POST "$BASE/api/v1/universes" \
      -H "Authorization: Bearer $CO_ADMIN_TOKEN" \
      -H 'Content-Type: application/json' \
      -d "{\"key\":\"$key\",\"name\":\"$name\",\"description\":\"$desc\",\"visibility\":\"$vis\"}")
    case "$status" in
      201) echo "$key: created" ;;
      409) echo "$key: already exists (ok)" ;;
      *)   echo "$key: ERROR $status: $(cat /tmp/co-seed-resp.json)" ;;
    esac
  }

  echo "Registering universes on $BASE ..."
  create_universe "mbya"      "Mbya Guarani"         "Léxico Mbya Guarani — universo de referência linguística"
  create_universe "topologia" "Topologia do Sentido"  "Dicionário translinguístico — conceitos e relações entre línguas"
  echo "Done."
  exit 0
fi
