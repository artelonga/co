//! CO-244: read-only SQL query endpoint for per-universe DuckDB/SQLite data.
//!
//! POST /api/v1/universes/{slug}/query — run a SELECT against a universe's data.db.
//! Auth required. Row cap: 1000. Only SELECT statements are allowed.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use rusqlite::types::ValueRef;
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
use crate::cache::{manifest_content_hash, query_cache_key};
use crate::error::AppError;
use crate::server::AppState;

const MAX_QUERY_ROWS: usize = 1000;

/// CO-79: build the query-cache params component from a request.
///
/// Combines the (trimmed) SQL with the effective row limit, separated by a NUL
/// byte so that `"SELECT 1"` with `limit=1` can never collide with a different
/// SQL string that happens to end in `\nlimit=1`.
fn query_params(sql: &str, limit: usize) -> String {
    format!("{sql}\u{0}limit={limit}")
}

/// CO-79: content hash of a universe manifest, read best-effort from disk.
///
/// Used as the schema-version component of the query cache key so that results
/// cached under an old manifest are never served after the schema changes.
/// Returns `0` when the manifest is absent or unparseable — a stable sentinel
/// that still produces a valid, universe-scoped key.
fn manifest_hash(universe_root: &std::path::Path) -> u64 {
    std::fs::read(universe_root.join(co::manifest::MANIFEST_FILENAME))
        .ok()
        .and_then(|bytes| co::manifest::parse(&bytes).ok())
        .map(|r| manifest_content_hash(&r.manifest))
        .unwrap_or(0)
}

fn default_limit() -> usize {
    MAX_QUERY_ROWS
}

#[derive(Deserialize)]
pub struct QueryRequest {
    pub sql: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Serialize, Deserialize)]
pub struct QueryResponse {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    pub truncated: bool,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/{slug}/query", post(query_universe))
}

pub async fn query_universe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
    Json(body): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, AppError> {
    // Validate the universe exists and the user can access it.
    let universe = {
        let storage = state.core.storage.lock();
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?
    };

    let is_accessible =
        universe.owner_id == user_id.0 || universe.is_public || universe.is_template || {
            let storage = state.core.storage.lock();
            storage.is_universe_member(&slug, &user_id.0)
        };

    if !is_accessible {
        return Err(AppError::Forbidden(
            "Not authorized to access this universe".into(),
        ));
    }

    // Only SELECT statements are allowed.
    let trimmed = body.sql.trim();
    if !trimmed.to_ascii_uppercase().starts_with("SELECT") {
        return Err(AppError::BadRequest(
            "Only SELECT statements are allowed".into(),
        ));
    }

    let limit = body.limit.min(MAX_QUERY_ROWS);

    // CO-79: query result cache (L1). The key is SHA-256 over the SQL+limit, the
    // universe key, and the manifest content hash, so a schema change (new
    // manifest hash) or a write that invalidates the universe prefix busts it.
    // Access is verified above, so a hit never leaks data to an unauthorized
    // user, and the slug component scopes keys per universe.
    let (conn_arc, universe_root) = {
        let storage = state.core.storage.lock();
        (storage.universe_conn(&slug), storage.universe_root(&slug))
    };
    let cache_key = query_cache_key(
        &query_params(trimmed, limit),
        &slug,
        manifest_hash(&universe_root),
    );
    if let Some(bytes) = state.index.cache.query.get(&cache_key)
        && let Ok(cached) = serde_json::from_slice::<QueryResponse>(&bytes)
    {
        return Ok(Json(cached));
    }

    let conn_guard = conn_arc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock poisoned".into()))?;

    let mut stmt = conn_guard
        .prepare(trimmed)
        .map_err(|e| AppError::BadRequest(format!("SQL error: {e}")))?;

    let columns: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let col_count = columns.len();
    let mut all_rows: Vec<Vec<serde_json::Value>> = Vec::new();

    let mut query_rows = stmt
        .query([])
        .map_err(|e| AppError::Internal(format!("query failed: {e}")))?;

    while let Some(row) = query_rows
        .next()
        .map_err(|e| AppError::Internal(format!("row fetch failed: {e}")))?
    {
        // Collect one extra row to detect truncation without over-allocating.
        if all_rows.len() > limit {
            break;
        }
        let mut cells = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let val = match row
                .get_ref(i)
                .map_err(|e| AppError::Internal(format!("cell read failed: {e}")))?
            {
                ValueRef::Null => serde_json::Value::Null,
                ValueRef::Integer(i) => serde_json::json!(i),
                ValueRef::Real(f) => serde_json::json!(f),
                ValueRef::Text(s) => serde_json::json!(std::str::from_utf8(s).unwrap_or("")),
                ValueRef::Blob(b) => serde_json::json!(format!("<blob {} bytes>", b.len())),
            };
            cells.push(val);
        }
        all_rows.push(cells);
    }

    let truncated = all_rows.len() > limit;
    if truncated {
        all_rows.truncate(limit);
    }
    let row_count = all_rows.len();

    let response = QueryResponse {
        columns,
        rows: all_rows,
        row_count,
        truncated,
    };

    // CO-79: populate the L1 query cache with the serialized result. Best-effort:
    // a serialization failure simply skips caching rather than failing the request.
    if let Ok(bytes) = serde_json::to_vec(&response) {
        state.index.cache.query.insert(cache_key, bytes);
    }

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    /// Run the core SQL validation + execution logic against an in-memory DB.
    fn run_query(
        conn: &Connection,
        sql: &str,
        limit: usize,
    ) -> Result<(Vec<String>, Vec<Vec<serde_json::Value>>, bool), String> {
        let trimmed = sql.trim();
        if !trimmed.to_ascii_uppercase().starts_with("SELECT") {
            return Err("Only SELECT statements are allowed".into());
        }
        let limit = limit.min(super::MAX_QUERY_ROWS);
        let mut stmt = conn
            .prepare(trimmed)
            .map_err(|e| format!("SQL error: {e}"))?;
        let columns: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let col_count = columns.len();
        let mut all_rows: Vec<Vec<serde_json::Value>> = Vec::new();
        let mut rows = stmt.query([]).map_err(|e| format!("query: {e}"))?;
        while let Some(row) = rows.next().map_err(|e| format!("row: {e}"))? {
            if all_rows.len() > limit {
                break;
            }
            let mut cells = Vec::with_capacity(col_count);
            for i in 0..col_count {
                let val = match row.get_ref(i).map_err(|e| format!("cell: {e}"))? {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(i) => serde_json::json!(i),
                    rusqlite::types::ValueRef::Real(f) => serde_json::json!(f),
                    rusqlite::types::ValueRef::Text(s) => {
                        serde_json::json!(std::str::from_utf8(s).unwrap_or(""))
                    }
                    rusqlite::types::ValueRef::Blob(b) => {
                        serde_json::json!(format!("<blob {} bytes>", b.len()))
                    }
                };
                cells.push(val);
            }
            all_rows.push(cells);
        }
        let truncated = all_rows.len() > limit;
        if truncated {
            all_rows.truncate(limit);
        }
        Ok((columns, all_rows, truncated))
    }

    #[test]
    fn test_non_select_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        let err = run_query(&conn, "DROP TABLE foo", 10).unwrap_err();
        assert!(
            err.contains("Only SELECT"),
            "expected rejection, got: {err}"
        );
    }

    #[test]
    fn test_insert_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        let err = run_query(&conn, "INSERT INTO foo VALUES (1)", 10).unwrap_err();
        assert!(
            err.contains("Only SELECT"),
            "expected rejection, got: {err}"
        );
    }

    #[test]
    fn test_select_from_in_memory_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER, name TEXT);
             INSERT INTO items VALUES (1, 'alpha');
             INSERT INTO items VALUES (2, 'beta');",
        )
        .unwrap();

        let (cols, rows, truncated) =
            run_query(&conn, "SELECT id, name FROM items ORDER BY id", 100).unwrap();

        assert_eq!(cols, vec!["id", "name"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], serde_json::json!(1_i64));
        assert_eq!(rows[0][1], serde_json::json!("alpha"));
        assert!(!truncated);
    }

    #[test]
    fn test_row_cap_truncation() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE nums (n INTEGER);
             INSERT INTO nums VALUES (1),(2),(3),(4),(5);",
        )
        .unwrap();

        let (_, rows, truncated) = run_query(&conn, "SELECT n FROM nums", 3).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(truncated);
    }

    #[test]
    fn test_select_case_insensitive() {
        let conn = Connection::open_in_memory().unwrap();
        // lowercase "select" should also pass
        let (cols, rows, _) = run_query(&conn, "select 42 as answer", 10).unwrap();
        assert_eq!(cols, vec!["answer"]);
        assert_eq!(rows[0][0], serde_json::json!(42_i64));
    }

    // --- CO-79: query cache wiring ---

    #[test]
    fn test_query_params_distinguishes_sql_and_limit() {
        // Same SQL + limit → identical params (cache hit).
        assert_eq!(
            super::query_params("SELECT 1", 100),
            super::query_params("SELECT 1", 100)
        );
        // Different SQL → different params (no false hit).
        assert_ne!(
            super::query_params("SELECT 1", 100),
            super::query_params("SELECT 2", 100)
        );
        // Different limit → different params (a smaller limit may truncate).
        assert_ne!(
            super::query_params("SELECT 1", 100),
            super::query_params("SELECT 1", 10)
        );
    }

    #[test]
    fn test_query_cache_key_is_universe_scoped() {
        use crate::cache::query_cache_key;
        let params = super::query_params("SELECT 1", 100);
        let a = query_cache_key(&params, "uni-a", 7);
        let b = query_cache_key(&params, "uni-b", 7);
        // Same SQL, different universe → different keys (no cross-universe leak).
        assert_ne!(a, b);
        // Keys are prefixed with the slug so prefix-invalidation works.
        assert!(a.starts_with("uni-a:"));
        assert!(b.starts_with("uni-b:"));
    }

    #[test]
    fn test_query_cache_key_busts_on_manifest_hash() {
        use crate::cache::query_cache_key;
        let params = super::query_params("SELECT 1", 100);
        // Same query + universe but a new manifest hash → different key, so
        // results cached under the old schema are never served afterward.
        assert_ne!(
            query_cache_key(&params, "uni", 1),
            query_cache_key(&params, "uni", 2)
        );
    }

    #[test]
    fn test_query_response_cache_roundtrip() {
        use crate::cache::QueryCache;

        let response = super::QueryResponse {
            columns: vec!["id".into(), "name".into()],
            rows: vec![vec![serde_json::json!(1_i64), serde_json::json!("alpha")]],
            row_count: 1,
            truncated: false,
        };

        let cache = QueryCache::new();
        let key = "uni:abc".to_string();

        // Miss before insertion.
        assert!(cache.get(&key).is_none());

        // Serialize → insert → get → deserialize must reconstruct the response.
        let bytes = serde_json::to_vec(&response).unwrap();
        cache.insert(key.clone(), bytes);
        let cached: super::QueryResponse =
            serde_json::from_slice(&cache.get(&key).unwrap()).unwrap();
        assert_eq!(cached.columns, response.columns);
        assert_eq!(cached.row_count, 1);
        assert_eq!(cached.rows[0][1], serde_json::json!("alpha"));

        // Universe-prefix invalidation clears it.
        cache.invalidate_prefix("uni:");
        assert!(cache.get(&key).is_none());
    }
}
