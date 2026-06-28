#!/usr/bin/env bash
# review-integrations.sh — e2e review of the CO-503 tool-integration samples.
#
# Runs each integration sample end-to-end so you can SEE the composability pattern
# (input -> output) and give feedback per sample. Offline-capable samples run as-is;
# samples needing an external dep (R, a browser) are noted and skipped gracefully.
#
#   Usage:  scripts/review-integrations.sh
#
# The point under review: every tool below plugs into the CO-503 canonical contract by
# a manifest (examples/integrations/<name>/<name>.yaml) — adding one needs NO recompile.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EX="$ROOT/examples/integrations"
ran=0; skip=0

hr(){ printf '\n\033[1m===== %s =====\033[0m\n' "$1"; }
note(){ printf '  \033[2m%s\033[0m\n' "$1"; }

hr "1/5  topology — comunicacao meaning-topology as a tool (deterministic, offline PPMI)"
note "manifest: examples/integrations/topology/topology.yaml  | input: {word, lang?, k?}"
if echo '{"word":"ñe'\''ẽ","lang":"gn","k":5}' | python3 "$EX/topology/topology.py"; then ran=$((ran+1)); fi

hr "2/5  embed — embeddings tool feeding the CO-517 Vectorizer (offline hashed fallback)"
note "manifest: examples/integrations/embed/embed.yaml  | input: {texts[]}  (neural via Ollama if up)"
if echo '{"texts":["palavra","word","alma"]}' | CO_EMBED_BACKEND=fallback python3 "$EX/embed/embed.py" \
   | python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps({"model":d["model"],"dim":d["dim"],"n_vectors":len(d["vectors"]),"vec0_first6":[round(x,3) for x in d["vectors"][0][:6]]}))'; then ran=$((ran+1)); fi

hr "3/5  rag-service — external HTTP RAG add-on (LlamaIndex / production-agentic-rag, CO-525)"
note "manifest: examples/integrations/rag-service/rag-service.yaml  | url: POST /query {query}"
python3 "$EX/rag-service/service.py" >/dev/null 2>&1 & SVC=$!
sleep 1
if curl -s --max-time 3 -X POST localhost:9100/query -H 'content-type: application/json' -d '{"query":"canonical tool contract"}'; then echo; ran=$((ran+1)); else echo "  (service did not respond)"; fi
kill "$SVC" 2>/dev/null

hr "4/5  r-stats — an R script as a deterministic tool (R / RStudio path; stateful REPL = CO-524)"
note "manifest: examples/integrations/r-stats/r-stats.yaml  | input: {values[] | csv_path}"
if command -v Rscript >/dev/null 2>&1; then
  echo '{"values":[10,12,23,16,21,23]}' | Rscript "$EX/r-stats/summary.R" && ran=$((ran+1))
else
  echo "  SKIP — Rscript not installed (it is a declared 'dependencies' entry; the stdin->stdout JSON contract is reviewable in summary.R)"; skip=$((skip+1))
fi

hr "5/5  ui-web-awesome — framework-agnostic web components (frontend path, NOT a CO-503 tool)"
note "Open in a browser to review:"
echo "    python3 -m http.server -d $EX/ui-web-awesome 8080   # then http://localhost:8080"
skip=$((skip+1))

hr "CO-503 contract tests (canonical contract + external adapter)"
if (cd "$ROOT" && cargo test -p co-engine canon_tool 2>/dev/null | grep "test result" | head -1); then :; else
  note "(run 'cargo test -p co-engine canon_tool' once the crate is built)"
fi

hr "SUMMARY"
echo "  ran: $ran   skipped/manual: $skip"
echo "  Review the input->output above. The pattern: drop a manifest in a trusted dir,"
echo "  flip status:active + CO_ENABLE_EXTERNAL_TOOLS=1, and the agent calls it — no recompile."
echo "  Feedback welcome per sample (shape of input_schema, the I/O contract, what's missing)."
