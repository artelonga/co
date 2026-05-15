//! Per-universe memory + disk dashboard (2.7.27).
//!
//! Surfaces the storage footprint of every universe so we can spot
//! growth before it bites: which universe is bloating its `data.db`,
//! which is hoarding `entry_events`, which has stale embeddings.
//!
//! See `co::public/storage-dashboard.md` for what each metric measures.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    pub content_count: i64,
    /// Sum of `*.md` file sizes under `universes/<key>/`.
    pub md_bytes: u64,
    /// Size of `universes/<key>/data.db` on disk (bytes).
    pub data_db_bytes: u64,
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
            } else if ft.is_file() && pred(&path) {
                if let Ok(m) = entry.metadata() {
                    *acc += m.len();
                }
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

/// Volume metrics for the data directory. v1: just the directory
/// path + recursive bytes-on-disk. statfs-based total/available
/// requires a libc binding we don't currently depend on; skipped
/// here to keep the dependency footprint flat. Add `nix::sys::statvfs`
/// later if precise volume telemetry is needed.
fn host_info(data_dir: &Path) -> HostInfo {
    let used = walk_bytes(data_dir, |_| true);
    HostInfo {
        data_dir: data_dir.display().to_string(),
        data_dir_total_bytes: 0,
        data_dir_used_bytes: used,
        data_dir_available_bytes: 0,
    }
}

/// Compute storage summary for every universe the caller is allowed
/// to see. Admin-only today; if scoped later, filter by `owner_id`.
pub fn compute_dashboard(state: &AppState) -> StorageDashboard {
    let storage = state.storage.lock();
    let data_dir: PathBuf = storage.data_dir.clone();
    drop(storage);

    let meta_conn_storage = state.storage.lock();
    let conn = meta_conn_storage.conn();

    // Pull universe rows from meta.db.
    let mut universes: Vec<(String, String, bool, bool, Option<String>, i64)> = {
        let mut stmt = match conn.prepare(
            "SELECT key, owner_id, is_public, is_template, visibility, content_count
             FROM universes
             ORDER BY key",
        ) {
            Ok(s) => s,
            Err(_) => return empty_dashboard(&data_dir),
        };
        let rows = match stmt.query_map([], |r| {
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
        let data_db_path = universe_dir.join("data.db");

        let md_bytes = walk_bytes(&universe_dir, |p| {
            p.extension().and_then(|e| e.to_str()) == Some("md")
        });
        let data_db_bytes = file_size(&data_db_path);

        // Row counts from the per-universe DB. Tables we care about:
        // entries (current state), entry_events (transaction log),
        // entry_relations (CO-74 FK graph), references_meta (CO-156).
        let mut tables = HashMap::new();
        let storage = state.storage.lock();
        let uc_arc = storage.universe_conn(&key);
        drop(storage);
        if let Ok(uc) = uc_arc.lock() {
            for t in ["entries", "entry_events", "entry_relations", "references_meta"] {
                let rows: i64 = uc
                    .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                    .unwrap_or(0);
                tables.insert(t.to_string(), TableSize { rows });
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

async fn storage_dashboard_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StorageDashboard>, Response> {
    // Admin-only.
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

    // Serve from 60-second cache when fresh.
    {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref c) = *cache
            && c.fetched_at.elapsed() < CACHE_TTL
        {
            return Ok(Json(c.data.clone()));
        }
    }

    let data = compute_dashboard(&state);
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        *cache = Some(CachedDashboard {
            fetched_at: Instant::now(),
            data: data.clone(),
        });
    }
    Ok(Json(data))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/storage", get(storage_dashboard_handler))
}
