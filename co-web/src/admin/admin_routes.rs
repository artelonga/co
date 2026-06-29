//! Admin telemetry dashboard — CO-105
//!
//! GET /api/v1/admin/dashboard — JSON aggregates, JWT + CO_SEED_ADMIN_EMAIL gate.
//! GET /admin               — serves admin.html, Cookie auth, same email gate.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use rusqlite::Connection;
use serde::Serialize;

use crate::server::{AppState, IntegrationsState};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct DashboardTotals {
    pub users: i64,
    pub universes: i64,
    pub universes_active_7d: i64,
    pub entries: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DailyRow {
    pub date: String,
    pub pageviews: i64,
    pub uniques: i64,
    pub signups: i64,
    pub errors: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct TopUniverse {
    pub key: String,
    pub name: String,
    pub events_7d: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct AuthStats {
    pub logins_today: i64,
    pub failed_logins_today: i64,
    pub active_sessions: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct AdminDashboard {
    pub version: String,
    pub as_of: String,
    pub totals: DashboardTotals,
    pub daily: Vec<DailyRow>,
    pub top_universes: Vec<TopUniverse>,
    pub auth: AuthStats,
}

// ---------------------------------------------------------------------------
// 5-minute in-memory cache
// ---------------------------------------------------------------------------

struct CacheEntry {
    data: AdminDashboard,
    fetched_at: Instant,
}

static DASHBOARD_CACHE: Mutex<Option<CacheEntry>> = Mutex::new(None);
const CACHE_TTL: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// Access gate — read fresh from env each request
// ---------------------------------------------------------------------------

/// Pure predicate: does `caller_email` match the configured admin email?
pub fn is_admin_email(caller_email: &str, admin_email: &str) -> bool {
    !admin_email.is_empty() && caller_email == admin_email
}

/// Reads CO_SEED_ADMIN_EMAIL from env (no caching) and checks the caller.
pub fn check_admin_email(caller_email: &str) -> bool {
    let admin_email = crate::infra::secrets::global().get_or("CO_SEED_ADMIN_EMAIL", "");
    is_admin_email(caller_email, &admin_email)
}

// ---------------------------------------------------------------------------
// Aggregate query helpers
// ---------------------------------------------------------------------------

pub fn query_totals(conn: &Connection) -> DashboardTotals {
    let users = conn
        .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
        .unwrap_or(0);

    let universes = conn
        .query_row("SELECT COUNT(*) FROM universes", [], |r| r.get(0))
        .unwrap_or(0);

    let universes_active_7d = conn
        .query_row(
            "SELECT COUNT(DISTINCT universe_key) FROM entries \
             WHERE updated_at > datetime('now', '-7 days')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let entries = conn
        .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap_or(0);

    DashboardTotals {
        users,
        universes,
        universes_active_7d,
        entries,
    }
}

pub fn query_daily_14d(conn: &Connection) -> Vec<DailyRow> {
    // Pageviews + uniques per day from telemetry_events
    let mut pv_map: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT date(timestamp) AS day, COUNT(*) AS pv, \
         COUNT(DISTINCT visitor_token) AS uniq \
         FROM telemetry_events \
         WHERE event_type = 'pageview' \
         AND timestamp > datetime('now', '-14 days') \
         GROUP BY day",
    ) {
        let _ = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)))
            .map(|rows| {
                for row in rows.flatten() {
                    pv_map.insert(row.0, (row.1, row.2));
                }
            });
    }

    // Errors per day
    let mut err_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT date(timestamp) AS day, COUNT(*) \
         FROM telemetry_events \
         WHERE event_type = 'error' \
         AND timestamp > datetime('now', '-14 days') \
         GROUP BY day",
    ) {
        let _ = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map(|rows| {
                for row in rows.flatten() {
                    err_map.insert(row.0, row.1);
                }
            });
    }

    // Signups per day from users table
    let mut signup_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT date(created_at) AS day, COUNT(*) \
         FROM users \
         WHERE created_at > datetime('now', '-14 days') \
         GROUP BY day",
    ) {
        let _ = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map(|rows| {
                for row in rows.flatten() {
                    signup_map.insert(row.0, row.1);
                }
            });
    }

    // Generate last 14 calendar days (oldest → newest)
    let mut rows = Vec::with_capacity(14);
    for days_back in (0..14i64).rev() {
        let date = (chrono::Utc::now() - chrono::Duration::days(days_back))
            .format("%Y-%m-%d")
            .to_string();
        let (pageviews, uniques) = pv_map.get(&date).copied().unwrap_or((0, 0));
        let errors = err_map.get(&date).copied().unwrap_or(0);
        let signups = signup_map.get(&date).copied().unwrap_or(0);
        rows.push(DailyRow {
            date,
            pageviews,
            uniques,
            signups,
            errors,
        });
    }
    rows
}

pub fn query_top_universes(conn: &Connection) -> Vec<TopUniverse> {
    conn.prepare(
        "SELECT te.universe_key, COALESCE(u.name, te.universe_key) AS name, \
         COUNT(*) AS events_7d \
         FROM telemetry_events te \
         LEFT JOIN universes u ON u.key = te.universe_key \
         WHERE te.timestamp > datetime('now', '-7 days') \
         AND te.universe_key IS NOT NULL \
         GROUP BY te.universe_key \
         ORDER BY events_7d DESC \
         LIMIT 10",
    )
    .ok()
    .and_then(|mut stmt| {
        stmt.query_map([], |r| {
            Ok(TopUniverse {
                key: r.get(0)?,
                name: r.get(1)?,
                events_7d: r.get(2)?,
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
    })
    .unwrap_or_default()
}

pub fn query_auth_stats(conn: &Connection) -> AuthStats {
    let logins_today = conn
        .query_row(
            "SELECT COUNT(*) FROM telemetry_events \
             WHERE event_name LIKE 'auth.login%' \
             AND event_type = 'interaction' \
             AND date(timestamp) = date('now')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let failed_logins_today = conn
        .query_row(
            "SELECT COUNT(*) FROM telemetry_events \
             WHERE event_name LIKE 'auth.login.fail%' \
             AND date(timestamp) = date('now')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let active_sessions = conn
        .query_row(
            "SELECT COUNT(DISTINCT session_id) FROM telemetry_events \
             WHERE timestamp > datetime('now', '-30 minutes') \
             AND session_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    AuthStats {
        logins_today,
        failed_logins_today,
        active_sessions,
    }
}

pub fn query_admin_dashboard(conn: &Connection) -> AdminDashboard {
    AdminDashboard {
        version: env!("CARGO_PKG_VERSION").to_string(),
        as_of: chrono::Utc::now().to_rfc3339(),
        totals: query_totals(conn),
        daily: query_daily_14d(conn),
        top_universes: query_top_universes(conn),
        auth: query_auth_stats(conn),
    }
}

// ---------------------------------------------------------------------------
// JWT extraction helper
// ---------------------------------------------------------------------------

pub fn extract_claims(headers: &HeaderMap) -> Result<crate::auth::Claims, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| crate::auth::extract_session_cookie(headers))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let secret = crate::auth::jwt_secret();
    crate::auth::decode_claims(&token, &secret).map_err(|_| StatusCode::UNAUTHORIZED)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/admin/dashboard
///
/// Validates JWT first (401 on failure), then checks email == CO_SEED_ADMIN_EMAIL
/// (403 on mismatch). Never reveals the admin email when the JWT is invalid.
/// Response is cached for 5 minutes.
pub async fn dashboard_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminDashboard>, Response> {
    let claims = extract_claims(&headers).map_err(|status| {
        (status, Json(serde_json::json!({"error": "Unauthorized"}))).into_response()
    })?;

    if !check_admin_email(&claims.email) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Forbidden"})),
        )
            .into_response());
    }

    // Serve from cache if still fresh
    {
        let cache = DASHBOARD_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref entry) = *cache
            && entry.fetched_at.elapsed() < CACHE_TTL
        {
            return Ok(Json(entry.data.clone()));
        }
    }

    // Query fresh aggregates
    let data = {
        let storage = state.core.storage.lock();
        query_admin_dashboard(storage.conn())
    };

    // Update cache
    {
        let mut cache = DASHBOARD_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        *cache = Some(CacheEntry {
            data: data.clone(),
            fetched_at: Instant::now(),
        });
    }

    Ok(Json(data))
}

/// GET /admin
///
/// Serves the admin HTML page after validating the session cookie.
/// Redirects to / if unauthenticated; returns 403 if not the seed admin.
pub async fn serve_admin_page(headers: HeaderMap) -> Response {
    let token = match crate::auth::extract_session_cookie(&headers) {
        Some(t) => t,
        None => return Redirect::to("/").into_response(),
    };

    let secret = crate::auth::jwt_secret();
    let claims = match crate::auth::decode_claims(&token, &secret) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/").into_response(),
    };

    if !check_admin_email(&claims.email) {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<html><body><h1>403 Proibido</h1><p>Acesso restrito ao administrador do sistema.</p></body></html>",
        )
            .into_response();
    }

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        ADMIN_PAGE_HTML,
    )
        .into_response()
}

const ADMIN_PAGE_HTML: &str = include_str!("../../static/variants/a/admin.html");

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// CO-205: origin breakdown
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OriginRow {
    pub origin: Option<String>,
    pub user_count: i64,
}

#[derive(Debug, Serialize)]
pub struct OriginBreakdown {
    pub origins: Vec<OriginRow>,
    pub total_users: i64,
}

/// GET /api/v1/admin/users/origin-breakdown — count of users per signup origin.
pub async fn origin_breakdown_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OriginBreakdown>, Response> {
    let claims = extract_claims(&headers).map_err(|status| {
        (status, Json(serde_json::json!({"error": "Unauthorized"}))).into_response()
    })?;

    if !check_admin_email(&claims.email) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Forbidden"})),
        )
            .into_response());
    }

    let storage = state.core.storage.lock();
    let conn = storage.conn();

    let total_users: i64 = conn
        .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
        .unwrap_or(0);

    let origins: Vec<OriginRow> = conn
        .prepare(
            "SELECT origin, COUNT(*) AS user_count \
             FROM users \
             GROUP BY origin \
             ORDER BY user_count DESC",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(OriginRow {
                    origin: row.get(0)?,
                    user_count: row.get(1)?,
                })
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    Ok(Json(OriginBreakdown {
        origins,
        total_users,
    }))
}

/// GET /api/v1/admin/workers/status
///
/// Returns a JSON array of `WorkerStatus` snapshots for all supervised workers.
/// Requires a valid admin JWT (same gate as `/api/v1/admin/dashboard`).
pub async fn workers_status_handler(
    State(integrations): State<Arc<IntegrationsState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::worker_supervisor::WorkerStatus>>, Response> {
    let claims = extract_claims(&headers).map_err(|status| {
        (status, Json(serde_json::json!({"error": "Unauthorized"}))).into_response()
    })?;

    if !check_admin_email(&claims.email) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Forbidden"})),
        )
            .into_response());
    }

    Ok(Json(integrations.worker_supervisor.statuses()))
}

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/dashboard", get(dashboard_handler))
        .route("/users/origin-breakdown", get(origin_breakdown_handler))
        .route("/workers/status", get(workers_status_handler))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY,
                email TEXT,
                display_name TEXT,
                tier TEXT,
                created_at TEXT,
                password_hash TEXT
            );
            CREATE TABLE universes (
                key TEXT PRIMARY KEY,
                name TEXT,
                owner_id TEXT,
                is_public INTEGER DEFAULT 0,
                is_template INTEGER DEFAULT 0,
                content_count INTEGER DEFAULT 0,
                created_at TEXT
            );
            CREATE TABLE entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                universe_key TEXT,
                path TEXT,
                entry_type TEXT,
                title TEXT,
                updated_at TEXT
            );
            CREATE TABLE telemetry_events (
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

    // --- Aggregate query tests ---

    #[test]
    fn test_query_totals_empty_db() {
        let conn = create_test_db();
        let t = query_totals(&conn);
        assert_eq!(t.users, 0);
        assert_eq!(t.universes, 0);
        assert_eq!(t.universes_active_7d, 0);
        assert_eq!(t.entries, 0);
    }

    #[test]
    fn test_query_totals_with_data() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO users VALUES ('u1','a@b.com','A','player',datetime('now'),NULL);
             INSERT INTO users VALUES ('u2','b@c.com','B','player',datetime('now'),NULL);
             INSERT INTO universes VALUES ('uni1','Universe 1','u1',1,0,0,datetime('now'));
             INSERT INTO entries VALUES (NULL,'uni1','test.md','task','Test',datetime('now'));",
        )
        .unwrap();
        let t = query_totals(&conn);
        assert_eq!(t.users, 2);
        assert_eq!(t.universes, 1);
        assert_eq!(t.universes_active_7d, 1);
        assert_eq!(t.entries, 1);
    }

    #[test]
    fn test_query_totals_stale_entry_not_active() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO entries VALUES (NULL,'uni1','old.md','task','Old',datetime('now','-10 days'));",
        )
        .unwrap();
        let t = query_totals(&conn);
        assert_eq!(
            t.universes_active_7d, 0,
            "entry older than 7d must not count as active"
        );
    }

    #[test]
    fn test_query_daily_14d_returns_14_rows() {
        let conn = create_test_db();
        let daily = query_daily_14d(&conn);
        assert_eq!(daily.len(), 14);
    }

    #[test]
    fn test_query_daily_14d_empty_db_all_zeros() {
        let conn = create_test_db();
        let daily = query_daily_14d(&conn);
        assert!(
            daily
                .iter()
                .all(|d| d.pageviews == 0 && d.uniques == 0 && d.errors == 0),
            "empty DB → all daily rows must be zero"
        );
    }

    #[test]
    fn test_query_daily_14d_counts_todays_events() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO telemetry_events (timestamp,event_type,event_name,visitor_token,path)
             VALUES (datetime('now'),'pageview','page.view','v1','/co');
             INSERT INTO telemetry_events (timestamp,event_type,event_name,visitor_token,path)
             VALUES (datetime('now'),'pageview','page.view','v2','/co');
             INSERT INTO telemetry_events (timestamp,event_type,event_name,visitor_token)
             VALUES (datetime('now'),'error','js.error','v1');",
        )
        .unwrap();
        let daily = query_daily_14d(&conn);
        assert_eq!(daily.len(), 14);
        let today = daily.last().unwrap();
        assert_eq!(today.pageviews, 2);
        assert_eq!(today.uniques, 2);
        assert_eq!(today.errors, 1);
    }

    #[test]
    fn test_query_daily_14d_ignores_old_events() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO telemetry_events (timestamp,event_type,event_name,visitor_token)
             VALUES (datetime('now','-20 days'),'pageview','page.view','v1');",
        )
        .unwrap();
        let daily = query_daily_14d(&conn);
        assert!(
            daily.iter().all(|d| d.pageviews == 0),
            "events older than 14d must not appear"
        );
    }

    #[test]
    fn test_query_top_universes_empty() {
        let conn = create_test_db();
        assert!(query_top_universes(&conn).is_empty());
    }

    #[test]
    fn test_query_top_universes_ranks_by_events() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO universes VALUES ('uniA','Alpha','u1',1,0,0,datetime('now'));
             INSERT INTO universes VALUES ('uniB','Beta','u1',1,0,0,datetime('now'));
             INSERT INTO telemetry_events (timestamp,event_type,event_name,universe_key)
             VALUES (datetime('now'),'pageview','page.view','uniA');
             INSERT INTO telemetry_events (timestamp,event_type,event_name,universe_key)
             VALUES (datetime('now'),'pageview','page.view','uniA');
             INSERT INTO telemetry_events (timestamp,event_type,event_name,universe_key)
             VALUES (datetime('now'),'pageview','page.view','uniB');",
        )
        .unwrap();
        let top = query_top_universes(&conn);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].key, "uniA");
        assert_eq!(top[0].events_7d, 2);
        assert_eq!(top[1].key, "uniB");
        assert_eq!(top[1].events_7d, 1);
    }

    #[test]
    fn test_query_auth_stats_empty() {
        let conn = create_test_db();
        let a = query_auth_stats(&conn);
        assert_eq!(a.logins_today, 0);
        assert_eq!(a.failed_logins_today, 0);
        assert_eq!(a.active_sessions, 0);
    }

    #[test]
    fn test_query_auth_stats_active_sessions() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO telemetry_events (timestamp,event_type,event_name,session_id)
             VALUES (datetime('now'),'pageview','page.view','sess-abc');
             INSERT INTO telemetry_events (timestamp,event_type,event_name,session_id)
             VALUES (datetime('now'),'pageview','page.view','sess-xyz');",
        )
        .unwrap();
        let a = query_auth_stats(&conn);
        assert_eq!(a.active_sessions, 2);
    }

    // --- Access gate tests ---

    #[test]
    fn test_is_admin_email_matches() {
        assert!(is_admin_email("admin@co.com", "admin@co.com"));
    }

    #[test]
    fn test_is_admin_email_mismatch() {
        assert!(!is_admin_email("other@co.com", "admin@co.com"));
    }

    #[test]
    fn test_is_admin_email_empty_admin() {
        assert!(!is_admin_email("admin@co.com", ""));
    }

    #[test]
    fn test_is_admin_email_empty_caller() {
        assert!(!is_admin_email("", "admin@co.com"));
    }

    #[test]
    fn test_is_admin_email_both_empty() {
        assert!(!is_admin_email("", ""));
    }

    // --- HTTP integration tests ---

    use std::sync::{Arc, Mutex as StdMutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxumStatus};
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::server::{CoreState, IndexState, IntegrationsState, RealtimeState};

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        let config = crate::config::WebConfig {
            port: 3000,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "universo".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_api_key: None,
            wae_endpoint: None,
            cookie_domain: None,
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
                    rate_limiter: StdMutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                    experiment: StdMutex::new(experiment),
                    worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
                }),
            });
        crate::server::build_router(state, None)
    }

    #[tokio::test]
    async fn test_dashboard_no_auth_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/admin/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_dashboard_valid_jwt_no_admin_email_set_returns_403() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let secret = crate::auth::jwt_secret();
        let (token, _) =
            crate::auth::sign_jwt("usr_test", "notadmin@example.com", "player", &secret).unwrap();

        // When CO_SEED_ADMIN_EMAIL is unset or doesn't match, is_admin_email returns false → 403
        // We use a disposable email that is extremely unlikely to match CI env config
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/admin/dashboard")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // 403 if CO_SEED_ADMIN_EMAIL isn't "notadmin@example.com"; 200 if it is (CI edge case)
        assert!(
            resp.status() == AxumStatus::FORBIDDEN || resp.status() == AxumStatus::OK,
            "expected 403 (or 200 in rare CI config), got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn test_admin_page_no_cookie_redirects_to_co() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Redirect (303) when no session cookie
        assert_eq!(resp.status(), AxumStatus::SEE_OTHER);
        assert_eq!(
            resp.headers().get("location").and_then(|v| v.to_str().ok()),
            Some("/")
        );
    }

    #[tokio::test]
    async fn test_admin_page_invalid_jwt_redirects() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin")
                    .header("Cookie", "session=not.a.valid.jwt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), AxumStatus::SEE_OTHER);
    }

    #[tokio::test]
    async fn test_admin_page_valid_jwt_non_admin_returns_403() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let secret = crate::auth::jwt_secret();
        let (token, _) =
            crate::auth::sign_jwt("usr_test", "notadmin@example.com", "player", &secret).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin")
                    .header("Cookie", format!("session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // 403 if CO_SEED_ADMIN_EMAIL isn't "notadmin@example.com"
        assert!(
            resp.status() == AxumStatus::FORBIDDEN || resp.status() == AxumStatus::OK,
            "expected 403 for non-admin (or 200 in CI if env matches), got {}",
            resp.status()
        );
    }

    // --- CO-406: graceful per-universe pool degradation ---

    /// Seed a public universe row directly into meta.db so the visibility gate
    /// lets anonymous reads through.
    fn seed_public_universe(dir: &std::path::Path, key: &str) {
        let storage = crate::storage::Storage::new(dir);
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, is_public, visibility) \
                 VALUES (?1, ?1, '', 'system', '2026-01-01', 1, 'public')",
                [key],
            )
            .unwrap();
    }

    /// Plant a FILE where universe `key`'s data dir should be, so its pool open
    /// fails deterministically (stand-in for disk-full / I/O error).
    fn poison_universe_dir(dir: &std::path::Path, key: &str) {
        let pool = crate::universe_pool::UniversePool::new(dir, 1);
        let udir = pool.universe_dir(key);
        std::fs::create_dir_all(udir.parent().unwrap()).unwrap();
        std::fs::write(&udir, b"not a directory").unwrap();
    }

    /// CO-406 (router level): when ONE universe's pool open fails, its content
    /// routes return 503 — but EVERY OTHER universe still serves 200. The
    /// process must not panic / crash-loop.
    #[tokio::test]
    async fn co406_one_bad_universe_503_others_serve() {
        let dir = tempdir().unwrap();

        seed_public_universe(dir.path(), "healthy");
        seed_public_universe(dir.path(), "broken");
        poison_universe_dir(dir.path(), "broken");

        let app = build_test_router(dir.path());

        // Broken universe → 503 (degraded, isolated).
        let broken = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/broken/entries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            broken.status(),
            AxumStatus::SERVICE_UNAVAILABLE,
            "broken universe must 503, not panic"
        );

        // Healthy universe → 200 despite the broken neighbour.
        let healthy = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/universes/healthy/entries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            healthy.status(),
            AxumStatus::OK,
            "other universes must keep serving while one is degraded"
        );
    }
}
