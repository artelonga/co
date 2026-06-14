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
}

#[derive(Debug, Serialize)]
pub struct DeploymentListResponse {
    pub units: Vec<DeploymentSnapshot>,
    pub refreshed_at: String,
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
                Ok((
                    row.get::<_, String>(0)?,
                    DeploymentSnapshot {
                        unit: row.get(0)?,
                        display: String::new(),
                        url: String::new(),
                        snapshot_at: row.get(1)?,
                        machine_id: row.get(2)?,
                        region: row.get(3)?,
                        vm_size: row.get(4)?,
                        state: row.get(5)?,
                        image: row.get(6)?,
                        version: row.get(7)?,
                        last_deploy_at: row.get(8)?,
                        health_status: row.get(9)?,
                        error_msg: row.get(10)?,
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

    Ok(Json(DeploymentListResponse {
        units,
        refreshed_at: Utc::now().to_rfc3339(),
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

    Ok(Json(DeploymentListResponse {
        units,
        refreshed_at: Utc::now().to_rfc3339(),
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
    fn load_snapshots_returns_six_rows_when_db_empty() {
        let storage = make_test_storage();
        let rows = load_snapshots(&storage);
        assert_eq!(rows.len(), 6);
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
        assert_eq!(rows.len(), 6);

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
