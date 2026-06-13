//! CO-366: billing HTTP endpoints.
//!
//! | Method | Path | Auth | Purpose |
//! |---|---|---|---|
//! | POST | `/api/v1/me/billing/checkout` | authed | Start a checkout, returns redirect URL |
//! | GET  | `/api/v1/me/billing/status`   | authed | Current tier / plan / paid_at / provider |
//! | POST | `/api/v1/billing/webhook/{provider}` | anon (HMAC) | Provider webhook → flips tier |
//! | POST | `/api/v1/gestao/users/{id}/mark-paid` | admin (email) | Manual provider — mark paid |
//!
//! Activity (CO-361): every billing event is mirrored to the `atividades` log
//! under `entidade = "billing"` so the audit feed and EDA bus carry all four
//! events (`checkout_created`, `payment_succeeded`, `payment_failed`,
//! `subscription_canceled`).

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::storage as billing_storage;
use super::{BillingError, Plan, WebhookEvent, active_provider, provider_by_name};
use crate::admin::admin_routes::{check_admin_email, extract_claims};
use crate::atividade::{Acao, Atividade, Tipo, log_atividade};
use crate::auth::extractors::AuthedUser;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CheckoutRequest {
    plan: String,
}

#[derive(Debug, Serialize)]
struct CheckoutResponse {
    url: String,
    session_id: String,
    expires_at: String,
    provider: String,
}

#[derive(Debug, Default, Deserialize)]
struct MarkPaidRequest {
    /// Defaults to `pro` when omitted.
    plan: Option<String>,
    /// Optional external invoice / customer reference for the audit trail.
    external_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn billing_error_response(err: BillingError) -> Response {
    let (status, msg) = match &err {
        BillingError::InvalidSignature => (StatusCode::UNAUTHORIZED, err.to_string()),
        BillingError::NotConfigured(_) => (StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
        BillingError::Unsupported(_) | BillingError::InvalidPayload(_) => {
            (StatusCode::BAD_REQUEST, err.to_string())
        }
        BillingError::Provider(_) => (StatusCode::BAD_GATEWAY, err.to_string()),
    };
    (status, Json(json!({ "error": msg }))).into_response()
}

/// Returns `Some(response)` when the caller is **not** an authorized admin
/// (the response to send), or `None` when authorized. Using `Option` rather
/// than `Result<(), Response>` avoids a large `Err` variant (clippy).
fn admin_guard(headers: &HeaderMap) -> Option<Response> {
    let claims = match extract_claims(headers) {
        Ok(c) => c,
        Err(status) => {
            return Some((status, Json(json!({"error": "Unauthorized"}))).into_response());
        }
    };
    if !check_admin_email(&claims.email) {
        return Some((StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))).into_response());
    }
    None
}

// ---------------------------------------------------------------------------
// Activity helper
// ---------------------------------------------------------------------------

/// Mirror a billing event to the `atividades` audit log + EDA bus.
fn log_billing(
    state: &AppState,
    user_id: &str,
    event_name: &str,
    acao: Acao,
    tipo: Tipo,
    detail: serde_json::Value,
) {
    log_atividade(
        state.clone(),
        Atividade {
            acao,
            entidade: "billing".to_string(),
            entidade_id: Some(event_name.to_string()),
            before: None,
            after: Some(detail),
            tipo,
            user_id: Some(user_id.to_string()),
            ip: None,
            user_agent: None,
        },
    );
}

// ---------------------------------------------------------------------------
// Handlers — authed (me)
// ---------------------------------------------------------------------------

/// POST /api/v1/me/billing/checkout — body `{ "plan": "starter" }`.
async fn checkout_handler(
    State(state): State<AppState>,
    user: AuthedUser,
    Json(req): Json<CheckoutRequest>,
) -> Response {
    let Some(plan) = Plan::parse(&req.plan) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("unknown plan '{}'", req.plan) })),
        )
            .into_response();
    };

    let provider = active_provider(state.core.secrets.as_ref());
    let session = match provider.create_checkout(&user.user_id, plan).await {
        Ok(s) => s,
        Err(e) => return billing_error_response(e),
    };

    let provider_name = provider.name();
    let now = Utc::now().to_rfc3339();
    // Record the checkout in the audit table (drop the lock before logging).
    {
        let storage = state.core.storage.lock();
        if let Err(e) = billing_storage::record_billing_event(
            storage.conn(),
            &user.user_id,
            "checkout_created",
            provider_name,
            Some(&json!({ "plan": plan.as_str(), "session_id": session.session_id }).to_string()),
            &now,
        ) {
            tracing::warn!("billing checkout: record_billing_event failed: {e}");
        }
    }

    log_billing(
        &state,
        &user.user_id,
        "checkout_created",
        Acao::Criar,
        Tipo::Sucesso,
        json!({ "plan": plan.as_str(), "provider": provider_name }),
    );

    Json(CheckoutResponse {
        url: session.url,
        session_id: session.session_id,
        expires_at: session.expires_at.to_rfc3339(),
        provider: provider_name.to_string(),
    })
    .into_response()
}

/// GET /api/v1/me/billing/status.
async fn status_handler(State(state): State<AppState>, user: AuthedUser) -> Response {
    let status = {
        let storage = state.core.storage.lock();
        billing_storage::get_billing_status(storage.conn(), &user.user_id)
    };
    match status {
        Ok(Some(s)) => Json(s).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "user not found" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("billing status query failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "billing status unavailable" })),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Handler — webhook (anon, HMAC-verified)
// ---------------------------------------------------------------------------

/// POST /api/v1/billing/webhook/{provider}. The signature header is provider-
/// agnostic (`X-Webhook-Signature`, falling back to `X-Hostinger-Signature`).
async fn webhook_handler(
    State(state): State<AppState>,
    Path(provider_name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(provider) = provider_by_name(&provider_name, state.core.secrets.as_ref()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("unknown provider '{provider_name}'") })),
        )
            .into_response();
    };

    let signature = headers
        .get("x-webhook-signature")
        .or_else(|| headers.get("x-hostinger-signature"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let event = match provider.verify_webhook(&body, signature) {
        Ok(ev) => ev,
        Err(e) => return billing_error_response(e),
    };

    let provider_label = provider.name();
    let now = Utc::now().to_rfc3339();
    let user_id = event.user_id().to_string();
    let event_type = event.event_type();

    // Apply the tier change + audit-record inside one lock scope, then drop it.
    {
        let storage = state.core.storage.lock();
        let conn = storage.conn();
        match &event {
            WebhookEvent::PaymentSucceeded {
                plan, amount_cents, ..
            } => {
                if let Err(e) = billing_storage::mark_user_paid(
                    conn,
                    &user_id,
                    *plan,
                    provider_label,
                    None,
                    &now,
                ) {
                    tracing::error!("billing webhook: mark_user_paid failed: {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "could not update tier" })),
                    )
                        .into_response();
                }
                let _ = billing_storage::record_billing_event(
                    conn,
                    &user_id,
                    event_type,
                    provider_label,
                    Some(
                        &json!({ "plan": plan.as_str(), "amount_cents": amount_cents }).to_string(),
                    ),
                    &now,
                );
            }
            WebhookEvent::SubscriptionCanceled { .. } => {
                if let Err(e) = billing_storage::mark_user_canceled(conn, &user_id) {
                    tracing::error!("billing webhook: mark_user_canceled failed: {e}");
                }
                let _ = billing_storage::record_billing_event(
                    conn,
                    &user_id,
                    event_type,
                    provider_label,
                    None,
                    &now,
                );
            }
            WebhookEvent::PaymentFailed { reason, .. } => {
                let _ = billing_storage::record_billing_event(
                    conn,
                    &user_id,
                    event_type,
                    provider_label,
                    Some(&json!({ "reason": reason }).to_string()),
                    &now,
                );
            }
        }
    }

    // Mirror to the activity log (outside the storage lock).
    let (acao, tipo) = match &event {
        WebhookEvent::PaymentSucceeded { .. } => (Acao::Atualizar, Tipo::Sucesso),
        WebhookEvent::PaymentFailed { .. } => (Acao::Atualizar, Tipo::Erro),
        WebhookEvent::SubscriptionCanceled { .. } => (Acao::Atualizar, Tipo::Sucesso),
    };
    log_billing(
        &state,
        &user_id,
        event_type,
        acao,
        tipo,
        json!({ "provider": provider_label }),
    );

    Json(json!({ "ok": true, "event": event_type })).into_response()
}

// ---------------------------------------------------------------------------
// Handler — admin mark-paid (manual provider)
// ---------------------------------------------------------------------------

/// POST /api/v1/gestao/users/{id}/mark-paid — body `{ "plan": "pro" }` (optional).
async fn mark_paid_handler(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    req: Option<Json<MarkPaidRequest>>,
) -> Response {
    if let Some(resp) = admin_guard(&headers) {
        return resp;
    }

    let req = req.map(|Json(r)| r).unwrap_or_default();
    let plan = match req.plan.as_deref() {
        None => Plan::Pro,
        Some(s) => match Plan::parse(s) {
            Some(p) => p,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("unknown plan '{s}'") })),
                )
                    .into_response();
            }
        },
    };

    let now = Utc::now().to_rfc3339();
    let updated = {
        let storage = state.core.storage.lock();
        let conn = storage.conn();
        let n = match billing_storage::mark_user_paid(
            conn,
            &user_id,
            plan,
            "manual",
            req.external_id.as_deref(),
            &now,
        ) {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("mark-paid: update failed: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "could not update tier" })),
                )
                    .into_response();
            }
        };
        if n > 0 {
            let _ = billing_storage::record_billing_event(
                conn,
                &user_id,
                "payment_succeeded",
                "manual",
                Some(&json!({ "plan": plan.as_str(), "manual": true }).to_string()),
                &now,
            );
        }
        n
    };

    if updated == 0 {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "user not found" })),
        )
            .into_response();
    }

    log_billing(
        &state,
        &user_id,
        "payment_succeeded",
        Acao::Atualizar,
        Tipo::Sucesso,
        json!({ "plan": plan.as_str(), "provider": "manual", "manual": true }),
    );

    Json(json!({ "ok": true, "user_id": user_id, "tier": "paid", "plan": plan.as_str() }))
        .into_response()
}

// ---------------------------------------------------------------------------
// Routers
// ---------------------------------------------------------------------------

/// Authed billing routes — nest under `/api/v1/me`.
pub fn me_router() -> Router<AppState> {
    Router::new()
        .route("/billing/checkout", post(checkout_handler))
        .route("/billing/status", get(status_handler))
}

/// Public webhook route (HMAC-verified in-handler) — nest under `/api/v1`.
pub fn webhook_router() -> Router<AppState> {
    Router::new().route("/billing/webhook/{provider}", post(webhook_handler))
}

/// Admin mark-paid route (email admin auth in-handler) — nest under `/api/v1/gestao`.
pub fn admin_router() -> Router<AppState> {
    Router::new().route("/users/{id}/mark-paid", post(mark_paid_handler))
}

// ---------------------------------------------------------------------------
// HTTP integration tests (in-process, no real port — tower::ServiceExt)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::billing::hmac_sha256_hex;
    use crate::server::{AppState, CoreState, IndexState, IntegrationsState, RealtimeState};

    /// Build an in-process router + a handle to the same `AppState` so tests can
    /// seed/inspect the DB. `secrets_pairs` is injected as a
    /// `StaticSecretsProvider` (no process-env mutation).
    fn build_app(
        dir: &std::path::Path,
        secrets_pairs: &[(&str, &str)],
    ) -> (axum::Router, AppState) {
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
            bypass_rate_limit: true,
        };
        let storage = crate::storage::Storage::new(&config.data_dir);
        let experiment = crate::experiment::ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path).expect("open test game storage"),
        );
        let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
        let secrets = crate::infra::secrets::StaticSecretsProvider::new(
            secrets_pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let state: AppState = AppState::new(crate::server::AppStateInner {
            core: Arc::new(CoreState::from_storage_with_secrets(
                storage, config, auth_store, secrets,
            )),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: StdMutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: StdMutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        let router = crate::server::build_router(state.clone(), None);
        (router, state)
    }

    fn seed_user(state: &AppState, id: &str, tier: &str) {
        let storage = state.core.storage.lock();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at) \
                 VALUES (?1, ?2, ?2, ?3, '2026-01-01T00:00:00Z')",
                rusqlite::params![id, format!("{id}@example.com"), tier],
            )
            .unwrap();
    }

    fn read_tier(state: &AppState, id: &str) -> String {
        let storage = state.core.storage.lock();
        storage
            .conn()
            .query_row(
                "SELECT tier FROM users WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn checkout_returns_url_for_authed_user() {
        let dir = tempdir().unwrap();
        let (app, state) = build_app(dir.path(), &[]);
        seed_user(&state, "usr_co", "player");
        let secret = crate::auth::jwt_secret();
        let (token, _) =
            crate::auth::sign_jwt("usr_co", "usr_co@example.com", "player", &secret).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/me/billing/checkout")
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"plan":"starter"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["url"].as_str().unwrap().contains("plan=starter"));
        assert_eq!(json["provider"], "manual");
        assert!(!json["session_id"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn checkout_requires_auth() {
        let dir = tempdir().unwrap();
        let (app, _state) = build_app(dir.path(), &[]);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/me/billing/checkout")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"plan":"starter"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn hostinger_webhook_flips_tier_to_paid() {
        let dir = tempdir().unwrap();
        let (app, state) = build_app(
            dir.path(),
            &[
                ("CO_BILLING_PROVIDER", "hostinger"),
                ("CO_BILLING_HOSTINGER_API_KEY", "key"),
                ("CO_BILLING_HOSTINGER_WEBHOOK_SECRET", "whsec"),
            ],
        );
        seed_user(&state, "usr_pay", "player");
        let body = br#"{"event":"payment_succeeded","user_id":"usr_pay","plan":"pro","amount_cents":2900}"#;
        let sig = hmac_sha256_hex("whsec", body);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/billing/webhook/hostinger")
                    .header("X-Webhook-Signature", sig)
                    .body(Body::from(body.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(read_tier(&state, "usr_pay"), "paid");
    }

    #[tokio::test]
    async fn hostinger_webhook_rejects_forged_signature() {
        let dir = tempdir().unwrap();
        let (app, state) = build_app(
            dir.path(),
            &[("CO_BILLING_HOSTINGER_WEBHOOK_SECRET", "whsec")],
        );
        seed_user(&state, "usr_safe", "player");
        let body = br#"{"event":"payment_succeeded","user_id":"usr_safe","plan":"pro"}"#;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/billing/webhook/hostinger")
                    .header("X-Webhook-Signature", "forged")
                    .body(Body::from(body.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // Tier unchanged.
        assert_eq!(read_tier(&state, "usr_safe"), "player");
    }

    #[tokio::test]
    async fn webhook_unknown_provider_is_404() {
        let dir = tempdir().unwrap();
        let (app, _state) = build_app(dir.path(), &[]);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/billing/webhook/nope")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn mark_paid_requires_admin() {
        let dir = tempdir().unwrap();
        let (app, _state) = build_app(dir.path(), &[]);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/gestao/users/usr_x/mark-paid")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"plan":"pro"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn status_returns_tier_for_authed_user() {
        let dir = tempdir().unwrap();
        let (app, state) = build_app(dir.path(), &[]);
        seed_user(&state, "usr_st", "player");
        let secret = crate::auth::jwt_secret();
        let (token, _) =
            crate::auth::sign_jwt("usr_st", "usr_st@example.com", "player", &secret).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/me/billing/status")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["tier"], "player");
    }
}
