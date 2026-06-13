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
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

use crate::eda::event::{Event, Visibility};
use crate::github_auth::{self, GitHubAdmin};
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
    pub resolved_by: Option<String>,
}

impl From<crate::storage::security::SecurityFindingRow> for FindingResponse {
    fn from(r: crate::storage::security::SecurityFindingRow) -> Self {
        Self {
            id: r.id,
            pr_number: r.pr_number,
            severity: r.severity,
            category: r.category,
            file_path: r.file_path,
            line_start: r.line_start,
            line_end: r.line_end,
            description: r.description,
            cwe: r.cwe,
            cve_match: r.cve_match,
            suggested_patch: r.suggested_patch,
            detected_at: r.detected_at,
            resolved_at: r.resolved_at,
            resolution_kind: r.resolution_kind,
            resolution_pr: r.resolution_pr,
            resolved_by: r.resolved_by,
        }
    }
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
    // CO-388 (security hardening): clamp LIMIT to a positive, bounded range.
    // SQLite treats LIMIT -1 as "unlimited", so a negative `limit` would be a
    // trivial DoS lever; floor at 1 and cap at 200.
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let resolved_flag: Option<i64> = q.resolved.map(|b| if b { 1 } else { 0 });

    let findings: Vec<FindingResponse> = match storage.list_security_findings(
        q.severity.as_deref(),
        resolved_flag,
        limit,
        offset,
    ) {
        Ok(rows) => rows.into_iter().map(FindingResponse::from).collect(),
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    Json(json!({"findings": findings, "limit": limit, "offset": offset})).into_response()
}

async fn get_finding(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let storage = state.core.storage.lock();
    match storage.get_security_finding(&id) {
        Ok(Some(f)) => Json(FindingResponse::from(f)).into_response(),
        Ok(None) => (
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
    // CO-388 (security hardening): the acting admin. The `require_github_admin`
    // middleware inserts this into request extensions; resolving a finding can
    // open the release gate, so we record WHO did it for the audit trail.
    actor: GitHubAdmin,
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

    let actor_login = actor.0;
    let now = chrono::Utc::now().to_rfc3339();
    let storage = state.core.storage.lock();
    let result = storage.resolve_security_finding(
        &id,
        &now,
        &body.resolution_kind,
        body.resolution_pr,
        &actor_login,
    );

    match result {
        Ok(0) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"error": "finding not found or already resolved"})),
        )
            .into_response(),
        Ok(_) => {
            drop(storage);
            info!(
                "security: finding {id} resolved as '{}' by admin '{actor_login}'",
                body.resolution_kind
            );
            state.core.eda_bus.publish(Event::new(
                "security.finding_resolved",
                None,
                None,
                json!({
                    "finding_id": id,
                    "resolution_kind": body.resolution_kind,
                    "resolution_pr": body.resolution_pr,
                    "resolved_by": actor_login,
                }),
                Visibility::System,
            ));
            Json(json!({"ok": true, "resolved_at": now, "resolved_by": actor_login}))
                .into_response()
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
    let unresolved_count: i64 = storage.count_unresolved_findings();
    let high_critical_count: i64 = storage.count_unresolved_high_critical_findings();

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
    // CO-388 (security hardening): record which admin triggered the scan /
    // activated the emergency override.
    actor: GitHubAdmin,
    Json(body): Json<ScanBody>,
) -> impl IntoResponse {
    let actor_login = actor.0;
    // Emergency override: log prominently before proceeding.
    if body.ignore_security_findings.unwrap_or(false) {
        tracing::error!(
            "security: --ignore-security-findings override activated by admin '{actor_login}' — \
             operator manually bypassing security gate"
        );
        state.core.eda_bus.publish(Event::new(
            "security.override_activated",
            None,
            None,
            json!({
                "override": "ignore-security-findings",
                "actor": actor_login,
                "icon": "⚠️",
            }),
            Visibility::System,
        ));
    }

    // CO-388 (security hardening): enforce the daily scan cap with a DB-backed
    // counter. `build_backend()` constructs a fresh backend per request, so the
    // backend's own in-memory counter never tripped — the limit must live in
    // shared, persistent state.
    let max_scans = std::env::var("CO_SECURITY_MAX_SCANS_PER_DAY")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(50);
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    {
        let storage = state.core.storage.lock();
        // Atomic upsert-then-read: increment today's count and read it back.
        let new_count: i64 = match storage.increment_security_scan_count(&today) {
            Ok(c) => c,
            Err(e) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        };
        if new_count > max_scans {
            tracing::warn!(
                "security: daily scan limit ({max_scans}) reached on {today} \
                 — rejecting scan request"
            );
            return (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "error": "daily scan limit reached",
                    "max_scans_per_day": max_scans,
                    "scans_today": new_count,
                })),
            )
                .into_response();
        }
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
        // CO-388 (security hardening): enforce GitHub admin auth on EVERY
        // security endpoint. Without this layer the whole findings API —
        // including `resolve_finding` (which opens the release gate) and
        // `trigger_scan` — was reachable unauthenticated. Mirrors the
        // pattern used by gestao_routes / webhook_routes.
        .layer(middleware::from_fn(github_auth::require_github_admin))
}
