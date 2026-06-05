//! Public analytics endpoints — CO-179
//!
//! GET /api/v1/analytics/public/summary?days=N
//! GET /api/v1/analytics/public/recent?limit=N
//!
//! Read-only, no auth. Hardcoded to universe_key = 'artelonga'.
//! Strips all PII (visitor_token, ip_hash, raw properties).
//! 5-minute in-memory cache per (endpoint, query params).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::server::AppState;

/// Sanitiza um universe id pra um handle seguro ([a-z0-9-], ≤64). Vazio → "artelonga".
/// Usado pra interpolar com segurança em SQL (sem injection).
fn sanitize_universe(s: &str) -> String {
    let c: String = s
        .chars()
        .filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '-')
        .take(64)
        .collect();
    if c.is_empty() {
        UNIVERSE_KEY.to_string()
    } else {
        c
    }
}

fn valid_day(s: &str) -> bool {
    s.len() == 10
        && s.as_bytes().iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

const UNIVERSE_KEY: &str = "artelonga";
const CACHE_TTL: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// Response shapes (PII-free)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct TimeseriesBucket {
    pub bucket: String,
    pub count: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct TopPage {
    pub path: String,
    pub views: i64,
    pub visitors: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct GeoRow {
    pub country: String,
    pub city: String,
    pub visitors: i64,
    pub sessions: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct PublicSummary {
    pub as_of: String,
    pub window_days: u32,
    pub views: i64,
    pub events_total: i64,
    pub visitors: i64,
    pub returning: i64,
    pub sessions: i64,
    pub session_avg_ms: i64,
    pub countries: i64,
    pub cities: i64,
    pub timeseries: Vec<TimeseriesBucket>,
    pub top_pages: Vec<TopPage>,
    pub geo: Vec<GeoRow>,
}

#[derive(Debug, Serialize, Clone)]
pub struct RecentEvent {
    pub ts: i64,
    pub name: String,
    pub path: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PublicRecent {
    pub events: Vec<RecentEvent>,
}

// ---------------------------------------------------------------------------
// Per-params in-memory cache
// ---------------------------------------------------------------------------

struct SummaryCacheEntry {
    data: PublicSummary,
    fetched_at: Instant,
}

struct RecentCacheEntry {
    data: PublicRecent,
    fetched_at: Instant,
}

// keyed by (universe, days)
static SUMMARY_CACHE: OnceLock<Mutex<HashMap<(String, u32), SummaryCacheEntry>>> = OnceLock::new();
static RECENT_CACHE: OnceLock<Mutex<HashMap<u32, RecentCacheEntry>>> = OnceLock::new();

fn summary_cache() -> &'static Mutex<HashMap<(String, u32), SummaryCacheEntry>> {
    SUMMARY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn recent_cache() -> &'static Mutex<HashMap<u32, RecentCacheEntry>> {
    RECENT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SummaryParams {
    pub days: Option<u32>,
    /// Universe a consultar. Default "artelonga" (rede). Ex. "yuri" → mostra a
    /// universe yuri através da artelonga geral, com a PONTE histórico↔surface.
    pub universe: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RecentParams {
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// Aggregate query helpers
// ---------------------------------------------------------------------------

/// Métricas escalares de um rollup diário (DailyRollup.metrics no schema do artelonga).
#[derive(Debug, Deserialize, Default, Clone)]
pub struct RollupMetrics {
    #[serde(default)]
    pub pageviews: i64,
    #[serde(default)]
    pub visitors: i64,
    #[serde(default)]
    pub returning: i64,
    #[serde(default)]
    pub sessions: i64,
    #[serde(default)]
    pub bounced: i64,
    #[serde(default)]
    pub dwell_ms_sum: i64,
    #[serde(default)]
    pub conversions: i64,
}

/// Rollups consentidos (sem PII) pushados por producers, dentro da janela.
/// Vazio se a tabela não existir (ex. DBs de teste antigos) — degrada pra eventos.
pub fn query_rollups(conn: &Connection, universe: &str, days: u32) -> Vec<(String, RollupMetrics)> {
    conn.prepare(&format!(
        "SELECT day, metrics FROM analytics_rollups \
         WHERE universe_key = ?1 AND day >= date('now', '-{days} days') \
         ORDER BY day ASC"
    ))
    .ok()
    .and_then(|mut stmt| {
        stmt.query_map(params![universe], |r| {
            let day: String = r.get(0)?;
            let metrics_json: String = r.get(1)?;
            Ok((day, metrics_json))
        })
        .ok()
        .map(|rows| {
            rows.filter_map(|r| r.ok())
                .map(|(day, mj)| {
                    (
                        day,
                        serde_json::from_str::<RollupMetrics>(&mj).unwrap_or_default(),
                    )
                })
                .collect()
        })
    })
    .unwrap_or_default()
}

/// Default-universe wrapper (rede artelonga). Mantém o contrato existente.
pub fn query_public_summary(conn: &Connection, days: u32) -> PublicSummary {
    query_universe_summary(conn, UNIVERSE_KEY, days)
}

/// Summary de UMA universe, com a PONTE histórico↔surface:
///   match de eventos = `universe_key = X OR path LIKE '/X/%'`
///     → captura o histórico de `/yuri/*` servido pelo apex (universe_key='artelonga').
///   rollups (pushados pela surface) sobrepõem o NOVO dado, particionado no CUTOVER
///     (primeiro dia com rollup) → eventos só contam ANTES, rollups DEPOIS: sem dupla
///     contagem na fronteira da migração path→CNAME. Uma série contínua.
pub fn query_universe_summary(conn: &Connection, universe: &str, days: u32) -> PublicSummary {
    let u = sanitize_universe(universe);
    // ponte: universe_key direto OU path histórico `/u/*`
    let m = format!("(universe_key = '{u}' OR path LIKE '/{u}/%')");

    let rollups = query_rollups(conn, &u, days);
    let cutover: Option<String> = rollups.iter().map(|(d, _)| d.clone()).min();
    // quando há rollups, eventos só contam ANTES do cutover (evita dupla contagem)
    let event_bound = match &cutover {
        Some(c) => format!(" AND date(timestamp) < '{c}'"),
        None => String::new(),
    };
    let win = format!("AND timestamp >= datetime('now', '-{days} days'){event_bound}");

    let mut views: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM telemetry_events \
                 WHERE {m} AND event_type = 'pageview' {win}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut events_total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM telemetry_events WHERE {m} {win}"),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut visitors: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT visitor_token) FROM telemetry_events \
                 WHERE {m} AND visitor_token IS NOT NULL {win}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Returning: visitors seen on >= 2 distinct days within the window
    let mut returning: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM (\
                   SELECT visitor_token FROM telemetry_events \
                   WHERE {m} AND visitor_token IS NOT NULL {win} \
                   GROUP BY visitor_token \
                   HAVING COUNT(DISTINCT date(timestamp)) >= 2\
                 )"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut sessions: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT session_id) FROM telemetry_events \
                 WHERE {m} AND session_id IS NOT NULL {win}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Average page duration as a proxy for session engagement (event-based)
    let session_avg_ms: i64 = conn
        .query_row(
            &format!(
                "SELECT COALESCE(CAST(AVG(duration_ms) AS INTEGER), 0) \
                 FROM telemetry_events \
                 WHERE {m} AND duration_ms > 0 {win}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Geo counts — NULL when CO-178 (country/city columns) not yet applied
    let countries: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT country) FROM telemetry_events \
                 WHERE {m} AND country IS NOT NULL {win}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let cities: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT city) FROM telemetry_events \
                 WHERE {m} AND city IS NOT NULL {win}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut timeseries: Vec<TimeseriesBucket> = conn
        .prepare(&format!(
            "SELECT date(timestamp) AS bucket, COUNT(*) AS cnt \
             FROM telemetry_events \
             WHERE {m} {win} \
             GROUP BY bucket ORDER BY bucket ASC"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(TimeseriesBucket {
                    bucket: r.get(0)?,
                    count: r.get(1)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    let top_pages: Vec<TopPage> = conn
        .prepare(&format!(
            "SELECT path, COUNT(*) AS views, COUNT(DISTINCT visitor_token) AS visitors \
             FROM telemetry_events \
             WHERE {m} AND event_type = 'pageview' AND path IS NOT NULL {win} \
             GROUP BY path ORDER BY views DESC LIMIT 20"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(TopPage {
                    path: r.get(0)?,
                    views: r.get(1)?,
                    visitors: r.get(2)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    // Geo aggregation — empty until CO-178 populates country/city columns
    let geo: Vec<GeoRow> = conn
        .prepare(&format!(
            "SELECT country, city, \
                    COUNT(DISTINCT visitor_token) AS visitors, \
                    COUNT(DISTINCT session_id) AS sessions \
             FROM telemetry_events \
             WHERE {m} AND country IS NOT NULL AND city IS NOT NULL {win} \
             GROUP BY country, city ORDER BY visitors DESC LIMIT 50"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(GeoRow {
                    country: r.get(0)?,
                    city: r.get(1)?,
                    visitors: r.get(2)?,
                    sessions: r.get(3)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    // ── overlay dos rollups (porção pós-cutover): soma escalares + estende a série ──
    if !rollups.is_empty() {
        views += rollups.iter().map(|(_, x)| x.pageviews).sum::<i64>();
        events_total += rollups.iter().map(|(_, x)| x.pageviews).sum::<i64>();
        visitors += rollups.iter().map(|(_, x)| x.visitors).sum::<i64>();
        returning += rollups.iter().map(|(_, x)| x.returning).sum::<i64>();
        sessions += rollups.iter().map(|(_, x)| x.sessions).sum::<i64>();
        for (day, x) in &rollups {
            timeseries.push(TimeseriesBucket {
                bucket: day.clone(),
                count: x.pageviews,
            });
        }
        timeseries.sort_by(|a, b| a.bucket.cmp(&b.bucket));
    }

    PublicSummary {
        as_of: chrono::Utc::now().to_rfc3339(),
        window_days: days,
        views,
        events_total,
        visitors,
        returning,
        sessions,
        session_avg_ms,
        countries,
        cities,
        timeseries,
        top_pages,
        geo,
    }
}

// ---------------------------------------------------------------------------
// Rollup ingest (producer push) — DailyRollup do schema do artelonga
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DailyRollupIn {
    pub universe: String,
    pub day: String,
    #[serde(default)]
    pub metrics: serde_json::Value,
    #[serde(default)]
    pub dims: serde_json::Value,
}

/// Upsert idempotente keyed by (universe, day).
pub fn upsert_rollup(
    conn: &Connection,
    universe: &str,
    day: &str,
    metrics: &str,
    dims: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO analytics_rollups (universe_key, day, metrics, dims, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(universe_key, day) DO UPDATE SET \
           metrics = excluded.metrics, dims = excluded.dims, updated_at = excluded.updated_at",
        params![
            universe,
            day,
            metrics,
            dims,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

/// POST /api/v1/analytics/public/rollups — recebe um DailyRollup consentido (sem PII)
/// de um producer (surface universe-owned, parceiro, universe co, SDK). Auth: bearer
/// token `CO_ROLLUP_TOKEN` (se a env não estiver setada, o ingest fica desabilitado).
pub async fn rollups_ingest_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<DailyRollupIn>,
) -> Response {
    let Ok(expected) = std::env::var("CO_ROLLUP_TOKEN") else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "rollup ingest disabled"})),
        )
            .into_response();
    };
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != format!("Bearer {expected}") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }
    if !valid_day(&body.day) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "day must be YYYY-MM-DD"})),
        )
            .into_response();
    }
    let u = sanitize_universe(&body.universe);
    let metrics = serde_json::to_string(&body.metrics).unwrap_or_else(|_| "{}".to_string());
    let dims = serde_json::to_string(&body.dims).unwrap_or_else(|_| "{}".to_string());
    {
        let storage = state.core.storage.lock();
        if upsert_rollup(storage.conn(), &u, &body.day, &metrics, &dims).is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "db error"})),
            )
                .into_response();
        }
    }
    // invalida o cache de summary (qualquer universe/days)
    summary_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"ok": true, "universe": u, "day": body.day})),
    )
        .into_response()
}

pub fn query_public_recent(conn: &Connection, limit: u32) -> PublicRecent {
    // ts as milliseconds since epoch; country/city are NULL until CO-178
    let events: Vec<RecentEvent> = conn
        .prepare(&format!(
            "SELECT \
               CAST((julianday(timestamp) - 2440587.5) * 86400000 AS INTEGER) AS ts, \
               event_name, \
               path, \
               NULL AS country, \
               NULL AS city \
             FROM telemetry_events \
             WHERE universe_key = '{UNIVERSE_KEY}' \
             ORDER BY timestamp DESC \
             LIMIT {limit}"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(RecentEvent {
                    ts: r.get(0)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                    country: r.get(3)?,
                    city: r.get(4)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    PublicRecent { events }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/analytics/public/summary?days=N
///
/// `days` clamped to [1, 365], default 30. Returns 400 for days=0.
pub async fn summary_handler(
    State(state): State<AppState>,
    Query(params): Query<SummaryParams>,
) -> Result<Json<PublicSummary>, Response> {
    let raw = params.days.unwrap_or(30);
    if raw == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "days must be >= 1"})),
        )
            .into_response());
    }
    let days = raw.min(365);
    let universe = sanitize_universe(params.universe.as_deref().unwrap_or(UNIVERSE_KEY));
    let key = (universe.clone(), days);

    {
        let cache = summary_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&key)
            && entry.fetched_at.elapsed() < CACHE_TTL
        {
            return Ok(Json(entry.data.clone()));
        }
    }

    let data = {
        let storage = state.core.storage.lock();
        query_universe_summary(storage.conn(), &universe, days)
    };

    {
        let mut cache = summary_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(
            key,
            SummaryCacheEntry {
                data: data.clone(),
                fetched_at: Instant::now(),
            },
        );
    }

    Ok(Json(data))
}

/// GET /api/v1/analytics/public/recent?limit=N
///
/// `limit` clamped to [1, 200], default 50.
pub async fn recent_handler(
    State(state): State<AppState>,
    Query(params): Query<RecentParams>,
) -> Json<PublicRecent> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    {
        let cache = recent_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&limit)
            && entry.fetched_at.elapsed() < CACHE_TTL
        {
            return Json(entry.data.clone());
        }
    }

    let data = {
        let storage = state.core.storage.lock();
        query_public_recent(storage.conn(), limit)
    };

    {
        let mut cache = recent_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(
            limit,
            RecentCacheEntry {
                data: data.clone(),
                fetched_at: Instant::now(),
            },
        );
    }

    Json(data)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

// ===========================================================================
// CO-180: popularity endpoint
// ===========================================================================
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct PopularityItem {
    pub path: String,
    pub slug: String,
    pub views: i64,
    pub visitors: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct PopularityResponse {
    pub as_of: String,
    pub window_days: u32,
    pub prefix: String,
    pub items: Vec<PopularityItem>,
}

// ---------------------------------------------------------------------------
// Cache — 5-minute TTL per (prefix, days)
// ---------------------------------------------------------------------------

#[derive(Hash, PartialEq, Eq, Clone)]
struct CacheKey {
    prefix: String,
    days: u32,
}

struct CacheEntry {
    data: PopularityResponse,
    fetched_at: Instant,
}

static POPULARITY_CACHE: OnceLock<Mutex<HashMap<CacheKey, CacheEntry>>> = OnceLock::new();

fn popularity_cache() -> &'static Mutex<HashMap<CacheKey, CacheEntry>> {
    POPULARITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PopularityParams {
    pub prefix: Option<String>,
    pub days: Option<u32>,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate_prefix(prefix: &str) -> Result<(), &'static str> {
    if !prefix.starts_with('/') {
        return Err("prefix must start with /");
    }
    if prefix.contains("..") {
        return Err("prefix must not contain ..");
    }
    if prefix.len() > 64 {
        return Err("prefix must be at most 64 characters");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Query helper
// ---------------------------------------------------------------------------

pub fn query_popularity(
    conn: &rusqlite::Connection,
    prefix: &str,
    days: u32,
) -> Vec<PopularityItem> {
    let days_str = format!("-{days} days");
    let prefix_pattern = format!("{prefix}%");

    let Ok(mut stmt) = conn.prepare(
        "SELECT path, COUNT(*) AS views, COUNT(DISTINCT visitor_token) AS visitors \
         FROM telemetry_events \
         WHERE universe_key = ?1 \
           AND event_name = 'page_view' \
           AND path LIKE ?2 \
           AND timestamp >= datetime('now', ?3) \
         GROUP BY path \
         ORDER BY views DESC, path ASC \
         LIMIT 200",
    ) else {
        return vec![];
    };

    stmt.query_map(params![UNIVERSE_KEY, prefix_pattern, days_str], |r| {
        let path: String = r.get(0)?;
        let views: i64 = r.get(1)?;
        let visitors: i64 = r.get(2)?;
        let slug = path
            .strip_prefix(prefix)
            .unwrap_or(&path)
            .trim_end_matches('/')
            .to_string();
        Ok(PopularityItem {
            path,
            slug,
            views,
            visitors,
        })
    })
    .ok()
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /api/v1/analytics/public/popularity?prefix=<path>&days=<n>
///
/// `prefix` is required, must start with `/`, no `..`, max 64 chars.
/// `days` is clamped to [1, 365], default 30.
pub async fn popularity_handler(
    State(state): State<AppState>,
    Query(params): Query<PopularityParams>,
) -> Result<Json<PopularityResponse>, Response> {
    let prefix = match params.prefix {
        Some(p) => p,
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "prefix is required"})),
            )
                .into_response());
        }
    };

    if let Err(msg) = validate_prefix(&prefix) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": msg})),
        )
            .into_response());
    }

    let days = params.days.unwrap_or(30).clamp(1, 365);

    let cache_key = CacheKey {
        prefix: prefix.clone(),
        days,
    };

    {
        let cache = popularity_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&cache_key)
            && entry.fetched_at.elapsed() < CACHE_TTL
        {
            return Ok(Json(entry.data.clone()));
        }
    }

    let items = {
        let storage = state.core.storage.lock();
        query_popularity(storage.conn(), &prefix, days)
    };

    let data = PopularityResponse {
        as_of: chrono::Utc::now().to_rfc3339(),
        window_days: days,
        prefix: prefix.clone(),
        items,
    };

    {
        let mut cache = popularity_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(
            cache_key,
            CacheEntry {
                data: data.clone(),
                fetched_at: Instant::now(),
            },
        );
    }

    Ok(Json(data))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/summary", get(summary_handler))
        .route("/recent", get(recent_handler))
        .route("/popularity", get(popularity_handler))
        .route("/rollups", post(rollups_ingest_handler))
}
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE telemetry_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                visitor_token TEXT,
                user_id TEXT,
                session_id TEXT,
                event_type TEXT NOT NULL,
                event_name TEXT NOT NULL,
                universe_key TEXT,
                path TEXT,
                properties TEXT,
                duration_ms INTEGER,
                ip_hash TEXT,
                ua_device TEXT,
                ua_browser TEXT,
                ua_os TEXT
            );",
        )
        .unwrap();
        conn
    }

    fn insert_pageview(
        conn: &Connection,
        universe: &str,
        visitor: &str,
        session: &str,
        path: &str,
        offset_days: i64,
    ) {
        conn.execute(
            &format!(
                "INSERT INTO telemetry_events \
                 (timestamp, visitor_token, session_id, event_type, event_name, universe_key, path) \
                 VALUES (datetime('now', '{offset_days} days'), ?1, ?2, 'pageview', 'page_view', ?3, ?4)"
            ),
            rusqlite::params![visitor, session, universe, path],
        )
        .unwrap();
    }

    // --- summary shape ---

    #[test]
    fn test_summary_empty_db_returns_zeros() {
        let conn = create_test_db();
        let s = query_public_summary(&conn, 7);
        assert_eq!(s.window_days, 7);
        assert_eq!(s.views, 0);
        assert_eq!(s.events_total, 0);
        assert_eq!(s.visitors, 0);
        assert_eq!(s.returning, 0);
        assert_eq!(s.sessions, 0);
        assert_eq!(s.session_avg_ms, 0);
        assert_eq!(s.countries, 0);
        assert_eq!(s.cities, 0);
        assert!(s.timeseries.is_empty());
        assert!(s.top_pages.is_empty());
        assert!(s.geo.is_empty());
    }

    #[test]
    fn test_summary_counts_artelonga_only() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/about", 0);
        insert_pageview(&conn, "other", "v3", "s3", "/", 0);
        let s = query_public_summary(&conn, 7);
        assert_eq!(s.views, 2, "only artelonga events should be counted");
        assert_eq!(s.visitors, 2);
    }

    #[test]
    fn test_summary_days_window_filters_old_events() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/", -10);
        let s = query_public_summary(&conn, 7);
        assert_eq!(s.views, 1, "events older than window must be excluded");
    }

    #[test]
    fn test_summary_returning_visitors() {
        let conn = create_test_db();
        // visitor v1 on two different days → returning
        insert_pageview(&conn, "artelonga", "v1", "s1", "/", 0);
        insert_pageview(&conn, "artelonga", "v1", "s2", "/", -1);
        // visitor v2 only once → not returning
        insert_pageview(&conn, "artelonga", "v2", "s3", "/", 0);
        let s = query_public_summary(&conn, 30);
        assert_eq!(s.returning, 1);
    }

    #[test]
    fn test_summary_timeseries_buckets() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/", -1);
        let s = query_public_summary(&conn, 7);
        assert_eq!(s.timeseries.len(), 2);
        assert!(s.timeseries.iter().all(|b| b.count >= 1));
    }

    #[test]
    fn test_summary_top_pages_ordered_by_views() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/about", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/", 0);
        insert_pageview(&conn, "artelonga", "v3", "s3", "/", 0);
        let s = query_public_summary(&conn, 7);
        assert_eq!(s.top_pages[0].path, "/", "/ has 2 views, should be first");
        assert_eq!(s.top_pages[0].views, 2);
    }

    #[test]
    fn test_summary_geo_empty_without_co178_columns() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/", 0);
        // No country/city columns → geo must be empty, no panic
        let s = query_public_summary(&conn, 7);
        assert!(
            s.geo.is_empty(),
            "geo must be empty until CO-178 adds country/city columns"
        );
    }

    #[test]
    fn test_summary_no_pii_in_struct() {
        let conn = create_test_db();
        let s = query_public_summary(&conn, 7);
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            !json.contains("visitor_token"),
            "visitor_token must not appear"
        );
        assert!(!json.contains("ip_hash"), "ip_hash must not appear");
        assert!(
            !json.contains("properties"),
            "raw properties must not appear"
        );
    }

    // --- recent shape ---

    #[test]
    fn test_recent_empty_db() {
        let conn = create_test_db();
        let r = query_public_recent(&conn, 50);
        assert!(r.events.is_empty());
    }

    #[test]
    fn test_recent_returns_artelonga_only() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/", 0);
        insert_pageview(&conn, "other", "v2", "s2", "/x", 0);
        let r = query_public_recent(&conn, 50);
        assert_eq!(r.events.len(), 1);
    }

    #[test]
    fn test_recent_limit_respected() {
        let conn = create_test_db();
        for i in 0..10 {
            insert_pageview(
                &conn,
                "artelonga",
                &format!("v{i}"),
                &format!("s{i}"),
                "/",
                0,
            );
        }
        let r = query_public_recent(&conn, 3);
        assert_eq!(r.events.len(), 3);
    }

    #[test]
    fn test_recent_no_pii_in_struct() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/", 0);
        let r = query_public_recent(&conn, 50);
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            !json.contains("visitor_token"),
            "visitor_token must not appear"
        );
        assert!(!json.contains("ip_hash"), "ip_hash must not appear");
        assert!(
            !json.contains("properties"),
            "raw properties must not appear"
        );
    }

    // --- universe bridge + rollups (CO-340) ---

    fn create_rollups_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE analytics_rollups (
                universe_key TEXT NOT NULL, day TEXT NOT NULL,
                metrics TEXT NOT NULL, dims TEXT NOT NULL DEFAULT '{}',
                updated_at TEXT NOT NULL, PRIMARY KEY (universe_key, day));",
        )
        .unwrap();
    }
    fn today() -> String {
        chrono::Utc::now().format("%Y-%m-%d").to_string()
    }

    #[test]
    fn test_universe_bridge_matches_historical_path() {
        // historical /yuri served by apex: universe_key='artelonga', path '/yuri/...'
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/yuri/", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/yuri/resume", 0);
        insert_pageview(&conn, "artelonga", "v3", "s3", "/", 0); // not yuri
        let s = query_universe_summary(&conn, "yuri", 7);
        assert_eq!(s.views, 2, "yuri bridges historical /yuri/* paths");
        let s_art = query_universe_summary(&conn, "artelonga", 7);
        assert_eq!(s_art.views, 3, "artelonga still counts all");
    }

    #[test]
    fn test_universe_matches_direct_universe_key() {
        let conn = create_test_db();
        insert_pageview(&conn, "yuri", "v1", "s1", "/", 0); // new surface: universe_key='yuri'
        let s = query_universe_summary(&conn, "yuri", 7);
        assert_eq!(s.views, 1);
    }

    #[test]
    fn test_rollup_overlay_extends_timeseries_and_totals() {
        let conn = create_test_db();
        create_rollups_table(&conn);
        insert_pageview(&conn, "artelonga", "v1", "s1", "/yuri/", -5); // historical (pre-cutover)
        upsert_rollup(
            &conn,
            "yuri",
            &today(),
            r#"{"pageviews":10,"visitors":4,"sessions":6}"#,
            "{}",
        )
        .unwrap();
        let s = query_universe_summary(&conn, "yuri", 30);
        assert_eq!(s.views, 11, "1 historical event + 10 rollup pageviews");
        assert!(s.visitors >= 4, "rollup visitors added");
        assert!(s.timeseries.len() >= 2, "historical day + rollup day");
        assert!(
            s.timeseries.iter().any(|b| b.count == 10),
            "rollup day count present in series"
        );
    }

    #[test]
    fn test_rollup_cutover_excludes_same_day_events() {
        // events on/after the cutover (first rollup day) are NOT double-counted
        let conn = create_test_db();
        create_rollups_table(&conn);
        insert_pageview(&conn, "artelonga", "v1", "s1", "/yuri/", 0); // TODAY (= cutover)
        upsert_rollup(&conn, "yuri", &today(), r#"{"pageviews":10}"#, "{}").unwrap();
        let s = query_universe_summary(&conn, "yuri", 30);
        assert_eq!(
            s.views, 10,
            "same-day event excluded; rollup wins; no double count"
        );
    }

    #[test]
    fn test_upsert_rollup_idempotent_latest_wins() {
        let conn = create_test_db();
        create_rollups_table(&conn);
        upsert_rollup(&conn, "yuri", "2026-06-01", r#"{"pageviews":5}"#, "{}").unwrap();
        upsert_rollup(&conn, "yuri", "2026-06-01", r#"{"pageviews":8}"#, "{}").unwrap();
        let r = query_rollups(&conn, "yuri", 3650);
        assert_eq!(r.len(), 1, "one row per (universe, day)");
        assert_eq!(r[0].1.pageviews, 8, "latest upsert wins");
    }

    #[test]
    fn test_sanitize_universe_strips_unsafe() {
        assert_eq!(sanitize_universe("yuri"), "yuri");
        assert_eq!(sanitize_universe("yu ri'; DROP"), "yuri");
        assert_eq!(sanitize_universe(""), "artelonga");
    }

    #[test]
    fn test_valid_day_format() {
        assert!(valid_day("2026-06-05"));
        assert!(!valid_day("2026-6-5"));
        assert!(!valid_day("not-a-date"));
    }

    // --- HTTP integration tests ---

    use crate::server::{CoreState, IndexState, IntegrationsState, RealtimeState};
    use std::sync::{Arc, Mutex as StdMutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxumStatus};
    use tempfile::tempdir;
    use tower::ServiceExt;

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
        let state: crate::server::AppState =
            crate::server::AppState::new(crate::server::AppStateInner {
                core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
                realtime: Arc::new(RealtimeState {
                    doc_rooms: crate::ws::new_room_manager(),
                    sync_rooms: crate::sync_ws::new_sync_room_manager(),
                    chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                    chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
                }),
                index: Arc::new(IndexState {
                    cache: crate::cache::CacheLayer::new(),
                    embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                    embedding_tx,
                }),
                integrations: Arc::new(IntegrationsState {
                    mail,
                    geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
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
    async fn test_summary_days_default_returns_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/summary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
    }

    #[tokio::test]
    async fn test_summary_days_7_returns_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/summary?days=7")
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
        assert_eq!(json["window_days"], 7);
        assert!(json["views"].as_i64().unwrap() >= 0);
        assert!(json["events_total"].as_i64().unwrap() >= 0);
        assert!(json["visitors"].as_i64().unwrap() >= 0);
        assert!(json["timeseries"].is_array());
        assert!(json["top_pages"].is_array());
        assert!(json["geo"].is_array());
    }

    #[tokio::test]
    async fn test_summary_days_999_clamped_to_365() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/summary?days=999")
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
        assert_eq!(json["window_days"], 365, "days=999 must be clamped to 365");
    }

    #[tokio::test]
    async fn test_summary_days_0_returns_400() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/summary?days=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_recent_limit_200_returns_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/recent?limit=200")
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
        assert!(json["events"].is_array());
        assert!(json["events"].as_array().unwrap().len() <= 200);
    }

    #[tokio::test]
    async fn test_response_no_pii_fields() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/summary?days=7")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_048_576)
            .await
            .unwrap();
        let s = std::str::from_utf8(&body).unwrap();
        assert!(
            !s.contains("visitor_token"),
            "visitor_token must not appear in response"
        );
        assert!(
            !s.contains("ip_hash"),
            "ip_hash must not appear in response"
        );
        assert!(
            !s.contains("\"properties\""),
            "raw properties must not appear in response"
        );
    }

    #[tokio::test]
    async fn test_cors_preflight_allowed() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/v1/analytics/public/summary")
                    .header("Origin", "https://artelonga.com.br")
                    .header("Access-Control-Request-Method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // CORS preflight must not return 405
        assert_ne!(
            resp.status(),
            AxumStatus::METHOD_NOT_ALLOWED,
            "OPTIONS preflight must be accepted"
        );
    }
}
