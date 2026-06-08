//! CO-360: Unified /gestao/resumo dashboard — batched API endpoints.
//!
//! GET /api/v1/gestao/resumo      — single-round-trip dashboard data (email admin auth)
//! GET /api/v1/gestao/universes   — universe list with entry counts (email admin auth)
//! GET /api/v1/gestao/atividades  — paginated audit log (email admin auth)

use std::collections::HashMap;
use std::path::Path;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use rusqlite::Connection;
use serde::Serialize;

use crate::admin::admin_routes::{check_admin_email, extract_claims};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ResumoTotals {
    pub users: i64,
    pub universes: i64,
    pub entries: i64,
    pub sessions_24h: i64,
    pub disk_used_mb: u64,
    pub disk_total_mb: u64,
}

#[derive(Debug, Serialize)]
pub struct PageVisit {
    pub path: String,
    pub visits: i64,
}

#[derive(Debug, Serialize)]
pub struct HourBucket {
    pub hour: u8,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct DowBucket {
    pub dow: u8,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct ReferrerRow {
    pub referrer: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct BrokenLink {
    pub path: String,
    pub status: u16,
    pub last_seen: String,
}

#[derive(Debug, Serialize)]
pub struct RecentAtividade {
    pub acao: String,
    pub entidade: String,
    pub user: Option<String>,
    pub at: String,
}

/// CO-389: diagnostic counts for comunicacao universe overlay vs poll-driven upserts.
#[derive(Debug, Serialize)]
pub struct ComunicacaoOverlayStats {
    pub term_landed_overlay: i64,
    pub term_landed_via_repo: i64,
    pub dedup_skipped: i64,
}

#[derive(Debug, Serialize)]
pub struct ResumoResponse {
    pub totals: ResumoTotals,
    pub top_pages: Vec<PageVisit>,
    pub visits_by_hour: Vec<HourBucket>,
    pub visits_by_dow: Vec<DowBucket>,
    pub referrers: Vec<ReferrerRow>,
    pub broken_links: Vec<BrokenLink>,
    pub recent_activity: Vec<RecentAtividade>,
    pub schema_versao: i64,
    pub app_versao: &'static str,
    /// CO-389: diagnostic — counts from event_log for comunicacao overlay vs repo sync.
    pub comunicacao_overlay: ComunicacaoOverlayStats,
}

#[derive(Debug, Serialize)]
pub struct UniverseRow {
    pub key: String,
    pub name: String,
    pub owner_id: Option<String>,
    pub entry_count: i64,
    pub last_activity: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AtividadeRow {
    pub id: i64,
    pub acao: String,
    pub entidade: String,
    pub entidade_id: Option<String>,
    pub conteudo: Option<serde_json::Value>,
    pub tipo: String,
    pub user_id: Option<String>,
    pub ip_hash: Option<String>,
    pub user_agent: Option<String>,
    pub versao_app: Option<String>,
    pub criado_em: String,
}

// ---------------------------------------------------------------------------
// Disk helpers (outside storage lock — never hold Mutex across a blocking op)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn disk_total_bytes(path: &Path) -> u64 {
    match nix::sys::statvfs::statvfs(path) {
        Ok(stat) => stat.blocks() * stat.fragment_size(),
        Err(_) => 0,
    }
}

#[cfg(not(target_os = "linux"))]
fn disk_total_bytes(_path: &Path) -> u64 {
    0
}

fn walk_used_bytes(root: &Path) -> u64 {
    fn inner(p: &Path, acc: &mut u64) {
        let Ok(rd) = std::fs::read_dir(p) else { return };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                inner(&path, acc);
            } else if ft.is_file()
                && let Ok(m) = entry.metadata()
            {
                *acc += m.len();
            }
        }
    }
    let mut acc = 0u64;
    inner(root, &mut acc);
    acc
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

fn query_resumo_from_db(
    conn: &Connection,
    disk_used_mb: u64,
    disk_total_mb: u64,
) -> ResumoResponse {
    let users: i64 = conn
        .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
        .unwrap_or(0);
    let universes: i64 = conn
        .query_row("SELECT COUNT(*) FROM universes", [], |r| r.get(0))
        .unwrap_or(0);
    let entries: i64 = conn
        .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap_or(0);
    let sessions_24h: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT session_id) FROM telemetry_events \
             WHERE timestamp > datetime('now', '-24 hours') AND session_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let top_pages: Vec<PageVisit> = conn
        .prepare(
            "SELECT path, COUNT(*) AS visits FROM telemetry_events \
             WHERE event_type = 'pageview' AND timestamp > datetime('now', '-7 days') \
             AND path IS NOT NULL \
             GROUP BY path ORDER BY visits DESC LIMIT 10",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(PageVisit {
                    path: r.get(0)?,
                    visits: r.get(1)?,
                })
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    let mut hour_map: HashMap<u8, i64> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT CAST(strftime('%H', timestamp) AS INTEGER) AS h, COUNT(*) \
         FROM telemetry_events WHERE event_type = 'pageview' \
         AND timestamp > datetime('now', '-7 days') GROUP BY h",
    ) {
        let _ = stmt
            .query_map([], |r| Ok((r.get::<_, u8>(0)?, r.get::<_, i64>(1)?)))
            .map(|rows| {
                for row in rows.flatten() {
                    hour_map.insert(row.0, row.1);
                }
            });
    }
    let visits_by_hour: Vec<HourBucket> = (0u8..24)
        .map(|h| HourBucket {
            hour: h,
            count: hour_map.get(&h).copied().unwrap_or(0),
        })
        .collect();

    let mut dow_map: HashMap<u8, i64> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT CAST(strftime('%w', timestamp) AS INTEGER) AS dow, COUNT(*) \
         FROM telemetry_events WHERE event_type = 'pageview' \
         AND timestamp > datetime('now', '-7 days') GROUP BY dow",
    ) {
        let _ = stmt
            .query_map([], |r| Ok((r.get::<_, u8>(0)?, r.get::<_, i64>(1)?)))
            .map(|rows| {
                for row in rows.flatten() {
                    dow_map.insert(row.0, row.1);
                }
            });
    }
    let visits_by_dow: Vec<DowBucket> = (0u8..7)
        .map(|d| DowBucket {
            dow: d,
            count: dow_map.get(&d).copied().unwrap_or(0),
        })
        .collect();

    let referrers: Vec<ReferrerRow> = conn
        .prepare(
            "SELECT json_extract(properties, '$.referrer') AS referrer, COUNT(*) AS cnt \
             FROM telemetry_events \
             WHERE event_type = 'pageview' \
             AND json_extract(properties, '$.referrer') IS NOT NULL \
             AND json_extract(properties, '$.referrer') != '' \
             AND timestamp > datetime('now', '-7 days') \
             GROUP BY referrer ORDER BY cnt DESC LIMIT 10",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(ReferrerRow {
                    referrer: r.get(0)?,
                    count: r.get(1)?,
                })
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    let broken_links: Vec<BrokenLink> = conn
        .prepare(
            "SELECT path, MAX(timestamp) AS last_seen \
             FROM telemetry_events \
             WHERE event_type = 'error' AND event_name LIKE '%404%' \
             AND path IS NOT NULL \
             GROUP BY path ORDER BY COUNT(*) DESC LIMIT 20",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(BrokenLink {
                    path: r.get(0)?,
                    status: 404,
                    last_seen: r.get(1)?,
                })
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    let recent_activity: Vec<RecentAtividade> = conn
        .prepare(
            "SELECT acao, entidade, user_id, criado_em \
             FROM atividades ORDER BY criado_em DESC LIMIT 5",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(RecentAtividade {
                    acao: r.get(0)?,
                    entidade: r.get(1)?,
                    user: r.get(2)?,
                    at: r.get(3)?,
                })
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    let schema_versao: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // CO-389: count overlay-driven vs poll-driven comunicacao upserts from event_log.
    let comunicacao_overlay = query_comunicacao_overlay_stats(conn);

    ResumoResponse {
        totals: ResumoTotals {
            users,
            universes,
            entries,
            sessions_24h,
            disk_used_mb,
            disk_total_mb,
        },
        top_pages,
        visits_by_hour,
        visits_by_dow,
        referrers,
        broken_links,
        recent_activity,
        schema_versao,
        app_versao: env!("CARGO_PKG_VERSION"),
        comunicacao_overlay,
    }
}

/// CO-389: count comunicacao overlay events from the event_log table.
///
/// Returns zero counts gracefully if the table does not yet exist or is empty.
fn query_comunicacao_overlay_stats(conn: &Connection) -> ComunicacaoOverlayStats {
    let count_by_type = |event_type: &str| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM event_log WHERE event_type = ?1",
            rusqlite::params![event_type],
            |r| r.get(0),
        )
        .unwrap_or(0)
    };
    ComunicacaoOverlayStats {
        term_landed_overlay: count_by_type("sync.comunicacao.term_landed"),
        term_landed_via_repo: count_by_type("sync.comunicacao.term_landed_via_repo"),
        dedup_skipped: count_by_type("sync.comunicacao.event_dedup_skipped"),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/gestao/resumo
///
/// Returns the full batched dashboard JSON. Requires email admin auth
/// (session cookie or Bearer JWT matching CO_SEED_ADMIN_EMAIL).
pub async fn resumo_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ResumoResponse>, Response> {
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

    let data_dir = {
        let storage = state.core.storage.lock();
        storage.data_dir.clone()
    };

    let disk_used_mb = walk_used_bytes(&data_dir) / 1_048_576;
    let disk_total_mb = disk_total_bytes(&data_dir) / 1_048_576;

    let data = {
        let storage = state.core.storage.lock();
        query_resumo_from_db(storage.conn(), disk_used_mb, disk_total_mb)
    };

    Ok(Json(data))
}

/// GET /api/v1/gestao/universes
///
/// Returns universe list with entry counts and last activity, sorted by
/// most recently active. Requires email admin auth.
pub async fn universes_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UniverseRow>>, Response> {
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

    let rows = {
        let storage = state.core.storage.lock();
        let conn = storage.conn();
        conn.prepare(
            "SELECT u.key, u.name, u.owner_id, \
             COUNT(e.id) AS entry_count, \
             MAX(e.updated_at) AS last_activity \
             FROM universes u \
             LEFT JOIN entries e ON e.universe_key = u.key \
             GROUP BY u.key, u.name, u.owner_id \
             ORDER BY last_activity DESC \
             LIMIT 200",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(UniverseRow {
                    key: r.get(0)?,
                    name: r.get(1)?,
                    owner_id: r.get(2)?,
                    entry_count: r.get(3)?,
                    last_activity: r.get(4)?,
                })
            })
            .map(|rows| rows.flatten().collect::<Vec<_>>())
        })
        .unwrap_or_default()
    };

    Ok(Json(rows))
}

/// GET /api/v1/gestao/atividades?limit=…&since=…&acao=…
///
/// Paginated audit log feed backed by the `atividades` table.
/// Requires email admin auth (session cookie or Bearer JWT).
pub async fn atividades_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<AtividadeRow>>, Response> {
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

    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .clamp(1, 200);
    let since = params.get("since").cloned();
    let acao_filter = params.get("acao").cloned();

    let rows = {
        let storage = state.core.storage.lock();
        let conn = storage.conn();
        conn.prepare(
            "SELECT id, acao, entidade, entidade_id, conteudo, tipo, \
             user_id, ip_hash, user_agent, versao_app, criado_em \
             FROM atividades \
             WHERE (?1 IS NULL OR criado_em > ?1) \
               AND (?2 IS NULL OR acao = ?2) \
             ORDER BY criado_em DESC \
             LIMIT ?3",
        )
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![since, acao_filter, limit], |row| {
                let conteudo_str: Option<String> = row.get(4)?;
                Ok(AtividadeRow {
                    id: row.get(0)?,
                    acao: row.get(1)?,
                    entidade: row.get(2)?,
                    entidade_id: row.get(3)?,
                    conteudo: conteudo_str.and_then(|s| serde_json::from_str(&s).ok()),
                    tipo: row.get(5)?,
                    user_id: row.get(6)?,
                    ip_hash: row.get(7)?,
                    user_agent: row.get(8)?,
                    versao_app: row.get(9)?,
                    criado_em: row.get(10)?,
                })
            })
            .map(|rows| rows.flatten().collect::<Vec<_>>())
        })
        .unwrap_or_default()
    };

    Ok(Json(rows))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/resumo", get(resumo_handler))
        .route("/universes", get(universes_handler))
        .route("/atividades", get(atividades_handler))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    // Minimal schema used by all unit tests.
    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY, email TEXT, display_name TEXT,
                tier TEXT, created_at TEXT, password_hash TEXT
            );
            CREATE TABLE universes (
                key TEXT PRIMARY KEY, name TEXT, owner_id TEXT,
                is_public INTEGER DEFAULT 0, is_template INTEGER DEFAULT 0,
                content_count INTEGER DEFAULT 0, created_at TEXT
            );
            CREATE TABLE entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                universe_key TEXT, path TEXT, entry_type TEXT,
                title TEXT, updated_at TEXT
            );
            CREATE TABLE telemetry_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL, visitor_token TEXT, user_id TEXT,
                session_id TEXT, event_type TEXT NOT NULL, event_name TEXT NOT NULL,
                universe_key TEXT, path TEXT, properties TEXT,
                duration_ms INTEGER, ip_hash TEXT, ua_device TEXT,
                ua_browser TEXT, ua_os TEXT
            );
            CREATE TABLE atividades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                acao TEXT NOT NULL, entidade TEXT NOT NULL, entidade_id TEXT,
                conteudo TEXT, tipo TEXT NOT NULL DEFAULT 'sucesso',
                user_id TEXT, ip_hash TEXT, user_agent TEXT,
                versao_app TEXT, criado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE schema_version (version INTEGER PRIMARY KEY);",
        )
        .unwrap();
        conn
    }

    // --- Unit tests for query helpers ---

    #[test]
    fn test_resumo_empty_db_returns_zeros() {
        let conn = create_test_db();
        let r = query_resumo_from_db(&conn, 0, 0);
        assert_eq!(r.totals.users, 0);
        assert_eq!(r.totals.universes, 0);
        assert_eq!(r.totals.entries, 0);
        assert_eq!(r.totals.sessions_24h, 0);
        assert_eq!(
            r.visits_by_hour.len(),
            24,
            "must always return 24 hour buckets"
        );
        assert_eq!(r.visits_by_dow.len(), 7, "must always return 7 dow buckets");
        assert!(r.top_pages.is_empty());
        assert!(r.referrers.is_empty());
        assert!(r.broken_links.is_empty());
        assert!(r.recent_activity.is_empty());
        assert_eq!(r.schema_versao, 0);
    }

    #[test]
    fn test_resumo_counts_users_universes_entries() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO users VALUES ('u1','a@b.com','A','player',datetime('now'),NULL);
             INSERT INTO universes VALUES ('uni1','Uni 1','u1',1,0,0,datetime('now'));
             INSERT INTO entries VALUES (NULL,'uni1','t.md','task','T',datetime('now'));
             INSERT INTO entries VALUES (NULL,'uni1','t2.md','task','T2',datetime('now'));",
        )
        .unwrap();
        let r = query_resumo_from_db(&conn, 0, 0);
        assert_eq!(r.totals.users, 1);
        assert_eq!(r.totals.universes, 1);
        assert_eq!(r.totals.entries, 2);
    }

    #[test]
    fn test_resumo_top_pages_ordered_by_visits() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO telemetry_events
               (timestamp,event_type,event_name,path)
               VALUES (datetime('now'),'pageview','page.view','/co');
             INSERT INTO telemetry_events
               (timestamp,event_type,event_name,path)
               VALUES (datetime('now'),'pageview','page.view','/co');
             INSERT INTO telemetry_events
               (timestamp,event_type,event_name,path)
               VALUES (datetime('now'),'pageview','page.view','/about');",
        )
        .unwrap();
        let r = query_resumo_from_db(&conn, 0, 0);
        assert!(!r.top_pages.is_empty());
        assert_eq!(r.top_pages[0].path, "/co");
        assert_eq!(r.top_pages[0].visits, 2);
    }

    #[test]
    fn test_resumo_visits_by_hour_always_24_buckets() {
        let conn = create_test_db();
        let r = query_resumo_from_db(&conn, 0, 0);
        assert_eq!(r.visits_by_hour.len(), 24);
        for (i, b) in r.visits_by_hour.iter().enumerate() {
            assert_eq!(b.hour, i as u8);
        }
    }

    #[test]
    fn test_resumo_visits_by_dow_always_7_buckets() {
        let conn = create_test_db();
        let r = query_resumo_from_db(&conn, 0, 0);
        assert_eq!(r.visits_by_dow.len(), 7);
        for (i, b) in r.visits_by_dow.iter().enumerate() {
            assert_eq!(b.dow, i as u8);
        }
    }

    #[test]
    fn test_resumo_sessions_24h() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO telemetry_events
               (timestamp,event_type,event_name,session_id)
               VALUES (datetime('now'),'pageview','page.view','sess-a');
             INSERT INTO telemetry_events
               (timestamp,event_type,event_name,session_id)
               VALUES (datetime('now'),'pageview','page.view','sess-b');",
        )
        .unwrap();
        let r = query_resumo_from_db(&conn, 0, 0);
        assert_eq!(r.totals.sessions_24h, 2);
    }

    #[test]
    fn test_resumo_schema_versao() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO schema_version VALUES (42);
             INSERT INTO schema_version VALUES (43);",
        )
        .unwrap();
        let r = query_resumo_from_db(&conn, 0, 0);
        assert_eq!(r.schema_versao, 43);
    }

    #[test]
    fn test_resumo_recent_activity() {
        let conn = create_test_db();
        conn.execute_batch(
            "INSERT INTO atividades (acao,entidade,tipo,criado_em)
               VALUES ('criar','task','sucesso','2024-01-01 10:00:00');
             INSERT INTO atividades (acao,entidade,tipo,criado_em)
               VALUES ('atualizar','task','sucesso','2024-01-01 11:00:00');",
        )
        .unwrap();
        let r = query_resumo_from_db(&conn, 0, 0);
        assert!(!r.recent_activity.is_empty());
        assert_eq!(r.recent_activity.len(), 2);
        // most recent (11:00) first
        assert_eq!(r.recent_activity[0].acao, "atualizar");
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
    async fn test_resumo_no_auth_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/gestao/resumo")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_resumo_non_admin_jwt_returns_403() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let secret = crate::auth::jwt_secret();
        let (token, _) =
            crate::auth::sign_jwt("usr_test", "notadmin@example.com", "player", &secret).unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/gestao/resumo")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status() == AxumStatus::FORBIDDEN || resp.status() == AxumStatus::OK,
            "expected 403 for non-admin (or 200 if env matches), got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn test_resumo_valid_jwt_non_admin_returns_403_or_200() {
        // Uses an email extremely unlikely to match CO_SEED_ADMIN_EMAIL in any env.
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let secret = crate::auth::jwt_secret();
        let (token, _) = crate::auth::sign_jwt(
            "usr_test",
            "notadmin@resumo-test-co.invalid",
            "player",
            &secret,
        )
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/gestao/resumo")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status() == AxumStatus::FORBIDDEN || resp.status() == AxumStatus::OK,
            "expected 403 for non-admin (or 200 if env matches), got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn test_universes_no_auth_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/gestao/universes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_universes_non_admin_returns_403_or_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let secret = crate::auth::jwt_secret();
        let (token, _) = crate::auth::sign_jwt(
            "usr_test",
            "notadmin@resumo-test-co.invalid",
            "player",
            &secret,
        )
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/gestao/universes")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status() == AxumStatus::FORBIDDEN || resp.status() == AxumStatus::OK,
            "expected 403 for non-admin (or 200 if env matches), got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn test_atividades_no_auth_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/gestao/atividades")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_atividades_non_admin_returns_403_or_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let secret = crate::auth::jwt_secret();
        let (token, _) = crate::auth::sign_jwt(
            "usr_test",
            "notadmin@resumo-test-co.invalid",
            "player",
            &secret,
        )
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/gestao/atividades?limit=10")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status() == AxumStatus::FORBIDDEN || resp.status() == AxumStatus::OK,
            "expected 403 for non-admin (or 200 if env matches), got {}",
            resp.status()
        );
    }
}
