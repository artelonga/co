//! CO-388: Security findings REST API (admin SPA under /gestao).
//!
//! Endpoints:
//!   GET  /api/v1/gestao/security/findings          — list findings
//!   GET  /api/v1/gestao/security/findings/:id      — get a finding
//!   PATCH /api/v1/gestao/security/findings/:id     — resolve a finding
//!   GET  /api/v1/gestao/security/scan/status       — daily scan count + backend
//!   POST /api/v1/gestao/security/scan              — trigger manual scan
//!
//! All endpoints require GitHub admin auth (via `AllowedAdmins` extension).

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

use crate::eda::event::{Event, Visibility};
use crate::security::audit::Severity;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FindingResponse {
    pub id: String,
    pub pr_number: i64,
    pub severity: String,
    pub category: String,
    pub file_path: String,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub description: String,
    pub cwe: Option<String>,
    pub cve_match: Option<String>,
    pub suggested_patch: Option<String>,
    pub detected_at: String,
    pub resolved_at: Option<String>,
    pub resolution_kind: Option<String>,
    pub resolution_pr: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub severity: Option<String>,
    pub resolved: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveBody {
    /// One of: patched | accepted-risk | false-positive | wont-fix
    pub resolution_kind: String,
    pub resolution_pr: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ScanBody {
    /// Base git ref for diff scan.
    pub base_ref: Option<String>,
    /// Head git ref for diff scan.
    pub head_ref: Option<String>,
    /// If true, scan the full repo instead of a diff.
    pub full_scan: Option<bool>,
    /// Emergency override — log + alert before proceeding.
    pub ignore_security_findings: Option<bool>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_findings(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let storage = state.core.storage.lock();
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let mut sql = "SELECT id, pr_number, severity, category, file_path, \
                   line_start, line_end, description, cwe, cve_match, \
                   suggested_patch, detected_at, resolved_at, resolution_kind, \
                   resolution_pr \
                   FROM security_findings WHERE 1=1"
        .to_string();

    if let Some(sev) = &q.severity {
        sql.push_str(&format!(" AND severity = '{}'", sev.replace('\'', "''")));
    }
    match q.resolved {
        Some(true) => sql.push_str(" AND resolved_at IS NOT NULL"),
        Some(false) => sql.push_str(" AND resolved_at IS NULL"),
        None => {}
    }
    sql.push_str(&format!(
        " ORDER BY detected_at DESC LIMIT {limit} OFFSET {offset}"
    ));

    let mut stmt = match storage.conn().prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    let findings: Vec<FindingResponse> = match stmt.query_map([], |row| {
        Ok(FindingResponse {
            id: row.get(0)?,
            pr_number: row.get(1)?,
            severity: row.get(2)?,
            category: row.get(3)?,
            file_path: row.get(4)?,
            line_start: row.get(5)?,
            line_end: row.get(6)?,
            description: row.get(7)?,
            cwe: row.get(8)?,
            cve_match: row.get(9)?,
            suggested_patch: row.get(10)?,
            detected_at: row.get(11)?,
            resolved_at: row.get(12)?,
            resolution_kind: row.get(13)?,
            resolution_pr: row.get(14)?,
        })
    }) {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => vec![],
    };

    Json(json!({"findings": findings, "limit": limit, "offset": offset})).into_response()
}

async fn get_finding(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let storage = state.core.storage.lock();
    let result = storage.conn().query_row(
        "SELECT id, pr_number, severity, category, file_path, \
         line_start, line_end, description, cwe, cve_match, \
         suggested_patch, detected_at, resolved_at, resolution_kind, resolution_pr \
         FROM security_findings WHERE id = ?1",
        params![id],
        |row| {
            Ok(FindingResponse {
                id: row.get(0)?,
                pr_number: row.get(1)?,
                severity: row.get(2)?,
                category: row.get(3)?,
                file_path: row.get(4)?,
                line_start: row.get(5)?,
                line_end: row.get(6)?,
                description: row.get(7)?,
                cwe: row.get(8)?,
                cve_match: row.get(9)?,
                suggested_patch: row.get(10)?,
                detected_at: row.get(11)?,
                resolved_at: row.get(12)?,
                resolution_kind: row.get(13)?,
                resolution_pr: row.get(14)?,
            })
        },
    );

    match result {
        Ok(f) => Json(f).into_response(),
        Err(rusqlite::Error::QueryReturnedNoRows) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"error": "finding not found"})),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn resolve_finding(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ResolveBody>,
) -> impl IntoResponse {
    let valid_kinds = ["patched", "accepted-risk", "false-positive", "wont-fix"];
    if !valid_kinds.contains(&body.resolution_kind.as_str()) {
        return (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "resolution_kind must be one of: patched, accepted-risk, false-positive, wont-fix"
            })),
        )
            .into_response();
    }

    let now = chrono::Utc::now().to_rfc3339();
    let storage = state.core.storage.lock();
    let result = storage.conn().execute(
        "UPDATE security_findings \
         SET resolved_at = ?1, resolution_kind = ?2, resolution_pr = ?3 \
         WHERE id = ?4 AND resolved_at IS NULL",
        params![now, body.resolution_kind, body.resolution_pr, id],
    );

    match result {
        Ok(0) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"error": "finding not found or already resolved"})),
        )
            .into_response(),
        Ok(_) => {
            drop(storage);
            state.core.eda_bus.publish(Event::new(
                "security.finding_resolved",
                None,
                None,
                json!({
                    "finding_id": id,
                    "resolution_kind": body.resolution_kind,
                    "resolution_pr": body.resolution_pr,
                }),
                Visibility::System,
            ));
            Json(json!({"ok": true, "resolved_at": now})).into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn scan_status(State(state): State<AppState>) -> impl IntoResponse {
    let storage = state.core.storage.lock();
    let unresolved_count: i64 = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM security_findings WHERE resolved_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let high_critical_count: i64 = storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM security_findings \
             WHERE resolved_at IS NULL AND severity IN ('high','critical')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let backend = std::env::var("CO_SECURITY_BACKEND").unwrap_or_else(|_| "local-grep".into());
    let max_scans = std::env::var("CO_SECURITY_MAX_SCANS_PER_DAY")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(50);

    Json(json!({
        "backend": backend,
        "max_scans_per_day": max_scans,
        "unresolved_findings": unresolved_count,
        "high_critical_unresolved": high_critical_count,
        "release_blocked": high_critical_count > 0,
    }))
    .into_response()
}

async fn trigger_scan(
    State(state): State<AppState>,
    Json(body): Json<ScanBody>,
) -> impl IntoResponse {
    // Emergency override: log prominently before proceeding.
    if body.ignore_security_findings.unwrap_or(false) {
        tracing::error!(
            "security: --ignore-security-findings override activated — \
             operator manually bypassing security gate"
        );
        state.core.eda_bus.publish(Event::new(
            "security.override_activated",
            None,
            None,
            json!({
                "override": "ignore-security-findings",
                "icon": "⚠️",
            }),
            Visibility::System,
        ));
    }

    let backend = crate::security::audit::build_backend();
    let base_ref = body.base_ref.as_deref().unwrap_or("origin/main");
    let head_ref = body.head_ref.as_deref().unwrap_or("HEAD");
    let full_scan = body.full_scan.unwrap_or(false);

    let findings_result = if full_scan {
        let repo_path = std::path::Path::new(".");
        backend.scan_full(repo_path).await
    } else {
        backend.scan_diff(base_ref, head_ref).await
    };

    match findings_result {
        Ok(findings) => {
            let count = findings.len();
            info!(
                "security: manual scan complete — {} finding(s) from {}",
                count,
                backend.name()
            );

            for finding in &findings {
                state.core.eda_bus.publish(Event::new(
                    "security.finding_detected",
                    None,
                    None,
                    json!({
                        "id": finding.id,
                        "pr_number": 0,
                        "severity": finding.severity.as_str(),
                        "category": finding.category.as_str(),
                        "file_path": finding.file_path,
                        "line_start": finding.line_range.0,
                        "line_end": finding.line_range.1,
                        "description": finding.description,
                        "cwe": finding.cwe,
                        "cve_match": finding.cve_match,
                        "icon": "🚨",
                    }),
                    Visibility::System,
                ));
            }

            let has_blockers = findings
                .iter()
                .any(|f| Severity::parse(f.severity.as_str()).blocks_merge());

            Json(json!({
                "ok": true,
                "backend": backend.name(),
                "finding_count": count,
                "blocked": has_blockers,
                "findings": findings,
            }))
            .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/findings", get(list_findings))
        .route("/findings/{id}", get(get_finding).patch(resolve_finding))
        .route("/scan/status", get(scan_status))
        .route("/scan", post(trigger_scan))
}
