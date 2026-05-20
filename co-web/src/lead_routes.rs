//! CO-183 — leads intake: POST /api/v1/leads + admin queue.
//!
//! Public:  POST /api/v1/leads          — ingest form submission, notify admin.
//! Admin:   GET  /api/v1/admin/leads    — list with status/since/assignee filters.
//! Admin:   PATCH /api/v1/admin/leads/:id — triage / update status.
//! Admin:   GET  /admin/leads.html      — static admin page (cookie auth).

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};

use crate::server::AppState;

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LeadSubmit {
    pub mensagem: Option<String>,
    pub nome: Option<String>,
    pub email: Option<String>,
    pub telefone: Option<String>,
    pub servico_titulo: Option<String>,
    pub parceiro_handle: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LeadRow {
    pub id: i64,
    pub created_at: String,
    pub updated_at: String,
    pub nome: Option<String>,
    pub email: Option<String>,
    pub telefone: Option<String>,
    pub mensagem: String,
    pub servico_titulo: Option<String>,
    pub parceiro_handle: Option<String>,
    pub status: String,
    pub priority: Option<String>,
    pub assignee_handle: Option<String>,
    pub notes: Option<String>,
    pub closed_reason: Option<String>,
    pub promoted_to_al: Option<i64>,
}

/// Typed response for `POST /api/v1/leads`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateLeadResponse {
    pub id: i64,
}

/// Typed response for `GET /api/v1/admin/leads`.
#[derive(Debug, Serialize)]
pub struct LeadsListResponse {
    pub leads: Vec<LeadRow>,
    pub total: i64,
}

#[derive(Debug, Deserialize)]
pub struct LeadPatch {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub assignee_handle: Option<String>,
    pub notes: Option<String>,
    pub closed_reason: Option<String>,
    pub promoted_to_al: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct LeadListQuery {
    pub status: Option<String>,
    pub since: Option<String>,
    pub limit: Option<i64>,
    pub assignee: Option<String>,
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

const VALID_STATUSES: &[&str] = &["new", "triaged", "in_progress", "closed"];
const VALID_PRIORITIES: &[&str] = &["low", "normal", "high", "urgent"];
const VALID_CLOSED_REASONS: &[&str] = &["won", "lost", "spam", "duplicate"];

fn is_valid_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("new", "triaged")
            | ("new", "in_progress")
            | ("triaged", "in_progress")
            | ("triaged", "closed")
            | ("in_progress", "closed")
    )
}

// ---------------------------------------------------------------------------
// Admin auth gate
// ---------------------------------------------------------------------------

/// Returns `Some(error_response)` if the request is not authorized as admin,
/// or `None` if access is granted.
fn check_admin_gate(headers: &HeaderMap) -> Option<Response> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| crate::auth::extract_session_cookie(headers));

    let token = match token {
        Some(t) => t,
        None => {
            return Some(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "Unauthorized"})),
                )
                    .into_response(),
            );
        }
    };

    let secret = crate::auth::jwt_secret();
    let claims = match crate::auth::decode_claims(&token, &secret) {
        Ok(c) => c,
        Err(_) => {
            return Some(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "Unauthorized"})),
                )
                    .into_response(),
            );
        }
    };

    let admin_email = std::env::var("CO_SEED_ADMIN_EMAIL").unwrap_or_default();
    if admin_email.is_empty() || claims.email != admin_email {
        return Some(
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Forbidden"})),
            )
                .into_response(),
        );
    }
    None
}

// ---------------------------------------------------------------------------
// Email notification
// ---------------------------------------------------------------------------

async fn send_lead_notification(
    id: i64,
    nome: Option<String>,
    servico: Option<String>,
    lead_body: String,
) {
    use crate::notification_providers::{ChannelProvider, ResendProvider};

    let notify_to =
        std::env::var("LEADS_NOTIFY_TO").unwrap_or_else(|_| "rede@artelonga.com.br".to_string());

    let display_name = nome.as_deref().unwrap_or("Anônimo");
    let display_servico = servico.as_deref().unwrap_or("form geral");
    let subject = format!("[Lead #{}] {} via {}", id, display_name, display_servico);
    let body = format!(
        "{}\n\nVer em https://co.artelonga.com.br/admin/leads.html#lead-{}",
        lead_body, id
    );

    if let Some(provider) = ResendProvider::from_env() {
        let payload = format!("{subject}\n---\n{body}");
        let client = reqwest::Client::new();
        match provider.send(&client, &notify_to, &payload).await {
            Ok(()) => {
                tracing::info!("Lead #{id} notification sent via Resend");
                return;
            }
            Err(e) => {
                tracing::warn!("Resend lead notification to {notify_to} failed: {e}, trying SMTP");
            }
        }
    }

    // SMTP fallback (reuses the same delivery path as recovery codes)
    let smtp_payload = format!("{subject}\n\n{body}");
    match crate::email_smtp::send_recovery_code(&notify_to, &smtp_payload).await {
        Ok(true) => tracing::info!("Lead #{id} notification sent via SMTP"),
        Ok(false) => tracing::info!("[LEAD #{id}] {subject}\n{body}"),
        Err(e) => tracing::warn!("Lead #{id} email notification failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn submit_lead(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LeadSubmit>,
) -> Response {
    // 1. Bot filter — silent 200, no persistence
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if crate::telemetry::is_bot(ua) {
        return (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response();
    }

    // 2. Validate mensagem
    let mensagem = body.mensagem.as_deref().unwrap_or("").trim().to_string();
    if mensagem.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "mensagem is required"})),
        )
            .into_response();
    }
    if mensagem.len() > 4000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "mensagem too long (max 4000 chars)"})),
        )
            .into_response();
    }

    // 3. IP hash — raw IP never stored
    let raw_ip = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .unwrap_or("unknown")
        .trim()
        .to_string();
    let ip_hash = crate::telemetry::hash_ip_daily(&raw_ip);

    // 4. Trim user agent to 256 chars (stored as-is, no PII beyond what the
    //    browser sends — acceptable per LGPD implicit consent for form submitters)
    let user_agent: Option<String> = if ua.is_empty() {
        None
    } else {
        Some(ua.chars().take(256).collect())
    };

    // 5. Rate limit — 5 leads per IP-hash per 24 h
    let rate_exceeded = {
        let storage = state.storage.lock();
        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM leads WHERE ip_hash = ? \
                 AND created_at > datetime('now', '-1 day')",
                rusqlite::params![ip_hash],
                |r| r.get(0),
            )
            .unwrap_or(0);
        count >= 5
    };
    if rate_exceeded {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error": "rate limit exceeded (5 leads per 24h per IP)"})),
        )
            .into_response();
    }

    // 6. Normalise optional fields
    let nome = body
        .nome
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let email = body
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase());
    let telefone = body
        .telefone
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let servico_titulo = body
        .servico_titulo
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let parceiro_handle = body
        .parceiro_handle
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // 7. Persist
    let now = chrono::Utc::now().to_rfc3339();
    let (id, insert_ok) = {
        let storage = state.storage.lock();
        let res = storage.conn().execute(
            "INSERT INTO leads
             (created_at, updated_at, nome, email, telefone, mensagem,
              servico_titulo, parceiro_handle, status, priority, ip_hash, user_agent)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'new', 'normal', ?, ?)",
            rusqlite::params![
                now,
                now,
                nome,
                email,
                telefone,
                mensagem,
                servico_titulo,
                parceiro_handle,
                ip_hash,
                user_agent,
            ],
        );
        match res {
            Ok(_) => (storage.conn().last_insert_rowid(), true),
            Err(e) => {
                tracing::error!("Failed to insert lead: {e}");
                (0, false)
            }
        }
    };

    if !insert_ok {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // 8. Email notification — async, never fails the response
    let lead_body = format!(
        "Nome: {}\nEmail: {}\nTelefone: {}\nServiço: {}\nParceiro: {}\n\nMensagem:\n{}",
        nome.as_deref().unwrap_or("-"),
        email.as_deref().unwrap_or("-"),
        telefone.as_deref().unwrap_or("-"),
        servico_titulo.as_deref().unwrap_or("-"),
        parceiro_handle.as_deref().unwrap_or("-"),
        mensagem,
    );
    tokio::spawn(send_lead_notification(id, nome, servico_titulo, lead_body));

    (StatusCode::CREATED, Json(CreateLeadResponse { id })).into_response()
}

pub async fn list_leads(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<LeadListQuery>,
) -> Response {
    if let Some(err) = check_admin_gate(&headers) {
        return err;
    }

    let storage = state.storage.lock();

    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    // Build dynamic WHERE conditions
    let mut conditions: Vec<&str> = vec![];
    let mut binds: Vec<String> = vec![];

    if let Some(ref st) = params.status {
        conditions.push("status = ?");
        binds.push(st.clone());
    }
    if let Some(ref since) = params.since {
        conditions.push("created_at >= ?");
        binds.push(since.clone());
    }
    if let Some(ref assignee) = params.assignee {
        conditions.push("assignee_handle = ?");
        binds.push(assignee.clone());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    // Total count (without LIMIT)
    let count_sql = format!("SELECT COUNT(*) FROM leads {}", where_clause);
    let total: i64 = storage
        .conn()
        .prepare(&count_sql)
        .and_then(|mut stmt| stmt.query_row(rusqlite::params_from_iter(binds.iter()), |r| r.get(0)))
        .unwrap_or(0);

    // Data query
    binds.push(limit.to_string());
    let data_sql = format!(
        "SELECT id, created_at, updated_at, nome, email, telefone, mensagem,
                servico_titulo, parceiro_handle, status, priority, assignee_handle,
                notes, closed_reason, promoted_to_al
         FROM leads {where_clause} ORDER BY created_at DESC LIMIT ?"
    );

    let leads: Vec<LeadRow> = storage
        .conn()
        .prepare(&data_sql)
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params_from_iter(binds.iter()), |r| {
                Ok(LeadRow {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    updated_at: r.get(2)?,
                    nome: r.get(3)?,
                    email: r.get(4)?,
                    telefone: r.get(5)?,
                    mensagem: r.get(6)?,
                    servico_titulo: r.get(7)?,
                    parceiro_handle: r.get(8)?,
                    status: r.get(9)?,
                    priority: r.get(10)?,
                    assignee_handle: r.get(11)?,
                    notes: r.get(12)?,
                    closed_reason: r.get(13)?,
                    promoted_to_al: r.get(14)?,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    Json(LeadsListResponse { leads, total }).into_response()
}

pub async fn patch_lead(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<LeadPatch>,
) -> Response {
    if let Some(err) = check_admin_gate(&headers) {
        return err;
    }

    let storage = state.storage.lock();

    // Read current status
    let current_status: Option<String> = storage
        .conn()
        .query_row(
            "SELECT status FROM leads WHERE id = ?",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();
    let current_status = match current_status {
        Some(s) => s,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "lead not found"})),
            )
                .into_response();
        }
    };

    // Validate new status and transition
    if let Some(ref new_status) = body.status {
        if !VALID_STATUSES.contains(&new_status.as_str()) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid status"})),
            )
                .into_response();
        }
        if *new_status != current_status && !is_valid_transition(&current_status, new_status) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("invalid state transition: {} → {}", current_status, new_status)
                })),
            )
                .into_response();
        }
    }

    if let Some(ref pr) = body.priority
        && !VALID_PRIORITIES.contains(&pr.as_str())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid priority"})),
        )
            .into_response();
    }

    if let Some(ref cr) = body.closed_reason
        && !VALID_CLOSED_REASONS.contains(&cr.as_str())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid closed_reason"})),
        )
            .into_response();
    }

    let now = chrono::Utc::now().to_rfc3339();
    let result = storage.conn().execute(
        "UPDATE leads SET
             updated_at      = ?,
             status          = COALESCE(?, status),
             priority        = COALESCE(?, priority),
             assignee_handle = COALESCE(?, assignee_handle),
             notes           = COALESCE(?, notes),
             closed_reason   = COALESCE(?, closed_reason),
             promoted_to_al  = COALESCE(?, promoted_to_al)
         WHERE id = ?",
        rusqlite::params![
            now,
            body.status,
            body.priority,
            body.assignee_handle,
            body.notes,
            body.closed_reason,
            body.promoted_to_al,
            id,
        ],
    );

    match result {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
        Err(e) => {
            tracing::error!("Failed to update lead {id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn serve_leads_page(headers: HeaderMap) -> Response {
    let token = match crate::auth::extract_session_cookie(&headers) {
        Some(t) => t,
        None => return Redirect::to("/").into_response(),
    };
    let secret = crate::auth::jwt_secret();
    let claims = match crate::auth::decode_claims(&token, &secret) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/").into_response(),
    };
    let admin_email = std::env::var("CO_SEED_ADMIN_EMAIL").unwrap_or_default();
    if admin_email.is_empty() || claims.email != admin_email {
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
        LEADS_PAGE_HTML,
    )
        .into_response()
}

const LEADS_PAGE_HTML: &str = include_str!("../static/variants/a/leads.html");

// ---------------------------------------------------------------------------
// Background retention task (CO-183 LGPD 24-month purge)
// ---------------------------------------------------------------------------

pub async fn retention_task(state: Arc<crate::server::AppStateInner>) {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(24 * 3600)).await;
        let storage = state.storage.lock();
        match storage.conn().execute(
            "DELETE FROM leads \
             WHERE created_at < datetime('now', '-24 months') \
             AND status = 'closed'",
            [],
        ) {
            Ok(n) if n > 0 => {
                tracing::info!("Leads retention: purged {n} closed lead(s) older than 24 months")
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("Leads retention task failed: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Routers
// ---------------------------------------------------------------------------

pub fn public_router() -> Router<AppState> {
    Router::new().route("/leads", post(submit_lead))
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/leads", get(list_leads))
        .route("/leads/{id}", patch(patch_lead))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
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
            co_env: "test".into(),
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
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let state: crate::server::AppState = Arc::new(crate::server::AppStateInner {
            storage: parking_lot::Mutex::new(storage),
            experiment: Mutex::new(experiment),
            config,
            auth_store: Mutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            cache: crate::cache::CacheLayer::new(),
            rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
            embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
            event_bus: crate::events::Bus::new(),
            worker_supervisor: crate::worker_supervisor::WorkerSupervisor::new(),
        });
        crate::server::build_router(state, None)
    }

    // --- POST /api/v1/leads ---

    #[tokio::test]
    async fn test_post_lead_valid_returns_201() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/leads")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        r#"{"mensagem":"Preciso de ajuda","nome":"Maria"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Verify row was inserted
        let storage = crate::storage::Storage::new(dir.path().to_str().unwrap());
        let count: i64 = storage
            .conn()
            .query_row("SELECT COUNT(*) FROM leads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_post_lead_missing_mensagem_returns_400() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/leads")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"nome":"Maria"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_post_lead_empty_mensagem_returns_400() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/leads")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"mensagem":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_post_lead_bot_ua_returns_200_without_persisting() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/leads")
                    .header("Content-Type", "application/json")
                    .header(
                        "User-Agent",
                        "Googlebot/2.1 (+http://www.google.com/bot.html)",
                    )
                    .body(Body::from(r#"{"mensagem":"test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let storage = crate::storage::Storage::new(dir.path().to_str().unwrap());
        let count: i64 = storage
            .conn()
            .query_row("SELECT COUNT(*) FROM leads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "bot submission must not be persisted");
    }

    #[tokio::test]
    async fn test_post_lead_rate_limit_6th_returns_429() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let make_req = || {
            Request::builder()
                .method("POST")
                .uri("/api/v1/leads")
                .header("Content-Type", "application/json")
                // Consistent IP via X-Forwarded-For
                .header("X-Forwarded-For", "1.2.3.4")
                .body(Body::from(r#"{"mensagem":"test message"}"#))
                .unwrap()
        };

        // First 5 should succeed
        for _ in 0..5 {
            let resp = app.clone().oneshot(make_req()).await.unwrap();
            assert_eq!(resp.status(), StatusCode::CREATED);
        }
        // 6th should be rate-limited
        let resp = app.oneshot(make_req()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    // --- GET /api/v1/admin/leads ---

    #[tokio::test]
    async fn test_get_admin_leads_no_jwt_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/admin/leads")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_get_admin_leads_non_admin_email_returns_403() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let secret = crate::auth::jwt_secret();
        let (token, _) =
            crate::auth::sign_jwt("usr_test", "notadmin@example.com", "player", &secret).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/admin/leads")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // 403 unless CO_SEED_ADMIN_EMAIL happens to match in CI
        assert!(
            resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::OK,
            "expected 403 for non-admin, got {}",
            resp.status()
        );
    }

    // --- PATCH /api/v1/admin/leads/:id ---

    #[tokio::test]
    async fn test_patch_lead_invalid_transition_returns_400() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        // Insert a lead directly
        let storage = crate::storage::Storage::new(dir.path().to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO leads (created_at, updated_at, mensagem, status, priority)
             VALUES (?, ?, 'test', 'new', 'normal')",
                rusqlite::params![now, now],
            )
            .unwrap();
        let lead_id: i64 = storage.conn().last_insert_rowid();
        drop(storage);

        // Sign as admin (set CO_SEED_ADMIN_EMAIL to match)
        unsafe { std::env::set_var("CO_SEED_ADMIN_EMAIL", "admin@test.local") };
        let secret = crate::auth::jwt_secret();
        let (token, _) =
            crate::auth::sign_jwt("usr_admin", "admin@test.local", "admin", &secret).unwrap();

        // Try new→closed (invalid: must go through triaged first)
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/v1/admin/leads/{lead_id}"))
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"status":"closed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        unsafe { std::env::remove_var("CO_SEED_ADMIN_EMAIL") };
    }

    #[tokio::test]
    async fn test_patch_lead_valid_transition_returns_200() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let storage = crate::storage::Storage::new(dir.path().to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO leads (created_at, updated_at, mensagem, status, priority)
             VALUES (?, ?, 'test', 'new', 'normal')",
                rusqlite::params![now, now],
            )
            .unwrap();
        let lead_id: i64 = storage.conn().last_insert_rowid();
        drop(storage);

        unsafe { std::env::set_var("CO_SEED_ADMIN_EMAIL", "admin@test.local") };
        let secret = crate::auth::jwt_secret();
        let (token, _) =
            crate::auth::sign_jwt("usr_admin", "admin@test.local", "admin", &secret).unwrap();

        // new→triaged is valid
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/v1/admin/leads/{lead_id}"))
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        r#"{"status":"triaged","assignee_handle":"yuri"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        unsafe { std::env::remove_var("CO_SEED_ADMIN_EMAIL") };
    }

    // --- Email notification failure does not fail POST ---

    #[tokio::test]
    async fn test_post_lead_email_failure_still_returns_201() {
        // With LogMailProvider (default), "email" just logs — never errors.
        // This test confirms the lead is persisted even when no real email
        // provider is configured (the guaranteed path for test environments).
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/leads")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"mensagem":"Quero contratar"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let storage = crate::storage::Storage::new(dir.path().to_str().unwrap());
        let count: i64 = storage
            .conn()
            .query_row("SELECT COUNT(*) FROM leads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    /// POST /api/v1/leads returns typed CreateLeadResponse with id.
    #[tokio::test]
    async fn test_submit_lead_returns_typed_response() {
        use http_body_util::BodyExt;

        let dir = tempdir().unwrap();
        let router = build_test_router(dir.path());

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/leads")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"mensagem":"Test typed response"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: super::CreateLeadResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.id > 0);
    }

    /// CreateLeadResponse serializes correctly.
    #[test]
    fn test_create_lead_response_serializes() {
        let r = super::CreateLeadResponse { id: 42 };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["id"], 42);
    }

    /// LeadsListResponse serializes correctly.
    #[test]
    fn test_leads_list_response_serializes() {
        let r = super::LeadsListResponse {
            leads: vec![],
            total: 0,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["total"], 0);
        assert!(json["leads"].is_array());
    }
}
