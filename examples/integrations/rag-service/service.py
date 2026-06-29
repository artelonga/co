#!/usr/bin/env python3
"""rag-service — CO-503 / CO-525 predictive HTTP tool stub.

A minimal Python *stdlib-only* HTTP service that mirrors the contract a real
LlamaIndex / production-agentic-rag deployment would expose:

    POST /query   {"query": "...", "k": 3}  -> {"answer": "...", "sources": [...]}
    GET  /health                            -> {"status": "ok"}

It ships with a tiny in-memory corpus and a trivial keyword "retriever" so a
reviewer can curl it offline. Swapping in a real RAG engine means replacing
`retrieve()` / `synthesize()` — the HTTP contract (and the CO manifest) stay
identical. That is the whole point: the agent calls this like any other tool and
never knows what runs behind the URL.

Run:  python3 service.py            # listens on 127.0.0.1:9100
"""

import json
import re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HOST = "127.0.0.1"
PORT = 9100

# --- fake corpus -----------------------------------------------------------
# (id, text) — a real service would index documents into a vector store.
CORPUS = [
    ("co-503", "CO-503 defines the canonical tool contract: any OSS becomes a "
               "tool via a manifest, with no binary recompile."),
    ("co-517", "CO-517 introduces the Vectorizer trait and a sqlite-vec store "
               "for embeddings used in semantic search."),
    ("co-524", "CO-524 is the stateful RStudio-web REPL, a follow-up to running "
               "one-shot R scripts as deterministic tools."),
    ("co-525", "CO-525 plugs a production-agentic-rag / LlamaIndex service into "
               "CO over HTTP behind the canonical tool manifest."),
    ("manifest", "A tool manifest declares name, description, input_schema, "
                 "tool_type, and either a command or a url."),
]

STOP = {"the", "a", "an", "is", "of", "to", "and", "for", "into", "via",
        "what", "how", "does", "do", "are", "with", "over"}


def retrieve(query: str, k: int):
    """Trivial keyword overlap scorer — stands in for vector retrieval."""
    terms = {t for t in re.findall(r"[a-z0-9]+", query.lower()) if t not in STOP}
    scored = []
    for doc_id, text in CORPUS:
        words = set(re.findall(r"[a-z0-9]+", text.lower()))
        score = len(terms & words)
        if score:
            scored.append((score, doc_id, text))
    scored.sort(reverse=True)
    return [
        {"id": d, "text": t, "score": s}
        for s, d, t in scored[: max(1, k)]
    ]


def synthesize(query: str, sources):
    """Stand-in for an LLM answer step: stitch the top passages together."""
    if not sources:
        return "No relevant passages were found for that query."
    lead = sources[0]["text"]
    extra = len(sources) - 1
    tail = f" (+{extra} more source{'s' if extra != 1 else ''})" if extra else ""
    return f"{lead}{tail}"


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, payload):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.rstrip("/") == "/health":
            self._send(200, {"status": "ok", "corpus_size": len(CORPUS)})
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        if self.path.rstrip("/") != "/query":
            self._send(404, {"error": "not found"})
            return
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            args = json.loads(raw or b"{}")
        except json.JSONDecodeError:
            self._send(400, {"error": "invalid JSON body"})
            return
        query = args.get("query")
        if not isinstance(query, str) or not query.strip():
            self._send(400, {"error": "`query` (string) is required"})
            return
        k = args.get("k", 3)
        if not isinstance(k, int):
            k = 3
        sources = retrieve(query, k)
        self._send(200, {"answer": synthesize(query, sources), "sources": sources})

    def log_message(self, *_args):
        pass  # keep stdout clean for reviewers


def main():
    srv = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"rag-service stub listening on http://{HOST}:{PORT}  "
          f"(POST /query, GET /health) — Ctrl-C to stop")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print("\nshutting down")
        srv.shutdown()


if __name__ == "__main__":
    main()
