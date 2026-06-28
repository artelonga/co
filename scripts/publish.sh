#!/usr/bin/env bash
#
# publish.sh — CO-489. The one deliberate, outward-facing "publish" hit.
#
#   organize / map  → automatic (the loop does these for you, continuously)
#   publish / send  → the deliberate hit (this script; explicit, outward-facing)
#
# This is a THIN wrapper around the canonical prod-direct deploy flow
# (docs/OPERATIONS.md → "Environments & Deploy", CLAUDE.md → "Deployment").
# It does NOT reinvent the gates — it calls the existing scripts:
#
#   1. cargo test                         (local correctness)
#   2. scripts/pipeline-deploy-gate.sh    (CO-446 disk gate + fresh green report)
#   3. flyctl deploy -a co-artelonga      (the single outward hit)
#   4. scripts/smoke-prod.sh              (post-deploy invariants)
#
# Modes:
#   --public      (default) managed path: gates → flyctl deploy → smoke.
#   --self-host   print the Cloudflare-Tunnel + launchd guidance (no action).
#   --rollback    one-hit UNDO: roll back to the previous Fly release.
#
# Safety:
#   --dry-run     print the EXACT commands that WOULD run; run nothing outward.
#   --yes         skip the interactive confirm (required for a non-interactive
#                 real deploy). Without it, a real deploy asks for confirmation.
#
# A real deploy can NEVER fire by accident: it requires a clean tree, the
# expected branch (or --allow-branch), a passing pipeline-deploy-gate, AND an
# explicit --yes / interactive "yes".
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

PROD_APP="${PROD_APP:-co-artelonga}"
EXPECTED_BRANCH="${PUBLISH_BRANCH:-main}"

# ── colours (no-op if not a tty) ─────────────────────────────────────────────
if [ -t 1 ]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; RED=$'\033[31m'; GRN=$'\033[32m'
  YEL=$'\033[33m'; CYN=$'\033[36m'; RST=$'\033[0m'
else
  BOLD=""; DIM=""; RED=""; GRN=""; YEL=""; CYN=""; RST=""
fi

# ── args ─────────────────────────────────────────────────────────────────────
mode="public"        # public | self-host | rollback
dry_run=false
assume_yes=false
allow_branch=false

usage() {
  cat <<EOF
${BOLD}publish.sh${RST} — the one deliberate, outward-facing publish hit (CO-489)

${BOLD}╔══════════════════════════════════════════════════════════════════════════╗${RST}
${BOLD}║${RST}  ${YEL}publish is an explicit, outward-facing action${RST}                          ${BOLD}║${RST}
${BOLD}║${RST}  organize / map  →  ${DIM}automatic${RST}   (the loop does these for you)            ${BOLD}║${RST}
${BOLD}║${RST}  publish / send  →  ${BOLD}the deliberate hit${RST}  (this script — one outward shot) ${BOLD}║${RST}
${BOLD}║${RST}  …and a one-hit undo:  ${BOLD}publish.sh --rollback${RST}                            ${BOLD}║${RST}
${BOLD}╚══════════════════════════════════════════════════════════════════════════╝${RST}

${BOLD}USAGE${RST}
  scripts/publish.sh [--public | --self-host] [--dry-run] [--yes] [--allow-branch]
  scripts/publish.sh --rollback [--dry-run] [--yes]
  scripts/publish.sh --help

${BOLD}MODES${RST}
  --public       (default) Managed path to production app '${PROD_APP}' (Fly gru).
                 Wraps the canonical prod-direct flow — calls existing scripts,
                 does NOT duplicate their logic:
                   1. cargo test
                   2. scripts/pipeline-deploy-gate.sh   (CO-446 disk + fresh report)
                   3. flyctl deploy -a ${PROD_APP}
                   4. scripts/smoke-prod.sh
  --self-host    Print the self-host path (Cloudflare Tunnel + launchd) from
                 docs/whatsapp-launch.md → "Deployment modes". Guidance only —
                 nothing is executed.
  --rollback     One-hit UNDO. Roll back '${PROD_APP}' to its PREVIOUS Fly release
                 (flyctl releases rollback, or deploy --image <prev> as fallback).

${BOLD}SAFETY FLAGS${RST}
  --dry-run      Print the EXACT commands that WOULD run. Runs nothing outward —
                 no cargo test, no gate, no flyctl, no smoke.
  --yes          Skip the interactive confirm (required for a non-interactive
                 real deploy / rollback).
  --allow-branch Permit a real deploy from a branch other than '${EXPECTED_BRANCH}'.
  --help, -h     This help.

${BOLD}GUARD RAILS${RST} (a real deploy refuses unless ALL hold)
  • git working tree is clean (no uncommitted changes)
  • on branch '${EXPECTED_BRANCH}'                (override: --allow-branch)
  • scripts/pipeline-deploy-gate.sh passes        (CO-446 disk + green report)
  • explicit --yes, or you type 'yes' at the prompt

${BOLD}SIBLING (B side)${RST}
  The yggdrasil B-side publishes separately and ${BOLD}deploys from ~/projects${RST}
  (its Dockerfile pulls sibling trees: co/game-core, comunicacao/*). See
  yggdrasil/docs/DEPLOY.md — not driven by this script.

${BOLD}DOCS${RST}
  docs/OPERATIONS.md → "Environments & Deploy"   (canonical flow; wins on conflict)
  docs/whatsapp-launch.md → "Deployment modes"   (managed vs self-host)
  CLAUDE.md → "Deployment"
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --public)       mode="public"; shift ;;
    --self-host)    mode="self-host"; shift ;;
    --rollback)     mode="rollback"; shift ;;
    --dry-run)      dry_run=true; shift ;;
    --yes|-y)       assume_yes=true; shift ;;
    --allow-branch) allow_branch=true; shift ;;
    --help|-h)      usage; exit 0 ;;
    *) echo "${RED}unknown arg: $1${RST}" >&2; echo "try: scripts/publish.sh --help" >&2; exit 2 ;;
  esac
done

# ── helpers ──────────────────────────────────────────────────────────────────
banner() {
  printf '%s\n' "${BOLD}${CYN}┌────────────────────────────────────────────────────────────────────┐${RST}"
  printf '%s\n' "${BOLD}${CYN}│${RST} ${YEL}publish is an explicit, outward-facing action${RST}                      ${BOLD}${CYN}│${RST}"
  printf '%s\n' "${BOLD}${CYN}│${RST} organize/map are ${DIM}automatic${RST} — publish/send are the deliberate hit. ${BOLD}${CYN}│${RST}"
  printf '%s\n' "${BOLD}${CYN}└────────────────────────────────────────────────────────────────────┘${RST}"
}

# run a command, or just print it under --dry-run.
run() {
  if [ "$dry_run" = true ]; then
    printf '%s  %s\n' "${DIM}WOULD RUN ▸${RST}" "${BOLD}$*${RST}"
  else
    printf '%s  %s\n' "${GRN}▸ RUN${RST}" "${BOLD}$*${RST}"
    "$@"
  fi
}

confirm_or_die() {
  # $1 = action description
  if [ "$assume_yes" = true ]; then
    echo "${DIM}--yes given; skipping interactive confirm.${RST}"
    return 0
  fi
  if [ ! -t 0 ]; then
    echo "${RED}REFUSING:${RST} $1 needs confirmation but stdin is not a TTY." >&2
    echo "  Re-run with ${BOLD}--yes${RST} to confirm non-interactively." >&2
    exit 1
  fi
  printf '%s' "${BOLD}${YEL}Confirm ${1}? type 'yes': ${RST}"
  read -r reply
  if [ "$reply" != "yes" ]; then
    echo "${RED}Aborted.${RST} (you typed: '${reply}')"
    exit 1
  fi
}

# guard rails that must hold before a REAL deploy/rollback.
check_guards() {
  local ok=true

  # clean tree
  if [ -n "$(git status --porcelain)" ]; then
    echo "${RED}✗ git working tree is dirty${RST} — commit or stash before publishing." >&2
    ok=false
  else
    echo "${GRN}✓${RST} git working tree clean"
  fi

  # branch
  local cur; cur="$(git rev-parse --abbrev-ref HEAD)"
  if [ "$cur" != "$EXPECTED_BRANCH" ]; then
    if [ "$allow_branch" = true ]; then
      echo "${YEL}!${RST} on branch '${cur}' (expected '${EXPECTED_BRANCH}') — allowed via --allow-branch"
    else
      echo "${RED}✗ on branch '${cur}', expected '${EXPECTED_BRANCH}'${RST} — use --allow-branch to override." >&2
      ok=false
    fi
  else
    echo "${GRN}✓${RST} on branch '${cur}'"
  fi

  if [ "$ok" != true ]; then
    echo "${RED}Guard rails failed — refusing to publish.${RST}" >&2
    exit 1
  fi
}

# ── self-host guidance (no action) ───────────────────────────────────────────
print_self_host() {
  banner
  cat <<EOF

${BOLD}SELF-HOST publish path${RST} (guidance only — nothing is executed)
Source of truth: ${BOLD}docs/whatsapp-launch.md${RST} → "Deployment modes".

You can ALWAYS self-host the whole thing — your data on your box, nothing
required from our infra or any third party. The code is identical across modes
(notification_providers::whatsapp_provider_cascade()); a self-host deploy with
only Evolution behaves exactly like a managed Cloud deploy — no fork.

  ${BOLD}1. Run co-web locally${RST} (binds loopback/LAN; tokens + content stay local,
     encrypted at rest). WhatsApp transport = ${BOLD}Evolution${RST} (QR-link your own
     number, outbound-only, no Meta app, no public webhook — survives CGNAT).

  ${BOLD}2. (Optional) public domain via Cloudflare Tunnel${RST} — CGNAT-proof, TLS,
     no open inbound port. Not needed for a private/LAN deploy.
        cloudflared tunnel login
        cloudflared tunnel create co-selfhost
        cloudflared tunnel route dns co-selfhost <your.domain>
        cloudflared tunnel run co-selfhost     # routes → local co-web

  ${BOLD}3. Autostart via launchd${RST} (macOS) so co-web + the tunnel survive reboot:
        ~/Library/LaunchAgents/com.artelonga.co.plist        (co-web)
        ~/Library/LaunchAgents/com.artelonga.cloudflared.plist (tunnel)
        launchctl load -w ~/Library/LaunchAgents/com.artelonga.co.plist
        launchctl load -w ~/Library/LaunchAgents/com.artelonga.cloudflared.plist

  ${BOLD}4. Ops you own${RST}: live backup (Litestream → B2/S3), UPS.

Self-host ops files (launchd plist, run/import scripts, cloudflared example)
live with the deploying repo, not the app. A private/LAN Evolution deploy needs
none of step 2–3. Full detail: docs/whatsapp-launch.md.
EOF
}

# ── managed public publish ───────────────────────────────────────────────────
do_public() {
  banner
  echo
  echo "${BOLD}Mode:${RST} --public  ${DIM}(managed → Fly app '${PROD_APP}', region gru)${RST}"
  [ "$dry_run" = true ] && echo "${BOLD}${YEL}DRY RUN — nothing outward will execute.${RST}"
  echo

  if [ "$dry_run" != true ]; then
    echo "${BOLD}Guard rails:${RST}"
    check_guards
    echo
  else
    echo "${DIM}(dry-run: guard rails + confirm are skipped; commands only printed)${RST}"
    echo
  fi

  echo "${BOLD}[1/4] Local correctness${RST}"
  run cargo test

  echo
  echo "${BOLD}[2/4] Pre-deploy gate${RST} ${DIM}(CO-446 disk check + fresh green pipeline report)${RST}"
  run bash scripts/pipeline-deploy-gate.sh

  echo
  echo "${BOLD}[3/4] The deliberate hit — deploy to production${RST}"
  if [ "$dry_run" != true ]; then
    confirm_or_die "deploy to PRODUCTION '${PROD_APP}'"
  fi
  run flyctl deploy -a "$PROD_APP" --build-arg GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"

  echo
  echo "${BOLD}[4/4] Post-deploy smoke${RST}"
  run bash scripts/smoke-prod.sh

  echo
  if [ "$dry_run" = true ]; then
    echo "${GRN}Dry run complete.${RST} Nothing was published. Re-run without --dry-run to publish."
  else
    echo "${GRN}${BOLD}Published.${RST} Undo with: ${BOLD}scripts/publish.sh --rollback${RST}"
  fi
}

# ── one-hit rollback / unpublish ─────────────────────────────────────────────
do_rollback() {
  banner
  echo
  echo "${BOLD}Mode:${RST} --rollback  ${DIM}(undo — return '${PROD_APP}' to its previous release)${RST}"
  [ "$dry_run" = true ] && echo "${BOLD}${YEL}DRY RUN — nothing outward will execute.${RST}"
  echo

  echo "${BOLD}[1/2] Inspect release history${RST}"
  run flyctl releases list -a "$PROD_APP"

  echo
  echo "${BOLD}[2/2] Roll back to the previous release${RST}"
  if [ "$dry_run" != true ]; then
    confirm_or_die "ROLL BACK production '${PROD_APP}' to the previous release"
    # Prefer the native rollback verb when this flyctl has it; otherwise fall
    # back to redeploying the previous release's image.
    if flyctl releases --help 2>/dev/null | grep -q 'rollback'; then
      run flyctl releases rollback -a "$PROD_APP"
    else
      echo "${YEL}This flyctl lacks 'releases rollback' — falling back to deploy --image <prev>.${RST}"
      prev_img="$(flyctl releases list -a "$PROD_APP" --json 2>/dev/null \
        | python3 -c 'import sys,json; r=json.load(sys.stdin); print(r[1]["ImageRef"] if len(r)>1 else "")' 2>/dev/null || true)"
      if [ -z "${prev_img:-}" ]; then
        echo "${RED}Could not determine the previous image ref.${RST} Roll back manually:" >&2
        echo "  flyctl releases list -a ${PROD_APP}" >&2
        echo "  flyctl deploy --image <previous-image-ref> -a ${PROD_APP}" >&2
        exit 1
      fi
      run flyctl deploy --image "$prev_img" -a "$PROD_APP"
    fi
    echo
    echo "${BOLD}Verify the rollback${RST}"
    run bash scripts/smoke-prod.sh
  else
    # dry-run: show the exact commands without resolving live state.
    echo "${DIM}WOULD RUN ▸${RST}  ${BOLD}flyctl releases rollback -a ${PROD_APP}${RST}   ${DIM}(if supported)${RST}"
    echo "${DIM}WOULD RUN ▸${RST}  ${BOLD}flyctl deploy --image <previous-image-ref> -a ${PROD_APP}${RST}   ${DIM}(fallback)${RST}"
    echo "${DIM}WOULD RUN ▸${RST}  ${BOLD}bash scripts/smoke-prod.sh${RST}"
  fi

  echo
  if [ "$dry_run" = true ]; then
    echo "${GRN}Dry run complete.${RST} Nothing was rolled back."
  else
    echo "${GRN}${BOLD}Rolled back.${RST}"
  fi
}

# ── dispatch ─────────────────────────────────────────────────────────────────
case "$mode" in
  public)    do_public ;;
  self-host) print_self_host ;;
  rollback)  do_rollback ;;
esac
