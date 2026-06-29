# topology — comunicacao meaning-topology as a CO tool (CO-503 → CO-517)

**What it demonstrates.** comunicacao's **cross-language meaning-topology** becomes
**just a tool**. Through the CO-503 canonical contract, the topology logic registers
by manifest with no recompile, so **any surface calls it identically** — yggdrasil /
topologia, co agents, the WhatsApp bot. The logic stops living inside yggdrasil-core
and stops depending on the `../comunicacao` sibling dir.

It is the `deterministic` / subprocess shape: CO's `SubprocessInvoker` writes the
JSON args to the script's stdin and parses its stdout as JSON (see
`co/src/canon_tool.rs`). Given `{word, lang?, k?}` it returns the word's nearest
**concept-neighbors** and its **translations** across languages.

Two interchangeable backends (the CO-517 Vectorizer swap):

- **Statistical** — co-occurrence / **PPMI** over a shipped artifact. Runs
  **offline**. This sample embeds its own tiny lexicon (a few pt/es/yoruba/guarani
  words linked by shared concepts with co-occurrence counts), so a reviewer can run
  it with **no `../comunicacao` dir and no Ollama**.
- **Neural overlay** — the predictive variant: `nomic` embeddings via Ollama,
  behind the same CO-517 Vectorizer trait. Not exercised here; choosing it is a
  flag, not a code change.

## AS-IS vs TO-BE

| | |
|---|---|
| **AS-IS** | yggdrasil reads `COMUNICACAO_DIR=../comunicacao` and **owns the topology logic** inside yggdrasil-core. The meaning-topology is reachable only from the game; it is coupled to a sibling checkout, so it can't ship to prod without the source dirs. |
| **TO-BE** | A portable **`topologia.json`** artifact + this tool. The topology is a CO-503 tool any surface calls through the canonical contract, backed by the **CO-517 Vectorizer** (neural `nomic` OR statistical PPMI). **Ship the artifact, not the sibling dirs** — this unblocks the **topologia prod deploy**. |

## How an agent calls it

The agent sees the `input_schema` and supplies matching JSON; the tool returns a
JSON object.

Input:
```json
{ "word": "ñe'ẽ", "lang": "gn", "k": 5 }
```
Output (`ñe'ẽ` = word AND soul in Guarani — the bridge term):
```json
{
  "word": "ñe'ẽ",
  "lang": "gn",
  "neighbors": [
    { "term": "ọrọ", "lang": "yo", "score": 0.9464 },
    { "term": "palabra", "lang": "es", "score": 0.9102 },
    { "term": "palavra", "lang": "pt", "score": 0.9102 }
  ],
  "translations": [
    { "term": "palavra", "lang": "pt" },
    { "term": "palabra", "lang": "es" },
    { "term": "ọrọ", "lang": "yo" }
  ],
  "method": "co-occurrence/PPMI",
  "note": "neural overlay = the CO-517 Vectorizer variant"
}
```

## Try it (stdlib Python, offline)

```bash
echo '{"word":"ñe'\''ẽ","lang":"gn","k":5}' \
  | python3 examples/integrations/topology/topology.py

echo '{"word":"palavra"}' \
  | python3 examples/integrations/topology/topology.py
```

No R, no Ollama, no `../comunicacao` — the sample ships its own lexicon. The
stdin→stdout JSON contract is the whole point; the artifact and the Vectorizer
behind it are interchangeable.
