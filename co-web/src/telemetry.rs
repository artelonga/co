//! Full-featured telemetry for CO — privacy-respecting event tracking.
//!
//! CO-46: https://github.com/artelonga/co/issues/46
//!
//! Tracks page views, interactions, errors, and performance without PII:
//! - No raw IP addresses (daily-salted hash only)
//! - No email addresses, no entry content
//! - Do Not Track respected in `telemetry.js` (client-side)
//! - Cookie consent gates client-side tracking
//! - Async batched inserts via `tokio::spawn` (< 5 ms impact per request)

use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, Response, StatusCode, header};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::server::AppState;

// ---------------------------------------------------------------------------
// IP hashing — daily-rotating salt prevents cross-day re-identification
// ---------------------------------------------------------------------------

/// Hash an IP address with a daily salt so the same IP gets a different hash
/// each day. Never stores the raw IP.
pub fn hash_ip_daily(ip: &str) -> String {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let salted = format!("{today}:{ip}");
    let hash = xxhash_rust::xxh3::xxh3_64(salted.as_bytes());
    format!("{hash:016x}")
}

// ---------------------------------------------------------------------------
// User-agent parsing
// ---------------------------------------------------------------------------

pub fn parse_ua_device(ua: &str) -> &'static str {
    let ua_lc = ua.to_lowercase();
    if ua_lc.contains("ipad") || ua_lc.contains("tablet") {
        "tablet"
    } else if ua_lc.contains("mobile") || ua_lc.contains("android") || ua_lc.contains("iphone") {
        "mobile"
    } else {
        "desktop"
    }
}

pub fn parse_ua_browser(ua: &str) -> &'static str {
    // Edge must be checked before Chrome (Edge UA contains "Chrome/")
    if ua.contains("Edg/") || ua.contains("Edge/") {
        "edge"
    } else if ua.contains("Firefox/") {
        "firefox"
    } else if ua.contains("Chrome/") {
        "chrome"
    } else if ua.contains("Safari/") {
        "safari"
    } else {
        "other"
    }
}

pub fn parse_ua_os(ua: &str) -> &'static str {
    // Android before Linux (Android UA contains "Linux")
    if ua.contains("Windows") {
        "windows"
    } else if ua.contains("Android") {
        "android"
    } else if ua.contains("iPhone") || ua.contains("iPad") {
        "ios"
    } else if ua.contains("Macintosh") || ua.contains("Mac OS") {
        "mac"
    } else if ua.contains("Linux") {
        "linux"
    } else {
        "other"
    }
}

// ---------------------------------------------------------------------------
// Bot detection
// ---------------------------------------------------------------------------

const BOT_UA_PATTERNS: &[&str] = &[
    "bot",
    "crawl",
    "spider",
    "curl",
    "wget",
    "python-requests",
    "go-http-client",
    "java/",
    "apache-httpclient",
    "okhttp",
    "postmanruntime",
    "insomnia",
    "axios",
    "node-fetch",
    "undici",
    "fly-healthcheck",
    "meta-externalagent",
    "ccbot",
    "amazonbot",
    "anthropic-ai",
    "cohere-ai",
    "gptbot",
    "claudebot",
    "headlesschrome",
    "phantomjs",
    "lighthouse",
];

pub fn is_bot(ua: &str) -> bool {
    let ua_lc = ua.to_lowercase();
    BOT_UA_PATTERNS.iter().any(|p| ua_lc.contains(p))
}

// ---------------------------------------------------------------------------
// Cookie / header helpers
// ---------------------------------------------------------------------------

fn get_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookies.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&format!("{name}=")) {
            return Some(v.to_string());
        }
    }
    None
}

/// CO-97 Option A: canonical visitor token — prefers `al_vid` (marketing apex
/// cookie) so both surfaces share one token; falls back to `visitante_id`.
fn get_visitor_token(headers: &HeaderMap) -> Option<String> {
    get_cookie(headers, "al_vid").or_else(|| get_cookie(headers, "visitante_id"))
}

fn extract_ip_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|xff| xff.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// CO-156: Universal CRUD telemetry envelope
// ---------------------------------------------------------------------------

/// A uniform server-side event emitted for every entry/asset/relation/WS/auth
/// state change.  Written to `telemetry_events` with `event_type = "crud"`.
pub struct CrudEvent {
    /// Event kind (e.g. "entry.upsert", "asset.upload", "ws.connect").
    pub kind: &'static str,
    /// Universe key (empty string for global events like auth.login).
    pub universe: String,
    /// Content type / directory the event targets (e.g. "reference", "refs").
    pub list: Option<String>,
    /// Entry path or asset sha256.
    pub key: Option<String>,
    /// Authenticated user ID; `None` for anonymous callers.
    pub actor: Option<String>,
    /// Stable session token derived from the JWT cookie or anon-cookie hash.
    pub session_id: Option<String>,
    /// Kind-specific extra data serialised into `properties`.
    pub extra: Option<serde_json::Value>,
}

/// Emit a [`CrudEvent`] to `telemetry_events` (fire-and-forget via `tokio::spawn`).
///
/// `deployment_version` is set from `CARGO_PKG_VERSION` at compile time.
pub fn emit_crud_event(state: &crate::server::AppState, ev: CrudEvent) {
    let state_clone = std::sync::Arc::clone(state);
    let timestamp_ns = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000);
    let timestamp = chrono::Utc::now().to_rfc3339();
    let props = serde_json::json!({
        "list": ev.list,
        "deployment_version": env!("CARGO_PKG_VERSION"),
        "timestamp_ns": timestamp_ns,
        "extra": ev.extra,
    });
    let row = EventRow {
        timestamp,
        visitor_token: None,
        user_id: ev.actor,
        session_id: ev.session_id,
        event_type: "crud".to_string(),
        event_name: ev.kind.to_string(),
        universe_key: if ev.universe.is_empty() {
            None
        } else {
            Some(ev.universe)
        },
        path: ev.key,
        properties: Some(props),
        duration_ms: None,
        ip_hash: None,
        ua_device: None,
        ua_browser: None,
        ua_os: None,
    };
    tokio::spawn(async move {
        let storage = state_clone.storage.lock();
        insert_event(storage.conn(), &row);
    });
}

/// Derive a stable session token from request headers without exposing the raw token.
///
/// Hashes the JWT session cookie (or anon visitante_id cookie) with xxh3.
pub fn extract_session_id(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(session) = get_cookie(headers, "co_session") {
        let hash = xxhash_rust::xxh3::xxh3_64(session.as_bytes());
        return Some(format!("ses_{hash:016x}"));
    }
    if let Some(vid) = get_cookie(headers, "visitante_id") {
        let hash = xxhash_rust::xxh3::xxh3_64(vid.as_bytes());
        return Some(format!("anon_{hash:016x}"));
    }
    None
}

// ---------------------------------------------------------------------------
// CO-156: CRUD event summary (24-hour window) for admin dashboard
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CrudEventSummary {
    pub total_crud_24h: i64,
    pub by_kind: Vec<TypeCount>,
}

pub fn crud_event_summary_24h(conn: &Connection) -> CrudEventSummary {
    let total_crud_24h: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM telemetry_events \
             WHERE event_type = 'crud' AND timestamp > datetime('now', '-24 hours')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let by_kind: Vec<TypeCount> = conn
        .prepare(
            "SELECT event_name, COUNT(*) AS cnt FROM telemetry_events \
             WHERE event_type = 'crud' AND timestamp > datetime('now', '-24 hours') \
             GROUP BY event_name ORDER BY cnt DESC",
        )
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(TypeCount {
                    event_type: r.get(0)?,
                    count: r.get(1)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    CrudEventSummary {
        total_crud_24h,
        by_kind,
    }
}

// ---------------------------------------------------------------------------
// Event row — written to telemetry_events
// ---------------------------------------------------------------------------

pub struct EventRow {
    pub timestamp: String,
    pub visitor_token: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub event_type: String,
    pub event_name: String,
    pub universe_key: Option<String>,
    pub path: Option<String>,
    pub properties: Option<serde_json::Value>,
    pub duration_ms: Option<i64>,
    pub ip_hash: Option<String>,
    pub ua_device: Option<String>,
    pub ua_browser: Option<String>,
    pub ua_os: Option<String>,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Insert a telemetry event. Ignores errors (telemetry is best-effort).
pub fn insert_event(conn: &Connection, ev: &EventRow) {
    let props = ev
        .properties
        .as_ref()
        .and_then(|p| serde_json::to_string(p).ok());
    let _ = conn.execute(
        "INSERT INTO telemetry_events \
         (timestamp, visitor_token, user_id, session_id, event_type, event_name, \
          universe_key, path, properties, duration_ms, ip_hash, ua_device, ua_browser, ua_os) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            ev.timestamp,
            ev.visitor_token,
            ev.user_id,
            ev.session_id,
            ev.event_type,
            ev.event_name,
            ev.universe_key,
            ev.path,
            props,
            ev.duration_ms,
            ev.ip_hash,
            ev.ua_device,
            ev.ua_browser,
            ev.ua_os,
        ],
    );
}

/// Delete events older than 90 days (retention policy).
pub fn cleanup_old_events(conn: &Connection) {
    let _ = conn.execute(
        "DELETE FROM telemetry_events WHERE timestamp < datetime('now', '-90 days')",
        [],
    );
}

// ---------------------------------------------------------------------------
// Summary aggregation
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct TelemetrySummary {
    pub total_events: i64,
    pub unique_visitors: i64,
    pub top_pages: Vec<PageCount>,
    pub error_count: i64,
    /// 95th-percentile server-side response time (pageviews, last 30 days).
    pub p95_latency_ms: Option<i64>,
    pub events_by_type: Vec<TypeCount>,
    /// Traffic over the last 30 days (one row per calendar day).
    pub events_by_day: Vec<DayCount>,
}

#[derive(Debug, Serialize)]
pub struct PageCount {
    pub path: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct TypeCount {
    pub event_type: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct DayCount {
    pub date: String,
    pub count: i64,
}

pub fn telemetry_summary(conn: &Connection) -> TelemetrySummary {
    let total_events: i64 = conn
        .query_row("SELECT COUNT(*) FROM telemetry_events", [], |r| r.get(0))
        .unwrap_or(0);

    let unique_visitors: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT visitor_token) FROM telemetry_events \
             WHERE visitor_token IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Top 10 pages by view count (last 30 days)
    let top_pages: Vec<PageCount> = conn
        .prepare(
            "SELECT path, COUNT(*) AS cnt FROM telemetry_events \
             WHERE event_type='pageview' AND path IS NOT NULL \
             AND timestamp > datetime('now','-30 days') \
             GROUP BY path ORDER BY cnt DESC LIMIT 10",
        )
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(PageCount {
                    path: r.get(0)?,
                    count: r.get(1)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    let error_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM telemetry_events WHERE event_type='error'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // p95 server latency (pageviews with duration_ms, last 30 days)
    let p95_latency_ms: Option<i64> = {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM telemetry_events \
                 WHERE event_type='pageview' AND duration_ms IS NOT NULL \
                 AND timestamp > datetime('now','-30 days')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if count > 0 {
            let offset = ((count as f64 * 0.95) as i64).max(0);
            conn.query_row(
                "SELECT duration_ms FROM telemetry_events \
                 WHERE event_type='pageview' AND duration_ms IS NOT NULL \
                 AND timestamp > datetime('now','-30 days') \
                 ORDER BY duration_ms LIMIT 1 OFFSET ?1",
                params![offset],
                |r| r.get(0),
            )
            .ok()
        } else {
            None
        }
    };

    let events_by_type: Vec<TypeCount> = conn
        .prepare(
            "SELECT event_type, COUNT(*) AS cnt FROM telemetry_events \
             GROUP BY event_type ORDER BY cnt DESC",
        )
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(TypeCount {
                    event_type: r.get(0)?,
                    count: r.get(1)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    // Daily counts, last 30 days
    let events_by_day: Vec<DayCount> = conn
        .prepare(
            "SELECT date(timestamp) AS day, COUNT(*) AS cnt FROM telemetry_events \
             WHERE timestamp > datetime('now','-30 days') \
             GROUP BY day ORDER BY day",
        )
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(DayCount {
                    date: r.get(0)?,
                    count: r.get(1)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    TelemetrySummary {
        total_events,
        unique_visitors,
        top_pages,
        error_count,
        p95_latency_ms,
        events_by_type,
        events_by_day,
    }
}

// ---------------------------------------------------------------------------
// Server-side middleware — records pageviews in telemetry_events
// ---------------------------------------------------------------------------

/// Telemetry middleware. Tracks GET page views in `telemetry_events`.
/// Filters bots, stores daily-salted IP hash, never stores raw IP or PII.
pub async fn telemetry_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response<Body> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // Skip API routes, static assets (anything with a file extension), WebSocket
    if path.starts_with("/api/")
        || path.starts_with("/_app/")
        || path.starts_with("/static/")
        || path.contains('.')
    {
        return next.run(req).await;
    }

    if method != axum::http::Method::GET {
        return next.run(req).await;
    }

    let ua = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if is_bot(&ua) {
        return next.run(req).await;
    }

    let ip = extract_ip_from_headers(req.headers());
    let ip_hash = hash_ip_daily(&ip);
    let visitor_token = get_visitor_token(req.headers());
    let session_id = get_cookie(req.headers(), "co_session");
    let referrer = req
        .headers()
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Extract universe slug from `/{slug}/...` path. After the v1.43 URL
    // refactor the platform is hosted at the root, so any non-reserved
    // top-level segment is treated as a universe key.
    const RESERVED_TOP: &[&str] = &[
        "api",
        "admin",
        "settings",
        "yggdrasil",
        "static",
        "health",
        "_app",
        "v1",
    ];
    let universe_key: Option<String> = path
        .split('/')
        .nth(1)
        .filter(|s| !s.is_empty() && !RESERVED_TOP.contains(s))
        .map(String::from);

    let start = Instant::now();
    let mut response = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as i64;

    let token = visitor_token.unwrap_or_else(|| nanoid::nanoid!(24));
    let props = referrer.map(|r| serde_json::json!({"referrer": r}));

    // CO-97: set apex-scoped al_vid so marketing site JS reads the same token.
    // HttpOnly intentionally omitted — analytics-only token, no auth role.
    // See docs/decisions/001-visitor-token-unification.md.
    if let Ok(cookie_val) = format!(
        "al_vid={token}; Domain=.artelonga.com.br; Path=/; SameSite=Lax; Secure; Max-Age=31536000"
    )
    .parse()
    {
        response
            .headers_mut()
            .append(header::SET_COOKIE, cookie_val);
    }
    // Snapshot universe_id for WAE before universe_key is moved into EventRow.
    let wae_universe_id = universe_key.as_deref().unwrap_or("global").to_string();

    let ev = EventRow {
        timestamp: chrono::Utc::now().to_rfc3339(),
        visitor_token: Some(token),
        user_id: None,
        session_id,
        event_type: "pageview".to_string(),
        event_name: "page.view".to_string(),
        universe_key,
        path: Some(path),
        properties: props,
        duration_ms: Some(duration_ms),
        ip_hash: Some(ip_hash),
        ua_device: Some(parse_ua_device(&ua).to_string()),
        ua_browser: Some(parse_ua_browser(&ua).to_string()),
        ua_os: Some(parse_ua_os(&ua).to_string()),
    };

    // OLTP write (primary store — CO-46)
    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        let storage = state_clone.storage.lock();
        insert_event(storage.conn(), &ev);
    });

    // CO-118: parallel WAE write (fire-and-forget, no-op when not configured)
    Arc::clone(&state.wae).emit(crate::wae::TelemetryEvent {
        event_type: "page_view".into(),
        universe_id: wae_universe_id,
        user_kind: "anon".into(),
        flag_key: None,
        variant: None,
        duration_ms: Some(duration_ms as u32),
        status: None,
    });

    response
}

// ---------------------------------------------------------------------------
// Client-side event endpoint — POST /api/v1/telemetry/event
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ClientEvent {
    pub event_name: String,
    #[serde(default = "default_interaction")]
    pub event_type: String,
    pub universe_key: Option<String>,
    pub path: Option<String>,
    pub properties: Option<serde_json::Value>,
    pub duration_ms: Option<i64>,
    pub session_id: Option<String>,
}

fn default_interaction() -> String {
    "interaction".to_string()
}

pub async fn track_event_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ClientEvent>,
) -> impl IntoResponse {
    if body.event_name.is_empty() || body.event_name.len() > 64 {
        return StatusCode::BAD_REQUEST;
    }

    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let visitor_token = get_visitor_token(&headers);
    let ip_hash = hash_ip_daily(&extract_ip_from_headers(&headers));

    let ev = EventRow {
        timestamp: chrono::Utc::now().to_rfc3339(),
        visitor_token,
        user_id: None,
        session_id: body.session_id,
        event_type: body.event_type,
        event_name: body.event_name,
        universe_key: body.universe_key,
        path: body.path,
        properties: body.properties,
        duration_ms: body.duration_ms,
        ip_hash: Some(ip_hash),
        ua_device: Some(parse_ua_device(&ua).to_string()),
        ua_browser: Some(parse_ua_browser(&ua).to_string()),
        ua_os: Some(parse_ua_os(&ua).to_string()),
    };

    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        let storage = state_clone.storage.lock();
        insert_event(storage.conn(), &ev);
    });

    StatusCode::ACCEPTED
}

// ---------------------------------------------------------------------------
// Marketing-schema event endpoint — POST /api/v1/telemetry/events (batched)
//
// Accepts the schema from artelonga/ArteLonga docs/analytics-api.md so the
// static marketing site can ship its batched events into this same table.
// Each marketing Event is translated to an EventRow; site-level fields not
// present in the column schema (utm, experiments, vw/vh, lang, ua_brand, tz,
// query, ref) are stashed into `properties`.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MarketingEvent {
    pub s: i64,
    pub site: String,
    pub name: String,
    pub sid: String,
    pub vid: String,
    pub ts: Option<i64>,
    pub tz: Option<String>,
    pub path: Option<String>,
    pub query: Option<String>,
    #[serde(rename = "ref")]
    pub referrer: Option<String>,
    pub vw: Option<i64>,
    pub vh: Option<i64>,
    pub lang: Option<String>,
    pub ua_brand: Option<String>,
    pub utm: Option<serde_json::Value>,
    pub experiments: Option<serde_json::Value>,
    pub props: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct MarketingBatch {
    pub schema: i64,
    pub batch: Vec<MarketingEvent>,
}

const MARKETING_MAX_BATCH: usize = 200;

pub async fn marketing_events_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MarketingBatch>,
) -> impl IntoResponse {
    if body.schema != 1 {
        return StatusCode::BAD_REQUEST;
    }
    if body.batch.is_empty() || body.batch.len() > MARKETING_MAX_BATCH {
        return StatusCode::BAD_REQUEST;
    }

    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if is_bot(&ua) {
        return StatusCode::NO_CONTENT;
    }

    let ip_hash = hash_ip_daily(&extract_ip_from_headers(&headers));
    let ua_device = parse_ua_device(&ua).to_string();
    let ua_browser = parse_ua_browser(&ua).to_string();
    let ua_os = parse_ua_os(&ua).to_string();

    let mut rows: Vec<EventRow> = Vec::with_capacity(body.batch.len());
    for ev in body.batch {
        if ev.name.is_empty() || ev.name.len() > 64 {
            continue;
        }

        let mut props_obj = match ev.props {
            Some(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        props_obj.insert("site".into(), serde_json::Value::String(ev.site.clone()));
        if let Some(v) = ev.tz {
            props_obj.insert("tz".into(), serde_json::Value::String(v));
        }
        if let Some(v) = ev.query {
            props_obj.insert("query".into(), serde_json::Value::String(v));
        }
        if let Some(v) = ev.referrer {
            props_obj.insert("ref".into(), serde_json::Value::String(v));
        }
        if let Some(v) = ev.vw {
            props_obj.insert("vw".into(), serde_json::Value::from(v));
        }
        if let Some(v) = ev.vh {
            props_obj.insert("vh".into(), serde_json::Value::from(v));
        }
        if let Some(v) = ev.lang {
            props_obj.insert("lang".into(), serde_json::Value::String(v));
        }
        if let Some(v) = ev.ua_brand {
            props_obj.insert("ua_brand".into(), serde_json::Value::String(v));
        }
        if let Some(v) = ev.utm {
            props_obj.insert("utm".into(), v);
        }
        if let Some(v) = ev.experiments {
            props_obj.insert("experiments".into(), v);
        }

        let timestamp = ev
            .ts
            .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();

        rows.push(EventRow {
            timestamp,
            visitor_token: Some(ev.vid),
            user_id: None,
            session_id: Some(ev.sid),
            event_type: derive_event_type_from_marketing(&ev.name),
            event_name: ev.name,
            universe_key: {
                let s = ev.site.trim().to_string();
                if !s.is_empty() && s.len() <= 64 {
                    Some(s)
                } else {
                    None
                }
            },
            path: ev.path,
            properties: Some(serde_json::Value::Object(props_obj)),
            duration_ms: None,
            ip_hash: Some(ip_hash.clone()),
            ua_device: Some(ua_device.clone()),
            ua_browser: Some(ua_browser.clone()),
            ua_os: Some(ua_os.clone()),
        });
    }

    if rows.is_empty() {
        return StatusCode::NO_CONTENT;
    }

    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        let storage = state_clone.storage.lock();
        for row in &rows {
            insert_event(storage.conn(), row);
        }
    });

    StatusCode::NO_CONTENT
}

fn derive_event_type_from_marketing(name: &str) -> String {
    if name == "page_view" || name == "page_end" || name.starts_with("page_") {
        "pageview".to_string()
    } else if name == "js_error" || name == "js_promise_rejection" {
        "error".to_string()
    } else if name.starts_with("perf_") {
        "performance".to_string()
    } else if name == "experiment_exposure" {
        "experiment".to_string()
    } else {
        "interaction".to_string()
    }
}

// ---------------------------------------------------------------------------
// Admin endpoints — require GitHub admin auth via extension layers in server.rs
// ---------------------------------------------------------------------------

/// GET /api/v1/admin/telemetry/summary — aggregated analytics dashboard data.
pub async fn summary_handler(State(state): State<AppState>) -> impl IntoResponse {
    let storage = state.storage.lock();
    Json(telemetry_summary(storage.conn())).into_response()
}

/// GET /api/v1/admin/telemetry/export — last 10 000 events as CSV.
pub async fn export_handler(State(state): State<AppState>) -> impl IntoResponse {
    let storage = state.storage.lock();
    let csv = build_csv(storage.conn());
    drop(storage);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"telemetry.csv\"",
            ),
        ],
        csv,
    )
        .into_response()
}

fn build_csv(conn: &Connection) -> String {
    let mut out = String::from(
        "timestamp,event_type,event_name,path,universe_key,duration_ms,device,browser,os\n",
    );
    let Ok(mut stmt) = conn.prepare(
        "SELECT timestamp, event_type, event_name, \
         COALESCE(path,''), COALESCE(universe_key,''), \
         COALESCE(CAST(duration_ms AS TEXT),''), \
         COALESCE(ua_device,''), COALESCE(ua_browser,''), COALESCE(ua_os,'') \
         FROM telemetry_events ORDER BY timestamp DESC LIMIT 10000",
    ) else {
        return out;
    };
    let _ = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            ))
        })
        .map(|rows| {
            for row in rows.flatten() {
                out.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{}\n",
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8
                ));
            }
        });
    out
}

// ---------------------------------------------------------------------------
// Admin dashboard — GET /co/co-dev/telemetria
// ---------------------------------------------------------------------------

pub async fn serve_admin_dashboard() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        ADMIN_DASHBOARD_HTML,
    )
}

const ADMIN_DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="pt-BR">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>CO — Telemetria</title>
<style>
  :root { --bg:#0f0f1a; --card:#1a1a2e; --border:#2a2a4a; --text:#e8e8f0; --accent:#7c6fcd; --muted:#888; }
  * { box-sizing:border-box; margin:0; padding:0; }
  body { background:var(--bg); color:var(--text); font:14px/1.6 system-ui,sans-serif; padding:24px; }
  h1 { font-size:1.5rem; margin-bottom:24px; color:var(--accent); }
  h2 { font-size:1rem; margin-bottom:12px; color:var(--muted); text-transform:uppercase; letter-spacing:.1em; }
  .auth { display:flex; gap:8px; margin-bottom:24px; }
  .auth input { flex:1; background:var(--card); border:1px solid var(--border); color:var(--text); padding:8px 12px; border-radius:6px; font-size:13px; }
  .btn { background:var(--accent); color:#fff; border:none; padding:8px 16px; border-radius:6px; cursor:pointer; font-size:13px; }
  .btn:hover { opacity:.85; }
  .btn-outline { background:transparent; border:1px solid var(--border); color:var(--text); }
  .cards { display:grid; grid-template-columns:repeat(auto-fill,minmax(160px,1fr)); gap:12px; margin-bottom:24px; }
  .card { background:var(--card); border:1px solid var(--border); border-radius:8px; padding:16px; }
  .card-val { font-size:2rem; font-weight:700; color:var(--accent); }
  .card-lbl { font-size:11px; color:var(--muted); margin-top:4px; }
  .section { background:var(--card); border:1px solid var(--border); border-radius:8px; padding:16px; margin-bottom:16px; }
  table { width:100%; border-collapse:collapse; font-size:13px; }
  th { text-align:left; padding:6px 8px; color:var(--muted); border-bottom:1px solid var(--border); }
  td { padding:6px 8px; border-bottom:1px solid var(--border); }
  tr:last-child td { border:none; }
  .bar { height:8px; background:var(--accent); border-radius:4px; opacity:.7; }
  .err { color:#e57373; padding:12px; background:rgba(229,115,115,.1); border-radius:6px; margin-bottom:16px; }
  #main { display:none; }
</style>
</head>
<body>
<h1>CO — Telemetria</h1>

<div class="auth">
  <input id="token" type="password" placeholder="GitHub PAT (admin access)">
  <button class="btn" onclick="loadData()">Carregar</button>
  <button class="btn btn-outline" id="btn-export" onclick="exportCSV()" style="display:none">Exportar CSV</button>
</div>

<div id="error" class="err" style="display:none"></div>

<div id="main">
  <div class="cards">
    <div class="card"><div class="card-val" id="total-events">—</div><div class="card-lbl">Total de eventos</div></div>
    <div class="card"><div class="card-val" id="unique-visitors">—</div><div class="card-lbl">Visitantes únicos</div></div>
    <div class="card"><div class="card-val" id="error-count">—</div><div class="card-lbl">Erros registrados</div></div>
    <div class="card"><div class="card-val" id="p95-latency">—</div><div class="card-lbl">Latência p95 (ms)</div></div>
  </div>

  <div class="section">
    <h2>Páginas mais visitadas (30 dias)</h2>
    <table id="top-pages"></table>
  </div>

  <div class="section">
    <h2>Eventos por tipo</h2>
    <table id="events-by-type"></table>
  </div>

  <div class="section">
    <h2>Tráfego — últimos 30 dias</h2>
    <div id="traffic-chart" style="display:flex;align-items:flex-end;gap:3px;height:80px;overflow:hidden"></div>
    <table id="events-by-day" style="margin-top:12px"></table>
  </div>

  <div class="section">
    <h2>CRUD Events — últimas 24 horas</h2>
    <div id="crud-total" style="font-size:1.5rem;font-weight:700;color:var(--accent);margin-bottom:12px">—</div>
    <table id="crud-by-kind"></table>
  </div>
</div>

<script>
let githubToken = '';

async function loadData() {
  githubToken = document.getElementById('token').value.trim();
  if (!githubToken) { showError('Insira o GitHub PAT.'); return; }
  clearError();
  try {
    const res = await fetch('/api/v1/admin/telemetry/summary', {
      headers: { 'Authorization': 'Bearer ' + githubToken }
    });
    if (!res.ok) { showError('Erro ' + res.status + ': ' + await res.text()); return; }
    const d = await res.json();
    render(d);
    document.getElementById('main').style.display = '';
    document.getElementById('btn-export').style.display = '';
    await loadCrudData();
  } catch(e) { showError(e.message); }
}

function render(d) {
  document.getElementById('total-events').textContent = fmt(d.total_events);
  document.getElementById('unique-visitors').textContent = fmt(d.unique_visitors);
  document.getElementById('error-count').textContent = fmt(d.error_count);
  document.getElementById('p95-latency').textContent = d.p95_latency_ms != null ? d.p95_latency_ms : '—';

  // Top pages
  const tp = document.getElementById('top-pages');
  const maxPg = Math.max(...(d.top_pages.map(p => p.count)), 1);
  tp.innerHTML = '<tr><th>Página</th><th>Visitas</th><th style="width:120px"></th></tr>' +
    d.top_pages.map(p =>
      `<tr><td>${esc(p.path)}</td><td>${fmt(p.count)}</td><td><div class="bar" style="width:${Math.round(p.count/maxPg*100)}%"></div></td></tr>`
    ).join('');

  // Events by type
  const et = document.getElementById('events-by-type');
  const maxEt = Math.max(...(d.events_by_type.map(t => t.count)), 1);
  et.innerHTML = '<tr><th>Tipo</th><th>Contagem</th><th style="width:120px"></th></tr>' +
    d.events_by_type.map(t =>
      `<tr><td>${esc(t.event_type)}</td><td>${fmt(t.count)}</td><td><div class="bar" style="width:${Math.round(t.count/maxEt*100)}%"></div></td></tr>`
    ).join('');

  // Traffic chart (bar per day)
  const chart = document.getElementById('traffic-chart');
  const maxDay = Math.max(...(d.events_by_day.map(x => x.count)), 1);
  chart.innerHTML = d.events_by_day.map(x => {
    const h = Math.max(4, Math.round(x.count / maxDay * 80));
    return `<div title="${x.date}: ${x.count}" style="flex:1;height:${h}px;background:var(--accent);opacity:.6;border-radius:2px 2px 0 0;min-width:4px"></div>`;
  }).join('');

  // Events by day table
  const ed = document.getElementById('events-by-day');
  ed.innerHTML = '<tr><th>Data</th><th>Eventos</th></tr>' +
    d.events_by_day.slice().reverse().slice(0,7).map(x =>
      `<tr><td>${esc(x.date)}</td><td>${fmt(x.count)}</td></tr>`
    ).join('');
}

async function loadCrudData() {
  try {
    const res = await fetch('/api/v1/admin/telemetry/crud-summary', {
      headers: { 'Authorization': 'Bearer ' + githubToken }
    });
    if (!res.ok) return;
    const d = await res.json();
    document.getElementById('crud-total').textContent = fmt(d.total_crud_24h) + ' eventos CRUD';
    const maxKind = Math.max(...(d.by_kind.map(k => k.count)), 1);
    document.getElementById('crud-by-kind').innerHTML =
      '<tr><th>Tipo</th><th>24h</th><th style="width:120px"></th></tr>' +
      d.by_kind.map(k =>
        `<tr><td>${esc(k.event_type)}</td><td>${fmt(k.count)}</td><td><div class="bar" style="width:${Math.round(k.count/maxKind*100)}%"></div></td></tr>`
      ).join('');
  } catch(_) {}
}

async function exportCSV() {
  const res = await fetch('/api/v1/admin/telemetry/export', {
    headers: { 'Authorization': 'Bearer ' + githubToken }
  });
  if (!res.ok) { showError('Export falhou: ' + res.status); return; }
  const blob = await res.blob();
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url; a.download = 'telemetry.csv'; a.click();
  URL.revokeObjectURL(url);
}

function fmt(n) { return n != null ? n.toLocaleString('pt-BR') : '—'; }
function esc(s) { return (s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'); }
function showError(msg) { const e = document.getElementById('error'); e.textContent = msg; e.style.display = ''; }
function clearError() { document.getElementById('error').style.display = 'none'; }
</script>
</body>
</html>
"#;

// ---------------------------------------------------------------------------
// Routers
// ---------------------------------------------------------------------------

/// Public telemetry endpoints — accept client-side events.
///
/// `POST /event`  — Co's internal client (single-event shape, /api/v1/telemetry/event).
/// `POST /events` — marketing batch shape (artelonga.com.br's analytics.js).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/event", post(track_event_handler))
        .route("/events", post(marketing_events_handler))
}

/// GET /api/v1/admin/telemetry/crud-summary — CRUD events in last 24 hours.
pub async fn crud_summary_handler(State(state): State<AppState>) -> impl IntoResponse {
    let storage = state.storage.lock();
    Json(crud_event_summary_24h(storage.conn())).into_response()
}

/// Admin-only telemetry endpoints.
/// Requires GitHub admin auth extensions to be layered in from server.rs.
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/telemetry/summary", get(summary_handler))
        .route("/telemetry/export", get(export_handler))
        .route("/telemetry/crud-summary", get(crud_summary_handler))
        .layer(axum::middleware::from_fn(
            crate::github_auth::require_github_admin,
        ))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ua_device() {
        assert_eq!(
            parse_ua_device("Mozilla/5.0 (iPhone; CPU iPhone OS 15_0)"),
            "mobile"
        );
        assert_eq!(
            parse_ua_device("Mozilla/5.0 (iPad; CPU OS 14_0) AppleWebKit"),
            "tablet"
        );
        assert_eq!(
            parse_ua_device("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"),
            "desktop"
        );
        assert_eq!(
            parse_ua_device("Mozilla/5.0 (Linux; Android 11; Pixel 5)"),
            "mobile"
        );
    }

    #[test]
    fn test_parse_ua_browser() {
        assert_eq!(
            parse_ua_browser("Mozilla/5.0 (Windows NT 10.0) Chrome/100.0.0 Safari/537.36"),
            "chrome"
        );
        assert_eq!(parse_ua_browser("Mozilla/5.0 Firefox/95.0"), "firefox");
        assert_eq!(
            parse_ua_browser("Mozilla/5.0 (Macintosh) AppleWebKit Safari/605.1"),
            "safari"
        );
        // Edge contains "Chrome/" but must be detected first
        assert_eq!(
            parse_ua_browser("Mozilla/5.0 Chrome/100.0 Safari/537.36 Edg/100.0.0"),
            "edge"
        );
    }

    #[test]
    fn test_parse_ua_os() {
        assert_eq!(
            parse_ua_os("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"),
            "windows"
        );
        assert_eq!(
            parse_ua_os("Mozilla/5.0 (Macintosh; Intel Mac OS X 12_3)"),
            "mac"
        );
        assert_eq!(
            parse_ua_os("Mozilla/5.0 (Linux; Android 11; Pixel 5)"),
            "android"
        );
        assert_eq!(
            parse_ua_os("Mozilla/5.0 (iPhone; CPU iPhone OS 15_0)"),
            "ios"
        );
        assert_eq!(
            parse_ua_os("Mozilla/5.0 (X11; Ubuntu; Linux x86_64)"),
            "linux"
        );
    }

    #[test]
    fn test_is_bot() {
        assert!(is_bot(
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"
        ));
        assert!(is_bot("curl/7.68.0"));
        assert!(is_bot("GPTBot/1.0"));
        assert!(is_bot("anthropic-ai/1.0"));
        assert!(!is_bot(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
        ));
    }

    #[test]
    fn test_hash_ip_daily_no_raw_ip() {
        let hash = hash_ip_daily("192.168.1.1");
        assert_eq!(hash.len(), 16, "hash is 16 hex chars");
        assert!(!hash.contains("192"), "raw IP not in hash");
        // Deterministic within the same day
        assert_eq!(hash, hash_ip_daily("192.168.1.1"));
        // Different IPs → different hashes
        assert_ne!(hash, hash_ip_daily("10.0.0.1"));
    }

    #[test]
    fn test_insert_and_summary() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        create_telemetry_table(&conn);

        insert_event(
            &conn,
            &EventRow {
                timestamp: "2026-04-10T12:00:00Z".to_string(),
                visitor_token: Some("visitor-a".to_string()),
                user_id: None,
                session_id: None,
                event_type: "pageview".to_string(),
                event_name: "page.view".to_string(),
                universe_key: None,
                path: Some("/".to_string()),
                properties: None,
                duration_ms: Some(42),
                ip_hash: Some("abc123".to_string()),
                ua_device: Some("desktop".to_string()),
                ua_browser: Some("chrome".to_string()),
                ua_os: Some("mac".to_string()),
            },
        );

        insert_event(
            &conn,
            &EventRow {
                timestamp: "2026-04-10T13:00:00Z".to_string(),
                visitor_token: Some("visitor-b".to_string()),
                user_id: None,
                session_id: None,
                event_type: "error".to_string(),
                event_name: "js.error".to_string(),
                universe_key: None,
                path: Some("/".to_string()),
                properties: Some(serde_json::json!({"message": "TypeError"})),
                duration_ms: None,
                ip_hash: Some("def456".to_string()),
                ua_device: Some("mobile".to_string()),
                ua_browser: Some("safari".to_string()),
                ua_os: Some("ios".to_string()),
            },
        );

        let summary = telemetry_summary(&conn);
        assert_eq!(summary.total_events, 2);
        assert_eq!(summary.unique_visitors, 2);
        assert_eq!(summary.error_count, 1);
    }

    #[test]
    fn test_cleanup_old_events() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        create_telemetry_table(&conn);

        // Insert one old event (91 days ago) and one recent
        conn.execute_batch(
            "INSERT INTO telemetry_events (timestamp, event_type, event_name) \
             VALUES (datetime('now','-91 days'), 'pageview', 'page.view');
             INSERT INTO telemetry_events (timestamp, event_type, event_name) \
             VALUES (datetime('now','-1 days'), 'pageview', 'page.view');",
        )
        .unwrap();

        cleanup_old_events(&conn);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM telemetry_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "old event deleted, recent event kept");
    }

    /// Helper: create the telemetry table for in-memory tests.
    fn create_telemetry_table(conn: &Connection) {
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
    }
}
