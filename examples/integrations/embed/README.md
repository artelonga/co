# embed — embeddings as a deterministic CO tool (feeds CO-517)

**What it demonstrates.** Embedding is **just a tool**. Through the CO-503
canonical contract, an embedding script registers by manifest with no recompile;
its output (`{vectors, model, dim}`) feeds the **CO-517 Vectorizer trait +
sqlite-vec store** that powers semantic search.

It is the `deterministic` / subprocess shape: CO's `SubprocessInvoker` writes the
JSON args to the script's stdin and parses its stdout as JSON. The script has two
interchangeable backends:

- **Neural** — a local **Ollama** embeddings endpoint (`nomic-embed-text` at
  `http://localhost:11434/api/embeddings`), used when reachable.
- **Statistical fallback** — a deterministic hashed bag-of-words vector, so the
  tool **runs offline for review** with nothing installed.

The swap is a flag: leave the env unset to try Ollama first, or set
`CO_EMBED_BACKEND=fallback` to force the offline path. Same tool contract either
way — the Vectorizer downstream doesn't care which produced the vectors.

## AS-IS vs TO-BE

| | |
|---|---|
| **AS-IS** | No embedding step in-platform. Semantic search needs a bespoke pipeline; producing vectors is out-of-band and not agent-reachable. |
| **TO-BE** | `embed.yaml` registers the script as a tool. CO (or an agent) calls it to vectorize content; the vectors land in the CO-517 sqlite-vec store. Upgrading from the statistical fallback to neural `nomic-embed-text` is a config flag, not a code change. |

## How an agent calls it

Input (matches `input_schema`):
```json
{ "texts": ["canonical tool contract", "vector store"] }
```
Output (dim depends on backend — 256 for fallback, 768 for nomic-embed-text):
```json
{
  "vectors": [[0.12, -0.04, ...], [-0.08, 0.31, ...]],
  "model": "hashed-bow-fallback",
  "dim": 256
}
```

## Try it

```bash
# offline statistical fallback (always works, deterministic)
echo '{"texts":["canonical tool contract","vector store"]}' \
  | CO_EMBED_BACKEND=fallback python3 examples/integrations/embed/embed.py

# neural path (requires: ollama pull nomic-embed-text + ollama serve)
echo '{"texts":["canonical tool contract"]}' \
  | python3 examples/integrations/embed/embed.py
```
