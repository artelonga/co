# MempalaceCoBackend — CO storage backend for mempalace

`mempalace_co_backend.py` implements mempalace's `BaseBackend` ABC using CO's HTTP API as the
durable storage layer.  Chunks (drawers) live in CO's CAS + entry graph, so you get versioning,
cross-device sync, and a unified search surface without touching mempalace's mining or interactive
UX.

## How it works

| What | Where in CO |
|------|-------------|
| Chunk content bytes | `POST /api/v1/blobs` (sha256 CAS, deduped) |
| Embedding bytes (raw f32) | `POST /api/v1/blobs` (same CAS, stored by their own hash) |
| Metadata + pointers | `PUT /api/v1/universes/:slug/vault/mempalace/<wing>/<room>/<id>.md` |

Every vault entry is a markdown file with YAML frontmatter:

```yaml
---
type: "mempalace-chunk"
chunk_id: "<id>"
blob_hash: "<sha256>"
wing: "<wing-name>"
room: "<room-name>"
metadata: { ...arbitrary mempalace metadata... }
embedding_dim: 384
embedding_blob: "<sha256-of-f32-bytes>"
---
<original document text>
```

CAS blobs are content-addressed and **never deleted** by the shim.  Two chunks with identical
content or identical embeddings automatically share storage.

## Requirements

- Python 3.11+
- `requests` or the stdlib `urllib.request` (the shim uses stdlib only — no extra dependencies)
- A running CO server (local `cargo run -p co-web -- serve` or `co-artelonga.fly.dev`)
- A long-lived API token for the CO server

## Configuration

In mempalace's `config.yaml`:

```yaml
backend: scripts.mempalace_co_backend.MempalaceCoBackend
backend_args:
  server: https://co.artelonga.com.br   # or http://127.0.0.1:3000 for local
  universe: mempalace                   # CO universe slug (auto-created on first write)
  token_env: CO_API_TOKEN               # name of env-var that holds your API token
  wing: default                         # optional — maps to vault sub-folder
  room: main                            # optional — maps to vault sub-folder
  timeout: 30                           # optional HTTP timeout in seconds
```

Set the token before starting mempalace:

```bash
export CO_API_TOKEN=<your-long-lived-token>
```

Generate a token via:

```bash
curl -X POST https://co.artelonga.com.br/api/v1/auth/token \
  -H "Content-Type: application/json" \
  -b "session=<your-session-cookie>"
```

## Semantic search caveat (keyword-only until CO-164)

`query(query_texts, n_results)` currently falls back to **keyword search** via CO's
`GET /api/v1/universes/:slug/entries?q=<text>` because CO does not yet have a vector index
(that's CO-164).

Keyword search works well for recall-oriented retrieval (e.g. "find notes containing 'neural
network'") but does not rank by embedding similarity.

When CO-164 ships, override `_vector_search` in a subclass:

```python
class MyBackend(MempalaceCoBackend):
    def _vector_search(self, query_text, n_results, where):
        resp = self._get(
            f"/api/v1/universes/{self._universe}/vector-search",
            params={"q": query_text, "n": n_results},
        )
        return {
            "ids": resp["ids"],
            "documents": resp["documents"],
            "embeddings": resp["embeddings"],
            "metadatas": resp["metadatas"],
            "distances": resp["distances"],
        }
```

The base class calls `_vector_search` first; if it returns `None` the keyword fallback is used.

## Filter support

The `where` parameter uses Chroma-style operators.  Operators supported server-side by CO's
frontmatter index:

| Operator | CO equivalent |
|----------|--------------|
| `$eq`    | `eq`         |
| `$gt`    | `gt`         |
| `$gte`   | `gte`        |
| `$lt`    | `lt`         |
| `$lte`   | `lte`        |
| `$in`    | `in`         |

`$ne`, `$nin`, `$and`, `$or` are evaluated **client-side** after fetching the candidate set.
They work correctly but are less efficient for large universes.

## Running the tests

Unit and mock-HTTP tests (no server needed):

```bash
python3 scripts/test_mempalace_co.py
```

Integration tests against a live CO server:

```bash
# Start CO locally
cargo run -p co-web -- serve --port 3000 &

# Generate a token and export it
export CO_API_TOKEN=<token>
export CO_INTEGRATION_TEST=1
export CO_SERVER=http://127.0.0.1:3000

python3 scripts/test_mempalace_co.py
```

## Out of scope (v1)

- **Vector index** — CO-164 unblocks true semantic search; `query` is keyword-only for now.
- **Multi-universe routing** — one CO universe per mempalace instance; per-wing routing is a
  sub-folder convention, not separate universes.
- **Conflict handling** — mempalace is single-user; the shim relies on CO's last-write-wins.
