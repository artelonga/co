//! CO-188 — Universe invitation endpoints.
//!
//! ## Endpoints
//!
//! **Auth-gated (caller must be universe owner or admin member)**:
//! - POST /api/v1/universes/:slug/invitations — mint a single-use invite token
//!
//! **Public (no auth)**:
//! - GET /api/v1/invitations/:token — preview invite before accepting
//!
//! **Auth-gated per-route**:
//! - POST /api/v1/invitations/:token/accept — accept and join universe

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
use crate::error::AppError;
use crate::server::AppState;
use crate::storage::Storage;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Routes that nest under /api/v1/universes/:slug (require auth from universe_api layer).
pub fn universe_invitation_router() -> Router<AppState> {
    Router::new()
        .route("/{slug}/invitations", post(create_invitation_handler))
        .route(
            "/{slug}/invitations",
            get(list_universe_invitations_handler),
        )
}

/// Standalone invitation routes (public preview + auth-per-route accept).
pub fn invitation_router() -> Router<AppState> {
    Router::new()
        .route("/{token}", get(preview_invitation_handler))
        .route(
            "/{token}/accept",
            post(accept_invitation_handler).layer(middleware::from_fn(crate::auth::require_auth)),
        )
}

/// Routes that nest under /api/v1/me (auth required).
pub fn me_invitations_router() -> Router<AppState> {
    Router::new()
        .route("/invitations", get(me_invitations_handler))
        .route("/invitations/accept", post(me_accept_invitation_handler))
        .layer(middleware::from_fn(crate::auth::require_auth))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateInvitationRequest {
    /// Invite by email address.
    pub email: Option<String>,
    /// Invite by usuario (username).
    pub usuario: Option<String>,
    /// Invite by CO user_id directly.
    pub user_id: Option<String>,
    /// Role to grant. Defaults to "member".
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "member".into()
}

#[derive(Debug, Serialize)]
pub struct CreateInvitationResponse {
    pub token_hash: String,
    pub expires_at: String,
    pub sent_to: String,
}

#[derive(Debug, Serialize)]
pub struct PreviewUniverseInfo {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct PreviewInviterInfo {
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub struct PreviewInvitationResponse {
    pub universe: PreviewUniverseInfo,
    pub invited_by: PreviewInviterInfo,
    pub expires_at: String,
    pub already_consumed: bool,
}

#[derive(Debug, Serialize)]
pub struct AcceptInvitationResponse {
    pub universe_key: String,
    pub role: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, Storage> {
    state.storage.lock()
}

fn redact_email(email: &str) -> String {
    if let Some((local, domain)) = email.split_once('@') {
        let head = local
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        format!("{head}***@{domain}")
    } else {
        "***".to_string()
    }
}

/// Spawn a fire-and-forget task to email the invite link.
fn send_invitation_email(
    to: String,
    inviter_name: String,
    universe_name: String,
    raw_token: String,
) {
    tokio::spawn(async move {
        deliver_invitation_email(&to, &inviter_name, &universe_name, &raw_token).await;
    });
}

async fn deliver_invitation_email(
    to: &str,
    inviter_name: &str,
    universe_name: &str,
    raw_token: &str,
) {
    use crate::notification_providers::{ChannelProvider, ResendProvider};

    let base_url =
        std::env::var("CO_BASE_URL").unwrap_or_else(|_| "https://co.artelonga.com.br".into());
    let accept_link = format!("{base_url}/invitations/{raw_token}");

    let subject = format!("Convite para {universe_name} no CO");
    let body = format!(
        "Olá,\n\n\
         {inviter_name} te convidou para o universo {universe_name} no CO.\n\n\
         Aceitar:\n{accept_link}\n\n\
         O convite expira em 14 dias.\n"
    );

    if let Some(provider) = ResendProvider::from_env() {
        let payload = format!("{subject}\n---\n{body}");
        let client = reqwest::Client::new();
        match provider.send(&client, to, &payload).await {
            Ok(()) => {
                tracing::info!("Invitation emailed to {} via Resend", redact_email(to));
                return;
            }
            Err(e) => {
                tracing::warn!(
                    "Resend invitation to {} failed: {e}. Trying SMTP fallback.",
                    redact_email(to)
                );
            }
        }
    }

    match crate::email_smtp::send_recovery_code(to, &accept_link).await {
        Ok(true) => {
            tracing::info!("Invitation emailed to {} via SMTP", redact_email(to));
        }
        Ok(false) => {
            tracing::info!(
                "Invitation for {}: link={accept_link} [no email provider — logged]",
                redact_email(to)
            );
        }
        Err(e) => {
            tracing::warn!(
                "SMTP invitation to {} failed: {e}. Link logged: {accept_link}",
                redact_email(to)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/universes/:slug/invitations
///
/// Caller must be universe owner or have role owner/admin in universe_members.
pub async fn create_invitation_handler(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
    axum::Json(body): axum::Json<CreateInvitationRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate role
    let role = body.role.trim().to_string();
    if !matches!(role.as_str(), "member" | "admin" | "viewer") {
        return Err(AppError::BadRequest(
            "role must be one of: member, admin, viewer".into(),
        ));
    }

    // Resolve recipient identity and check authorization together (single lock).
    let (invited_email, invited_user_id, universe_name, inviter_name, token_hash, raw_token) = {
        let storage = lock_storage(&state);

        // Universe must exist.
        let universe = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

        // Caller must be owner or admin member.
        let caller_role = storage.universe_member_role(&slug, &user_id.0);
        let authorized = universe.owner_id == user_id.0
            || matches!(caller_role.as_deref(), Some("owner") | Some("admin"));
        if !authorized {
            return Err(AppError::Forbidden(
                "Only the universe owner or admin members can send invitations".into(),
            ));
        }

        // Resolve recipient.
        let (invited_email, invited_user_id): (Option<String>, Option<String>) =
            if let Some(ref email) = body.email {
                let email_norm = email.trim().to_lowercase();
                let found_user = storage.get_user_by_email(&email_norm);
                (Some(email_norm), found_user.map(|u| u.id))
            } else if let Some(ref usuario) = body.usuario {
                let user = storage
                    .get_user_by_usuario(usuario.trim())
                    .ok_or_else(|| AppError::NotFound(format!("User '{usuario}' not found")))?;
                let email_opt = if user.email.is_empty() {
                    None
                } else {
                    Some(user.email.clone())
                };
                (email_opt, Some(user.id))
            } else if let Some(ref uid) = body.user_id {
                let user = storage
                    .get_user_by_id(uid.trim())
                    .ok_or_else(|| AppError::NotFound(format!("User '{uid}' not found")))?;
                let email_opt = if user.email.is_empty() {
                    None
                } else {
                    Some(user.email.clone())
                };
                (email_opt, Some(uid.trim().to_string()))
            } else {
                return Err(AppError::BadRequest(
                    "one of email, usuario, or user_id is required".into(),
                ));
            };

        // Reject if recipient is already a member.
        if let Some(ref uid) = invited_user_id
            && storage.is_universe_member(&slug, uid)
        {
            return Err(AppError::Conflict(
                "User is already a member of this universe".into(),
            ));
        }

        // Fetch inviter display name for the email.
        let inviter_name = storage
            .get_user_by_id(&user_id.0)
            .map(|u| u.display_name)
            .unwrap_or_else(|| user_id.0.clone());

        let universe_name = universe.name.clone();

        // Mint token.
        let (token_hash, raw_token) = storage
            .create_invitation(
                &slug,
                &user_id.0,
                invited_email.as_deref(),
                invited_user_id.as_deref(),
                &role,
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;

        (
            invited_email,
            invited_user_id,
            universe_name,
            inviter_name,
            token_hash,
            raw_token,
        )
    };

    // CO-220: emit NotificationRequested through the event bus so the
    // notification worker handles it without coupling invitation_routes to
    // the notification subsystem directly.
    if let Some(ref invited_uid) = invited_user_id {
        state.event_bus.publish(crate::events::DomainEvent::NotificationRequested {
            recipient_id: invited_uid.clone(),
            kind: "universe.invitation".into(),
            universe_key: Some(slug.clone()),
            room_id: None,
            actor_id: user_id.0.clone(),
            object_id: token_hash.clone(),
            summary_key: "notif.universe.invitation".into(),
            summary_params: serde_json::json!({"name": inviter_name, "universe": universe_name}),
        });
    }

    // Send email if we have an address (fire-and-forget, outside the lock).
    if let Some(ref email) = invited_email {
        send_invitation_email(email.clone(), inviter_name, universe_name, raw_token);
    }

    let sent_to = invited_email
        .as_deref()
        .map(redact_email)
        .or_else(|| invited_user_id.map(|id| format!("user:{id}")))
        .unwrap_or_else(|| "unknown".into());

    // Read expires_at from DB (14 days from now — derive it rather than re-query).
    let expires_at = (Utc::now() + chrono::Duration::days(14)).to_rfc3339();

    Ok((
        StatusCode::CREATED,
        axum::Json(CreateInvitationResponse {
            token_hash,
            expires_at,
            sent_to,
        }),
    ))
}

/// GET /api/v1/invitations/:token — public preview (no auth).
pub async fn preview_invitation_handler(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let token_hash = Storage::hash_invitation_token(&raw_token);
    let storage = lock_storage(&state);

    let inv = storage
        .get_invitation_by_token(&token_hash)
        .ok_or_else(|| AppError::NotFound("Invitation not found".into()))?;

    // Consumed → 200 with already_consumed: true (UI can say "already accepted").
    if inv.consumed_at.is_some() {
        let universe = storage
            .get_universe(&inv.universe_key)
            .ok_or_else(|| AppError::NotFound("Universe not found".into()))?;
        let inviter_name = storage
            .get_user_by_id(&inv.invited_by)
            .map(|u| u.display_name)
            .unwrap_or_default();
        return Ok(axum::Json(PreviewInvitationResponse {
            universe: PreviewUniverseInfo {
                key: universe.key,
                name: universe.name,
            },
            invited_by: PreviewInviterInfo {
                display_name: inviter_name,
            },
            expires_at: inv.expires_at.to_rfc3339(),
            already_consumed: true,
        }));
    }

    // Expired → 410.
    if Utc::now() > inv.expires_at {
        return Err(AppError::Gone("Invitation has expired".into()));
    }

    let universe = storage
        .get_universe(&inv.universe_key)
        .ok_or_else(|| AppError::NotFound("Universe not found".into()))?;
    let inviter_name = storage
        .get_user_by_id(&inv.invited_by)
        .map(|u| u.display_name)
        .unwrap_or_default();

    Ok(axum::Json(PreviewInvitationResponse {
        universe: PreviewUniverseInfo {
            key: universe.key,
            name: universe.name,
        },
        invited_by: PreviewInviterInfo {
            display_name: inviter_name,
        },
        expires_at: inv.expires_at.to_rfc3339(),
        already_consumed: false,
    }))
}

/// POST /api/v1/invitations/:token/accept — auth required.
pub async fn accept_invitation_handler(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let token_hash = Storage::hash_invitation_token(&raw_token);
    let storage = lock_storage(&state);

    let inv = storage
        .get_invitation_by_token(&token_hash)
        .ok_or_else(|| AppError::NotFound("Invitation not found".into()))?;

    // Consumed → 410.
    if inv.consumed_at.is_some() {
        return Err(AppError::Gone(
            "Invitation has already been accepted".into(),
        ));
    }

    // Expired → 410.
    if Utc::now() > inv.expires_at {
        return Err(AppError::Gone("Invitation has expired".into()));
    }

    // Verify caller identity matches the invitee.
    if let Some(ref target_uid) = inv.invited_user_id {
        if *target_uid != user_id.0 {
            return Err(AppError::Forbidden(
                "This invitation was issued for a different user".into(),
            ));
        }
    } else if let Some(ref target_email) = inv.invited_email {
        let caller = storage
            .get_user_by_id(&user_id.0)
            .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;
        if caller.email.to_lowercase() != target_email.to_lowercase() {
            return Err(AppError::Forbidden(
                "This invitation was issued for a different email address".into(),
            ));
        }
    } else {
        return Err(AppError::Internal("Invitation has no recipient".into()));
    }

    // Add the user to universe_members (INSERT OR IGNORE → idempotent).
    let now_str = Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![inv.universe_key, user_id.0, inv.role, now_str],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // Mark consumed.
    storage
        .consume_invitation(&token_hash)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let role = inv.role.clone();
    let universe_key = inv.universe_key.clone();

    Ok(axum::Json(AcceptInvitationResponse { universe_key, role }))
}

// ---------------------------------------------------------------------------
// List universe invitations
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct InvitationListItem {
    pub recipient: String,
    pub role: String,
    pub expires_at: String,
    pub created_at: String,
    pub consumed_at: Option<String>,
}

fn invitation_to_list_item(inv: &crate::storage::invitations::Invitation) -> InvitationListItem {
    let recipient = inv
        .invited_email
        .as_deref()
        .map(redact_email)
        .or_else(|| {
            inv.invited_user_id
                .as_deref()
                .map(|id| format!("user:{id}"))
        })
        .unwrap_or_else(|| "unknown".into());
    InvitationListItem {
        recipient,
        role: inv.role.clone(),
        expires_at: inv.expires_at.to_rfc3339(),
        created_at: inv.created_at.to_rfc3339(),
        consumed_at: inv.consumed_at.map(|t| t.to_rfc3339()),
    }
}

/// GET /api/v1/universes/:slug/invitations — list pending invitations (owner/admin only).
pub async fn list_universe_invitations_handler(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);

    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let caller_role = storage.universe_member_role(&slug, &user_id.0);
    let authorized = universe.owner_id == user_id.0
        || matches!(caller_role.as_deref(), Some("owner") | Some("admin"));
    if !authorized {
        return Err(AppError::Forbidden(
            "Only the universe owner or admin members can list invitations".into(),
        ));
    }

    let invitations = storage.list_invitations_for_universe(&slug);
    let items: Vec<InvitationListItem> = invitations
        .iter()
        .filter(|inv| inv.revoked_at.is_none())
        .map(invitation_to_list_item)
        .collect();

    Ok(axum::Json(items))
}

// ---------------------------------------------------------------------------
// Me invitations (inbox)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct MeInvitationItem {
    pub universe_key: String,
    pub universe_name: String,
    pub invited_by_name: String,
    pub role: String,
    pub expires_at: String,
    pub created_at: String,
}

/// CO-192: POST /api/v1/me/invitations/accept — accept a pending invite by universe_key.
/// No raw token needed since the caller is authenticated and the invite lookup
/// uses the user_id / email match from the invitations table directly.
#[derive(Debug, serde::Deserialize)]
pub struct MeAcceptInvitationBody {
    pub universe_key: String,
}

pub async fn me_accept_invitation_handler(
    State(state): State<AppState>,
    user_id: UserId,
    axum::Json(body): axum::Json<MeAcceptInvitationBody>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);

    let user = storage
        .get_user_by_id(&user_id.0)
        .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;

    let invitations = storage.list_invitations_for_me(&user_id.0, &user.email);
    let inv = invitations
        .iter()
        .find(|i| i.universe_key == body.universe_key)
        .ok_or_else(|| AppError::NotFound("No pending invitation for this universe".into()))?
        .clone();

    if inv.consumed_at.is_some() {
        return Err(AppError::Gone("Invitation already accepted".into()));
    }
    if Utc::now() > inv.expires_at {
        return Err(AppError::Gone("Invitation has expired".into()));
    }

    let now_str = Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![inv.universe_key, user_id.0, inv.role, now_str],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    storage
        .consume_invitation(&inv.token_hash)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(axum::Json(AcceptInvitationResponse {
        universe_key: inv.universe_key,
        role: inv.role,
    }))
}

/// GET /api/v1/me/invitations — list pending invitations for the current user.
pub async fn me_invitations_handler(
    State(state): State<AppState>,
    user_id: UserId,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state);

    let user = storage
        .get_user_by_id(&user_id.0)
        .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;

    let invitations = storage.list_invitations_for_me(&user_id.0, &user.email);
    let items: Vec<MeInvitationItem> = invitations
        .iter()
        .filter_map(|inv| {
            let universe = storage.get_universe(&inv.universe_key)?;
            let invited_by_name = storage
                .get_user_by_id(&inv.invited_by)
                .map(|u| u.display_name)
                .unwrap_or_default();
            Some(MeInvitationItem {
                universe_key: universe.key,
                universe_name: universe.name,
                invited_by_name,
                role: inv.role.clone(),
                expires_at: inv.expires_at.to_rfc3339(),
                created_at: inv.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(axum::Json(items))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use rusqlite::params;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::storage::Storage;

    fn isolate_env() {
        unsafe {
            std::env::set_var("JWT_SECRET", "test-jwt-secret");
        }
    }

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
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
            bypass_rate_limit: false,
        }
    }

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        let config = test_config(dir);
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path).expect("open test game storage"),
        );
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

    fn insert_user(dir: &std::path::Path, email: &str) -> String {
        let storage = Storage::new(dir.to_str().unwrap());
        let id = format!("usr_test_{}", &nanoid::nanoid!(8));
        let usuario = email.split('@').next().unwrap_or("user").to_lowercase();
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
                 VALUES (?1, ?2, ?3, 'player', ?4, ?5)",
                params![id, email, email, now, usuario],
            )
            .expect("insert test user");
        id
    }

    fn insert_universe(dir: &std::path::Path, key: &str, owner_id: &str) {
        let storage = Storage::new(dir.to_str().unwrap());
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, visibility) \
                 VALUES (?1, ?2, '', ?3, ?4, 'private')",
                params![key, key, owner_id, now],
            )
            .expect("insert test universe");
        // Add owner member row.
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, owner_id, now],
            )
            .expect("insert owner member");
    }

    fn make_jwt(user_id: &str) -> String {
        unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
        let (token, _) =
            crate::auth::sign_jwt(user_id, "test@example.com", "player", "test-jwt-secret")
                .unwrap();
        token
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    // --- 1. happy path: invite by email ---

    #[tokio::test]
    async fn test_create_invitation_by_email() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner@example.com");
        let _ = insert_user(dir.path(), "guest@example.com");
        insert_universe(dir.path(), "my-universe", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/my-universe/invitations")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        r#"{"email":"guest@example.com","role":"member"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "body: {:?}",
            body_json(resp.into_body()).await
        );
    }

    // --- 2. happy path: invite by usuario ---

    #[tokio::test]
    async fn test_create_invitation_by_usuario() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner2@example.com");
        let _ = insert_user(dir.path(), "guest2@example.com");
        insert_universe(dir.path(), "uni2", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni2/invitations")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"usuario":"guest2"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "body: {:?}",
            body_json(resp.into_body()).await
        );
    }

    // --- 3. happy path: invite by user_id ---

    #[tokio::test]
    async fn test_create_invitation_by_user_id() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner3@example.com");
        let guest_id = insert_user(dir.path(), "guest3@example.com");
        insert_universe(dir.path(), "uni3", &owner_id);
        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let body = format!(r#"{{"user_id":"{guest_id}"}}"#);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni3/invitations")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "body: {:?}",
            body_json(resp.into_body()).await
        );
    }

    // --- 4. 403 non-owner trying to create invitation ---

    #[tokio::test]
    async fn test_create_invitation_forbidden_for_non_owner() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner4@example.com");
        let other_id = insert_user(dir.path(), "other@example.com");
        insert_universe(dir.path(), "uni4", &owner_id);
        let token = make_jwt(&other_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni4/invitations")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"email":"x@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- 5. 409 already-member cannot be invited ---

    #[tokio::test]
    async fn test_create_invitation_conflict_already_member() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner5@example.com");
        let member_id = insert_user(dir.path(), "member5@example.com");
        insert_universe(dir.path(), "uni5", &owner_id);

        // Add member5 to the universe.
        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let now = chrono::Utc::now().to_rfc3339();
            storage
                .conn()
                .execute(
                    "INSERT OR IGNORE INTO universe_members \
                     (universe_key, user_id, role, joined_at) VALUES ('uni5', ?1, 'member', ?2)",
                    params![member_id, now],
                )
                .unwrap();
        }

        let token = make_jwt(&owner_id);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/uni5/invitations")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"email":"member5@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    // --- 6. GET preview — valid invitation ---

    #[tokio::test]
    async fn test_preview_invitation_valid() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner6@example.com");
        insert_universe(dir.path(), "uni6", &owner_id);

        let raw_token = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let (_, raw) = storage
                .create_invitation(
                    "uni6",
                    &owner_id,
                    Some("guest6@example.com"),
                    None,
                    "member",
                )
                .unwrap();
            raw
        };

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/invitations/{raw_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["already_consumed"], serde_json::Value::Bool(false));
    }

    // --- 7. GET preview — expired invitation → 410 ---

    #[tokio::test]
    async fn test_preview_invitation_expired() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner7@example.com");
        insert_universe(dir.path(), "uni7", &owner_id);

        // Insert an already-expired invitation directly.
        let raw_token = nanoid::nanoid!(32);
        let token_hash = Storage::hash_invitation_token(&raw_token);
        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let expired = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
            let now = chrono::Utc::now().to_rfc3339();
            storage
                .conn()
                .execute(
                    "INSERT INTO universe_invitations \
                     (token_hash, universe_key, invited_by, invited_email, role, expires_at, created_at) \
                     VALUES (?1, 'uni7', ?2, 'guest7@example.com', 'member', ?3, ?4)",
                    params![token_hash, owner_id, expired, now],
                )
                .unwrap();
        }

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/invitations/{raw_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::GONE);
    }

    // --- 8. POST accept — happy path ---

    #[tokio::test]
    async fn test_accept_invitation_happy_path() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner8@example.com");
        let guest_id = insert_user(dir.path(), "guest8@example.com");
        insert_universe(dir.path(), "uni8", &owner_id);

        let raw_token = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let (_, raw) = storage
                .create_invitation(
                    "uni8",
                    &owner_id,
                    Some("guest8@example.com"),
                    Some(&guest_id),
                    "member",
                )
                .unwrap();
            raw
        };

        let token = make_jwt(&guest_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/invitations/{raw_token}/accept"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "body: {:?}",
            body_json(resp.into_body()).await
        );
    }

    // --- 9. POST accept — re-accept → 410 ---

    #[tokio::test]
    async fn test_accept_invitation_already_consumed() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner9@example.com");
        let guest_id = insert_user(dir.path(), "guest9@example.com");
        insert_universe(dir.path(), "uni9", &owner_id);

        let raw_token = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let (token_hash, raw) = storage
                .create_invitation("uni9", &owner_id, None, Some(&guest_id), "member")
                .unwrap();
            // Mark it consumed already.
            storage.consume_invitation(&token_hash).unwrap();
            raw
        };

        let token = make_jwt(&guest_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/invitations/{raw_token}/accept"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::GONE);
    }

    // --- 10. POST accept — identity mismatch → 403 ---

    #[tokio::test]
    async fn test_accept_invitation_identity_mismatch() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner10@example.com");
        let target_id = insert_user(dir.path(), "target@example.com");
        let impostor_id = insert_user(dir.path(), "impostor@example.com");
        insert_universe(dir.path(), "uni10", &owner_id);

        let raw_token = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let (_, raw) = storage
                .create_invitation("uni10", &owner_id, None, Some(&target_id), "member")
                .unwrap();
            raw
        };

        let token = make_jwt(&impostor_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/invitations/{raw_token}/accept"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
