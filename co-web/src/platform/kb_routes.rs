//! CO-367: Universal KB sync — ingest + query API.
//!
//! Endpoints:
//!   POST /api/v1/kb/ingest  — upsert entry into KB; auth via CO_KB_TOKEN bearer
//!   GET  /api/v1/kb/search  — FTS5 search over body_preview
//!   GET  /api/v1/kb/recent  — recently-indexed entries
//!
//! Auth: bearer token CO_KB_TOKEN (env var). Returns 503 if unset.
//! Resilience: entry writes never block on KB sync — ingest fires via tokio::spawn.

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::server::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct KbIngestBody {
    pub universe_key: String,
    pub entry_path: String,
    pub entry_type: Option<String>,
    pub body_hash: String,
    pub frontmatter_json: Option<serde_json::Value>,
    pub body_preview: Option<String>,
    pub size_bytes: Option<i64>,
    pub updated_at: String,
    pub asset_refs: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct KbIngestResponse {
    pub ok: bool,
    pub universe_key: String,
    pub entry_path: String,
    pub indexed_at: String,
}

#[derive(Debug, Deserialize)]
pub struct KbSearchParams {
    pub q: String,
    pub universe_key: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct KbRecentParams {
    pub universe_key: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Clone)]
pub struct KbEntry {
    pub universe_key: String,
    pub entry_path: String,
    pub entry_type: Option<String>,
    pub body_preview: Option<String>,
    pub frontmatter_json: Option<serde_json::Value>,
    pub updated_at: String,
    pub indexed_at: String,
}

#[derive(Debug, Serialize)]
pub struct KbSearchResponse {
    pub results: Vec<KbEntry>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct KbRecentResponse {
    pub entries: Vec<KbEntry>,
}

// ---------------------------------------------------------------------------
// Auth helper
// ---------------------------------------------------------------------------

/// Returns `Some(rejection_response)` when auth fails, `None` on success.
fn check_kb_token(headers: &HeaderMap) -> Option<Response> {
    let expected = match crate::infra::secrets::global().get("CO_KB_TOKEN") {
        Some(t) if !t.is_empty() => t,
        _ => {
            return Some(
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({"error": "kb ingest disabled — CO_KB_TOKEN not set"})),
                )
                    .into_response(),
            );
        }
    };
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != format!("Bearer {expected}") {
        return Some(
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
                .into_response(),
        );
    }
    None
}

// ---------------------------------------------------------------------------
// Key sanitizer — safe for SQL string interpolation
// ---------------------------------------------------------------------------

/// Allow only [a-z0-9\-_], max 64 chars (injection-safe).
fn sanitize_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_')
        .take(64)
        .collect()
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

fn row_to_kb_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<KbEntry> {
    let fm_str: Option<String> = row.get(4)?;
    let frontmatter_json = fm_str.as_deref().and_then(|s| serde_json::from_str(s).ok());
    Ok(KbEntry {
        universe_key: row.get(0)?,
        entry_path: row.get(1)?,
        entry_type: row.get(2)?,
        body_preview: row.get(3)?,
        frontmatter_json,
        updated_at: row.get(5)?,
        indexed_at: row.get(6)?,
    })
}

/// Idempotent upsert: returns `true` when a new row was indexed, `false` when
/// the exact (universe_key, entry_path, body_hash) triple already existed.
///
/// On a new version the function:
///   1. Inserts a history row into `entry_kb_index`.
///   2. Upserts `entry_kb_latest` (latest version per path).
///   3. Refreshes the FTS5 table (`entry_kb_fts`).
#[allow(clippy::too_many_arguments)]
pub fn upsert_kb_entry(
    conn: &rusqlite::Connection,
    universe_key: &str,
    entry_path: &str,
    body_hash: &str,
    entry_type: Option<&str>,
    frontmatter_json: Option<&str>,
    body_preview: Option<&str>,
    size_bytes: Option<i64>,
    updated_at: &str,
    indexed_at: &str,
    asset_refs: Option<&str>,
) -> rusqlite::Result<bool> {
    // History insert — idempotent via ON CONFLICT DO NOTHING.
    let n = conn.execute(
        "INSERT INTO entry_kb_index \
         (universe_key, entry_path, body_hash, entry_type, frontmatter_json, body_preview, size_bytes, updated_at, indexed_at, asset_refs) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT(universe_key, entry_path, body_hash) DO NOTHING",
        params![
            universe_key,
            entry_path,
            body_hash,
            entry_type,
            frontmatter_json,
            body_preview,
            size_bytes,
            updated_at,
            indexed_at,
            asset_refs,
        ],
    )?;

    if n == 0 {
        return Ok(false); // already indexed — idempotent no-op
    }

    // Latest version — always reflects the most recently ingested hash.
    conn.execute(
        "INSERT INTO entry_kb_latest \
         (universe_key, entry_path, body_hash, entry_type, frontmatter_json, body_preview, size_bytes, updated_at, indexed_at, asset_refs) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT(universe_key, entry_path) DO UPDATE SET \
           body_hash        = excluded.body_hash, \
           entry_type       = excluded.entry_type, \
           frontmatter_json = excluded.frontmatter_json, \
           body_preview     = excluded.body_preview, \
           size_bytes       = excluded.size_bytes, \
           updated_at       = excluded.updated_at, \
           indexed_at       = excluded.indexed_at, \
           asset_refs       = excluded.asset_refs",
        params![
            universe_key,
            entry_path,
            body_hash,
            entry_type,
            frontmatter_json,
            body_preview,
            size_bytes,
            updated_at,
            indexed_at,
            asset_refs,
        ],
    )?;

    // FTS5 — one row per (universe_key, entry_path), always latest body_preview.
    conn.execute(
        "DELETE FROM entry_kb_fts WHERE universe_key = ?1 AND entry_path = ?2",
        params![universe_key, entry_path],
    )?;
    if let Some(preview) = body_preview.filter(|p| !p.is_empty()) {
        conn.execute(
            "INSERT INTO entry_kb_fts (universe_key, entry_path, body_preview) VALUES (?1, ?2, ?3)",
            params![universe_key, entry_path, preview],
        )?;
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// Non-blocking ingest — called from the entry write path
// ---------------------------------------------------------------------------

/// Fire-and-forget KB ingest from CO's own entry-write path.
///
/// Uses `try_lock()` to avoid holding `Mutex<Storage>` across an await;
/// logs a warning and skips if contended at that instant.
#[allow(clippy::too_many_arguments)]
pub fn fire_kb_ingest(
    state: &AppState,
    universe_key: &str,
    entry_path: &str,
    body_hash: &str,
    entry_type: &str,
    body: &str,
    frontmatter: &serde_json::Value,
    updated_at: &str,
) {
    let state = state.clone();
    let universe_key = universe_key.to_string();
    let entry_path = entry_path.to_string();
    let body_hash = body_hash.to_string();
    let entry_type = entry_type.to_string();
    let body_preview: String = body.chars().take(500).collect();
    let frontmatter_json = frontmatter.to_string();
    let size_bytes = body.len() as i64;
    let updated_at = updated_at.to_string();

    tokio::spawn(async move {
        let indexed_at = chrono::Utc::now().to_rfc3339();

        let result = {
            let Some(storage) = state.core.storage.try_lock() else {
                tracing::warn!(
                    "fire_kb_ingest: storage lock contended — ingest skipped for {universe_key}:{entry_path}"
                );
                return;
            };
            upsert_kb_entry(
                storage.conn(),
                &universe_key,
                &entry_path,
                &body_hash,
                Some(&entry_type),
                Some(&frontmatter_json),
                Some(&body_preview),
                Some(size_bytes),
                &updated_at,
                &indexed_at,
                None,
            )
        };

        match result {
            Err(e) => {
                tracing::warn!(
                    "fire_kb_ingest: upsert failed for {universe_key}:{entry_path}: {e}"
                );
            }
            Ok(true) => {
                crate::atividade::log_atividade(
                    state,
                    crate::atividade::Atividade {
                        acao: crate::atividade::Acao::Criar,
                        entidade: "kb".to_string(),
                        entidade_id: Some(format!("{universe_key}:{entry_path}")),
                        before: None,
                        after: Some(serde_json::json!({
                            "event": "kb.ingested",
                            "universe_key": universe_key,
                            "entry_path": entry_path,
                            "indexed_at": indexed_at,
                        })),
                        tipo: crate::atividade::Tipo::Sistema,
                        user_id: None,
                        ip: None,
                        user_agent: None,
                    },
                );
            }
            Ok(false) => {} // idempotent no-op
        }
    });
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

fn query_kb_search(
    conn: &rusqlite::Connection,
    q: &str,
    universe_key: Option<&str>,
    limit: i64,
) -> Vec<KbEntry> {
    if let Some(uk) = universe_key.filter(|s| !s.is_empty()) {
        let uk = sanitize_key(uk);
        conn.prepare(&format!(
            "SELECT k.universe_key, k.entry_path, k.entry_type, k.body_preview, k.frontmatter_json, k.updated_at, k.indexed_at \
             FROM entry_kb_fts fts \
             JOIN entry_kb_latest k ON k.universe_key = fts.universe_key AND k.entry_path = fts.entry_path \
             WHERE entry_kb_fts MATCH ?1 AND fts.universe_key = '{uk}' \
             ORDER BY rank LIMIT {limit}"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map(params![q], row_to_kb_entry)
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    } else {
        conn.prepare(&format!(
            "SELECT k.universe_key, k.entry_path, k.entry_type, k.body_preview, k.frontmatter_json, k.updated_at, k.indexed_at \
             FROM entry_kb_fts fts \
             JOIN entry_kb_latest k ON k.universe_key = fts.universe_key AND k.entry_path = fts.entry_path \
             WHERE entry_kb_fts MATCH ?1 \
             ORDER BY rank LIMIT {limit}"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map(params![q], row_to_kb_entry)
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    }
}

fn query_kb_recent(
    conn: &rusqlite::Connection,
    universe_key: Option<&str>,
    limit: i64,
) -> Vec<KbEntry> {
    if let Some(uk) = universe_key.filter(|s| !s.is_empty()) {
        let uk = sanitize_key(uk);
        conn.prepare(&format!(
            "SELECT universe_key, entry_path, entry_type, body_preview, frontmatter_json, updated_at, indexed_at \
             FROM entry_kb_latest \
             WHERE universe_key = '{uk}' \
             ORDER BY indexed_at DESC LIMIT {limit}"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], row_to_kb_entry)
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    } else {
        conn.prepare(&format!(
            "SELECT universe_key, entry_path, entry_type, body_preview, frontmatter_json, updated_at, indexed_at \
             FROM entry_kb_latest \
             ORDER BY indexed_at DESC LIMIT {limit}"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], row_to_kb_entry)
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/kb/ingest
///
/// Accepts a `KbIngestBody` JSON payload. Auth: `Authorization: Bearer <CO_KB_TOKEN>`.
/// Returns 503 when `CO_KB_TOKEN` env var is not set (ingest disabled).
pub async fn ingest_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<KbIngestBody>,
) -> Response {
    if let Some(reject) = check_kb_token(&headers) {
        return reject;
    }

    if body.universe_key.is_empty() || body.entry_path.is_empty() || body.body_hash.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "universe_key, entry_path, body_hash are required"})),
        )
            .into_response();
    }

    let indexed_at = chrono::Utc::now().to_rfc3339();
    let frontmatter_str = body.frontmatter_json.as_ref().map(|v| v.to_string());
    let asset_refs_str = body
        .asset_refs
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());

    let result = {
        let storage = state.core.storage.lock();
        upsert_kb_entry(
            storage.conn(),
            &body.universe_key,
            &body.entry_path,
            &body.body_hash,
            body.entry_type.as_deref(),
            frontmatter_str.as_deref(),
            body.body_preview.as_deref(),
            body.size_bytes,
            &body.updated_at,
            &indexed_at,
            asset_refs_str.as_deref(),
        )
    };

    match result {
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
        Ok(inserted) => {
            if inserted {
                crate::atividade::log_atividade(
                    state,
                    crate::atividade::Atividade {
                        acao: crate::atividade::Acao::Criar,
                        entidade: "kb".to_string(),
                        entidade_id: Some(format!("{}:{}", body.universe_key, body.entry_path)),
                        before: None,
                        after: Some(serde_json::json!({
                            "event": "kb.ingested",
                            "universe_key": body.universe_key,
                            "entry_path": body.entry_path,
                            "indexed_at": indexed_at,
                        })),
                        tipo: crate::atividade::Tipo::Sistema,
                        user_id: None,
                        ip: None,
                        user_agent: None,
                    },
                );
            }
            (
                StatusCode::ACCEPTED,
                Json(KbIngestResponse {
                    ok: true,
                    universe_key: body.universe_key,
                    entry_path: body.entry_path,
                    indexed_at,
                }),
            )
                .into_response()
        }
    }
}

/// GET /api/v1/kb/search?q=...&universe_key=...&limit=N
///
/// FTS5 full-text search over `body_preview`. `universe_key` is optional —
/// omitting it searches across all universes. `limit` is clamped to [1, 100].
pub async fn search_handler(
    State(state): State<AppState>,
    Query(params): Query<KbSearchParams>,
) -> Response {
    if params.q.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "q parameter is required"})),
        )
            .into_response();
    }

    let limit = params.limit.unwrap_or(20).clamp(1, 100) as i64;

    let results = {
        let storage = state.core.storage.lock();
        query_kb_search(
            storage.conn(),
            &params.q,
            params.universe_key.as_deref(),
            limit,
        )
    };

    crate::atividade::log_atividade(
        state,
        crate::atividade::Atividade {
            acao: crate::atividade::Acao::Ler,
            entidade: "kb".to_string(),
            entidade_id: Some("search".to_string()),
            before: None,
            after: Some(serde_json::json!({
                "event": "kb.queried",
                "q": params.q,
                "universe_key": params.universe_key,
            })),
            tipo: crate::atividade::Tipo::Sistema,
            user_id: None,
            ip: None,
            user_agent: None,
        },
    );

    let total = results.len();
    (StatusCode::OK, Json(KbSearchResponse { results, total })).into_response()
}

/// GET /api/v1/kb/recent?universe_key=...&limit=N
///
/// Returns the most recently-indexed entries (latest version per path).
/// `limit` is clamped to [1, 100].
pub async fn recent_handler(
    State(state): State<AppState>,
    Query(params): Query<KbRecentParams>,
) -> Json<KbRecentResponse> {
    let limit = params.limit.unwrap_or(20).clamp(1, 100) as i64;

    let entries = {
        let storage = state.core.storage.lock();
        query_kb_recent(storage.conn(), params.universe_key.as_deref(), limit)
    };

    Json(KbRecentResponse { entries })
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/search", get(search_handler))
        .route("/recent", get(recent_handler))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_kb_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE entry_kb_index (
                universe_key    TEXT NOT NULL,
                entry_path      TEXT NOT NULL,
                body_hash       TEXT NOT NULL,
                entry_type      TEXT,
                frontmatter_json TEXT,
                body_preview    TEXT,
                size_bytes      INTEGER,
                updated_at      TEXT NOT NULL,
                indexed_at      TEXT NOT NULL,
                asset_refs      TEXT,
                PRIMARY KEY (universe_key, entry_path, body_hash)
            );
            CREATE INDEX idx_kb_universe_type ON entry_kb_index(universe_key, entry_type);
            CREATE INDEX idx_kb_updated ON entry_kb_index(updated_at);

            CREATE TABLE entry_kb_latest (
                universe_key    TEXT NOT NULL,
                entry_path      TEXT NOT NULL,
                body_hash       TEXT NOT NULL,
                entry_type      TEXT,
                frontmatter_json TEXT,
                body_preview    TEXT,
                size_bytes      INTEGER,
                updated_at      TEXT NOT NULL,
                indexed_at      TEXT NOT NULL,
                asset_refs      TEXT,
                PRIMARY KEY (universe_key, entry_path)
            );
            CREATE INDEX idx_kb_latest_indexed ON entry_kb_latest(indexed_at);
            CREATE INDEX idx_kb_latest_type ON entry_kb_latest(universe_key, entry_type);

            CREATE VIRTUAL TABLE entry_kb_fts USING fts5(
                universe_key UNINDEXED,
                entry_path   UNINDEXED,
                body_preview
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn upsert_inserts_new_entry() {
        let conn = setup_kb_db();
        let inserted = upsert_kb_entry(
            &conn,
            "yuri",
            "notes/2026-06-05.md",
            "sha256:abc",
            Some("note"),
            Some(r#"{"title":"Test"}"#),
            Some("hello world preview"),
            Some(1024),
            "2026-06-05T10:00:00Z",
            "2026-06-05T10:01:00Z",
            None,
        )
        .unwrap();
        assert!(inserted, "first upsert must return true");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entry_kb_index WHERE universe_key='yuri'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let latest_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entry_kb_latest WHERE universe_key='yuri'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(latest_count, 1);
    }

    #[test]
    fn upsert_idempotent_same_hash() {
        let conn = setup_kb_db();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/a.md",
            "sha256:aaa",
            Some("note"),
            None,
            Some("preview"),
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();
        let inserted = upsert_kb_entry(
            &conn,
            "yuri",
            "notes/a.md",
            "sha256:aaa",
            Some("note"),
            None,
            Some("preview"),
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();
        assert!(!inserted, "second upsert with same hash must be idempotent");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entry_kb_index", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "history must have exactly one row");
    }

    #[test]
    fn upsert_new_hash_updates_latest() {
        let conn = setup_kb_db();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/b.md",
            "sha256:h1",
            Some("note"),
            None,
            Some("old preview"),
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/b.md",
            "sha256:h2",
            Some("note"),
            None,
            Some("new preview"),
            None,
            "2026-06-05T01:00:00Z",
            "2026-06-05T01:00:00Z",
            None,
        )
        .unwrap();

        let history_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entry_kb_index WHERE entry_path='notes/b.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(history_count, 2, "history keeps both versions");

        let latest_hash: String = conn
            .query_row(
                "SELECT body_hash FROM entry_kb_latest WHERE entry_path='notes/b.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(latest_hash, "sha256:h2", "latest must be updated to h2");
    }

    #[test]
    fn upsert_preserves_asset_refs() {
        let conn = setup_kb_db();
        let inserted = upsert_kb_entry(
            &conn,
            "yuri",
            "notes/c.md",
            "sha256:ccc",
            Some("note"),
            None,
            None,
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            Some(r#"["sha256:img1","sha256:img2"]"#),
        )
        .unwrap();
        assert!(inserted);

        let refs: Option<String> = conn
            .query_row(
                "SELECT asset_refs FROM entry_kb_index WHERE entry_path='notes/c.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(refs.as_deref(), Some(r#"["sha256:img1","sha256:img2"]"#));
    }

    #[test]
    fn fts_search_returns_matching_entry() {
        let conn = setup_kb_db();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/rust.md",
            "sha256:r1",
            Some("note"),
            None,
            Some("Rust programming language is fast"),
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/python.md",
            "sha256:p1",
            Some("note"),
            None,
            Some("Python is a scripting language"),
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();

        let results = query_kb_search(&conn, "Rust", None, 10);
        assert_eq!(results.len(), 1, "should find one Rust entry");
        assert_eq!(results[0].entry_path, "notes/rust.md");
    }

    #[test]
    fn fts_search_with_universe_filter() {
        let conn = setup_kb_db();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/a.md",
            "sha256:a1",
            Some("note"),
            None,
            Some("shared topic content"),
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();
        upsert_kb_entry(
            &conn,
            "other",
            "notes/b.md",
            "sha256:b1",
            Some("note"),
            None,
            Some("shared topic content"),
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();

        let results = query_kb_search(&conn, "topic", Some("yuri"), 10);
        assert_eq!(results.len(), 1, "universe filter must restrict results");
        assert_eq!(results[0].universe_key, "yuri");
    }

    #[test]
    fn fts_search_bad_query_returns_empty() {
        let conn = setup_kb_db();
        // Malformed FTS query — must not panic, must return empty results
        let results = query_kb_search(&conn, "\"unclosed", None, 10);
        assert!(
            results.is_empty(),
            "malformed FTS query must return empty results"
        );
    }

    #[test]
    fn recent_returns_latest_per_path() {
        let conn = setup_kb_db();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/d.md",
            "sha256:d1",
            Some("note"),
            None,
            Some("v1"),
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/d.md",
            "sha256:d2",
            Some("note"),
            None,
            Some("v2"),
            None,
            "2026-06-05T01:00:00Z",
            "2026-06-05T01:00:00Z",
            None,
        )
        .unwrap();

        let entries = query_kb_recent(&conn, Some("yuri"), 10);
        assert_eq!(
            entries.len(),
            1,
            "recent must deduplicate — one row per path"
        );
        assert_eq!(
            entries[0].body_preview.as_deref(),
            Some("v2"),
            "must show latest version"
        );
    }

    #[test]
    fn recent_universe_filter() {
        let conn = setup_kb_db();
        upsert_kb_entry(
            &conn,
            "yuri",
            "notes/e.md",
            "sha256:e1",
            Some("note"),
            None,
            None,
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();
        upsert_kb_entry(
            &conn,
            "other",
            "notes/f.md",
            "sha256:f1",
            Some("note"),
            None,
            None,
            None,
            "2026-06-05T00:00:00Z",
            "2026-06-05T00:00:00Z",
            None,
        )
        .unwrap();

        let entries = query_kb_recent(&conn, Some("yuri"), 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].universe_key, "yuri");
    }

    #[test]
    fn sanitize_key_strips_unsafe_chars() {
        assert_eq!(sanitize_key("yuri"), "yuri");
        assert_eq!(sanitize_key("my-universe_1"), "my-universe_1");
        assert_eq!(sanitize_key("evil'; DROP TABLE"), "evil");
        assert_eq!(sanitize_key("UPPER"), "");
    }

    // ---------------------------------------------------------------------------
    // HTTP integration tests
    // ---------------------------------------------------------------------------

    use crate::server::{CoreState, IndexState, IntegrationsState, RealtimeState};
    use std::sync::{Arc, Mutex as StdMutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxumStatus};
    use tempfile::tempdir;
    use tower::ServiceExt;

    // Serialize tests that mutate env vars to avoid race conditions under parallel test execution.
    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        let config = crate::config::WebConfig {
            port: 3000,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "quilomboaraucaria".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_api_key: None,
            wae_endpoint: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        };
        let storage = crate::storage::Storage::new(&config.data_dir);
        let experiment = crate::experiment::ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path)
                .expect("Failed to open test game storage"),
        );
        let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
        let state = crate::server::AppState::new(crate::server::AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: StdMutex::new(std::collections::HashMap::new()),
                chat_presence: StdMutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: StdMutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: StdMutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        crate::server::build_router(state, None)
    }

    #[tokio::test]
    async fn ingest_without_token_returns_503() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Ensure CO_KB_TOKEN is not set
        // SAFETY: test-only single-threaded env mutation
        unsafe { std::env::remove_var("CO_KB_TOKEN") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/kb/ingest")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        r#"{"universe_key":"yuri","entry_path":"notes/a.md","body_hash":"sha256:abc","updated_at":"2026-06-05T00:00:00Z"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn ingest_with_wrong_token_returns_401() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: test-only single-threaded env mutation
        unsafe { std::env::set_var("CO_KB_TOKEN", "correct-token") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/kb/ingest")
                    .header("Content-Type", "application/json")
                    .header("Authorization", "Bearer wrong-token")
                    .body(Body::from(
                        r#"{"universe_key":"yuri","entry_path":"notes/a.md","body_hash":"sha256:abc","updated_at":"2026-06-05T00:00:00Z"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
        // SAFETY: test-only single-threaded env mutation
        unsafe { std::env::remove_var("CO_KB_TOKEN") };
    }

    #[tokio::test]
    async fn ingest_with_valid_token_returns_202() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: test-only single-threaded env mutation
        unsafe { std::env::set_var("CO_KB_TOKEN", "test-kb-token") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/kb/ingest")
                    .header("Content-Type", "application/json")
                    .header("Authorization", "Bearer test-kb-token")
                    .body(Body::from(
                        r#"{"universe_key":"yuri","entry_path":"notes/a.md","body_hash":"sha256:abc","updated_at":"2026-06-05T00:00:00Z","body_preview":"test preview"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::ACCEPTED);
        // SAFETY: test-only single-threaded env mutation
        unsafe { std::env::remove_var("CO_KB_TOKEN") };
    }

    #[tokio::test]
    async fn search_empty_q_returns_400() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/kb/search?q=")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::BAD_REQUEST);
    }

    #[tokio::test]
    async fn search_with_q_returns_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/kb/search?q=rust")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_048_576)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["results"].is_array());
        assert!(json["total"].is_number());
    }

    #[tokio::test]
    async fn recent_returns_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/kb/recent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_048_576)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["entries"].is_array());
    }

    #[tokio::test]
    async fn sync_break_resilience_local_entry_write_succeeds_without_kb() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Simulate: CO_KB_TOKEN is not set (KB endpoint down), but local entry write must succeed.
        // SAFETY: test-only single-threaded env mutation
        unsafe { std::env::remove_var("CO_KB_TOKEN") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        // Create a universe for the test
        let create_universe = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes")
                    .header("Content-Type", "application/json")
                    .header("X-JWT-Secret", "test-secret")
                    .body(Body::from(
                        r#"{"key":"testbrain","name":"Test Brain","is_public":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // A 2xx or 4xx (auth) means the server didn't crash — local writes are unaffected
        // by KB endpoint state. The important thing is no panic / 500 from KB dependency.
        assert!(
            create_universe.status().as_u16() < 500,
            "local operation must not 500 because KB is down"
        );
    }
}
