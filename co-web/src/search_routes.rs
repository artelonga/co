//! Cross-universe semantic search — CO-164.
//!
//! GET /api/v1/search?semantic=<query>&k=10
//!
//! Aggregates top-K results across all universes the requesting user can read
//! (public universes + universes the user owns or is a member of when logged in).

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::entry_index::EntryRow;
use crate::error::AppError;
use crate::server::AppState;

#[derive(Debug, Deserialize)]
pub struct GlobalSearchQuery {
    pub semantic: String,
    pub k: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct GlobalSearchResponse {
    pub entries: Vec<EntryRow>,
    pub total: usize,
}

/// GET /api/v1/search?semantic=<query>&k=10
pub async fn global_semantic_search(
    State(state): State<AppState>,
    Query(q): Query<GlobalSearchQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<GlobalSearchResponse>, AppError> {
    let k = q.k.unwrap_or(10).min(200);

    let query_embedding = match state.embeddings.embed_one(&q.semantic) {
        Some(e) => e,
        None => {
            return Ok(Json(GlobalSearchResponse {
                entries: vec![],
                total: 0,
            }));
        }
    };

    // Determine which universes the user can read.
    let universe_keys: Vec<String> = {
        let storage = state.storage.lock();

        // Public universes (search_public_universes("") returns all — LIKE "%%" matches all)
        let mut keys: Vec<String> = storage
            .search_public_universes("")
            .into_iter()
            .map(|u| u.key)
            .collect();

        // Add user-owned/member universes when logged in.
        if let Some(uid) = crate::auth::resolve_user_id(&state, &headers) {
            for k in storage.list_user_universes(&uid) {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys
    };

    // Collect top-K hits per universe, then merge globally.
    let mut all_scored: Vec<(String, String, f32)> = Vec::new(); // (universe_key, path, score)

    for uni_key in &universe_keys {
        let uc = {
            let storage = state.storage.lock();
            storage.universe_conn(uni_key)
        };
        let conn = match uc.lock() {
            Ok(c) => c,
            Err(_) => continue,
        };
        let emb_idx = crate::embedding_index::EmbeddingIndex::new(&conn);
        let scored = emb_idx
            .search_similar(uni_key, &query_embedding, k, None)
            .unwrap_or_default();
        for (path, score) in scored {
            all_scored.push((uni_key.clone(), path, score));
        }
    }

    // Sort globally by score, take top-K.
    all_scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    all_scored.truncate(k);

    // Fetch full EntryRows.
    let mut entries = Vec::with_capacity(all_scored.len());
    for (uni_key, path, score) in all_scored {
        let uc = {
            let storage = state.storage.lock();
            storage.universe_conn(&uni_key)
        };
        let conn = match uc.lock() {
            Ok(c) => c,
            Err(_) => continue,
        };
        let idx = crate::entry_index::EntryIndex::new(&conn);
        if let Ok(Some(mut row)) = idx.get(&uni_key, &path) {
            row._score = Some(score);
            entries.push(row);
        }
    }

    let total = entries.len();
    Ok(Json(GlobalSearchResponse { entries, total }))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/search", get(global_semantic_search))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_search_query_deserializes() {
        let q: GlobalSearchQuery = serde_urlencoded::from_str("semantic=hello+world&k=5").unwrap();
        assert_eq!(q.semantic, "hello world");
        assert_eq!(q.k, Some(5));
    }

    #[test]
    fn test_global_search_query_k_defaults() {
        let q: GlobalSearchQuery = serde_urlencoded::from_str("semantic=test").unwrap();
        assert!(q.k.is_none());
    }
}
