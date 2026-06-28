# CO integration samples (CO-503)

Reviewable, self-contained samples showing how candidate OSS tools plug into the
**CO-503 canonical tool contract** — **any OSS becomes a CO tool by manifest, with
no binary recompile**. For owner review; nothing here is wired into the build.

## The pattern

A tool is two things:

1. **A manifest** (`*.yaml`) — `name`, `description`, `input_schema`, `tool_type`
   (`deterministic` | `predictive`), and either a `command` (subprocess) or a
   `url` (HTTP service). Optional: `category`, `status`, `dependencies`, `secrets`.
   This is the exact `ToolManifest` from `co/src/canon_tool.rs`; see the
   references in `tools.d/echo.example.yaml` and `tools.d/yt-summarizer.example.yaml`.
2. **An implementation** — a script (stdin JSON → stdout JSON) for `command`
   tools, or an HTTP service (`POST` JSON → JSON) for `url` tools.

CO discovers manifests from a **trusted directory** only, and **only when
`CO_ENABLE_EXTERNAL_TOOLS=1`** (default OFF — see `external_tools_enabled` and the
trust-boundary doc in `canon_tool.rs`). Subprocess tools run with **no ambient
env**; only variables named in `secrets:` are forwarded. An agent enumerates tools
via `CanonicalToolRegistry::list` and calls `run(args)` — native in-binary tools
and these out-of-process tools are invoked **identically**.

> The sample manifests ship with `status: inactive` so they are documentation,
> not live dependencies. Flip to `active` (and place in a trusted tools dir) to
> register one.

## The samples

| Sample | Kind | tool_type | Reach | Demonstrates |
|---|---|---|---|---|
| [`r-stats/`](r-stats/) | CO-503 tool | `deterministic` | `command` (Rscript) | R / RStudio vision — any R script as a tool (stateful REPL = CO-524). |
| [`rag-service/`](rag-service/) | CO-503 tool | `predictive` | `url` (HTTP) | LlamaIndex / production-agentic-rag behind the manifest (CO-525). |
| [`embed/`](embed/) | CO-503 tool | `deterministic` | `command` (python3) | Embeddings as a tool; output feeds the CO-517 Vectorizer + sqlite-vec. |
| [`topology/`](topology/) | CO-503 tool | `deterministic` | `command` (python3) | comunicacao meaning-topology as a tool any surface calls; PPMI offline / neural overlay = CO-517 Vectorizer (CO-528). |
| [`ui-web-awesome/`](ui-web-awesome/) | **frontend (NOT a tool)** | — | — | Web Awesome (Shoelace v3) components, no-rewrite UI path; CSS-var theming. |

## How to try each

```bash
# r-stats (needs R/Rscript)
echo '{"values":[10,12,23,23,16,23,21,16]}' | Rscript examples/integrations/r-stats/summary.R

# rag-service (stdlib Python; start it, then curl)
python3 examples/integrations/rag-service/service.py   # then: curl -s -X POST localhost:9100/query -d '{"query":"tool contract"}'

# embed (offline deterministic fallback; drop CO_EMBED_BACKEND to try Ollama)
echo '{"texts":["hello","world"]}' | CO_EMBED_BACKEND=fallback python3 examples/integrations/embed/embed.py

# topology (offline; ships its own tiny cross-language lexicon)
echo '{"word":"palavra"}' | python3 examples/integrations/topology/topology.py

# ui-web-awesome (static server, needs internet for the CDN)
python3 -m http.server -d examples/integrations/ui-web-awesome 8080   # open http://localhost:8080
```

Each sample's own `README.md` covers what it demonstrates, the AS-IS vs TO-BE
integration, and how an agent calls it (input_schema → output).
