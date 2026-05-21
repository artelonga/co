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
use crate::error::AppError;
use crate::server::AppState;

const MAX_QUERY_ROWS: usize = 1000;

fn default_limit() -> usize {
    MAX_QUERY_ROWS
}

#[derive(Deserialize)]
pub struct QueryRequest {
    pub sql: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Serialize)]
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

    // Get the universe connection, then drop the storage lock.
    let conn_arc = {
        let storage = state.core.storage.lock();
        storage.universe_conn(&slug)
    };

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

    Ok(Json(QueryResponse {
        columns,
        rows: all_rows,
        row_count,
        truncated,
    }))
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
}
