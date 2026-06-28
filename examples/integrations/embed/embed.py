#!/usr/bin/env python3
"""embed — CO-503 deterministic embedding tool (feeds the CO-517 Vectorizer).

Contract (matches co/src/canon_tool.rs::SubprocessInvoker):
  * Reads ONE JSON object from stdin: {"texts": ["...", "..."]}.
  * Writes ONE JSON object to stdout: {"vectors": [[...], ...], "model", "dim"}.
  * Exits non-zero with a message on stderr on error.

Two backends, selected automatically:
  * NEURAL   — a local Ollama embeddings endpoint (nomic-embed-text at
               http://localhost:11434/api/embeddings), when reachable.
  * FALLBACK — a deterministic hashed bag-of-words vector, so the tool runs
               offline for review with no model installed.

"Embedding is just a tool": this output feeds the Vectorizer trait + sqlite-vec
store. Neural vs statistical is a swap — set CO_EMBED_BACKEND=fallback to force
the offline path, or leave it unset to try Ollama first.

Stdlib only (urllib); no third-party deps.
"""

import hashlib
import json
import math
import os
import sys
import urllib.error
import urllib.request

OLLAMA_URL = os.environ.get(
    "CO_OLLAMA_URL", "http://localhost:11434/api/embeddings"
)
OLLAMA_MODEL = os.environ.get("CO_EMBED_MODEL", "nomic-embed-text")
FALLBACK_DIM = 256


def fail(msg: str):
    sys.stderr.write(msg + "\n")
    sys.exit(1)


def ollama_embed(text: str, timeout: float = 2.0):
    """Return a vector from Ollama, or raise on any failure."""
    body = json.dumps({"model": OLLAMA_MODEL, "prompt": text}).encode("utf-8")
    req = urllib.request.Request(
        OLLAMA_URL, data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        data = json.loads(resp.read())
    vec = data.get("embedding")
    if not isinstance(vec, list) or not vec:
        raise ValueError("ollama returned no embedding")
    return [float(x) for x in vec]


def fallback_embed(text: str, dim: int = FALLBACK_DIM):
    """Deterministic hashed bag-of-words vector, L2-normalized.

    Same text -> same vector, always, with no network. Good enough to prove the
    pipeline; not semantically meaningful like a real model.
    """
    vec = [0.0] * dim
    for token in text.lower().split():
        h = hashlib.sha256(token.encode("utf-8")).digest()
        idx = int.from_bytes(h[:4], "big") % dim
        sign = 1.0 if h[4] & 1 else -1.0
        vec[idx] += sign
    norm = math.sqrt(sum(v * v for v in vec))
    if norm > 0:
        vec = [v / norm for v in vec]
    return vec


def main():
    raw = sys.stdin.read() or "{}"
    try:
        args = json.loads(raw)
    except json.JSONDecodeError as e:
        fail(f"invalid JSON on stdin: {e}")

    texts = args.get("texts")
    if not isinstance(texts, list) or not texts:
        fail("`texts` (non-empty array of strings) is required")
    if not all(isinstance(t, str) for t in texts):
        fail("every item in `texts` must be a string")

    forced = os.environ.get("CO_EMBED_BACKEND", "").strip().lower()
    use_neural = forced != "fallback"

    vectors = []
    model = None
    if use_neural:
        try:
            vectors = [ollama_embed(t) for t in texts]
            model = OLLAMA_MODEL
        except (urllib.error.URLError, OSError, ValueError, TimeoutError):
            vectors = []  # fall through to statistical backend

    if not vectors:
        vectors = [fallback_embed(t) for t in texts]
        model = "hashed-bow-fallback"

    out = {
        "vectors": vectors,
        "model": model,
        "dim": len(vectors[0]) if vectors else 0,
    }
    sys.stdout.write(json.dumps(out) + "\n")


if __name__ == "__main__":
    main()
