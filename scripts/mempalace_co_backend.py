#!/usr/bin/env python3
"""MempalaceCoBackend — mempalace BaseBackend implementation backed by CO's HTTP API.

Chunks (documents) are stored as:
  - content bytes  → POST /api/v1/blobs  (CAS, deduped by sha256)
  - embedding bytes → POST /api/v1/blobs (raw f32 array, same CAS)
  - metadata entry  → PUT /api/v1/universes/:slug/vault/mempalace/<wing>/<room>/<id>.md

This file has no import-time dependency on mempalace.  It only needs `requests`.
Mempalace loads it via its plugin config:

    backend: scripts.mempalace_co_backend.MempalaceCoBackend
    backend_args:
      server: https://co.artelonga.com.br
      universe: mempalace
      token_env: CO_API_TOKEN      # env-var name that holds the API token

See mempalace_co_README.md for full setup instructions.
"""

import hashlib
import json
import os
import struct
import threading
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Any, Optional

_DEFAULT_WING = "default"
_DEFAULT_ROOM = "main"


class MempalaceCoBackend:
    """BaseBackend-compatible backend that stores mempalace drawers in CO."""

    def __init__(
        self,
        server: str,
        universe: str = "mempalace",
        token_env: str = "CO_API_TOKEN",
        wing: str = _DEFAULT_WING,
        room: str = _DEFAULT_ROOM,
        timeout: int = 30,
    ) -> None:
        self._server = server.rstrip("/")
        self._universe = universe
        self._token = os.environ.get(token_env, "")
        self._wing = wing
        self._room = room
        self._timeout = timeout
        self._lock = threading.Lock()

    # ------------------------------------------------------------------
    # BaseBackend interface
    # ------------------------------------------------------------------

    def upsert(
        self,
        ids: list[str],
        documents: list[str],
        embeddings: list[list[float]],
        metadatas: list[dict[str, Any]],
    ) -> None:
        """Write chunks in parallel: blob (content), blob (embedding), vault entry."""
        for chunk_id, document, embedding, metadata in zip(
            ids, documents, embeddings, metadatas
        ):
            self._upsert_one(chunk_id, document, embedding, metadata)

    def get(
        self,
        ids: list[str],
        where: Optional[dict] = None,
    ) -> dict[str, Any]:
        """Fetch chunks by id.  Returns {ids, documents, embeddings, metadatas}."""
        result_ids: list[str] = []
        result_docs: list[str] = []
        result_embeddings: list[list[float]] = []
        result_metas: list[dict] = []

        for chunk_id in ids:
            row = self._get_one(chunk_id)
            if row is None:
                continue
            if where and not _matches_where(row["metadata"], where):
                continue
            result_ids.append(chunk_id)
            result_docs.append(row["document"])
            result_embeddings.append(row["embedding"])
            result_metas.append(row["metadata"])

        return {
            "ids": result_ids,
            "documents": result_docs,
            "embeddings": result_embeddings,
            "metadatas": result_metas,
        }

    def query(
        self,
        query_texts: list[str],
        n_results: int = 10,
        where: Optional[dict] = None,
    ) -> dict[str, Any]:
        """Search chunks.  Falls back to keyword search until CO-164 ships.

        When CO-164 (vector index) is available, override `_vector_search` in a
        subclass and this method will use it automatically.
        """
        all_ids: list[list[str]] = []
        all_docs: list[list[str]] = []
        all_embeddings: list[list[list[float]]] = []
        all_metas: list[list[dict]] = []
        all_distances: list[list[float]] = []

        for query_text in query_texts:
            vector_hits = self._vector_search(query_text, n_results, where)
            if vector_hits is not None:
                all_ids.append(vector_hits["ids"])
                all_docs.append(vector_hits["documents"])
                all_embeddings.append(vector_hits["embeddings"])
                all_metas.append(vector_hits["metadatas"])
                all_distances.append(vector_hits["distances"])
            else:
                hits = self._keyword_search(query_text, n_results, where)
                all_ids.append(hits["ids"])
                all_docs.append(hits["documents"])
                all_embeddings.append(hits["embeddings"])
                all_metas.append(hits["metadatas"])
                all_distances.append(hits["distances"])

        return {
            "ids": all_ids,
            "documents": all_docs,
            "embeddings": all_embeddings,
            "metadatas": all_metas,
            "distances": all_distances,
        }

    def delete(
        self,
        ids: Optional[list[str]] = None,
        where: Optional[dict] = None,
    ) -> None:
        """Remove vault entries.  CAS blobs are content-addressed and never deleted."""
        if ids:
            for chunk_id in ids:
                self._delete_vault_entry(chunk_id)
        elif where:
            candidates = self._search_entries(q=None, where=where, limit=10000)
            for entry in candidates:
                fm = entry.get("frontmatter") or {}
                if chunk_id := fm.get("chunk_id"):
                    self._delete_vault_entry(chunk_id)

    # ------------------------------------------------------------------
    # Hook for CO-164 vector search (no-op until the endpoint ships)
    # ------------------------------------------------------------------

    def _vector_search(
        self,
        query_text: str,
        n_results: int,
        where: Optional[dict],
    ) -> Optional[dict]:
        """Return vector-search results, or None to fall back to keyword search.

        Override in a subclass once CO-164 ships:

            def _vector_search(self, query_text, n_results, where):
                resp = self._get(f"/api/v1/universes/{self._universe}/vector-search",
                                 params={"q": query_text, "n": n_results})
                # parse and return {ids, documents, embeddings, metadatas, distances}
        """
        return None

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _upsert_one(
        self,
        chunk_id: str,
        document: str,
        embedding: list[float],
        metadata: dict,
    ) -> None:
        content_bytes = document.encode("utf-8")
        embedding_bytes = struct.pack(f"{len(embedding)}f", *embedding)

        with ThreadPoolExecutor(max_workers=2) as ex:
            f_content = ex.submit(self._put_blob, content_bytes)
            f_emb = ex.submit(self._put_blob, embedding_bytes)
            blob_hash = f_content.result()
            emb_hash = f_emb.result()

        frontmatter = {
            "type": "mempalace-chunk",
            "chunk_id": chunk_id,
            "blob_hash": blob_hash,
            "wing": self._wing,
            "room": self._room,
            "metadata": metadata,
            "embedding_dim": len(embedding),
            "embedding_blob": emb_hash,
        }
        body = _render_vault_md(frontmatter, document)
        path = self._chunk_path(chunk_id)
        self._put_vault(path, body)

    def _get_one(self, chunk_id: str) -> Optional[dict]:
        path = self._chunk_path(chunk_id)
        entry = self._get_vault(path)
        if entry is None:
            return None
        fm = entry.get("frontmatter") or {}
        blob_hash = fm.get("blob_hash", "")
        emb_hash = fm.get("embedding_blob", "")
        emb_dim = fm.get("embedding_dim", 0)

        document = ""
        embedding: list[float] = []

        if blob_hash:
            raw = self._get_blob(blob_hash)
            if raw is not None:
                document = raw.decode("utf-8", errors="replace")

        if emb_hash and emb_dim:
            raw_emb = self._get_blob(emb_hash)
            if raw_emb is not None:
                embedding = list(struct.unpack(f"{emb_dim}f", raw_emb))

        return {
            "document": document,
            "embedding": embedding,
            "metadata": fm.get("metadata") or {},
        }

    def _keyword_search(
        self,
        query_text: str,
        n_results: int,
        where: Optional[dict],
    ) -> dict:
        entries = self._search_entries(q=query_text, where=where, limit=n_results)
        ids, docs, embeddings, metas, distances = [], [], [], [], []
        for entry in entries:
            fm = entry.get("frontmatter") or {}
            chunk_id = fm.get("chunk_id")
            if not chunk_id:
                continue
            row = self._get_one(chunk_id)
            if row is None:
                continue
            ids.append(chunk_id)
            docs.append(row["document"])
            embeddings.append(row["embedding"])
            metas.append(row["metadata"])
            distances.append(1.0)  # no distance available from keyword search

        return {
            "ids": ids,
            "documents": docs,
            "embeddings": embeddings,
            "metadatas": metas,
            "distances": distances,
        }

    def _search_entries(
        self,
        q: Optional[str],
        where: Optional[dict],
        limit: int,
    ) -> list[dict]:
        params: dict[str, Any] = {
            "type": "mempalace-chunk",
            "limit": limit,
        }
        if q:
            params["q"] = q
        if where:
            co_filter = _where_to_co_filter(where)
            if co_filter:
                params["filter"] = json.dumps(co_filter)

        try:
            resp = self._get(
                f"/api/v1/universes/{self._universe}/entries",
                params=params,
            )
        except urllib.error.HTTPError:
            return []

        entries = resp.get("entries", [])

        if where:
            entries = [
                e
                for e in entries
                if _matches_where((e.get("frontmatter") or {}).get("metadata") or {}, where)
            ]
        return entries

    def _chunk_path(self, chunk_id: str) -> str:
        safe_id = urllib.parse.quote(chunk_id, safe="")
        return f"mempalace/{self._wing}/{self._room}/{safe_id}.md"

    # ------------------------------------------------------------------
    # HTTP primitives
    # ------------------------------------------------------------------

    def _headers(self, content_type: str = "application/json") -> dict:
        h = {"Authorization": f"Bearer {self._token}"}
        if content_type:
            h["Content-Type"] = content_type
        return h

    def _url(self, path: str) -> str:
        return self._server + path

    def _put_blob(self, data: bytes) -> str:
        req = urllib.request.Request(
            self._url("/api/v1/blobs"),
            method="POST",
            data=data,
            headers={
                "Authorization": f"Bearer {self._token}",
                "Content-Type": "application/octet-stream",
            },
        )
        with urllib.request.urlopen(req, timeout=self._timeout) as resp:
            body = json.loads(resp.read())
            return body["hash"]

    def _get_blob(self, hash_hex: str) -> Optional[bytes]:
        req = urllib.request.Request(
            self._url(f"/api/v1/blobs/{hash_hex}"),
            headers={"Authorization": f"Bearer {self._token}"},
        )
        try:
            with urllib.request.urlopen(req, timeout=self._timeout) as resp:
                return resp.read()
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            raise

    def _put_vault(self, path: str, body: str) -> None:
        url = self._url(f"/api/v1/universes/{self._universe}/vault/{path}")
        req = urllib.request.Request(
            url,
            method="PUT",
            data=body.encode("utf-8"),
            headers={
                "Authorization": f"Bearer {self._token}",
                "Content-Type": "text/markdown",
            },
        )
        with urllib.request.urlopen(req, timeout=self._timeout):
            pass

    def _get_vault(self, path: str) -> Optional[dict]:
        url = self._url(f"/api/v1/universes/{self._universe}/vault/{path}")
        req = urllib.request.Request(
            url,
            headers={"Authorization": f"Bearer {self._token}"},
        )
        try:
            with urllib.request.urlopen(req, timeout=self._timeout) as resp:
                raw = resp.read().decode("utf-8")
                fm, _ = _parse_vault_md(raw)
                return {"frontmatter": fm}
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            raise

    def _delete_vault_entry(self, chunk_id: str) -> None:
        path = self._chunk_path(chunk_id)
        url = self._url(f"/api/v1/universes/{self._universe}/vault/{path}")
        req = urllib.request.Request(
            url,
            method="DELETE",
            headers={"Authorization": f"Bearer {self._token}"},
        )
        try:
            with urllib.request.urlopen(req, timeout=self._timeout):
                pass
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return
            raise

    def _get(self, api_path: str, params: Optional[dict] = None) -> dict:
        qs = ("?" + urllib.parse.urlencode(params)) if params else ""
        req = urllib.request.Request(
            self._url(api_path) + qs,
            headers={"Authorization": f"Bearer {self._token}"},
        )
        with urllib.request.urlopen(req, timeout=self._timeout) as resp:
            return json.loads(resp.read())


# ------------------------------------------------------------------
# Markdown serialization helpers
# ------------------------------------------------------------------

def _render_vault_md(frontmatter: dict, body: str) -> str:
    fm_lines = ["---"]
    for k, v in frontmatter.items():
        fm_lines.append(f"{k}: {json.dumps(v, ensure_ascii=False)}")
    fm_lines.append("---")
    return "\n".join(fm_lines) + "\n\n" + body


def _parse_vault_md(raw: str) -> tuple[dict, str]:
    if not raw.startswith("---"):
        return {}, raw
    end = raw.index("---", 3)
    fm_block = raw[3:end].strip()
    body = raw[end + 3:].strip()
    fm: dict = {}
    for line in fm_block.splitlines():
        if ": " in line:
            k, _, v = line.partition(": ")
            try:
                fm[k.strip()] = json.loads(v.strip())
            except json.JSONDecodeError:
                fm[k.strip()] = v.strip()
    return fm, body


# ------------------------------------------------------------------
# Chroma-style `where` clause helpers
# ------------------------------------------------------------------

_CHROMA_TO_CO: dict[str, str] = {
    "$eq": "eq",
    "$gt": "gt",
    "$gte": "gte",
    "$lt": "lt",
    "$lte": "lte",
    "$in": "in",
}

_CLIENT_SIDE_OPS = {"$ne", "$nin"}


def _where_to_co_filter(where: dict) -> Optional[dict]:
    """Convert a Chroma where clause to CO's filter dict (best-effort).

    Operators not supported by CO's index ($ne, $nin) are handled client-side
    in _matches_where; they are dropped from the server-side filter.
    """
    if not where:
        return None
    co_filter: dict = {}
    for field, condition in where.items():
        if field in ("$and", "$or"):
            continue  # compound ops: do client-side only
        if isinstance(condition, dict):
            for op, val in condition.items():
                if op in _CHROMA_TO_CO:
                    co_filter[field] = {_CHROMA_TO_CO[op]: val}
                    break
                # $ne / $nin: skip server-side, handled client-side
        else:
            co_filter[field] = {"eq": condition}
    return co_filter or None


def _matches_where(metadata: dict, where: dict) -> bool:
    """Client-side evaluation of a Chroma where clause."""
    for field, condition in where.items():
        if field == "$and":
            if not all(_matches_where(metadata, sub) for sub in condition):
                return False
            continue
        if field == "$or":
            if not any(_matches_where(metadata, sub) for sub in condition):
                return False
            continue
        val = metadata.get(field)
        if isinstance(condition, dict):
            for op, operand in condition.items():
                if not _eval_op(op, val, operand):
                    return False
        elif val != condition:
            return False
    return True


def _eval_op(op: str, val: Any, operand: Any) -> bool:
    if op == "$eq":
        return val == operand
    if op == "$ne":
        return val != operand
    if op == "$gt":
        return val is not None and val > operand
    if op == "$gte":
        return val is not None and val >= operand
    if op == "$lt":
        return val is not None and val < operand
    if op == "$lte":
        return val is not None and val <= operand
    if op == "$in":
        return val in operand
    if op == "$nin":
        return val not in operand
    return False
