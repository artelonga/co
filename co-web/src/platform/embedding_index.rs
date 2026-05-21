//! EmbeddingIndex — per-universe vector store for CO-164 semantic search.
//!
//! Embeddings are stored as BLOB (384 × f32 little-endian) in `entry_embeddings`.
//! Similarity search is brute-force cosine — correct up to ~10k entries/universe.

use rusqlite::{Connection, params};

/// Cosine similarity ∈ [0, 1] clamped. Returns 0 when either vector is zero.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < f32::EPSILON || norm_b < f32::EPSILON {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(0.0, 1.0)
}

pub fn f32s_to_bytes(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub fn bytes_to_f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub struct EmbeddingIndex<'a> {
    conn: &'a Connection,
}

impl<'a> EmbeddingIndex<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Insert or replace an embedding row.
    pub fn upsert(
        &self,
        universe_key: &str,
        path: &str,
        body_hash: &str,
        embedding: &[f32],
        model: &str,
    ) -> anyhow::Result<()> {
        let blob = f32s_to_bytes(embedding);
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO entry_embeddings (universe_key, path, body_hash, embedding, model, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(universe_key, path) DO UPDATE SET
               body_hash  = excluded.body_hash,
               embedding  = excluded.embedding,
               model      = excluded.model,
               indexed_at = excluded.indexed_at",
            params![universe_key, path, body_hash, blob, model, now],
        )?;
        Ok(())
    }

    /// Remove an embedding row (called on entry delete).
    pub fn remove(&self, universe_key: &str, path: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM entry_embeddings WHERE universe_key = ?1 AND path = ?2",
            params![universe_key, path],
        )?;
        Ok(())
    }

    /// Returns (path, score) pairs sorted by cosine similarity descending.
    ///
    /// `exclude_path` skips the source entry itself (used by `/similar`).
    pub fn search_similar(
        &self,
        universe_key: &str,
        query_embedding: &[f32],
        k: usize,
        exclude_path: Option<&str>,
    ) -> anyhow::Result<Vec<(String, f32)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, embedding FROM entry_embeddings WHERE universe_key = ?1")?;
        let mut scored: Vec<(String, f32)> = stmt
            .query_map(params![universe_key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?
            .filter_map(|r| r.ok())
            .filter(|(path, _)| exclude_path.is_none_or(|ep| path != ep))
            .map(|(path, bytes)| {
                let emb = bytes_to_f32s(&bytes);
                let score = cosine_similarity(query_embedding, &emb);
                (path, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }

    /// Get the stored embedding for a single path (used by `/similar` to seed the query).
    pub fn get_embedding(
        &self,
        universe_key: &str,
        path: &str,
    ) -> anyhow::Result<Option<Vec<f32>>> {
        let result = self.conn.query_row(
            "SELECT embedding FROM entry_embeddings WHERE universe_key = ?1 AND path = ?2",
            params![universe_key, path],
            |row| row.get::<_, Vec<u8>>(0),
        );
        match result {
            Ok(bytes) => Ok(Some(bytes_to_f32s(&bytes))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Paths whose embedding is missing or whose body_hash differs from `entries.body_hash`.
    ///
    /// Returns `(path, body_hash, body)` triples — body is needed so the worker can re-embed.
    pub fn stale_paths(&self, universe_key: &str) -> anyhow::Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.path, e.body_hash, e.body
             FROM entries e
             WHERE e.universe_key = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM entry_embeddings ee
                   WHERE ee.universe_key = e.universe_key
                     AND ee.path = e.path
                     AND ee.body_hash = e.body_hash
               )",
        )?;
        let rows = stmt.query_map(params![universe_key], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::universe_pool::UNIVERSE_SCHEMA_FOR_TEST)
            .unwrap();
        conn
    }

    fn insert_entry(conn: &Connection, key: &str, path: &str, body: &str, hash: &str) {
        conn.execute(
            "INSERT INTO entries (path, universe_key, entry_type, title, frontmatter_json, payload, body, body_hash)
             VALUES (?1, ?2, 'note', '', '{}', '{}', ?3, ?4)",
            params![path, key, body, hash],
        )
        .unwrap();
    }

    fn make_embedding(seed: f32) -> Vec<f32> {
        // 384-dim unit vector seeded from a scalar
        let mut v: Vec<f32> = (0..384).map(|i| (seed * (i + 1) as f32).sin()).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > f32::EPSILON {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        v
    }

    #[test]
    fn test_upsert_and_retrieve() {
        let conn = open_mem_db();
        let idx = EmbeddingIndex::new(&conn);
        let emb = make_embedding(1.0);
        idx.upsert("uni", "a.md", "h1", &emb, "test-model").unwrap();

        let got = idx.get_embedding("uni", "a.md").unwrap().unwrap();
        assert_eq!(got.len(), 384);
        // First element should survive round-trip
        let diff = (got[0] - emb[0]).abs();
        assert!(diff < 1e-5, "embedding round-trip error: {diff}");
    }

    #[test]
    fn test_remove() {
        let conn = open_mem_db();
        let idx = EmbeddingIndex::new(&conn);
        idx.upsert("uni", "a.md", "h1", &make_embedding(1.0), "m")
            .unwrap();
        idx.remove("uni", "a.md").unwrap();
        assert!(idx.get_embedding("uni", "a.md").unwrap().is_none());
    }

    #[test]
    fn test_search_similar_returns_top_k() {
        let conn = open_mem_db();
        let idx = EmbeddingIndex::new(&conn);

        let query = make_embedding(1.0);
        // Insert 5 embeddings; embedding(1.0) is identical to query → score 1.0
        for i in 0..5_u32 {
            let emb = make_embedding(i as f32 + 1.0);
            idx.upsert("uni", &format!("{i}.md"), "h", &emb, "m")
                .unwrap();
        }

        let results = idx.search_similar("uni", &query, 3, None).unwrap();
        assert_eq!(results.len(), 3);
        // Best match should be closest to query (seed 1.0 = index 0)
        assert_eq!(results[0].0, "0.md");
        let best_score = results[0].1;
        assert!(
            best_score > 0.9,
            "expected high similarity, got {best_score}"
        );
        // Scores should be in [0, 1]
        for (_, s) in &results {
            assert!(*s >= 0.0 && *s <= 1.0, "score out of range: {s}");
        }
    }

    #[test]
    fn test_search_excludes_self() {
        let conn = open_mem_db();
        let idx = EmbeddingIndex::new(&conn);

        let emb = make_embedding(1.0);
        idx.upsert("uni", "self.md", "h", &emb, "m").unwrap();
        idx.upsert("uni", "other.md", "h", &make_embedding(2.0), "m")
            .unwrap();

        let results = idx
            .search_similar("uni", &emb, 10, Some("self.md"))
            .unwrap();
        assert!(!results.iter().any(|(p, _)| p == "self.md"));
    }

    #[test]
    fn test_stale_paths_returns_missing_embeddings() {
        let conn = open_mem_db();
        insert_entry(&conn, "uni", "a.md", "body a", "hash-a");
        insert_entry(&conn, "uni", "b.md", "body b", "hash-b");

        let idx = EmbeddingIndex::new(&conn);
        // No embeddings yet — both are stale
        let stale = idx.stale_paths("uni").unwrap();
        assert_eq!(stale.len(), 2);
    }

    #[test]
    fn test_stale_paths_ignores_fresh_embedding() {
        let conn = open_mem_db();
        insert_entry(&conn, "uni", "a.md", "body a", "hash-a");
        insert_entry(&conn, "uni", "b.md", "body b", "hash-b");

        let idx = EmbeddingIndex::new(&conn);
        idx.upsert("uni", "a.md", "hash-a", &make_embedding(1.0), "m")
            .unwrap();

        let stale = idx.stale_paths("uni").unwrap();
        // Only b.md is stale
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].0, "b.md");
    }

    #[test]
    fn test_stale_paths_detects_body_change() {
        let conn = open_mem_db();
        insert_entry(&conn, "uni", "a.md", "new body", "hash-new");

        let idx = EmbeddingIndex::new(&conn);
        // Embedding was indexed with old hash
        idx.upsert("uni", "a.md", "hash-old", &make_embedding(1.0), "m")
            .unwrap();

        let stale = idx.stale_paths("uni").unwrap();
        assert_eq!(stale.len(), 1, "should detect hash mismatch");
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0f32, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "identical vectors should have sim=1, got {sim}"
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 1e-6,
            "orthogonal vectors should have sim=0, got {sim}"
        );
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0f32, 0.0];
        let b = vec![1.0f32, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0, "zero vector should return 0");
    }
}
