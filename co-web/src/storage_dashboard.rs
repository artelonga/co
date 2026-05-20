//! Per-universe memory + disk dashboard (2.7.27).
//!
//! Surfaces the storage footprint of every universe so we can spot
//! growth before it bites: which universe is bloating its `data.db`,
//! which is hoarding `entry_events`, which has stale embeddings.
//!
//! See `co::public/storage-dashboard.md` for what each metric measures.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::server::AppState;

const CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSize {
    pub rows: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseStorage {
    pub key: String,
    pub owner_id: String,
    pub is_public: bool,
    pub is_template: bool,
    pub visibility: Option<String>,
    /// Number of .md files indexed (file count, not line count).
    pub content_count: i64,
    /// Sum of `*.md` file sizes under `universes/<key>/`.
    pub md_bytes: u64,
    /// Size of `universes/<key>/data.db` on disk (bytes).
    pub data_db_bytes: u64,
    /// Total body lines across all entries (CO-241).
    pub body_lines: i64,
    /// Total body words across all entries (CO-241).
    pub body_words: i64,
    /// Total body chars across all entries (CO-241).
    pub body_chars: i64,
    /// Row counts for the heavy per-universe tables. Bytes-per-row
    /// estimates omitted in v1 — would need DBSTAT virtual table,
    /// not all builds enable it. The row counts already point to
    /// the storage hotspots clearly enough.
    pub tables: HashMap<String, TableSize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageTotals {
    pub universes: usize,
    pub md_bytes: u64,
    pub data_db_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub data_dir: String,
    pub data_dir_total_bytes: u64,
    pub data_dir_used_bytes: u64,
    pub data_dir_available_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageDashboard {
    pub generated_at: String,
    pub host: HostInfo,
    pub totals: StorageTotals,
    pub universes: Vec<UniverseStorage>,
}

#[derive(Debug)]
struct CachedDashboard {
    fetched_at: Instant,
    data: StorageDashboard,
}

static CACHE: Mutex<Option<CachedDashboard>> = Mutex::new(None);

/// Recursively sum sizes of files matching `predicate` under `root`.
fn walk_bytes(root: &Path, predicate: impl Fn(&Path) -> bool + Copy) -> u64 {
    fn inner(p: &Path, pred: &impl Fn(&Path) -> bool, acc: &mut u64) {
        let Ok(entries) = std::fs::read_dir(p) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                inner(&path, pred, acc);
            } else if ft.is_file()
                && pred(&path)
                && let Ok(m) = entry.metadata()
            {
                *acc += m.len();
            }
        }
    }
    let mut acc = 0u64;
    inner(root, &predicate, &mut acc);
    acc
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn disk_stats(path: &Path) -> (u64, u64) {
    match nix::sys::statvfs::statvfs(path) {
        Ok(stat) => {
            let frsize = stat.fragment_size() as u64;
            (
                stat.blocks() as u64 * frsize,
                stat.blocks_available() as u64 * frsize,
            )
        }
        Err(e) => {
            tracing::warn!("statvfs({path:?}) failed: {e}");
            (0, 0)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn disk_stats(_path: &Path) -> (u64, u64) {
    tracing::warn!("data_dir disk total/available not available on non-Linux platforms");
    (0, 0)
}

fn host_info(data_dir: &Path) -> HostInfo {
    let used = walk_bytes(data_dir, |_| true);
    let (total, available) = disk_stats(data_dir);
    HostInfo {
        data_dir: data_dir.display().to_string(),
        data_dir_total_bytes: total,
        data_dir_used_bytes: used,
        data_dir_available_bytes: available,
    }
}

/// Filter governing which universes appear in the dashboard.
///
/// 2.7.29: dropped the `OwnedBy`-only variant in favor of
/// `AccessibleBy`, which covers BOTH ownership and membership.
/// "Remove admin functionality, users are either members of or not" —
/// the dashboard surface follows the same rule. A member of a private
/// universe sees its stats too.
pub enum DashboardFilter<'a> {
    /// Every universe — kept for legacy admin endpoint.
    All,
    /// Universes the user owns OR is a member of.
    AccessibleBy(&'a str),
    /// A single universe by key.
    Single(&'a str),
}

/// Compute storage summary for the universes matching `filter`.
pub fn compute_dashboard_filtered(
    state: &AppState,
    filter: DashboardFilter<'_>,
) -> StorageDashboard {
    let (data_dir, universe_pool) = {
        let storage = state.core.storage.lock();
        (storage.data_dir.clone(), storage.universe_pool.clone())
    };

    let meta_conn_storage = state.core.storage.lock();
    let conn = meta_conn_storage.conn();

    // Pull universe rows from meta.db, applying the filter at SQL.
    let (sql, params_iter): (&str, Vec<String>) = match filter {
        DashboardFilter::All => (
            "SELECT key, owner_id, is_public, is_template, visibility, content_count \
             FROM universes ORDER BY key",
            vec![],
        ),
        DashboardFilter::AccessibleBy(uid) => (
            "SELECT u.key, u.owner_id, u.is_public, u.is_template, u.visibility, u.content_count \
             FROM universes u \
             WHERE u.owner_id = ?1 \
                OR EXISTS (SELECT 1 FROM universe_members m \
                           WHERE m.universe_key = u.key AND m.user_id = ?1) \
             ORDER BY u.key",
            vec![uid.to_string()],
        ),
        DashboardFilter::Single(slug) => (
            "SELECT key, owner_id, is_public, is_template, visibility, content_count \
             FROM universes WHERE key = ?1",
            vec![slug.to_string()],
        ),
    };
    let mut universes: Vec<(String, String, bool, bool, Option<String>, i64)> = {
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return empty_dashboard(&data_dir),
        };
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_iter
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        let rows = match stmt.query_map(params_refs.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1).unwrap_or_default(),
                r.get::<_, i64>(2).unwrap_or(0) != 0,
                r.get::<_, i64>(3).unwrap_or(0) != 0,
                r.get::<_, Option<String>>(4).unwrap_or(None),
                r.get::<_, i64>(5).unwrap_or(0),
            ))
        }) {
            Ok(r) => r,
            Err(_) => return empty_dashboard(&data_dir),
        };
        rows.filter_map(|r| r.ok()).collect()
    };
    drop(meta_conn_storage);

    let mut results: Vec<UniverseStorage> = Vec::with_capacity(universes.len());
    for (key, owner_id, is_public, is_template, visibility, content_count) in universes.drain(..) {
        let universe_dir = data_dir.join("universes").join(&key);
        let data_db_path = universe_pool.db_path(&key);

        let md_bytes = walk_bytes(&universe_dir, |p| {
            p.extension().and_then(|e| e.to_str()) == Some("md")
        });
        let data_db_bytes = file_size(&data_db_path);

        // Row counts from the per-universe DB. Tables we care about:
        // entries (current state), entry_events (transaction log),
        // entry_relations (CO-74 FK graph), references_meta (CO-156).
        let mut tables = HashMap::new();
        let mut body_lines: i64 = 0;
        let mut body_words: i64 = 0;
        let mut body_chars: i64 = 0;
        let storage = state.core.storage.lock();
        let uc_arc = storage.universe_conn(&key);
        drop(storage);
        if let Ok(uc) = uc_arc.lock() {
            for t in [
                "entries",
                "entry_events",
                "entry_relations",
                "references_meta",
            ] {
                let rows: i64 = uc
                    .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                    .unwrap_or(0);
                tables.insert(t.to_string(), TableSize { rows });
            }
            // CO-241: aggregate content-volume metrics.
            if let Ok((l, w, c)) = uc.query_row(
                "SELECT COALESCE(SUM(body_lines),0), \
                        COALESCE(SUM(body_words),0), \
                        COALESCE(SUM(body_chars),0) \
                 FROM entries",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            ) {
                body_lines = l;
                body_words = w;
                body_chars = c;
            }
        }

        results.push(UniverseStorage {
            key,
            owner_id,
            is_public,
            is_template,
            visibility,
            content_count,
            md_bytes,
            data_db_bytes,
            body_lines,
            body_words,
            body_chars,
            tables,
        });
    }

    let totals = StorageTotals {
        universes: results.len(),
        md_bytes: results.iter().map(|u| u.md_bytes).sum(),
        data_db_bytes: results.iter().map(|u| u.data_db_bytes).sum(),
    };

    StorageDashboard {
        generated_at: chrono::Utc::now().to_rfc3339(),
        host: host_info(&data_dir),
        totals,
        universes: results,
    }
}

fn empty_dashboard(data_dir: &Path) -> StorageDashboard {
    StorageDashboard {
        generated_at: chrono::Utc::now().to_rfc3339(),
        host: host_info(data_dir),
        totals: StorageTotals {
            universes: 0,
            md_bytes: 0,
            data_db_bytes: 0,
        },
        universes: vec![],
    }
}

// ---------------------------------------------------------------------------
// HTTP handler
// ---------------------------------------------------------------------------

/// Back-compat shim — `compute_dashboard(state)` returns the full
/// (all-universes) dashboard. Calls `compute_dashboard_filtered`
/// under the hood.
pub fn compute_dashboard(state: &AppState) -> StorageDashboard {
    compute_dashboard_filtered(state, DashboardFilter::All)
}

/// Admin view — every universe. Cached 60s.
async fn admin_storage_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StorageDashboard>, Response> {
    let claims = crate::admin_routes::extract_claims(&headers).map_err(|status| {
        (status, Json(serde_json::json!({"error": "Unauthorized"}))).into_response()
    })?;
    if !crate::admin_routes::check_admin_email(&claims.email) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Forbidden"})),
        )
            .into_response());
    }

    {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref c) = *cache
            && c.fetched_at.elapsed() < CACHE_TTL
        {
            return Ok(Json(c.data.clone()));
        }
    }
    let data = compute_dashboard_filtered(&state, DashboardFilter::All);
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        *cache = Some(CachedDashboard {
            fetched_at: Instant::now(),
            data: data.clone(),
        });
    }
    Ok(Json(data))
}

/// Per-user view — every universe the caller owns. Any authenticated
/// caller can hit this; the response is scoped by `owner_id`.
async fn me_storage_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StorageDashboard>, Response> {
    let user_id = crate::auth::resolve_user_id(&state, &headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Unauthorized"})),
        )
            .into_response()
    })?;
    let data = compute_dashboard_filtered(&state, DashboardFilter::AccessibleBy(&user_id));
    Ok(Json(data))
}

/// Per-universe view — single universe by slug. Authed caller must
/// be the owner. Useful when drilling in from the per-user view.
async fn universe_storage_handler(
    State(state): State<AppState>,
    axum::extract::Path(slug): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Result<Json<StorageDashboard>, Response> {
    let user_id = crate::auth::resolve_user_id(&state, &headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Unauthorized"})),
        )
            .into_response()
    })?;
    // 2.7.29: owner OR member (no admin tier). "Users are either
    // members of or not." Public universes are also visible to any
    // authenticated reader since they're public — same posture as
    // the entries read API.
    let allowed = {
        let storage = state.core.storage.lock();
        let Some(u) = storage.get_universe(&slug) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Universe not found"})),
            )
                .into_response());
        };
        if u.owner_id == user_id || u.is_public {
            true
        } else {
            storage
                .conn()
                .query_row(
                    "SELECT 1 FROM universe_members \
                     WHERE universe_key = ?1 AND user_id = ?2",
                    rusqlite::params![&slug, &user_id],
                    |_| Ok(true),
                )
                .unwrap_or(false)
        }
    };
    if !allowed {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Forbidden — owner/member only"})),
        )
            .into_response());
    }
    let data = compute_dashboard_filtered(&state, DashboardFilter::Single(&slug));
    Ok(Json(data))
}

/// Admin-only router (mounted at `/api/v1/admin`).
pub fn router() -> Router<AppState> {
    Router::new().route("/storage", get(admin_storage_handler))
}

/// Per-user router (mounted at `/api/v1/me`).
pub fn me_router() -> Router<AppState> {
    Router::new().route("/storage", get(me_storage_handler))
}

/// Per-universe router (mounted at `/api/v1/universes`).
pub fn universe_router() -> Router<AppState> {
    Router::new().route("/{slug}/storage", get(universe_storage_handler))
}

// ---------------------------------------------------------------------------
// Static HTML page
// ---------------------------------------------------------------------------

const STORAGE_HTML: &str = include_str!("../static/shared/storage.html");

pub async fn serve_storage_page() -> Response {
    let mut resp = Response::new(STORAGE_HTML.into());
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    resp
}
