//! Public analytics endpoints — CO-179
//!
//! GET /api/v1/analytics/public/summary?days=N
//! GET /api/v1/analytics/public/recent?limit=N
//! GET /api/v1/analytics/public/funnel?days=N   (CO-378)
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
use sha2::{Digest, Sha256};

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
// CO-378: privacy helpers
// ---------------------------------------------------------------------------

/// Returns true if the path matches a private/internal segment pattern.
pub fn is_private_path(path: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "/_drafts/",
        "/_proposals/",
        "/_smoke/",
        "/_t/",
        "/_proof/",
        "/probe/",
    ];
    PATTERNS.iter().any(|p| path.contains(p))
}

/// Returns a deterministic opaque token for a private path. Used in admin-only views.
pub fn redact_path(path: &str) -> String {
    let hash = Sha256::digest(path.as_bytes());
    let hex16: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("<private-path-{hex16}>")
}

/// SQL predicate selecting private-path events.
const PRIVATE_PATH_EXPR: &str = "(path LIKE '%/_drafts/%' \
    OR path LIKE '%/_proposals/%' \
    OR path LIKE '%/_smoke/%' \
    OR path LIKE '%/_t/%' \
    OR path LIKE '%/_proof/%' \
    OR path LIKE '%/probe/%')";

/// SQL predicate selecting public-path events.
const PUBLIC_PATH_EXPR: &str = "NOT (path LIKE '%/_drafts/%' \
    OR path LIKE '%/_proposals/%' \
    OR path LIKE '%/_smoke/%' \
    OR path LIKE '%/_t/%' \
    OR path LIKE '%/_proof/%' \
    OR path LIKE '%/probe/%')";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private: Option<bool>,
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
// CO-378: funnel types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct FunnelEntry {
    pub path: String,
    pub views: i64,
    pub visitors: i64,
    pub pct: f64,
}

#[derive(Debug, Serialize, Clone)]
pub struct FunnelResponse {
    pub as_of: String,
    pub window_days: u32,
    pub total_views: i64,
    pub total_private_views: i64,
    pub by_path: Vec<FunnelEntry>,
}

#[derive(Debug, Deserialize)]
pub struct FunnelParams {
    pub days: Option<u32>,
    pub universe: Option<String>,
    // CO-371: acquisition funnel params (admin-only when `window` is present)
    pub window: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub breakdown: Option<String>,
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

type SummaryCacheMap = Mutex<HashMap<(String, u32, bool), SummaryCacheEntry>>;

// keyed by (universe, days, include_private)
static SUMMARY_CACHE: OnceLock<SummaryCacheMap> = OnceLock::new();
static RECENT_CACHE: OnceLock<Mutex<HashMap<u32, RecentCacheEntry>>> = OnceLock::new();

fn summary_cache() -> &'static SummaryCacheMap {
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
    /// Universe a consultar. Default "artelonga" (rede).
    pub universe: Option<String>,
    /// CO-378: include private paths in top_pages (redacted). Requires admin auth.
    pub include_private: Option<bool>,
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

/// Rollups públicos (path_private=0) dentro da janela.
pub fn query_rollups(conn: &Connection, universe: &str, days: u32) -> Vec<(String, RollupMetrics)> {
    conn.prepare(&format!(
        "SELECT day, metrics FROM analytics_rollups \
         WHERE universe_key = ?1 AND day >= date('now', '-{days} days') \
           AND (path_private = 0 OR path_private IS NULL) \
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

/// Todos os rollups dentro da janela, incluindo linhas marcadas como privadas.
pub fn query_rollups_all(
    conn: &Connection,
    universe: &str,
    days: u32,
) -> Vec<(String, RollupMetrics)> {
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
    query_universe_summary(conn, UNIVERSE_KEY, days, false)
}

/// Builds the top_pages list with privacy filtering.
///
/// Default (include_private=false): public paths only + a single `(private)` cluster entry.
/// Admin (include_private=true): all paths; private paths shown as `<private-path-{hex}>`.
fn build_top_pages(conn: &Connection, m: &str, win: &str, include_private: bool) -> Vec<TopPage> {
    let mut pages: Vec<TopPage> = conn
        .prepare(&format!(
            "SELECT path, COUNT(*) AS views, COUNT(DISTINCT visitor_token) AS visitors \
             FROM telemetry_events \
             WHERE {m} AND event_type = 'pageview' AND path IS NOT NULL AND {PUBLIC_PATH_EXPR} {win} \
             GROUP BY path ORDER BY views DESC LIMIT 20"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(TopPage {
                    path: r.get(0)?,
                    views: r.get(1)?,
                    visitors: r.get(2)?,
                    private: None,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    if include_private {
        let private_pages: Vec<TopPage> = conn
            .prepare(&format!(
                "SELECT path, COUNT(*) AS views, COUNT(DISTINCT visitor_token) AS visitors \
                 FROM telemetry_events \
                 WHERE {m} AND event_type = 'pageview' AND path IS NOT NULL AND {PRIVATE_PATH_EXPR} {win} \
                 GROUP BY path ORDER BY views DESC"
            ))
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |r| {
                    let path: String = r.get(0)?;
                    Ok(TopPage {
                        path: redact_path(&path),
                        views: r.get(1)?,
                        visitors: r.get(2)?,
                        private: Some(true),
                    })
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();
        pages.extend(private_pages);
        pages.sort_by(|a, b| b.views.cmp(&a.views));
        pages.truncate(20);
    } else {
        let (priv_views, priv_visitors): (i64, i64) = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*), COUNT(DISTINCT visitor_token) \
                     FROM telemetry_events \
                     WHERE {m} AND event_type = 'pageview' AND path IS NOT NULL \
                       AND {PRIVATE_PATH_EXPR} {win}"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        if priv_views > 0 {
            pages.push(TopPage {
                path: "(private)".to_string(),
                views: priv_views,
                visitors: priv_visitors,
                private: Some(true),
            });
        }
    }
    pages
}

/// Summary de UMA universe, com a PONTE histórico↔surface:
///   match de eventos = `universe_key = X OR path LIKE '/X/%'`
///     → captura o histórico de `/yuri/*` servido pelo apex (universe_key='artelonga').
///   rollups (pushados pela surface) sobrepõem o NOVO dado, particionado no CUTOVER
///     (primeiro dia com rollup) → eventos só contam ANTES, rollups DEPOIS: sem dupla
///     contagem na fronteira da migração path→CNAME. Uma série contínua.
pub fn query_universe_summary(
    conn: &Connection,
    universe: &str,
    days: u32,
    include_private: bool,
) -> PublicSummary {
    let u = sanitize_universe(universe);
    let m = format!("(universe_key = '{u}' OR path LIKE '/{u}/%')");

    let rollups = if include_private {
        query_rollups_all(conn, &u, days)
    } else {
        query_rollups(conn, &u, days)
    };
    let cutover: Option<String> = rollups.iter().map(|(d, _)| d.clone()).min();
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

    let top_pages = build_top_pages(conn, &m, &win, include_private);

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

/// Funnel report: total views (including private) + per-path breakdown (public only).
pub fn query_funnel(conn: &Connection, universe: &str, days: u32) -> FunnelResponse {
    let u = sanitize_universe(universe);
    let m = format!("(universe_key = '{u}' OR path LIKE '/{u}/%')");
    let win = format!("AND timestamp >= datetime('now', '-{days} days')");

    let total_views: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM telemetry_events \
                 WHERE {m} AND event_type = 'pageview' {win}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let total_private_views: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM telemetry_events \
                 WHERE {m} AND event_type = 'pageview' AND path IS NOT NULL \
                   AND {PRIVATE_PATH_EXPR} {win}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let by_path: Vec<FunnelEntry> = conn
        .prepare(&format!(
            "SELECT path, COUNT(*) AS views, COUNT(DISTINCT visitor_token) AS visitors \
             FROM telemetry_events \
             WHERE {m} AND event_type = 'pageview' AND path IS NOT NULL \
               AND {PUBLIC_PATH_EXPR} {win} \
             GROUP BY path ORDER BY views DESC LIMIT 20"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                let views: i64 = r.get(1)?;
                let visitors: i64 = r.get(2)?;
                let pct = if total_views > 0 {
                    100.0 * views as f64 / total_views as f64
                } else {
                    0.0
                };
                Ok(FunnelEntry {
                    path: r.get(0)?,
                    views,
                    visitors,
                    pct,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    FunnelResponse {
        as_of: chrono::Utc::now().to_rfc3339(),
        window_days: days,
        total_views,
        total_private_views,
        by_path,
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
    /// CO-378: marks this rollup as covering private-path traffic (excluded from default summary).
    #[serde(default)]
    pub private: bool,
}

/// Upsert idempotente keyed by (universe, day).
pub fn upsert_rollup(
    conn: &Connection,
    universe: &str,
    day: &str,
    metrics: &str,
    dims: &str,
    path_private: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO analytics_rollups (universe_key, day, metrics, dims, path_private, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(universe_key, day) DO UPDATE SET \
           metrics = excluded.metrics, dims = excluded.dims, \
           path_private = excluded.path_private, updated_at = excluded.updated_at",
        params![
            universe,
            day,
            metrics,
            dims,
            path_private as i64,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

/// True when the request carries a valid CO_ROLLUP_TOKEN bearer.
fn is_admin_authed(headers: &HeaderMap) -> bool {
    let Ok(expected) = std::env::var("CO_ROLLUP_TOKEN") else {
        return false;
    };
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    auth == format!("Bearer {expected}")
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
        if upsert_rollup(storage.conn(), &u, &body.day, &metrics, &dims, body.private).is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "db error"})),
            )
                .into_response();
        }
    }
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

/// GET /api/v1/analytics/public/summary?days=N&include_private=true
///
/// `days` clamped to [1, 365], default 30. Returns 400 for days=0.
/// `include_private=true` requires CO_ROLLUP_TOKEN bearer auth; silently ignored otherwise.
pub async fn summary_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
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
    let include_private = params.include_private.unwrap_or(false) && is_admin_authed(&headers);

    if include_private {
        crate::atividade::log_atividade(
            state.clone(),
            crate::atividade::Atividade {
                acao: crate::atividade::Acao::Ler,
                entidade: "analytics".to_string(),
                entidade_id: Some("private_path_viewed".to_string()),
                before: None,
                after: Some(serde_json::json!({
                    "event": "analytics.private_path_viewed",
                    "universe": universe,
                    "days": days,
                })),
                tipo: crate::atividade::Tipo::Sistema,
                user_id: None,
                ip: None,
                user_agent: None,
            },
        );
    }

    let key = (universe.clone(), days, include_private);

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
        query_universe_summary(storage.conn(), &universe, days, include_private)
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

/// GET /api/v1/analytics/public/funnel
///
/// Two modes:
///   - `?days=N` (CO-378): public path-funnel, no auth required.
///   - `?window=7d|30d|90d|custom` (CO-371): 8-step acquisition funnel, admin-only.
pub async fn funnel_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<FunnelParams>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if params.window.is_some() {
        // CO-371: 8-step acquisition funnel — admin-only
        let acq_params = crate::admin::funnel_routes::AcquisitionFunnelParams {
            window: params.window,
            start: params.start,
            end: params.end,
            breakdown: params.breakdown,
        };
        match crate::admin::funnel_routes::acquisition_funnel_handler(&state, &headers, &acq_params)
            .await
        {
            Ok(json) => json.into_response(),
            Err(err) => err,
        }
    } else {
        // CO-378: public path-based funnel
        let days = params.days.unwrap_or(30).clamp(1, 365);
        let universe = sanitize_universe(params.universe.as_deref().unwrap_or(UNIVERSE_KEY));
        let data = {
            let storage = state.core.storage.lock();
            query_funnel(storage.conn(), &universe, days)
        };
        Json(data).into_response()
    }
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
        .route("/funnel", get(funnel_handler))
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

    // --- CO-378: privacy helpers ---

    #[test]
    fn test_is_private_path_matches_drafts() {
        assert!(is_private_path("/blog/_drafts/my-post"));
        assert!(is_private_path("/content/_proposals/idea"));
        assert!(is_private_path("/app/_smoke/test"));
        assert!(is_private_path("/app/_t/token"));
        assert!(is_private_path("/app/_proof/check"));
        assert!(is_private_path("/probe/health"));
    }

    #[test]
    fn test_is_private_path_public_paths_not_matched() {
        assert!(!is_private_path("/about"));
        assert!(!is_private_path("/blog/my-post"));
        assert!(!is_private_path("/"));
        assert!(!is_private_path("/public/page"));
    }

    #[test]
    fn test_redact_path_deterministic() {
        let a = redact_path("/_drafts/secret");
        let b = redact_path("/_drafts/secret");
        assert_eq!(a, b, "same path must produce same redacted token");
        assert!(a.starts_with("<private-path-"), "must use expected prefix");
        assert_eq!(a.len(), "<private-path-".len() + 16 + 1, "hex16 + '>'");
    }

    #[test]
    fn test_redact_path_different_for_different_paths() {
        let a = redact_path("/_drafts/foo");
        let b = redact_path("/_drafts/bar");
        assert_ne!(a, b, "different paths must produce different tokens");
    }

    #[test]
    fn test_top_pages_filters_private_paths_by_default() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/public-page", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/_drafts/secret", 0);
        let m = "(universe_key = 'artelonga' OR path LIKE '/artelonga/%')";
        let win = "AND timestamp >= datetime('now', '-30 days')";
        let pages = build_top_pages(&conn, m, win, false);
        let public_paths: Vec<&str> = pages
            .iter()
            .filter(|p| p.private != Some(true))
            .map(|p| p.path.as_str())
            .collect();
        assert!(
            public_paths.contains(&"/public-page"),
            "public path must appear"
        );
        assert!(
            !public_paths.contains(&"/_drafts/secret"),
            "raw private path must not appear"
        );
        // private traffic aggregated as single "(private)" entry
        let private_cluster = pages
            .iter()
            .any(|p| p.path == "(private)" && p.private == Some(true));
        assert!(private_cluster, "must have a (private) cluster entry");
    }

    #[test]
    fn test_top_pages_include_private_shows_redacted_hashes() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/public-page", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/_drafts/secret", 0);
        let m = "(universe_key = 'artelonga' OR path LIKE '/artelonga/%')";
        let win = "AND timestamp >= datetime('now', '-30 days')";
        let pages = build_top_pages(&conn, m, win, true);
        let paths: Vec<&str> = pages.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"/public-page"), "public path must appear");
        // Raw private path must NOT appear; redacted hash must appear instead
        assert!(
            !paths.contains(&"/_drafts/secret"),
            "raw private path must not appear even in admin mode"
        );
        let has_redacted = pages
            .iter()
            .any(|p| p.path.starts_with("<private-path-") && p.private == Some(true));
        assert!(has_redacted, "redacted hash entry must appear for admin");
    }

    #[test]
    fn test_aggregate_counts_unaffected_by_privacy() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/public", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/_drafts/secret", 0);
        let s = query_public_summary(&conn, 30);
        // Total views = 2 (both public and private paths are counted in aggregates)
        assert_eq!(s.views, 2, "aggregate counts must include private paths");
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
        insert_pageview(&conn, "artelonga", "v1", "s1", "/", 0);
        insert_pageview(&conn, "artelonga", "v1", "s2", "/", -1);
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
                updated_at TEXT NOT NULL, path_private INTEGER DEFAULT 0,
                PRIMARY KEY (universe_key, day));",
        )
        .unwrap();
    }
    fn today() -> String {
        chrono::Utc::now().format("%Y-%m-%d").to_string()
    }

    #[test]
    fn test_universe_bridge_matches_historical_path() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/yuri/", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/yuri/resume", 0);
        insert_pageview(&conn, "artelonga", "v3", "s3", "/", 0);
        let s = query_universe_summary(&conn, "yuri", 7, false);
        assert_eq!(s.views, 2, "yuri bridges historical /yuri/* paths");
        let s_art = query_universe_summary(&conn, "artelonga", 7, false);
        assert_eq!(s_art.views, 3, "artelonga still counts all");
    }

    #[test]
    fn test_universe_matches_direct_universe_key() {
        let conn = create_test_db();
        insert_pageview(&conn, "yuri", "v1", "s1", "/", 0);
        let s = query_universe_summary(&conn, "yuri", 7, false);
        assert_eq!(s.views, 1);
    }

    #[test]
    fn test_rollup_overlay_extends_timeseries_and_totals() {
        let conn = create_test_db();
        create_rollups_table(&conn);
        insert_pageview(&conn, "artelonga", "v1", "s1", "/yuri/", -5);
        upsert_rollup(
            &conn,
            "yuri",
            &today(),
            r#"{"pageviews":10,"visitors":4,"sessions":6}"#,
            "{}",
            false,
        )
        .unwrap();
        let s = query_universe_summary(&conn, "yuri", 30, false);
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
        let conn = create_test_db();
        create_rollups_table(&conn);
        insert_pageview(&conn, "artelonga", "v1", "s1", "/yuri/", 0);
        upsert_rollup(&conn, "yuri", &today(), r#"{"pageviews":10}"#, "{}", false).unwrap();
        let s = query_universe_summary(&conn, "yuri", 30, false);
        assert_eq!(
            s.views, 10,
            "same-day event excluded; rollup wins; no double count"
        );
    }

    #[test]
    fn test_upsert_rollup_idempotent_latest_wins() {
        let conn = create_test_db();
        create_rollups_table(&conn);
        upsert_rollup(
            &conn,
            "yuri",
            "2026-06-01",
            r#"{"pageviews":5}"#,
            "{}",
            false,
        )
        .unwrap();
        upsert_rollup(
            &conn,
            "yuri",
            "2026-06-01",
            r#"{"pageviews":8}"#,
            "{}",
            false,
        )
        .unwrap();
        let r = query_rollups(&conn, "yuri", 3650);
        assert_eq!(r.len(), 1, "one row per (universe, day)");
        assert_eq!(r[0].1.pageviews, 8, "latest upsert wins");
    }

    #[test]
    fn test_private_rollup_excluded_from_default_rollups() {
        let conn = create_test_db();
        create_rollups_table(&conn);
        upsert_rollup(
            &conn,
            "yuri",
            "2026-06-01",
            r#"{"pageviews":5}"#,
            "{}",
            false,
        )
        .unwrap();
        upsert_rollup(
            &conn,
            "yuri",
            "2026-06-02",
            r#"{"pageviews":3}"#,
            "{}",
            true,
        )
        .unwrap();
        let public = query_rollups(&conn, "yuri", 3650);
        assert_eq!(
            public.len(),
            1,
            "private rollup excluded from query_rollups"
        );
        assert_eq!(public[0].1.pageviews, 5);
        let all = query_rollups_all(&conn, "yuri", 3650);
        assert_eq!(all.len(), 2, "query_rollups_all includes both");
    }

    #[test]
    fn test_funnel_private_paths_in_total_not_by_path() {
        let conn = create_test_db();
        insert_pageview(&conn, "artelonga", "v1", "s1", "/public", 0);
        insert_pageview(&conn, "artelonga", "v2", "s2", "/_drafts/hidden", 0);
        let f = query_funnel(&conn, "artelonga", 30);
        assert_eq!(f.total_views, 2, "total includes private paths");
        assert_eq!(f.total_private_views, 1, "private count is tracked");
        let by_path_paths: Vec<&str> = f.by_path.iter().map(|e| e.path.as_str()).collect();
        assert!(
            !by_path_paths.contains(&"/_drafts/hidden"),
            "private path must not appear in by_path"
        );
        assert!(
            by_path_paths.contains(&"/public"),
            "public path must appear in by_path"
        );
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
    async fn test_include_private_without_auth_ignored() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/summary?include_private=true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Without CO_ROLLUP_TOKEN, include_private=true must be silently ignored → 200 OK
        assert_eq!(
            resp.status(),
            AxumStatus::OK,
            "include_private without auth must return 200, not an error"
        );
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
    async fn test_funnel_returns_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/funnel")
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
        assert!(json["total_views"].is_number());
        assert!(json["total_private_views"].is_number());
        assert!(json["by_path"].is_array());
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
        assert_ne!(
            resp.status(),
            AxumStatus::METHOD_NOT_ALLOWED,
            "OPTIONS preflight must be accepted"
        );
    }
}
