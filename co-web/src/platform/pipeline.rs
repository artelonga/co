//! CO-88b — content-pipeline telemetry on the server.
//!
//! The pipeline runner (CO-88a) announces each vault PUT/GET's layer combo via
//! `X-Co-*` request headers. We record those as `pipeline.*` events in the same
//! `telemetry_events` table (CO-46), so the admin dashboard can pivot the
//! per-universe size/perf numbers by universe and combo.
//!
//! Event types written:
//! - `pipeline.transfer` — one per PUT/GET that carried pipeline headers,
//!   holding `co_format`, `compression`, `encryption`, `size_uncompressed`,
//!   `size_on_wire`, `encode_ns`, `decode_ns`, `combo`.
//! - `pipeline.encode` / `pipeline.decode` — emitted when the corresponding
//!   timing header is present, so encode/decode can be queried independently.

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::server::AppState;
use crate::telemetry::{EventRow, extract_session_id, insert_event};

// ---------------------------------------------------------------------------
// Header extraction
// ---------------------------------------------------------------------------

/// Per-request pipeline metrics parsed from `X-Co-*` headers + the body length.
#[derive(Debug, Clone, Serialize)]
pub struct PipelineMetrics {
    pub co_format: String,
    pub compression: String,
    pub encryption: String,
    pub combo: String,
    pub size_uncompressed: i64,
    pub size_on_wire: i64,
    pub encode_ns: Option<i64>,
    pub decode_ns: Option<i64>,
}

impl PipelineMetrics {
    /// Parse pipeline headers from a request. Returns `None` when the request
    /// carries no `X-Co-Format` header (i.e. it is not a pipeline request).
    pub fn from_headers(headers: &HeaderMap, body_len: usize) -> Option<PipelineMetrics> {
        let format = header_str(headers, "x-co-format")?;
        let compression = header_str(headers, "x-co-compression").unwrap_or_else(|| "raw".into());
        let encryption = header_str(headers, "x-co-encryption").unwrap_or_else(|| "none".into());
        let combo = header_str(headers, "x-co-combo").unwrap_or_else(|| "raw".into());
        let size_uncompressed =
            header_i64(headers, "x-co-size-uncompressed").unwrap_or(body_len as i64);
        let size_on_wire = header_i64(headers, "x-co-size-on-wire").unwrap_or(body_len as i64);
        Some(PipelineMetrics {
            co_format: format,
            compression,
            encryption,
            combo,
            size_uncompressed,
            size_on_wire,
            encode_ns: header_i64(headers, "x-co-encode-ns"),
            decode_ns: header_i64(headers, "x-co-decode-ns"),
        })
    }
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn header_i64(headers: &HeaderMap, name: &str) -> Option<i64> {
    header_str(headers, name).and_then(|s| s.parse().ok())
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// Record pipeline events for one vault request (fire-and-forget).
///
/// `op` is `"put"` or `"get"`. No-op when the request had no pipeline headers.
pub fn emit_pipeline_event(
    state: &AppState,
    universe: &str,
    path: &str,
    op: &'static str,
    metrics: &PipelineMetrics,
    headers: &HeaderMap,
) {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let session_id = extract_session_id(headers);
    let universe_key = Some(universe.to_string());
    let path = Some(path.to_string());

    let props = serde_json::json!({
        "co_format": metrics.co_format,
        "compression": metrics.compression,
        "encryption": metrics.encryption,
        "combo": metrics.combo,
        "size_uncompressed": metrics.size_uncompressed,
        "size_on_wire": metrics.size_on_wire,
        "encode_ns": metrics.encode_ns,
        "decode_ns": metrics.decode_ns,
        "op": op,
    });

    let mut rows = vec![EventRow {
        timestamp: timestamp.clone(),
        visitor_token: None,
        user_id: None,
        session_id: session_id.clone(),
        event_type: "pipeline.transfer".to_string(),
        event_name: op.to_string(),
        universe_key: universe_key.clone(),
        path: path.clone(),
        properties: Some(props.clone()),
        duration_ms: None,
        ip_hash: None,
        ua_device: None,
        ua_browser: None,
        ua_os: None,
        country: None,
        city: None,
    }];

    if let Some(encode_ns) = metrics.encode_ns {
        rows.push(timing_row(
            &timestamp,
            "pipeline.encode",
            op,
            &universe_key,
            &path,
            &session_id,
            &props,
            encode_ns,
        ));
    }
    if let Some(decode_ns) = metrics.decode_ns {
        rows.push(timing_row(
            &timestamp,
            "pipeline.decode",
            op,
            &universe_key,
            &path,
            &session_id,
            &props,
            decode_ns,
        ));
    }

    let state_clone = state.clone();
    tokio::spawn(async move {
        let storage = state_clone.core.storage.lock();
        for row in &rows {
            insert_event(storage.conn(), row);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn timing_row(
    timestamp: &str,
    event_type: &str,
    op: &str,
    universe_key: &Option<String>,
    path: &Option<String>,
    session_id: &Option<String>,
    props: &serde_json::Value,
    duration_ns: i64,
) -> EventRow {
    EventRow {
        timestamp: timestamp.to_string(),
        visitor_token: None,
        user_id: None,
        session_id: session_id.clone(),
        event_type: event_type.to_string(),
        event_name: op.to_string(),
        universe_key: universe_key.clone(),
        path: path.clone(),
        properties: Some(props.clone()),
        duration_ms: Some(duration_ns / 1_000_000),
        ip_hash: None,
        ua_device: None,
        ua_browser: None,
        ua_os: None,
        country: None,
        city: None,
    }
}

// ---------------------------------------------------------------------------
// Aggregation query — pipeline_summary(universe_key, since)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ComboPipelineStats {
    pub combo: String,
    pub co_format: String,
    pub compression: String,
    pub encryption: String,
    pub count: i64,
    pub raw_bytes: i64,
    pub encoded_bytes: i64,
    pub compression_ratio: f64,
    pub avg_encode_ns: i64,
    pub avg_decode_ns: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UniversePipelineStats {
    pub universe_key: String,
    pub files: i64,
    pub raw_bytes: i64,
    pub encoded_bytes: i64,
    pub compression_ratio: f64,
    pub avg_encode_ns: i64,
    pub avg_decode_ns: i64,
    pub by_combo: Vec<ComboPipelineStats>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineSummary {
    pub since: String,
    pub total_events: i64,
    pub per_universe: Vec<UniversePipelineStats>,
}

/// Aggregate `pipeline.transfer` events into the per-universe report shape.
///
/// `universe_key = None` summarises every universe. `since` is a SQLite
/// datetime modifier expression already resolved by the caller (an ISO-8601
/// timestamp); pass the empty string for "all time".
pub fn pipeline_summary(
    conn: &Connection,
    universe_key: Option<&str>,
    since: &str,
) -> PipelineSummary {
    // Group transfer events by (universe, combo, format, compression, encryption).
    let mut sql = String::from(
        "SELECT universe_key, \
                json_extract(properties,'$.combo')        AS combo, \
                json_extract(properties,'$.co_format')    AS co_format, \
                json_extract(properties,'$.compression')  AS compression, \
                json_extract(properties,'$.encryption')   AS encryption, \
                COUNT(*)                                   AS cnt, \
                COALESCE(SUM(json_extract(properties,'$.size_uncompressed')),0) AS raw_bytes, \
                COALESCE(SUM(json_extract(properties,'$.size_on_wire')),0)      AS enc_bytes, \
                COALESCE(AVG(json_extract(properties,'$.encode_ns')),0)         AS avg_enc, \
                COALESCE(AVG(json_extract(properties,'$.decode_ns')),0)         AS avg_dec \
         FROM telemetry_events \
         WHERE event_type = 'pipeline.transfer'",
    );
    if !since.is_empty() {
        sql.push_str(" AND timestamp >= ?1");
    }
    if universe_key.is_some() {
        sql.push_str(if since.is_empty() {
            " AND universe_key = ?1"
        } else {
            " AND universe_key = ?2"
        });
    }
    sql.push_str(
        " GROUP BY universe_key, combo, co_format, compression, encryption \
          ORDER BY universe_key, combo",
    );

    let params: Vec<&dyn rusqlite::ToSql> = match (since.is_empty(), universe_key.as_ref()) {
        (true, None) => vec![],
        (true, Some(u)) => vec![u],
        (false, None) => vec![&since],
        (false, Some(u)) => vec![&since, u],
    };

    let rows: Vec<(String, ComboPipelineStats)> = conn
        .prepare(&sql)
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map(params.as_slice(), |r| {
                let universe: String = r.get::<_, Option<String>>(0)?.unwrap_or_default();
                let raw_bytes: i64 = r.get(6)?;
                let enc_bytes: i64 = r.get(7)?;
                Ok((
                    universe,
                    ComboPipelineStats {
                        combo: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        co_format: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        compression: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        encryption: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        count: r.get(5)?,
                        raw_bytes,
                        encoded_bytes: enc_bytes,
                        compression_ratio: ratio(raw_bytes, enc_bytes),
                        avg_encode_ns: r.get::<_, f64>(8)? as i64,
                        avg_decode_ns: r.get::<_, f64>(9)? as i64,
                    },
                ))
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    // Roll combos up into per-universe stats.
    let mut per_universe: Vec<UniversePipelineStats> = Vec::new();
    let mut total_events = 0i64;
    for (universe, combo) in rows {
        total_events += combo.count;
        let entry = match per_universe.iter_mut().find(|u| u.universe_key == universe) {
            Some(e) => e,
            None => {
                per_universe.push(UniversePipelineStats {
                    universe_key: universe.clone(),
                    files: 0,
                    raw_bytes: 0,
                    encoded_bytes: 0,
                    compression_ratio: 1.0,
                    avg_encode_ns: 0,
                    avg_decode_ns: 0,
                    by_combo: Vec::new(),
                });
                per_universe.last_mut().unwrap()
            }
        };
        entry.files += combo.count;
        entry.raw_bytes += combo.raw_bytes;
        entry.encoded_bytes += combo.encoded_bytes;
        entry.by_combo.push(combo);
    }
    for u in &mut per_universe {
        u.compression_ratio = ratio(u.raw_bytes, u.encoded_bytes);
        // Weighted-by-count average of the per-combo averages.
        let (mut enc_sum, mut dec_sum, mut n) = (0i64, 0i64, 0i64);
        for c in &u.by_combo {
            enc_sum += c.avg_encode_ns * c.count;
            dec_sum += c.avg_decode_ns * c.count;
            n += c.count;
        }
        if n > 0 {
            u.avg_encode_ns = enc_sum / n;
            u.avg_decode_ns = dec_sum / n;
        }
    }

    PipelineSummary {
        since: since.to_string(),
        total_events,
        per_universe,
    }
}

fn ratio(raw: i64, encoded: i64) -> f64 {
    if encoded <= 0 {
        return 1.0;
    }
    ((raw as f64 / encoded as f64) * 100.0).round() / 100.0
}

// ---------------------------------------------------------------------------
// Admin endpoint + dashboard
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SummaryQuery {
    /// Optional universe filter.
    pub universe: Option<String>,
    /// Optional ISO-8601 lower bound; defaults to the last 24 hours.
    pub since: Option<String>,
}

/// GET /api/v1/admin/pipeline/summary — per-universe pipeline stats (yuri tier).
pub async fn pipeline_summary_handler(
    State(state): State<AppState>,
    Query(q): Query<SummaryQuery>,
) -> impl IntoResponse {
    let since = q
        .since
        .unwrap_or_else(|| (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339());
    let storage = state.core.storage.lock();
    let summary = pipeline_summary(storage.conn(), q.universe.as_deref(), &since);
    Json(summary).into_response()
}

/// Admin-only pipeline endpoints. Requires GitHub admin auth layered in router.
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/pipeline/summary", get(pipeline_summary_handler))
        .layer(axum::middleware::from_fn(
            crate::github_auth::require_github_admin,
        ))
}

/// GET /co/co-dev/pipeline — admin dashboard (sister to /co/telemetria).
pub async fn serve_pipeline_dashboard() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        PIPELINE_DASHBOARD_HTML,
    )
}

const PIPELINE_DASHBOARD_HTML: &str = include_str!("pipeline_dashboard.html");

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE telemetry_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                visitor_token TEXT, user_id TEXT, session_id TEXT,
                event_type TEXT NOT NULL, event_name TEXT NOT NULL,
                universe_key TEXT, path TEXT, properties TEXT,
                duration_ms INTEGER, ip_hash TEXT,
                ua_device TEXT, ua_browser TEXT, ua_os TEXT,
                country TEXT, city TEXT
            );",
        )
        .unwrap();
        conn
    }

    fn insert_transfer(conn: &Connection, universe: &str, combo: &str, raw: i64, wire: i64) {
        let props = serde_json::json!({
            "combo": combo,
            "co_format": "co/protobuf",
            "compression": if combo == "zstd" { "zstd-3" } else { "raw" },
            "encryption": "none",
            "size_uncompressed": raw,
            "size_on_wire": wire,
            "encode_ns": 1000,
            "decode_ns": 500,
            "op": "put",
        });
        let row = EventRow {
            timestamp: chrono::Utc::now().to_rfc3339(),
            visitor_token: None,
            user_id: None,
            session_id: None,
            event_type: "pipeline.transfer".to_string(),
            event_name: "put".to_string(),
            universe_key: Some(universe.to_string()),
            path: Some("a.md".to_string()),
            properties: Some(props),
            duration_ms: None,
            ip_hash: None,
            ua_device: None,
            ua_browser: None,
            ua_os: None,
            country: None,
            city: None,
        };
        insert_event(conn, &row);
    }

    #[test]
    fn from_headers_requires_format() {
        let mut h = HeaderMap::new();
        assert!(PipelineMetrics::from_headers(&h, 100).is_none());
        h.insert("x-co-format", "co/protobuf".parse().unwrap());
        h.insert("x-co-compression", "zstd-3".parse().unwrap());
        h.insert("x-co-size-on-wire", "40".parse().unwrap());
        let m = PipelineMetrics::from_headers(&h, 100).unwrap();
        assert_eq!(m.compression, "zstd-3");
        assert_eq!(m.size_on_wire, 40);
        assert_eq!(m.size_uncompressed, 100); // falls back to body_len
    }

    #[test]
    fn summary_aggregates_per_universe_and_combo() {
        let conn = mem_db();
        insert_transfer(&conn, "artelonga", "zstd", 1000, 400);
        insert_transfer(&conn, "artelonga", "zstd", 1000, 400);
        insert_transfer(&conn, "artelonga", "raw", 1000, 1010);
        insert_transfer(&conn, "rfq", "private", 500, 260);

        let s = pipeline_summary(&conn, None, "");
        assert_eq!(s.total_events, 4);
        let art = s
            .per_universe
            .iter()
            .find(|u| u.universe_key == "artelonga")
            .unwrap();
        assert_eq!(art.files, 3);
        assert_eq!(art.raw_bytes, 3000);
        assert_eq!(art.by_combo.len(), 2); // zstd + raw
        let zstd = art.by_combo.iter().find(|c| c.combo == "zstd").unwrap();
        assert_eq!(zstd.count, 2);
        assert_eq!(zstd.encoded_bytes, 800);
        assert!((zstd.compression_ratio - 2.5).abs() < 0.01);
    }

    #[test]
    fn summary_filters_by_universe() {
        let conn = mem_db();
        insert_transfer(&conn, "artelonga", "zstd", 1000, 400);
        insert_transfer(&conn, "rfq", "private", 500, 260);
        let s = pipeline_summary(&conn, Some("rfq"), "");
        assert_eq!(s.per_universe.len(), 1);
        assert_eq!(s.per_universe[0].universe_key, "rfq");
    }
}
