//! Public analytics — CO-180: popularity endpoint.
//!
//! GET /api/v1/analytics/public/popularity?prefix=/servicos/&days=30
//!
//! Returns page-view counts for paths matching a given prefix,
//! ordered by views DESC (tie-break path ASC). No auth required.
//! Cached 5 min in-memory per (prefix, days).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::server::AppState;

const UNIVERSE_KEY: &str = "artelonga";
const CACHE_TTL: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// Response shape
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
        let storage = state.storage.lock();
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
    Router::new().route("/popularity", get(popularity_handler))
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

    fn insert_page_view(
        conn: &Connection,
        universe: &str,
        visitor: &str,
        path: &str,
        offset_days: i64,
    ) {
        conn.execute(
            &format!(
                "INSERT INTO telemetry_events \
                 (timestamp, visitor_token, event_type, event_name, universe_key, path) \
                 VALUES (datetime('now', '{offset_days} days'), ?1, 'pageview', 'page_view', ?2, ?3)"
            ),
            rusqlite::params![visitor, universe, path],
        )
        .unwrap();
    }

    // --- validate_prefix ---

    #[test]
    fn test_validate_prefix_ok() {
        assert!(validate_prefix("/servicos/").is_ok());
        assert!(validate_prefix("/").is_ok());
        assert!(validate_prefix("/a/b/c/").is_ok());
    }

    #[test]
    fn test_validate_prefix_must_start_with_slash() {
        assert!(validate_prefix("servicos/").is_err());
        assert!(validate_prefix("").is_err());
    }

    #[test]
    fn test_validate_prefix_no_dotdot() {
        assert!(validate_prefix("/../etc").is_err());
        assert!(validate_prefix("/servicos/../secrets/").is_err());
    }

    #[test]
    fn test_validate_prefix_max_64_chars() {
        let at_limit = format!("/{}", "a".repeat(63));
        assert!(validate_prefix(&at_limit).is_ok(), "64 chars should pass");
        let over_limit = format!("/{}", "a".repeat(64));
        assert!(
            validate_prefix(&over_limit).is_err(),
            "65 chars should fail"
        );
    }

    // --- query_popularity ---

    #[test]
    fn test_popularity_empty_db_returns_empty() {
        let conn = create_test_db();
        let items = query_popularity(&conn, "/servicos/", 30);
        assert!(items.is_empty());
    }

    #[test]
    fn test_popularity_filters_by_prefix() {
        let conn = create_test_db();
        insert_page_view(&conn, "artelonga", "v1", "/servicos/dev/", 0);
        insert_page_view(&conn, "artelonga", "v2", "/about/", 0);
        insert_page_view(&conn, "artelonga", "v3", "/servicos/art/", 0);
        let items = query_popularity(&conn, "/servicos/", 30);
        assert_eq!(items.len(), 2, "only /servicos/ paths should match");
        let paths: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"/servicos/dev/"));
        assert!(paths.contains(&"/servicos/art/"));
    }

    #[test]
    fn test_popularity_filters_by_universe_key() {
        let conn = create_test_db();
        insert_page_view(&conn, "artelonga", "v1", "/servicos/dev/", 0);
        insert_page_view(&conn, "other", "v2", "/servicos/dev/", 0);
        let items = query_popularity(&conn, "/servicos/", 30);
        assert_eq!(items.len(), 1, "only artelonga events should count");
    }

    #[test]
    fn test_popularity_order_views_desc() {
        let conn = create_test_db();
        insert_page_view(&conn, "artelonga", "v1", "/servicos/a/", 0);
        insert_page_view(&conn, "artelonga", "v2", "/servicos/a/", 0);
        insert_page_view(&conn, "artelonga", "v3", "/servicos/b/", 0);
        let items = query_popularity(&conn, "/servicos/", 30);
        assert_eq!(items[0].path, "/servicos/a/", "higher views comes first");
        assert_eq!(items[0].views, 2);
        assert_eq!(items[1].views, 1);
    }

    #[test]
    fn test_popularity_tie_break_slug_asc() {
        let conn = create_test_db();
        insert_page_view(&conn, "artelonga", "v1", "/servicos/z/", 0);
        insert_page_view(&conn, "artelonga", "v2", "/servicos/a/", 0);
        let items = query_popularity(&conn, "/servicos/", 30);
        assert_eq!(
            items[0].path, "/servicos/a/",
            "equal views: alphabetical by path"
        );
        assert_eq!(items[1].path, "/servicos/z/");
    }

    #[test]
    fn test_popularity_slug_derived_correctly() {
        let conn = create_test_db();
        insert_page_view(
            &conn,
            "artelonga",
            "v1",
            "/servicos/desenvolvimento-web/",
            0,
        );
        let items = query_popularity(&conn, "/servicos/", 30);
        assert_eq!(items[0].slug, "desenvolvimento-web");
    }

    #[test]
    fn test_popularity_visitors_distinct() {
        let conn = create_test_db();
        insert_page_view(&conn, "artelonga", "v1", "/servicos/dev/", 0);
        insert_page_view(&conn, "artelonga", "v1", "/servicos/dev/", 0);
        insert_page_view(&conn, "artelonga", "v1", "/servicos/dev/", 0);
        insert_page_view(&conn, "artelonga", "v2", "/servicos/dev/", 0);
        let items = query_popularity(&conn, "/servicos/", 30);
        assert_eq!(items[0].views, 4);
        assert_eq!(items[0].visitors, 2, "distinct visitors only");
    }

    #[test]
    fn test_popularity_window_excludes_old_events() {
        let conn = create_test_db();
        insert_page_view(&conn, "artelonga", "v1", "/servicos/dev/", 0);
        insert_page_view(&conn, "artelonga", "v2", "/servicos/dev/", -40);
        let items = query_popularity(&conn, "/servicos/", 30);
        assert_eq!(items[0].views, 1, "events outside window must be excluded");
    }

    // --- HTTP integration tests ---

    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxumStatus};
    use http_body_util::BodyExt as _;
    use std::sync::{Arc, Mutex as StdMutex};
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
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
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
        let state: crate::server::AppState = Arc::new(crate::server::AppStateInner {
            storage: parking_lot::Mutex::new(storage),
            experiment: StdMutex::new(experiment),
            config,
            auth_store: StdMutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            cache: crate::cache::CacheLayer::new(),
            rate_limiter: StdMutex::new(crate::rate_limit::RateLimiter::new()),
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
            embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        });
        crate::server::build_router(state, None)
    }

    async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec()
    }

    #[tokio::test]
    async fn test_http_popularity_returns_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/popularity?prefix=%2Fservicos-200%2F")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
    }

    #[tokio::test]
    async fn test_http_popularity_missing_prefix_returns_400() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/popularity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_http_popularity_dotdot_prefix_returns_400() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/popularity?prefix=%2F..%2Fetc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_http_popularity_empty_db_shape() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/popularity?prefix=%2Fservicos-empty%2F")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::OK);
        let body = body_bytes(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["items"].is_array());
        assert!(json["items"].as_array().unwrap().is_empty());
        assert!(json["as_of"].is_string());
        assert_eq!(json["window_days"], 30);
        assert_eq!(json["prefix"], "/servicos-empty/");
    }

    #[tokio::test]
    async fn test_http_popularity_shape_documented() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/popularity?prefix=%2Fservicos-shape%2F&days=7")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_bytes(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("as_of").is_some());
        assert_eq!(json["window_days"], 7);
        assert_eq!(json["prefix"], "/servicos-shape/");
        assert!(json["items"].is_array());
    }

    #[tokio::test]
    async fn test_http_popularity_cors_preflight() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/v1/analytics/public/popularity?prefix=%2Fservicos%2F")
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

    #[tokio::test]
    async fn test_http_popularity_stable_items_on_rerun() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/popularity?prefix=%2Fservicos-stable%2F")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let first_body = body_bytes(first).await;
        let second = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/analytics/public/popularity?prefix=%2Fservicos-stable%2F")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let second_body = body_bytes(second).await;
        let j1: serde_json::Value = serde_json::from_slice(&first_body).unwrap();
        let j2: serde_json::Value = serde_json::from_slice(&second_body).unwrap();
        assert_eq!(
            j1["items"], j2["items"],
            "items must be identical on rerun with no new events"
        );
    }
}
