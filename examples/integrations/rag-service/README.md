# rag-service — RAG as a predictive HTTP CO tool

**What it demonstrates.** A retrieval-augmented-generation service (the
**production-agentic-rag / LlamaIndex** add-on, **CO-525**) plugs into CO
**unchanged, behind the manifest**. It is the `predictive` / HTTP shape of the
CO-503 contract: the manifest carries a `url`, CO's HTTP invoker (injected from
`co-web`, which has `reqwest`) POSTs the JSON args and reads JSON back. The agent
calls it exactly like any other tool and never knows an LLM-backed RAG pipeline
runs behind the URL — same adapter shape as `tools.d/yt-summarizer.example.yaml`.

`service.py` is a stdlib-only stub with a fake in-memory corpus so a reviewer can
run and curl it offline. A real deployment replaces `retrieve()` / `synthesize()`
with LlamaIndex; **the HTTP contract and the CO manifest stay identical.**

## AS-IS vs TO-BE

| | |
|---|---|
| **AS-IS** | Knowledge retrieval is a separate app. To "ask the corpus" you leave CO, query a notebook/service by hand, and copy the answer back. The agent has no retrieval. |
| **TO-BE** | `rag-service.yaml` registers the endpoint as a `predictive` tool. The agent picks it when a question needs grounded answers, calls `POST /query`, and gets `{answer, sources}` back in-loop. Swapping the engine (LlamaIndex ↔ another RAG stack) is a service-side change; CO is untouched. |

## How an agent calls it

Input (matches `input_schema`):
```json
{ "query": "What is the canonical tool contract?", "k": 2 }
```
Output:
```json
{
  "answer": "CO-503 defines the canonical tool contract: any OSS becomes a tool via a manifest, with no binary recompile. (+1 more source)",
  "sources": [
    { "id": "co-503", "text": "CO-503 defines the canonical tool contract...", "score": 3 },
    { "id": "manifest", "text": "A tool manifest declares name, description...", "score": 1 }
  ]
}
```

## Try it

```bash
# 1. start the stub (foreground; Ctrl-C to stop)
python3 examples/integrations/rag-service/service.py

# 2. in another shell:
curl -s localhost:9100/health
curl -s -X POST localhost:9100/query \
  -H 'Content-Type: application/json' \
  -d '{"query":"What is the canonical tool contract?","k":2}' | python3 -m json.tool
```
