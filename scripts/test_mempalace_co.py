#!/usr/bin/env python3
"""Offline tests for MempalaceCoBackend against a local CO server.

Run:
    python3 scripts/test_mempalace_co.py

Requires a running CO server at http://127.0.0.1:3000 with:
  - universe "mempalace" existing (or auto-created by the first vault PUT)
  - CO_API_TOKEN set to a valid token for that server

For quick local testing:
    cargo run -p co-web -- serve --port 3000 &
    # create token via /api/v1/auth/token, then:
    export CO_API_TOKEN=<token>
    python3 scripts/test_mempalace_co.py
"""

import json
import os
import struct
import sys
import threading
import unittest
import urllib.error
import urllib.request
from unittest.mock import MagicMock, patch, call
from io import BytesIO

# ---------------------------------------------------------------------------
# Add scripts/ to path so the import works regardless of cwd
# ---------------------------------------------------------------------------
sys.path.insert(0, os.path.dirname(__file__))
from mempalace_co_backend import (
    MempalaceCoBackend,
    _matches_where,
    _where_to_co_filter,
    _render_vault_md,
    _parse_vault_md,
    _eval_op,
)


# ===========================================================================
# Unit tests — no network required
# ===========================================================================


class TestRenderParseVaultMd(unittest.TestCase):
    def test_roundtrip(self):
        fm = {"type": "mempalace-chunk", "chunk_id": "abc", "blob_hash": "def"}
        body = "hello world"
        raw = _render_vault_md(fm, body)
        parsed_fm, parsed_body = _parse_vault_md(raw)
        self.assertEqual(parsed_fm["chunk_id"], "abc")
        self.assertEqual(parsed_fm["blob_hash"], "def")
        self.assertEqual(parsed_body, body)

    def test_no_frontmatter(self):
        fm, body = _parse_vault_md("just text")
        self.assertEqual(fm, {})
        self.assertEqual(body, "just text")


class TestMatchesWhere(unittest.TestCase):
    def _m(self, meta, where):
        return _matches_where(meta, where)

    def test_eq(self):
        self.assertTrue(self._m({"x": 1}, {"x": {"$eq": 1}}))
        self.assertFalse(self._m({"x": 2}, {"x": {"$eq": 1}}))

    def test_ne(self):
        self.assertTrue(self._m({"x": 2}, {"x": {"$ne": 1}}))
        self.assertFalse(self._m({"x": 1}, {"x": {"$ne": 1}}))

    def test_in(self):
        self.assertTrue(self._m({"x": "b"}, {"x": {"$in": ["a", "b"]}}))
        self.assertFalse(self._m({"x": "c"}, {"x": {"$in": ["a", "b"]}}))

    def test_nin(self):
        self.assertTrue(self._m({"x": "c"}, {"x": {"$nin": ["a", "b"]}}))
        self.assertFalse(self._m({"x": "a"}, {"x": {"$nin": ["a", "b"]}}))

    def test_gt_lt(self):
        self.assertTrue(self._m({"n": 5}, {"n": {"$gt": 3}}))
        self.assertFalse(self._m({"n": 2}, {"n": {"$gt": 3}}))
        self.assertTrue(self._m({"n": 1}, {"n": {"$lt": 3}}))

    def test_and(self):
        self.assertTrue(self._m({"a": 1, "b": 2}, {"$and": [{"a": {"$eq": 1}}, {"b": {"$eq": 2}}]}))
        self.assertFalse(self._m({"a": 1, "b": 3}, {"$and": [{"a": {"$eq": 1}}, {"b": {"$eq": 2}}]}))

    def test_or(self):
        self.assertTrue(self._m({"x": 1}, {"$or": [{"x": {"$eq": 1}}, {"x": {"$eq": 2}}]}))
        self.assertFalse(self._m({"x": 3}, {"$or": [{"x": {"$eq": 1}}, {"x": {"$eq": 2}}]}))

    def test_shorthand_eq(self):
        self.assertTrue(self._m({"tag": "ml"}, {"tag": "ml"}))
        self.assertFalse(self._m({"tag": "nlp"}, {"tag": "ml"}))


class TestWhereToCOFilter(unittest.TestCase):
    def test_simple_eq(self):
        f = _where_to_co_filter({"tag": "ml"})
        self.assertEqual(f, {"tag": {"eq": "ml"}})

    def test_dollar_eq(self):
        f = _where_to_co_filter({"score": {"$gt": 0.5}})
        self.assertEqual(f, {"score": {"gt": 0.5}})

    def test_ne_excluded(self):
        # $ne is not mappable to CO index — filter should be empty (None)
        f = _where_to_co_filter({"x": {"$ne": 1}})
        self.assertIsNone(f)

    def test_compound_and_dropped(self):
        f = _where_to_co_filter({"$and": [{"a": 1}]})
        self.assertIsNone(f)


class TestEvalOp(unittest.TestCase):
    def test_all_ops(self):
        self.assertTrue(_eval_op("$eq", 1, 1))
        self.assertTrue(_eval_op("$ne", 2, 1))
        self.assertTrue(_eval_op("$gt", 5, 3))
        self.assertTrue(_eval_op("$gte", 3, 3))
        self.assertTrue(_eval_op("$lt", 1, 3))
        self.assertTrue(_eval_op("$lte", 3, 3))
        self.assertTrue(_eval_op("$in", "b", ["a", "b"]))
        self.assertTrue(_eval_op("$nin", "c", ["a", "b"]))
        self.assertFalse(_eval_op("$unknown", "x", "y"))


class TestChunkPath(unittest.TestCase):
    def setUp(self):
        os.environ["CO_API_TOKEN"] = "test-token"
        self.backend = MempalaceCoBackend(
            server="http://127.0.0.1:3000",
            universe="mempalace",
            wing="notes",
            room="daily",
        )

    def test_chunk_path(self):
        path = self.backend._chunk_path("my-chunk-01")
        self.assertEqual(path, "mempalace/notes/daily/my-chunk-01.md")

    def test_chunk_path_url_encodes(self):
        path = self.backend._chunk_path("chunk with spaces")
        self.assertNotIn(" ", path)


# ===========================================================================
# Mock-HTTP tests — verifies HTTP calls without a real server
# ===========================================================================


def _make_mock_response(data, status=200):
    if isinstance(data, dict):
        body = json.dumps(data).encode()
    elif isinstance(data, str):
        body = data.encode()
    elif isinstance(data, bytes):
        body = data
    else:
        body = b""
    mock = MagicMock()
    mock.read.return_value = body
    mock.status = status
    mock.__enter__ = lambda s: s
    mock.__exit__ = MagicMock(return_value=False)
    return mock


class TestUpsertHTTPCalls(unittest.TestCase):
    def setUp(self):
        os.environ["CO_API_TOKEN"] = "tok"
        self.backend = MempalaceCoBackend(
            server="http://127.0.0.1:3000",
            universe="mempalace",
            wing="w",
            room="r",
        )

    def test_upsert_makes_three_http_calls(self):
        content_hash = "a" * 64
        emb_hash = "b" * 64

        call_count = 0
        results = [
            _make_mock_response({"hash": content_hash, "size": 5}),
            _make_mock_response({"hash": emb_hash, "size": 16}),
            _make_mock_response(b"", 200),  # vault PUT returns empty on success
        ]

        def fake_urlopen(req, timeout=None):
            nonlocal call_count
            r = results[call_count]
            call_count += 1
            return r

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            self.backend.upsert(
                ids=["c1"],
                documents=["hello"],
                embeddings=[[0.1, 0.2, 0.3, 0.4]],
                metadatas=[{"tag": "test"}],
            )

        self.assertEqual(call_count, 3)

    def test_upsert_vault_body_contains_chunk_id(self):
        captured_bodies = []
        content_hash = "c" * 64
        emb_hash = "d" * 64

        idx = [0]
        responses = [
            _make_mock_response({"hash": content_hash, "size": 5}),
            _make_mock_response({"hash": emb_hash, "size": 16}),
            _make_mock_response(b""),
        ]

        def fake_urlopen(req, timeout=None):
            r = responses[idx[0]]
            if hasattr(req, "data") and req.data and req.get_header("Content-type") == "text/markdown":
                captured_bodies.append(req.data.decode())
            idx[0] += 1
            return r

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            self.backend.upsert(
                ids=["xyz"],
                documents=["doc text"],
                embeddings=[[1.0]],
                metadatas=[{"source": "test"}],
            )

        self.assertEqual(len(captured_bodies), 1)
        self.assertIn("chunk_id", captured_bodies[0])
        self.assertIn("xyz", captured_bodies[0])


class TestGetHTTPCalls(unittest.TestCase):
    def setUp(self):
        os.environ["CO_API_TOKEN"] = "tok"
        self.backend = MempalaceCoBackend(
            server="http://127.0.0.1:3000",
            universe="mempalace",
            wing="w",
            room="r",
        )

    def test_get_returns_document_and_embedding(self):
        content_hash = "e" * 64
        emb_hash = "f" * 64
        emb = [0.5, 0.6]
        emb_bytes = struct.pack("2f", *emb)

        fm = {
            "type": "mempalace-chunk",
            "chunk_id": "c1",
            "blob_hash": content_hash,
            "embedding_blob": emb_hash,
            "embedding_dim": 2,
            "metadata": {"tag": "unit"},
        }
        vault_body = (
            "---\n"
            + "\n".join(f"{k}: {json.dumps(v)}" for k, v in fm.items())
            + "\n---\n\nhello world"
        )

        idx = [0]
        responses = [
            _make_mock_response(vault_body),  # GET vault
            _make_mock_response(b"hello world"),  # GET blob (content)
            _make_mock_response(emb_bytes),  # GET blob (embedding)
        ]

        def fake_urlopen(req, timeout=None):
            r = responses[idx[0]]
            idx[0] += 1
            return r

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            result = self.backend.get(ids=["c1"])

        self.assertEqual(result["ids"], ["c1"])
        self.assertEqual(result["documents"], ["hello world"])
        self.assertAlmostEqual(result["embeddings"][0][0], 0.5, places=5)
        self.assertEqual(result["metadatas"][0]["tag"], "unit")

    def test_get_missing_chunk_returns_empty(self):
        def fake_urlopen(req, timeout=None):
            raise urllib.error.HTTPError(url="", code=404, msg="Not Found", hdrs=None, fp=None)

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            result = self.backend.get(ids=["missing"])

        self.assertEqual(result["ids"], [])


class TestDeleteHTTPCalls(unittest.TestCase):
    def setUp(self):
        os.environ["CO_API_TOKEN"] = "tok"
        self.backend = MempalaceCoBackend(
            server="http://127.0.0.1:3000",
            universe="mempalace",
            wing="w",
            room="r",
        )

    def test_delete_by_id_sends_delete_request(self):
        methods_seen = []

        def fake_urlopen(req, timeout=None):
            methods_seen.append(req.get_method())
            return _make_mock_response(b"")

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            self.backend.delete(ids=["c1"])

        self.assertIn("DELETE", methods_seen)

    def test_delete_404_is_silent(self):
        def fake_urlopen(req, timeout=None):
            raise urllib.error.HTTPError(url="", code=404, msg="Not Found", hdrs=None, fp=None)

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            self.backend.delete(ids=["ghost"])  # must not raise


class TestQueryFallback(unittest.TestCase):
    def setUp(self):
        os.environ["CO_API_TOKEN"] = "tok"
        self.backend = MempalaceCoBackend(
            server="http://127.0.0.1:3000",
            universe="mempalace",
            wing="w",
            room="r",
        )

    def test_query_uses_keyword_when_vector_search_returns_none(self):
        content_hash = "a" * 64
        emb_hash = "b" * 64
        emb_bytes = struct.pack("1f", 0.9)

        entries_resp = {
            "entries": [
                {
                    "frontmatter": {
                        "type": "mempalace-chunk",
                        "chunk_id": "c1",
                        "blob_hash": content_hash,
                        "embedding_blob": emb_hash,
                        "embedding_dim": 1,
                        "metadata": {"tag": "ml"},
                    }
                }
            ],
            "total": 1,
        }
        fm = entries_resp["entries"][0]["frontmatter"]
        vault_body = (
            "---\n"
            + "\n".join(f"{k}: {json.dumps(v)}" for k, v in fm.items())
            + "\n---\n\nsome text"
        )

        idx = [0]
        responses = [
            _make_mock_response(entries_resp),  # GET entries
            _make_mock_response(vault_body),    # GET vault for c1
            _make_mock_response(b"some text"),  # GET blob (content)
            _make_mock_response(emb_bytes),     # GET blob (embedding)
        ]

        def fake_urlopen(req, timeout=None):
            r = responses[idx[0]]
            idx[0] += 1
            return r

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            result = self.backend.query(query_texts=["some text"], n_results=5)

        self.assertEqual(len(result["ids"]), 1)
        self.assertEqual(result["ids"][0], ["c1"])

    def test_vector_search_hook_overrides_keyword(self):
        class VectorBackend(MempalaceCoBackend):
            def _vector_search(self, query_text, n_results, where):
                return {
                    "ids": ["v1"],
                    "documents": ["vector doc"],
                    "embeddings": [[0.1]],
                    "metadatas": [{"source": "vector"}],
                    "distances": [0.05],
                }

        os.environ["CO_API_TOKEN"] = "tok"
        backend = VectorBackend(server="http://127.0.0.1:3000")

        result = backend.query(query_texts=["anything"])
        self.assertEqual(result["ids"][0], ["v1"])
        self.assertEqual(result["distances"][0], [0.05])


# ===========================================================================
# Integration tests — require a live CO server (skipped unless CO_API_TOKEN set)
# ===========================================================================

INTEGRATION = os.environ.get("CO_API_TOKEN") and os.environ.get("CO_INTEGRATION_TEST")


@unittest.skipUnless(INTEGRATION, "set CO_INTEGRATION_TEST=1 and CO_API_TOKEN to run")
class TestIntegrationLiveServer(unittest.TestCase):
    SERVER = os.environ.get("CO_SERVER", "http://127.0.0.1:3000")

    def setUp(self):
        self.backend = MempalaceCoBackend(
            server=self.SERVER,
            universe="mempalace-test",
            wing="unit",
            room="integration",
        )

    def test_upsert_get_delete_roundtrip(self):
        chunk_id = "integration-test-chunk-001"
        document = "The quick brown fox jumps over the lazy dog."
        embedding = [float(i) / 10 for i in range(8)]
        metadata = {"source": "integration-test", "score": 0.99}

        self.backend.upsert(
            ids=[chunk_id],
            documents=[document],
            embeddings=[embedding],
            metadatas=[metadata],
        )

        result = self.backend.get(ids=[chunk_id])
        self.assertEqual(result["ids"], [chunk_id])
        self.assertEqual(result["documents"][0], document)
        self.assertAlmostEqual(result["embeddings"][0][0], 0.0, places=4)
        self.assertEqual(result["metadatas"][0]["source"], "integration-test")

        self.backend.delete(ids=[chunk_id])
        after_delete = self.backend.get(ids=[chunk_id])
        self.assertEqual(after_delete["ids"], [])

    def test_query_returns_results(self):
        chunk_id = "integration-query-chunk-001"
        self.backend.upsert(
            ids=[chunk_id],
            documents=["machine learning neural network"],
            embeddings=[[0.1, 0.2]],
            metadatas=[{"topic": "ml"}],
        )

        results = self.backend.query(
            query_texts=["neural network"],
            n_results=5,
        )
        all_ids = [id_ for batch in results["ids"] for id_ in batch]
        self.assertIn(chunk_id, all_ids)

        self.backend.delete(ids=[chunk_id])


if __name__ == "__main__":
    unittest.main(verbosity=2)
