//! CO-273: Centralized deployment dashboard.
//!
//! GET  /api/v1/admin/deployments         — list all unit snapshots (admin only)
//! POST /api/v1/admin/deployments/refresh — trigger immediate re-probe (admin only)
//! GET  /admin/deployments                — serve dashboard HTML (admin session only)

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::Serialize;

use crate::admin::admin_routes::{check_admin_email, extract_claims};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Cost estimation (CO-288)
// ---------------------------------------------------------------------------

/// Base Fly.io hourly rate in USD for a shared-cpu-1x with 256 MB RAM.
/// Derived from the published $1.94/mo / 730 hrs/mo ≈ $0.002658/hr.
const FLY_BASE_HOURLY_USD: f32 = 0.002_658;

/// Estimate the monthly Fly.io compute cost for a single machine.
///
/// Formula: `ram_factor × cpu_factor × uptime_ratio × FLY_BASE_HOURLY_USD × 730`
/// where `ram_factor = ram_mb / 256`.
pub fn estimate_monthly_cost(machine_size_factor: f32, ram_mb: u32, uptime_ratio: f32) -> f32 {
    let ram_factor = ram_mb as f32 / 256.0;
    let cpu_factor = machine_size_factor;
    ram_factor * cpu_factor * uptime_ratio * FLY_BASE_HOURLY_USD * 730.0
}

/// Parse `vm_size` (format `"{cpu_kind}-{cpus}x-{memory_mb}mb"`, e.g.
/// `"shared-1x-512mb"`) into `(machine_size_factor, ram_mb)`.
fn vm_size_factors(vm_size: &str) -> (f32, u32) {
    let parts: Vec<&str> = vm_size.split('-').collect();
    let cpu_kind = parts.first().copied().unwrap_or("shared");
    let cpus: f32 = parts
        .get(1)
        .and_then(|s| s.strip_suffix('x'))
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let ram_mb: u32 = parts
        .get(2)
        .and_then(|s| s.strip_suffix("mb"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(256);
    // Performance/dedicated CPUs are priced ~4× higher than shared per core.
    let machine_size_factor = match cpu_kind {
        "performance" | "dedicated" => cpus * 4.0,
        _ => cpus,
    };
    (machine_size_factor, ram_mb)
}

/// Convert a Fly machine `state` string to an expected uptime ratio.
fn state_uptime_ratio(state: &str) -> f32 {
    match state {
        "started" => 1.0,
        "stopped" => 0.1,
        "suspended" => 0.05,
        _ => 1.0,
    }
}

// ---------------------------------------------------------------------------
// API response shape
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct DeploymentSnapshot {
    pub unit: String,
    pub display: String,
    pub url: String,
    pub snapshot_at: i64,
    pub machine_id: String,
    pub region: String,
    pub vm_size: String,
    pub state: String,
    pub image: String,
    pub version: String,
    pub last_deploy_at: String,
    pub health_status: String,
    pub error_msg: String,
    pub monthly_cost_usd: f32,
}

#[derive(Debug, Serialize)]
pub struct DeploymentListResponse {
    pub units: Vec<DeploymentSnapshot>,
    pub refreshed_at: String,
    pub total_monthly_cost_usd: f32,
}

// ---------------------------------------------------------------------------
// Load all 6 units from DB (always returns 6 rows — missing = defaults)
// ---------------------------------------------------------------------------

pub(crate) fn load_snapshots(storage: &crate::storage::Storage) -> Vec<DeploymentSnapshot> {
    use crate::platform::deployment_snapshot_worker::build_units;

    // CO-338: units (and their resolved URLs) come from the surface registry.
    let units = build_units(&storage.list_surface_nodes());
    let conn = storage.conn();

    // Load whatever is in DB
    let db_rows: std::collections::HashMap<String, DeploymentSnapshot> = conn
        .prepare(
            "SELECT unit, snapshot_at, machine_id, region, vm_size, state, image,
                    version, last_deploy_at, health_status, error_msg
             FROM deployment_snapshots",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                let vm_size: String = row.get(4)?;
                let state: String = row.get(5)?;
                let (msf, ram_mb) = vm_size_factors(&vm_size);
                let cost = estimate_monthly_cost(msf, ram_mb, state_uptime_ratio(&state));
                Ok((
                    row.get::<_, String>(0)?,
                    DeploymentSnapshot {
                        unit: row.get(0)?,
                        display: String::new(),
                        url: String::new(),
                        snapshot_at: row.get(1)?,
                        machine_id: row.get(2)?,
                        region: row.get(3)?,
                        vm_size,
                        state,
                        image: row.get(6)?,
                        version: row.get(7)?,
                        last_deploy_at: row.get(8)?,
                        health_status: row.get(9)?,
                        error_msg: row.get(10)?,
                        monthly_cost_usd: cost,
                    },
                ))
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    // Always return every registry unit (with DB data merged in).
    units
        .iter()
        .map(|u| {
            let mut s = db_rows
                .get(u.id.as_str())
                .cloned()
                .unwrap_or(DeploymentSnapshot {
                    unit: u.id.clone(),
                    display: u.display.clone(),
                    url: u.url.clone(),
                    snapshot_at: 0,
                    machine_id: String::new(),
                    region: String::new(),
                    vm_size: String::new(),
                    state: String::new(),
                    image: String::new(),
                    version: String::new(),
                    last_deploy_at: String::new(),
                    health_status: "unknown".to_string(),
                    error_msg: String::new(),
                    monthly_cost_usd: 0.0,
                });
            s.display = u.display.clone();
            s.url = u.url.clone();
            s
        })
        .collect()
}

// ---------------------------------------------------------------------------
// GET /api/v1/admin/deployments
// ---------------------------------------------------------------------------

pub async fn list_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DeploymentListResponse>, Response> {
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

    let units = {
        let storage = state.core.storage.lock();
        load_snapshots(&storage)
    };

    let total_monthly_cost_usd = units.iter().map(|u| u.monthly_cost_usd).sum();
    Ok(Json(DeploymentListResponse {
        units,
        refreshed_at: Utc::now().to_rfc3339(),
        total_monthly_cost_usd,
    }))
}

// ---------------------------------------------------------------------------
// POST /api/v1/admin/deployments/refresh
// ---------------------------------------------------------------------------

pub async fn refresh_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DeploymentListResponse>, Response> {
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

    // Run the probe inline — holds no DB lock during HTTP calls.
    if let Err(e) = crate::platform::deployment_snapshot_worker::tick(&state).await {
        tracing::warn!("deployment refresh failed: {e}");
    }

    let units = {
        let storage = state.core.storage.lock();
        load_snapshots(&storage)
    };

    let total_monthly_cost_usd = units.iter().map(|u| u.monthly_cost_usd).sum();
    Ok(Json(DeploymentListResponse {
        units,
        refreshed_at: Utc::now().to_rfc3339(),
        total_monthly_cost_usd,
    }))
}

// ---------------------------------------------------------------------------
// GET /admin/deployments (HTML page)
// ---------------------------------------------------------------------------

pub async fn serve_deployments_page(headers: HeaderMap) -> Response {
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
            "<html><body><h1>403 Proibido</h1></body></html>",
        )
            .into_response();
    }

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        DEPLOYMENTS_PAGE_HTML,
    )
        .into_response()
}

const DEPLOYMENTS_PAGE_HTML: &str = include_str!("../../static/variants/a/deployments.html");

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/deployments", get(list_handler))
        .route("/deployments/refresh", post(refresh_handler))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    #[test]
    fn estimate_monthly_cost_base_case() {
        // shared-cpu-1x 256MB always-on → ~$1.94/mo
        let cost = estimate_monthly_cost(1.0, 256, 1.0);
        assert!(
            (cost - 1.94).abs() < 0.05,
            "expected ~$1.94 but got {cost:.2}"
        );
    }

    #[test]
    fn estimate_monthly_cost_scales_with_ram() {
        // 512MB should be ~2× 256MB
        let cost_256 = estimate_monthly_cost(1.0, 256, 1.0);
        let cost_512 = estimate_monthly_cost(1.0, 512, 1.0);
        assert!((cost_512 / cost_256 - 2.0).abs() < 0.01);
    }

    #[test]
    fn estimate_monthly_cost_uptime_ratio_zero() {
        let cost = estimate_monthly_cost(1.0, 512, 0.0);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn vm_size_factors_shared_1x_512mb() {
        let (msf, ram) = vm_size_factors("shared-1x-512mb");
        assert_eq!(msf, 1.0);
        assert_eq!(ram, 512);
    }

    #[test]
    fn vm_size_factors_empty_defaults() {
        let (msf, ram) = vm_size_factors("");
        assert_eq!(msf, 1.0);
        assert_eq!(ram, 256);
    }

    #[test]
    fn state_uptime_ratio_values() {
        assert_eq!(state_uptime_ratio("started"), 1.0);
        assert!(state_uptime_ratio("stopped") < 0.5);
        assert!(state_uptime_ratio("suspended") < state_uptime_ratio("stopped"));
        assert_eq!(state_uptime_ratio("unknown"), 1.0);
    }

    /// CO-338: `load_snapshots` now reads the unit registry from `Storage`
    /// (surface rows) rather than a bare connection. A fresh Storage has no
    /// `universes.surface_dns` rows, so units come purely from the registry seed.
    fn make_test_storage() -> Storage {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        std::mem::forget(dir); // keep the SQLite files alive for the test
        storage
    }

    #[test]
    fn load_snapshots_returns_five_rows_when_db_empty() {
        let storage = make_test_storage();
        let rows = load_snapshots(&storage);
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn load_snapshots_merges_db_data() {
        let storage = make_test_storage();
        storage
            .conn()
            .execute(
                "INSERT INTO deployment_snapshots (unit, snapshot_at, version, health_status)
             VALUES ('co', 1234567890, '2.28.0', 'ok')",
                [],
            )
            .unwrap();

        let rows = load_snapshots(&storage);
        assert_eq!(rows.len(), 5);

        let co = rows.iter().find(|r| r.unit == "co").unwrap();
        assert_eq!(co.version, "2.28.0");
        assert_eq!(co.health_status, "ok");
        assert_eq!(co.snapshot_at, 1234567890);
    }

    #[test]
    fn load_snapshots_fills_display_and_url() {
        let storage = make_test_storage();
        let rows = load_snapshots(&storage);
        for row in &rows {
            assert!(!row.display.is_empty(), "unit {} has no display", row.unit);
            assert!(!row.url.is_empty(), "unit {} has no url", row.unit);
        }
    }
}
