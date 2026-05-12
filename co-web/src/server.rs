use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{DefaultBodyLimit, Json, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use rust_embed::Embed;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use chrono::Utc;

use crate::auth::{AuthStore, generate_code, new_code_entry, sign_jwt};
use crate::baseline;
use crate::config::WebConfig;
use crate::error::AppError;
use crate::experiment::ExperimentStore;
use crate::models::*;
use crate::storage::Storage;

// --- Embedded Static Assets ---

#[derive(Embed)]
#[folder = "static/"]
struct StaticAssets;

/// Resolve a file: try embedded assets first, then filesystem fallback for dev.
fn resolve_asset(embed_path: &str, fs_path: Option<&std::path::Path>) -> Option<Vec<u8>> {
    // Embedded assets (works from any directory)
    if let Some(file) = StaticAssets::get(embed_path) {
        return Some(file.data.to_vec());
    }
    // Filesystem fallback (dev mode: live-reload of static files)
    if let Some(path) = fs_path
        && let Ok(contents) = std::fs::read(path)
    {
        return Some(contents);
    }
    None
}

// --- App State ---

pub struct AppStateInner {
    pub storage: parking_lot::Mutex<Storage>,
    pub experiment: Mutex<ExperimentStore>,
    pub config: WebConfig,
    pub auth_store: Mutex<AuthStore>,
    pub mail: Arc<dyn co::MailProvider>,
    pub game_storage: Arc<game_core::storage::Storage>,
    pub plugin_registry: game_core::plugin::PluginRegistry,
    /// CRDT document rooms — keyed by `"slug:doc_path"`.
    pub doc_rooms: crate::ws::DocRoomManager,
    /// CO-151: protobuf SyncDelta rooms — keyed by universe_key.
    pub sync_rooms: crate::sync_ws::SyncRoomManager,
    /// CO-79: in-process LRU caching layer (manifest, theme CSS, query results).
    pub cache: std::sync::Arc<crate::cache::CacheLayer>,
    /// CO-80: token-bucket rate limiter shared across request handlers.
    pub rate_limiter: Mutex<crate::rate_limit::RateLimiter>,
    /// CO-118: Workers Analytics Engine emitter (no-op when env vars absent).
    pub wae: Arc<crate::wae::WaeEmitter>,
    /// CO-166: EC P-256 key pair for ES256 JWT signing and JWKS endpoint.
    pub jwt_key: Arc<crate::auth::JwtKey>,
    /// CO-164: shared embedding model (all-MiniLM-L6-v2).
    pub embeddings: Arc<crate::embedding::EmbeddingService>,
    /// CO-164: channel to send embedding jobs to the background worker.
    pub embedding_tx: crate::embedding_worker::EmbeddingSender,
    /// CO-194: per-room broadcast channels for chat WebSocket fan-out.
    pub chat_rooms_broadcast: std::sync::Mutex<
        std::collections::HashMap<
            String,
            tokio::sync::broadcast::Sender<crate::chat_ws::ChatEvent>,
        >,
    >,
    /// CO-194: per-room presence refcounts (room_id → user_id → connection count).
    pub chat_presence:
        std::sync::Mutex<std::collections::HashMap<String, std::collections::HashMap<String, u32>>>,
}

pub type AppState = Arc<AppStateInner>;

fn lock_storage(state: &AppState) -> parking_lot::MutexGuard<'_, Storage> {
    state.storage.lock()
}

fn lock_experiment(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, ExperimentStore>, AppError> {
    state
        .experiment
        .lock()
        .map_err(|_| AppError::Internal("Experiment lock failed".into()))
}

fn lock_auth(state: &AppState) -> Result<std::sync::MutexGuard<'_, AuthStore>, AppError> {
    state
        .auth_store
        .lock()
        .map_err(|_| AppError::Internal("Auth store lock failed".into()))
}

// --- Input Validation ---

fn validate_task_title(title: &str) -> Result<(), AppError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Task title cannot be empty".into()));
    }
    if title.len() > 500 {
        return Err(AppError::BadRequest(
            "Task title must be 500 characters or fewer".into(),
        ));
    }
    Ok(())
}

fn validate_task_description(desc: &str) -> Result<(), AppError> {
    if desc.len() > 10_000 {
        return Err(AppError::BadRequest(
            "Task description must be 10,000 characters or fewer".into(),
        ));
    }
    Ok(())
}

fn validate_comment_body(body: &str) -> Result<(), AppError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Comment body cannot be empty".into()));
    }
    if body.len() > 5_000 {
        return Err(AppError::BadRequest(
            "Comment body must be 5,000 characters or fewer".into(),
        ));
    }
    Ok(())
}

fn validate_comment_author(author: &str) -> Result<(), AppError> {
    if author.len() > 100 {
        return Err(AppError::BadRequest(
            "Comment author must be 100 characters or fewer".into(),
        ));
    }
    Ok(())
}

fn validate_project_name(name: &str) -> Result<(), AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Project name cannot be empty".into()));
    }
    if name.len() > 100 {
        return Err(AppError::BadRequest(
            "Project name must be 100 characters or fewer".into(),
        ));
    }
    Ok(())
}

fn validate_project_key(key: &str) -> Result<(), AppError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Project key cannot be empty".into()));
    }
    if key.len() > 10 {
        return Err(AppError::BadRequest(
            "Project key must be 10 characters or fewer".into(),
        ));
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::BadRequest(
            "Project key must contain only alphanumeric characters".into(),
        ));
    }
    Ok(())
}

fn validate_labels(labels: &[String]) -> Result<(), AppError> {
    if labels.len() > 20 {
        return Err(AppError::BadRequest("Maximum 20 labels allowed".into()));
    }
    for label in labels {
        if label.len() > 50 {
            return Err(AppError::BadRequest(
                "Each label must be 50 characters or fewer".into(),
            ));
        }
    }
    Ok(())
}

// --- Router ---

pub fn build_router(state: AppState, plugin_routes: Option<Router<AppState>>) -> Router {
    // --- co-web auth (email codes + UAT password login) ---
    let auth_api = Router::new()
        .route("/v1/auth/login", post(login_handler))
        .route("/v1/auth/verify", post(verify_handler))
        .route(
            "/v1/auth/me",
            get(me_handler).layer(axum::middleware::from_fn(crate::auth::require_auth)),
        )
        .route(
            "/v1/auth/stats",
            get(user_stats_handler).layer(axum::middleware::from_fn(crate::auth::require_auth)),
        )
        .route("/v1/auth/logout", post(logout_handler)) // State extracted inside
        // CO-85: generic password-based login (any env, user must have password_hash set)
        .route("/v1/auth/password-login", post(password_login_handler))
        // CO-175 (G3): public signup — usuario + password (+ optional email).
        // Rate-limited 100 / day cluster-wide.
        .route("/v1/auth/signup", post(signup_handler))
        // CO-177: Google OAuth sign-in. Status returns whether the deployment
        // has client_id+secret set; UI hides the button when not configured.
        .route("/v1/auth/google/status", get(google_status_handler))
        .nest("/v1/auth", crate::oauth_google::router())
        // CO-44: compat alias — returns 404 in prod (kept for scripts and CLAUDE.md docs)
        .route("/v1/auth/uat-login", post(uat_login_handler));

    // --- Board public routes (GET — no auth required) ---
    let board_public = Router::new()
        .route("/projects/{key}", get(get_project))
        .route("/projects/{key}/tasks", get(list_tasks))
        .route("/projects/{key}/tasks/{id}", get(get_task))
        .route("/projects/{key}/tasks/{id}/comments", get(list_comments))
        .route("/projects/{key}/activity", get(list_activity))
        .route("/projects/{key}/dashboard", get(get_dashboard))
        .route("/health", get(health_check))
        .route("/health/deep", get(health_check_deep));

    // --- Board protected routes (write ops + list — JWT required) ---
    let board_protected = Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{key}", delete(delete_project))
        .route("/projects/{key}/tasks", post(create_task))
        .route(
            "/projects/{key}/tasks/{id}",
            put(update_task).delete(delete_task),
        )
        .route("/projects/{key}/tasks/{id}/comments", post(create_comment))
        .route("/projects/{key}/tasks/bulk-update", post(bulk_update_tasks))
        .route("/projects/{key}/tasks/bulk-delete", post(bulk_delete_tasks))
        .layer(axum::middleware::from_fn(crate::auth::require_auth));

    // --- Experiments ---
    let experiment_api = Router::new()
        .route("/experiment/variant", get(get_variant).post(switch_variant))
        .route("/experiment/feedback", post(submit_feedback))
        .route("/experiment/summary", get(get_summary));

    // --- Game routes (from game/server) ---
    use crate::game_routes;

    let game_public = Router::new()
        .route("/v1/health", get(game_routes::health))
        .route("/v1/plugins", get(game_routes::list_plugins))
        .route("/v1/auth/register", post(game_routes::register))
        .route("/v1/auth/legacy-login", post(game_routes::legacy_login))
        .route(
            "/v1/games/{game_name}/leaderboard",
            get(game_routes::get_leaderboard),
        )
        .route(
            "/v1/games/leaderboard/global",
            get(game_routes::get_global_leaderboard),
        )
        .route("/v1/games/recent", get(game_routes::get_recent_activity))
        .route(
            "/v1/players/{username}",
            get(game_routes::get_player_profile),
        );

    let game_protected = Router::new()
        .route("/v1/profile", get(game_routes::get_profile))
        .route("/v1/wallet", get(game_routes::get_wallet))
        .route(
            "/v1/games/{game_name}/result",
            post(game_routes::record_game_result),
        )
        .route(
            "/v1/games/{game_name}/stats",
            get(game_routes::get_game_stats),
        )
        .layer(axum::middleware::from_fn(crate::auth::require_auth));

    // Middleware stack
    // CO-205: allow_credentials enables cross-origin cookie sharing (e.g.
    // artelonga.com.br signup form → co.artelonga.com.br). mirror_request()
    // echoes the caller's Origin so `credentials: 'include'` works for any
    // safelisted origin without hard-coding them here.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static("target-type"),
            HeaderName::from_static("target"),
            HeaderName::from_static("operation"),
            HeaderName::from_static("x-admin-override-quota"),
        ]);

    // --- Quilombo community routes ---
    let quilombo_api = crate::quilombo_routes::router();

    // --- Gestão (admin) routes with GitHub auth ---
    let github_token_cache = crate::github_auth::new_token_cache();
    let allowed_admins =
        crate::github_auth::AllowedAdmins(state.config.gestao_github_admins.clone());
    let gestao_api = crate::gestao_routes::router()
        .layer(axum::Extension(github_token_cache.clone()))
        .layer(axum::Extension(allowed_admins.clone()));

    // --- Telemetry admin routes (CO-46) ---
    let telemetry_admin = crate::telemetry::admin_router()
        .layer(axum::Extension(github_token_cache.clone()))
        .layer(axum::Extension(allowed_admins.clone()));

    // --- CO-166: OIDC OAuth client admin routes ---
    let gestao_oauth_api = crate::oidc_routes::gestao_oauth_router()
        .layer(axum::Extension(github_token_cache.clone()))
        .layer(axum::Extension(allowed_admins.clone()));

    // --- A/B flag admin routes (CO-121) ---
    let ab_admin = crate::ab_routes::admin_router()
        .layer(axum::Extension(github_token_cache.clone()))
        .layer(axum::Extension(allowed_admins.clone()));

    // CO-168: outbound webhook admin routes
    let webhook_admin = crate::webhook_routes::router()
        .layer(axum::Extension(github_token_cache))
        .layer(axum::Extension(allowed_admins));

    // --- Telemetry public route (CO-46) ---
    let telemetry_public = crate::telemetry::router();

    // --- Universe multi-tenancy routes ---
    let universe_api = crate::universe_routes::router();

    // --- CO-188: Universe invitation routes ---
    let universe_invitation_api = crate::invitation_routes::universe_invitation_router()
        .layer(axum::middleware::from_fn(crate::auth::require_auth));
    let invitation_api = crate::invitation_routes::invitation_router();

    // --- CO-193: Chat rooms + messages ---
    let chat_api = crate::chat_routes::chat_router()
        .layer(axum::middleware::from_fn(crate::auth::require_auth));

    // --- CO-198: Private DMs (inbox, policy, blocks) ---
    let dm_api =
        crate::dm_routes::dm_router().layer(axum::middleware::from_fn(crate::auth::require_auth));

    // --- Theme tier routes ---
    let themes_api = crate::universe_routes::themes_router();

    // --- Vault REST API + API token management (CO-35) ---
    let vault_api = crate::vault_routes::vault_router();
    let token_api = crate::vault_routes::token_router();

    // --- Entry abstraction API (CO-36) ---
    let entry_api = crate::entry_routes::router();

    // --- CO-153: cross-universe relation query endpoints ---
    let relation_api = crate::relation_routes::router();

    // --- CO-146: Binary asset upload + content-addressable storage ---
    let asset_api = crate::asset_routes::asset_router();

    // --- CO-156: Reference content type CRUD ---
    let reference_api = crate::reference_routes::reference_router();

    // --- CO-161: Combine all universe-content sub-routers under a single
    // visibility gate + writer gate. Every route nested here automatically
    // inherits the access-control check — no per-handler boilerplate needed.
    // visibility_gate runs first (outer), writer_gate is inner (write-methods only).
    let universe_content_api = Router::new()
        .merge(vault_api)
        .merge(entry_api)
        .merge(relation_api)
        .merge(asset_api)
        .merge(reference_api)
        // 1.47.0: states (CO-native versioning Phase 1)
        .merge(crate::state_routes::router())
        // 1.48.0: branches — named pointers to a state (Phase 2)
        .merge(crate::branch_routes::router())
        // 1.49.0: proposals + merges — cross-universe change requests (Phase 3)
        .merge(crate::proposal_routes::router())
        // CO-162: template scaffold + type audit (POST /{slug}/apply-template)
        .merge(crate::universe_routes::universe_actions_router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::universe_writer_gate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::universe_visibility_gate,
        ));

    // --- CO-124: Vercel Log Drain receiver ---
    let log_drain_api = crate::log_drain_routes::router();

    // --- CO-45: UAT change promotion endpoints ---
    let uat_api = crate::uat_routes::router();

    // --- CO-43: Hidden dev board (admin only) ---
    let dev_board_api = crate::dev_board::router();

    // --- CO-79: Cache metrics ---
    let cache_api = Router::new().route("/stats", get(cache_stats_handler));

    // --- CRDT WebSocket route (no body limit, no auth middleware — auth done inside) ---
    let ws_route = Router::new().route("/ws/doc/{slug}/{doc_id}", get(crate::ws::ws_handler));

    // --- CO-151: SyncDelta WebSocket route ---
    let sync_ws_route =
        Router::new().route("/api/v1/sync/ws", get(crate::sync_ws::sync_ws_handler));

    // --- CO-194: Chat WebSocket (auth done inside handler, no middleware needed) ---
    let chat_ws_route = Router::new().route(
        "/api/v1/universes/{slug}/chat/rooms/{room_slug}/ws",
        get(crate::chat_ws::chat_ws_handler),
    );

    // --- SPA routes (serve index.html for client-side routing) ---
    //
    // The `/co` URL prefix was dropped: the platform is hosted at the root,
    // and `co` is now just one universe slug among many (its own dogfooding
    // instance, no special path). Reserved top-level paths that must NOT be
    // matched as universe slugs: `api`, `admin`, `settings`, `yggdrasil`,
    // `static`, `health`. All literal routes are registered before `/{slug}`
    // so axum's matcher prefers them over the param.
    let co_routes = Router::new()
        // SPA hub at root.
        .route("/", get(serve_co_index))
        // CO-105: /admin page — server-side auth.
        .route("/admin", get(crate::admin_routes::serve_admin_page))
        // CO-183: leads admin page — server-side auth.
        .route(
            "/admin/leads.html",
            get(crate::lead_routes::serve_leads_page),
        )
        // Telemetry admin dashboard for the `co` universe (dogfooding).
        .route(
            "/co/telemetria",
            get(crate::telemetry::serve_admin_dashboard),
        )
        // Sync settings page — creates/shows API token for co-sync.
        .route("/settings/sync", get(serve_sync_settings))
        // CO-38: Yggdrasil game view — served by the SPA.
        .route("/yggdrasil/{game}", get(serve_co_index))
        // CO-202: /notifications full-page view — served by the SPA.
        .route("/notifications", get(serve_co_index))
        // CO-172: /recover — serves the SPA pinned to the forgot-password step.
        // Goes through `serve_recover` so a malformed `?return_to=` rejects with
        // 400 server-side instead of reaching the SPA — closes the open-redirect-
        // -looking phishing vector flagged in CO-172 review.
        .route("/recover", get(serve_recover))
        // CO-189: /invitations/:token — SPA accept page (no server-side auth).
        .route("/invitations/{token}", get(serve_co_index))
        // CO-170: friendly aliases for the timeline composite view that the
        // SPA renders from the bundled `tempo`, `universo`, `humanity`
        // universes. The actual page is `/shared/timeline.html`; both
        // `/linhadotempo` (PT) and `/timeline` (EN) redirect users there
        // with a sensible default selection of all three universes.
        .route(
            "/linhadotempo",
            get(|| async {
                axum::response::Redirect::temporary(
                    "/shared/timeline.html?u=tempo,universo,humanity",
                )
            }),
        )
        .route(
            "/timeline",
            get(|| async {
                axum::response::Redirect::temporary(
                    "/shared/timeline.html?u=tempo,universo,humanity",
                )
            }),
        )
        // 301 redirect for legacy `/co/{slug}/...` URLs to the new
        // `/{slug}/...` shape. axum picks this over `/{slug}/{*subpath}`
        // because the literal `co` segment is more specific than `{slug}`.
        .route("/co/{slug}", get(redirect_legacy_co_slug))
        .route("/co/{slug}/{*subpath}", get(redirect_legacy_co_subpath))
        // CO-150: asset browser page for universe owners.
        .route("/{slug}/assets", get(serve_assets_page))
        // Universe view (SPA).
        .route("/{slug}", get(serve_co_index))
        // CO-144: any deeper SPA path under a universe — must come AFTER the
        // more specific `/{slug}/assets` and top-level routes so axum's
        // matcher prefers them.
        .route("/{slug}/{*subpath}", get(serve_co_index));

    // --- CO-166: OIDC OAuth endpoints (authorization, token, userinfo) ---
    let oauth_api = crate::oidc_routes::oauth_router();

    // --- CO-105: Admin dashboard API + static page ---
    let admin_dashboard_api = crate::admin_routes::api_router();

    // --- CO-183: leads intake (public) + admin queue ---
    let leads_public_api = crate::lead_routes::public_router();
    let leads_admin_api = crate::lead_routes::admin_router();

    let mut router = Router::new()
        .merge(ws_route)
        .merge(sync_ws_route)
        .merge(chat_ws_route)
        .merge(co_routes)
        .nest("/api", board_public)
        .nest("/api", board_protected)
        .nest("/api", auth_api)
        .nest("/api", experiment_api)
        .nest("/api", game_public)
        .nest("/api", game_protected)
        .nest("/api/v1/quilombo", quilombo_api)
        // CO-46: telemetry middleware — records pageviews in telemetry_events
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::telemetry::telemetry_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::quilombo_telemetria::telemetry_middleware,
        ))
        .layer(axum::middleware::from_fn(
            crate::quilombo_telemetria::csrf_middleware,
        ))
        .layer(axum::middleware::from_fn(
            crate::quilombo_telemetria::canonical_host_middleware,
        ))
        .nest("/api/v1/gestao", gestao_api)
        // CO-168: outbound webhook admin endpoints (admin-only, same auth as gestao)
        .nest("/api/v1/gestao", webhook_admin)
        // CO-142 (Phase A): dev board moved to /api/v1/admin to un-shadow the public universe_api.
        .nest("/api/v1/admin", dev_board_api)
        .nest("/api/v1/universes", universe_api)
        // CO-188: universe invitation create (auth-gated, under universe namespace)
        .nest("/api/v1/universes", universe_invitation_api)
        // CO-188: invitation preview + accept (public preview, per-route auth on accept)
        .nest("/api/v1/invitations", invitation_api)
        // CO-193: per-universe chat (auth required on all chat endpoints)
        .nest("/api/v1/universes", chat_api)
        // CO-198: private DMs — inbox + policy + blocks
        .nest("/api/v1", dm_api)
        // CO-189: me/invitations inbox (auth required)
        .nest(
            "/api/v1/me",
            crate::invitation_routes::me_invitations_router(),
        )
        // CO-199: me/notifications + notification-preferences (auth required)
        .nest(
            "/api/v1/me",
            crate::notification_routes::me_notifications_router(),
        )
        // CO-201: VAPID public key endpoint (anonymous)
        .merge(crate::push_routes::vapid_router())
        // CO-201: push subscription management (auth required)
        .nest("/api/v1/me", crate::push_routes::me_push_router())
        // CO-191: me/universes bucketed endpoint (auth required)
        .route(
            "/api/v1/me/universes",
            axum::routing::get(crate::universe_routes::me_universes_handler)
                .layer(axum::middleware::from_fn(crate::auth::require_auth)),
        )
        // CO-161: all universe-content routes under a single visibility + writer gate.
        .nest("/api/v1/universes", universe_content_api)
        // 1.75.0: blob CAS API (foundation for mempalace BaseBackend shim).
        // Accepts JWT or long-lived API token via require_auth_with_token.
        .nest(
            "/api/v1",
            crate::blob_routes::router().layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::auth::require_auth_with_token,
            )),
        )
        .nest("/api/v1/auth", token_api)
        .nest("/api/v1/themes", themes_api)
        // CO-124: Vercel Log Drain receiver
        .nest("/v1/log-drains/vercel", log_drain_api)
        // CO-45: UAT change promotion
        .nest("/api/v1/uat", uat_api)
        // CO-46: public event ingestion + admin summary/export
        .nest("/api/v1/telemetry", telemetry_public)
        .nest("/api/v1/admin", telemetry_admin)
        .nest("/api/v1/ab", ab_admin)
        // CO-79: cache hit/miss/eviction metrics
        .nest("/api/v1/cache", cache_api)
        // CO-105: admin dashboard JSON endpoint (JWT + email gate, no GitHub auth)
        .nest("/api/v1/admin", admin_dashboard_api)
        // CO-183: public leads intake + admin queue API
        .nest("/api/v1", leads_public_api)
        .nest("/api/v1/admin", leads_admin_api)
        // CO-144 Phase C: process model — first process is alterar-pagina-na-web
        .nest("/api/v1/processos", crate::processos::router())
        // CO-166: OIDC authorization code flow + userinfo
        .nest("/oauth", oauth_api)
        // CO-166: OIDC client management (gestão admin)
        .nest("/api/v1/gestao/oauth", gestao_oauth_api)
        // CO-166: OIDC discovery endpoints (well-known)
        .route(
            "/.well-known/openid-configuration",
            get(crate::oidc_routes::openid_configuration),
        )
        .route("/.well-known/jwks.json", get(crate::oidc_routes::jwks_json))
        // CO-164: cross-universe semantic search
        .nest("/api/v1", crate::search_routes::router())
        // CO-165: recovery channels (requires auth) + forgot/reset password (public)
        .nest(
            "/api/v1/auth/recovery",
            crate::recovery_routes::recovery_router()
                .layer(axum::middleware::from_fn(crate::auth::require_auth)),
        )
        .nest(
            "/api/v1/auth",
            crate::recovery_routes::forgot_password_router(),
        )
        // CO-190: passwordless onboarding via email (public — no auth required)
        .nest(
            "/api/v1/auth",
            crate::onboarding_routes::onboarding_router(),
        );

    // Mount plugin routes if any plugins were loaded
    if let Some(plugin_router) = plugin_routes {
        router = router.nest("/api/v1/plugins", plugin_router);
    }

    // CO-80: rate limiting applied after ALL routes are registered so the layer
    // covers every endpoint, including universe_api, entry_api, and vault_api.
    router = router.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::rate_limit::rate_limit_middleware,
    ));

    router
        .fallback(serve_variant_file)
        .with_state(state)
        .layer(DefaultBodyLimit::max(1_048_576)) // 1MB max body
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

// ---------------------------------------------------------------------------
// UAT startup helpers (CO-44)
// ---------------------------------------------------------------------------

/// Recursively copy all files from `src` into `dst`, creating directories as needed.
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Runs all UAT-specific startup tasks when `CO_ENV=uat`.
///
/// # Reset flag
/// If `{data_dir}/uat-reset.flag` exists:
/// 1. Back up all users (with password hashes) from SQLite.
/// 2. Delete the SQLite database files.
/// 3. Remove anonymous universe directories.
/// 4. Re-open the database (runs all migrations from scratch).
/// 5. Restore the backed-up users.
/// 6. Re-seed the template universe.
/// 7. Delete the flag file.
///
/// # Always (after optional reset)
/// - Seed or update `yuri@uat.local` (tier=admin, password=`uat`).
/// - Clean up anonymous universes from the previous session.
/// - Seed `{data_dir}/co/` from `/app/seed-co/` if the directory is missing
///   (so the CO dev board has content on first boot).
///
/// Returns `true` if the reset flag was processed during this startup (CO-82
/// uses this to gate the prod-mirror task).
fn uat_startup(config: &WebConfig) -> bool {
    let data_dir = std::path::Path::new(&config.data_dir);
    let reset_flag = data_dir.join("uat-reset.flag");
    let reset_just_happened = reset_flag.exists();

    // --- Reset flag handling ---
    if reset_just_happened {
        tracing::info!("UAT: reset flag detected — resetting database...");

        // 0. Delete flag FIRST so a crash/restart doesn't re-trigger reset.
        let _ = std::fs::remove_file(&reset_flag);

        // 1. Back up users.
        let backup = {
            let storage = Storage::new(&config.data_dir);
            storage.get_all_users_with_hashes()
        };
        tracing::info!("UAT: backed up {} user(s)", backup.len());

        // 2. Delete SQLite database files.
        for suffix in &["co.db", "co.db-shm", "co.db-wal"] {
            let _ = std::fs::remove_file(data_dir.join(suffix));
        }

        // 3. Remove anonymous universe directories.
        let universes_dir = data_dir.join("universes");
        if universes_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&universes_dir)
        {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("anon-") {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }

        // 4. Re-open DB (runs all migrations from scratch).
        let mut storage = Storage::new(&config.data_dir);

        // 5. Restore users.
        storage.restore_users_with_hashes(&backup);

        // 6. Re-seed all universes.
        if !storage.template_exists() {
            storage.seed_template_universe();
        }
        if !storage.quilombo_universe_exists() {
            storage.seed_quilombo_universe();
        }
        if !storage.yggdrasil_universe_exists() {
            storage.seed_yggdrasil_universe();
        }

        drop(storage);

        tracing::info!("UAT: reset complete");
    }

    // --- Seed yuri@uat.local (idempotent) ---
    {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};

        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(b"uat", &salt)
            .expect("Argon2 hash failed")
            .to_string();

        let mut storage = Storage::new(&config.data_dir);
        if let Err(e) = storage.seed_uat_user(&hash) {
            tracing::error!("UAT: failed to seed yuri user: {e}");
        }

        // Add yuri as member of quilomboaraucaria so it appears in their sidebar
        let _ = storage.conn().execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES ('quilomboaraucaria', 'usr_yuri_uat', 'admin', datetime('now'))",
            rusqlite::params![],
        );

        // --- Clean up anonymous universes from previous session ---
        let cleaned = storage.cleanup_anon_universes();
        if cleaned > 0 {
            tracing::info!("UAT: removed {cleaned} anonymous universe(s) from previous session");
        }

        // CO-45: snapshot current UAT state so mutations can be diffed later.
        match crate::uat_routes::create_snapshot(&config.data_dir, &storage) {
            Ok(snap) => tracing::info!("UAT: snapshot {} created", snap.version),
            Err(e) => tracing::warn!("UAT: could not create snapshot: {e}"),
        }
    }

    // --- Seed co-dev tasks from bundled data ---
    let co_dir = data_dir.join("co");
    if !co_dir.exists() {
        let seed_src = std::path::Path::new("/app/seed-co");
        if seed_src.exists() {
            match copy_dir_all(seed_src, &co_dir) {
                Ok(()) => tracing::info!("UAT: seeded co-dev tasks from /app/seed-co"),
                Err(e) => tracing::warn!("UAT: could not seed co-dev tasks: {e}"),
            }
        } else {
            tracing::warn!(
                "UAT: /app/seed-co not found — co-dev board will be empty. \
                 Add co task files manually at {}/co/",
                config.data_dir
            );
        }
    }

    reset_just_happened
}

/// Start the web server with the given config.
/// This is the main entry point used by both `co-web` binary and `co board` subcommand.
pub async fn start_server(config: WebConfig) {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "co_web=info,tower_http=info".parse().unwrap()),
        )
        .init();

    let storage = Storage::new(&config.data_dir);

    if !storage.has_data() {
        tracing::info!("No projects found — seeding baseline data...");
        drop(storage);
        baseline::seed_baseline(&config.data_dir);
        tracing::info!("Seeded projects: Design System (DS), Backend API (API), Platform (PLT)");
    } else {
        drop(storage);
    }

    // Seed template universe on every boot (idempotent).
    //
    // `seed_template_universe()` is internally idempotent: it `INSERT OR IGNORE`s
    // the universe row, then early-returns BEFORE creating the project + tutorial
    // tasks if `projects/CO/_project.md` already exists. Calling it
    // unconditionally fixes a class of "anon sees empty board" bugs where the
    // universe row exists from a prior deploy but the per-universe entries DB
    // has lost the project + tasks (so anon lands on an empty kanban).
    //
    // After seeding, content pages are always re-seeded from the bundled
    // markdown — they're treated as binary-shipped truth, unlike user-editable
    // tutorial tasks which are only seeded if missing.
    {
        let mut storage = Storage::new(&config.data_dir);
        let had_template = storage.template_exists();
        storage.seed_template_universe();
        storage.reseed_template_content_pages();
        // Force theme preset back to 'modern' so the public landing page
        // always renders with the intended default — old migrations could
        // have left it on 'scholarly-light'.
        storage.ensure_template_theme_preset("modern");
        if had_template {
            tracing::info!(
                "Template refreshed: project+tasks seeded if missing, content pages reseeded, theme pinned to 'modern'"
            );
        } else {
            tracing::info!("No template universe found — seeded fresh template");
        }
        // CO-41: seed quilomboaraucaria public universe once on first boot.
        if !storage.quilombo_universe_exists() {
            tracing::info!("Seeding quilomboaraucaria universe...");
            storage.seed_quilombo_universe();
            tracing::info!("quilomboaraucaria universe seeded (public, quilombo theme)");
        }
        // CO-38: seed Yggdrasil minigames hub once on first boot.
        if !storage.yggdrasil_universe_exists() {
            tracing::info!("Seeding Yggdrasil universe...");
            storage.seed_yggdrasil_universe();
        }
        // CO-142 Phase C: hard-delete deprecated co-dev / co-experience rows on every boot.
        storage.delete_deprecated_universes();
        // CO-170: soft-hide empty parent placeholders + pre-merge mbya from
        // sidebar listings. Idempotent.
        storage.hide_deprecated_universes();
        // CO-170 Phase B (1.87.0): drop empty cross-leaked project stubs.
        // Idempotent — re-running after cleanup no-ops.
        let n_dropped = storage.cleanup_empty_projects();
        if n_dropped > 0 {
            tracing::info!("CO-170: cleaned up {n_dropped} empty project stub(s)");
        }
        // CO-170 Phase B (1.88.0): surgical moves of misplaced project content.
        // Per user direction: AL → artelonga, QA → quilomboaraucaria, CO-platform
        // projects (API/CW/DS/PLT) consolidated under `co`, tutorial leaks dropped.
        // Idempotent: source matches no rows after first successful run.
        let n_moved = storage.migrate_co170_phase_b();
        if n_moved > 0 {
            tracing::info!("CO-170 phase B: moved/dropped {n_moved} entries total");
        }
        // CO-170 phase B follow-up: rebuild project_universe_index on EVERY
        // boot — cheap (<100 rows), and we need it to happen on boots where
        // migrate_co170_phase_b finds nothing to move (the post-cleanup
        // steady state). Without this, the index stays in whatever state
        // the prior deploy left it in.
        storage.rebuild_project_universe_index();
        // Seed admin-owned content universes (artelonga, rfq, co) so they
        // appear in the sidebar without manual creation after every deploy.
        storage.seed_admin_content_universes();
        // Timeline trio (`tempo`, `humanity`, `universo`). Always re-seed —
        // the JSON manifests in the binary are the source of truth, and
        // `upsert_entry_row` makes overwriting safe.
        storage.seed_all_timeline_universes();
        // CO-142 Phase D: remove stale quilombo variants that have no documented purpose.
        storage.delete_stale_quilombo_variants();
        // CO-142 Phase B: recompute content_count from each universe's entry DB.
        // Seed paths (reseed_template_content_pages, seed_timeline_universe, etc.)
        // call upsert_entry_row but not increment_universe_content_count; this
        // corrects the drift on every boot.
        storage.recompute_content_counts();
        // Filesystem cruft cleanup: deletes /data/universes/<key>/ dirs from
        // a NARROW allowlist of known-deprecated keys (CO-142 Phase C+D).
        // 2026-05-02 fix: previous version was generic "any orphan", which
        // wiped UniversePool's hash-prefix shard dirs. Recovery handled in
        // rebuild_entries_from_filesystem below.
        storage.prune_orphan_universe_dirs();
        // 2026-05-02 recovery: re-ingest entries from filesystem for system
        // universes whose per-universe data.db got wiped by the buggy prune.
        // Idempotent — skipped per-universe when entries table already has rows.
        storage.rebuild_entries_from_filesystem(&[
            "template",
            "tempo",
            "humanity",
            "universo",
            "quilomboaraucaria",
            "artelonga",
            "rfq",
            "co",
            "yuri",
            "dados",
        ]);
        // Re-run content_count recompute now that per-universe entries are
        // populated; otherwise content_count stays at the pre-rebuild zero.
        storage.recompute_content_counts();
    }

    // CO-142 Phase E: refresh data/co/ from bundled /app/seed-co/ on every boot.
    // The seed dir is injected at Docker build time (COPY work/co/ /app/seed-co/).
    // This keeps the dev board in sync with repo state without writing back.
    {
        let co_dir = std::path::Path::new(&config.data_dir).join("co");
        let seed_src = std::path::Path::new("/app/seed-co");
        if seed_src.exists() {
            match copy_dir_all(seed_src, &co_dir) {
                Ok(()) => tracing::info!("CO-142: refreshed data/co/ from /app/seed-co/"),
                Err(e) => tracing::warn!("CO-142: could not refresh data/co/: {e}"),
            }
            // CO-142 follow-up (2026-05-02): Phase E populated /data/co/ for
            // the dev_board admin scan, but the SPA's /co/co board reads from
            // the per-universe `entries` table — which stayed empty, hence
            // user report "co has 0 entries, we have 140 tasks". This pass
            // bridges the gap by upserting each CO-*.md into the `co`
            // universe's entries at path tasks/<filename>.
            let mut storage = Storage::new(&config.data_dir);
            storage.seed_co_universe_tasks(seed_src);
        }
    }

    // CO-44: UAT-specific startup — runs only when CO_ENV=uat.
    let uat_reset_just_happened = if config.is_uat() {
        tracing::info!("UAT mode enabled (CO_ENV=uat)");
        uat_startup(&config)
    } else {
        false
    };

    // CO-85: seed admin user from env (idempotent, runs in any env).
    // CO-90 (preview): also ensure the seeded admin is a member of every
    // existing system universe so the SPA shows them post-login.
    {
        let email = std::env::var("CO_SEED_ADMIN_EMAIL").ok();
        let hash = std::env::var("CO_SEED_ADMIN_PASSWORD_HASH").ok();
        if let (Some(email), Some(hash)) = (email, hash) {
            let mut storage = Storage::new(&config.data_dir);
            if let Err(e) = storage.seed_admin_user_from_env(&email, &hash) {
                tracing::error!("Failed to seed admin user from env: {e}");
            }
            if let Err(e) = storage.ensure_admin_universe_memberships(&email) {
                tracing::error!("Failed to ensure admin universe memberships: {e}");
            }
            // 1.46.0: subscribe every existing user to default universes
            // (currently just yggdrasil) so the v29 migration's flag
            // actually reaches their sidebar.
            match storage.backfill_default_subscriptions() {
                Ok(0) => {}
                Ok(n) => tracing::info!("Default-subscriptions backfill: added {n} row(s)"),
                Err(e) => tracing::error!("backfill_default_subscriptions: {e}"),
            }
            // CO-165 (1.85.0): every user with a `users.email` set gets that
            // email as a verified recovery channel, so `forgot-password`
            // Just Works without an extra add-channel step. Idempotent —
            // existing channels are re-confirmed verified, never duplicated.
            let n_channels = storage.backfill_email_recovery_channels();
            if n_channels > 0 {
                tracing::info!(
                    "Recovery channel backfill: {n_channels} user(s) have a verified email channel"
                );
            }
            // CO-172 Phase 2: backfill recovery channels for linked quilombo users.
            let n_quilombo = storage.backfill_quilombo_recovery_channels();
            if n_quilombo > 0 {
                tracing::info!("Quilombo recovery channel backfill: {n_quilombo} user(s) promoted");
            }
            // CO-176 deployment-readiness: surface unbridged quilombo users so
            // operators can spot a regression. Counts `quilombo_usuarios`
            // rows where `linked_co_user_id IS NULL` — every quilombo signup
            // since 1.91.3 should be linked synchronously, so a non-zero
            // count here means either pre-1.91.3 legacy rows or a bridge
            // regression.
            let unbridged: i64 = storage
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM quilombo_usuarios WHERE linked_co_user_id IS NULL",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if unbridged > 0 {
                tracing::warn!(
                    "CO-176 integrity: {unbridged} quilombo user(s) without linked_co_user_id — \
                     legacy rows recover lazily via /recover, but new signups must always link"
                );
            }
            // 1.73.0 (Phase 8 step 3): backfill CAS blobs from existing
            // entries on every boot. Cheap after the first run (hash
            // collisions hit INSERT OR IGNORE no-op) — first run on prod
            // imports ~5K bodies into blobs.
            let (us, ents, added) = storage.backfill_blobs_from_entries();
            if added > 0 || ents > 0 {
                tracing::info!(
                    "blob backfill at boot: {us} universe(s), {ents} entries, {added} new blob(s)"
                );
            }
            // Re-home universes whose original owner user_id is no longer in
            // the users table (e.g. after a wipe / re-seed). Without this, the
            // universe remains in the DB but the new admin can't see it.
            match storage.rescue_orphan_universes(&email) {
                Ok(0) => {}
                Ok(n) => {
                    tracing::info!("Re-homed {n} orphan universe(s) to {email}");
                }
                Err(e) => {
                    tracing::error!("Failed to rescue orphan universes: {e}");
                }
            }
            // Force the well-known personal universes (bootstrapped via
            // scripts/seed-prod-universes.sh) to belong to the current admin
            // user, even when their prior owner_id is still a valid (stale)
            // user — not caught by rescue_orphan_universes.
            // 2026-05-02: free up legacy `*@co.local` username slugs so the
            // real admin can claim their email-prefix slug. Then derive the
            // admin's username from the email prefix.
            if let Err(e) = storage.free_legacy_co_local_usernames() {
                tracing::warn!("free_legacy_co_local_usernames failed: {e}");
            }
            if let Err(e) = storage.ensure_admin_username(&email) {
                tracing::warn!("ensure_admin_username failed: {e}");
            }
            // 2026-05-02: include `yuri` so the admin's slug-keyed personal
            // universe is owned by them (not the legacy yuri@co.local test
            // user that previously held it). Per user directive: "always use
            // slug as user name by default" — admin's slug is `yuri`, the
            // universe key matches.
            const PERSONAL_KEYS: &[&str] = &["artelonga", "rfq", "yuri"];
            match storage.ensure_admin_owns_personal_universes(&email, PERSONAL_KEYS) {
                Ok(0) => {}
                Ok(n) => {
                    tracing::info!("Re-homed {n} personal universe(s) to {email}");
                }
                Err(e) => {
                    tracing::error!("Failed to ensure admin owns personal universes: {e}");
                }
            }
            // Sweep "Meu Co" clutter from admin's sidebar — anon-clone
            // universes that an earlier rescue_orphan_universes grabbed.
            match storage.cleanup_admin_anon_clutter(&email) {
                Ok(0) => {}
                Ok(n) => {
                    tracing::info!("Removed {n} stale anon-clone(s) from {email}");
                }
                Err(e) => {
                    tracing::error!("Failed to clean admin anon clutter: {e}");
                }
            }
        }
    }

    // CO-193: backfill default `general` chat room for pre-existing universes.
    {
        let chat_storage = Storage::new(&config.data_dir);
        let n = chat_storage.backfill_default_rooms();
        if n > 0 {
            tracing::info!("CO-193: seeded default general room for {n} universe(s)");
        }
        // CO-198: backfill chat_room_members from universe_members.
        let n_members = chat_storage.backfill_chat_room_members_from_universe_members();
        tracing::info!("CO-198: chat_room_members backfill: {n_members} row(s) total");
        // CO-199: backfill default notification_preferences for every existing user.
        let n_prefs = chat_storage.backfill_default_preferences();
        tracing::info!("CO-199: notification_preferences backfill: {n_prefs} row(s) inserted");
        // CO-201: create push_subscriptions table if not yet present.
        chat_storage.ensure_push_subscriptions_table();
    }

    // One-shot SQL seed file: place `seed.sql` in data_dir, it runs once on startup then is deleted.
    let seed_path = std::path::Path::new(&config.data_dir).join("seed.sql");
    if seed_path.exists() {
        tracing::info!("Running one-shot seed file: {}", seed_path.display());
        match std::fs::read_to_string(&seed_path) {
            Ok(sql) => {
                let seed_storage = Storage::new(&config.data_dir);
                match seed_storage.conn().execute_batch(&sql) {
                    Ok(()) => {
                        tracing::info!("Seed SQL executed successfully");
                        let _ = std::fs::remove_file(&seed_path);
                    }
                    Err(e) => tracing::error!("Seed SQL failed: {e}"),
                }
            }
            Err(e) => tracing::error!("Could not read seed file: {e}"),
        }
    }

    // CO-121: seed A/B feature flags from embedded YAML (idempotent).
    {
        let seed_storage = Storage::new(&config.data_dir);
        if let Err(e) = crate::ab::seed_flags(seed_storage.conn()) {
            tracing::error!("CO-121: failed to seed feature flags: {e}");
        } else {
            tracing::info!("CO-121: feature flags seeded");
        }
    }

    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = AuthStore::new(std::path::Path::new(&config.data_dir))
        .expect("Failed to create auth store");

    let mail_provider: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    tracing::info!("Email: log provider (codes printed to stdout)");

    // Initialize game-core encrypted storage
    let game_db_path = config.game_db_path.clone().unwrap_or_else(|| {
        let data_dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        data_dir
            .join("game")
            .join("game.db")
            .to_string_lossy()
            .to_string()
    });
    let game_db_dir = std::path::Path::new(&game_db_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(game_db_dir).ok();
    let game_storage = Arc::new(
        game_core::storage::Storage::open(std::path::Path::new(&game_db_path))
            .expect("Failed to open game storage"),
    );
    tracing::info!("Game storage opened at {}", game_db_path);

    // Load plugins
    let plugins_dir = std::path::Path::new(&config.plugins_dir);
    let (plugin_registry, _plugin_router) =
        crate::plugin_loader::load_plugins(plugins_dir, &game_storage);
    let plugin_count = plugin_registry.len();
    tracing::info!("Loaded {} plugin(s)", plugin_count);

    // CO-118: build WAE emitter (no-op when WAE_ENDPOINT / WAE_API_KEY are absent).
    let wae = crate::wae::WaeEmitter::new(config.wae_endpoint.clone(), config.wae_api_key.clone());

    // CO-166: load or generate EC P-256 key for ES256 JWT signing and JWKS.
    let jwt_key = Arc::new(crate::auth::JwtKey::load_or_generate());
    // CO-164: embedding service starts disabled so the server binds quickly.
    // Model is loaded in the background after the server is accepting traffic.
    let model_dir = crate::embedding::default_model_dir();
    let embeddings = Arc::new(crate::embedding::EmbeddingService::disabled());
    let (embedding_tx, embedding_rx) = crate::embedding_worker::channel();

    let state: AppState = Arc::new(AppStateInner {
        storage: parking_lot::Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config: config.clone(),
        auth_store: Mutex::new(auth_store),
        mail: mail_provider,
        game_storage,
        plugin_registry,
        doc_rooms: crate::ws::new_room_manager(),
        sync_rooms: crate::sync_ws::new_sync_room_manager(),
        cache: crate::cache::CacheLayer::new(),
        rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
        wae,
        jwt_key,
        embeddings,
        embedding_tx,
        chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
        chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
    });

    let plugin_routes: Option<Router<AppState>> = None; // TODO: integrate plugin routes with AppState

    let app = build_router(state.clone(), plugin_routes);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("\n  Project Board\n  http://localhost:{}\n", config.port);

    // CO-72: spawn doc-gen worker loop.
    crate::job_queue::spawn_worker(Arc::clone(&state));

    // CO-164: spawn embedding background worker + load model after server binds.
    crate::embedding_worker::spawn(embedding_rx, Arc::clone(&state));
    crate::embedding_worker::boot_scan(Arc::clone(&state));
    // Load the embedding model in the background so startup doesn't block on it.
    // Server health check passes immediately; model becomes available ~10–60s later.
    Arc::clone(&state.embeddings).load_deferred(model_dir);
    // CO-168: spawn outbound webhook delivery worker.
    crate::webhook_worker::spawn_worker(Arc::clone(&state));
    // CO-200: email digest delivery worker.
    tokio::spawn(crate::notification_email_worker::run(Arc::clone(&state)));
    // CO-201: web push delivery worker (10-second tick).
    tokio::spawn(crate::notification_push_worker::run(Arc::clone(&state)));

    // CO-183: daily LGPD lead retention purge (24-month closed leads).
    tokio::spawn(crate::lead_routes::retention_task(Arc::clone(&state)));

    // CO-82: spawn UAT mirror task if reset just happened and env is configured.
    // Runs in the background after the server binds; failures are logged, not fatal.
    if uat_reset_just_happened
        && std::env::var("UAT_MIRROR_PROD").is_ok_and(|v| v == "true" || v == "1")
    {
        let prod_url = std::env::var("UAT_PROD_URL").unwrap_or_default();
        let prod_token = std::env::var("UAT_PROD_TOKEN").unwrap_or_default();
        let local_url = format!("http://localhost:{}", config.port);
        if !prod_url.is_empty() && !prod_token.is_empty() {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if let Err(e) =
                    crate::uat_mirror::mirror_prod_to_uat(&prod_url, &prod_token, &local_url).await
                {
                    tracing::error!("UAT mirror failed: {e:#}");
                }
            });
        } else {
            tracing::warn!(
                "UAT_MIRROR_PROD set but UAT_PROD_URL or UAT_PROD_TOKEN missing — skipping mirror"
            );
        }
    }

    // CO-118: emit a deploy event to WAE so the dataset is immediately queryable
    // after every startup — visible in WAE SQL within 60s per acceptance criteria.
    {
        let wae = Arc::clone(&state.wae);
        let co_env = config.co_env.clone();
        tokio::spawn(async move {
            wae.emit(crate::wae::TelemetryEvent {
                event_type: "deploy".into(),
                universe_id: "system".into(),
                user_kind: "admin".into(),
                flag_key: Some(co_env),
                variant: None,
                duration_ms: None,
                status: None,
            });
        });
    }

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("Shutdown signal received, finishing requests...");
}

// --- Health Check ---

async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

async fn health_check_deep(State(state): State<AppState>) -> impl IntoResponse {
    let (db_status, disk_status) = {
        let storage = lock_storage(&state);
        let db = match storage.conn().execute_batch(
            "SAVEPOINT health_deep; ROLLBACK TO SAVEPOINT health_deep; RELEASE SAVEPOINT health_deep;",
        ) {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        };
        let disk = if storage.data_dir.exists() {
            "ok".to_string()
        } else {
            "missing".to_string()
        };
        (db, disk)
    };

    let all_ok = db_status == "ok" && disk_status == "ok";
    let code = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(HealthDeepResponse {
            status: if all_ok {
                "ok".into()
            } else {
                "degraded".into()
            },
            db: db_status,
            disk: disk_status,
        }),
    )
}

// --- CO-79: Cache metrics ---

/// GET /api/v1/cache/stats — hit rate, miss rate, eviction rate per cache layer.
async fn cache_stats_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(state.cache.stats())
}

// --- Variant-aware static file serving ---

fn extract_variant(headers: &HeaderMap, config: &WebConfig) -> String {
    if let Some(cookie) = headers.get(header::COOKIE)
        && let Ok(cookie_str) = cookie.to_str()
    {
        for part in cookie_str.split(';') {
            let part = part.trim();
            if let Some(val) = part.strip_prefix("co_variant=") {
                let v = val.trim();
                if matches!(v, "a" | "b" | "c" | "d" | "e" | "f" | "g" | "h") {
                    return v.to_string();
                }
            }
        }
    }
    config.default_variant.clone()
}

/// Read `co_lang` cookie. Returns `None` if not set or invalid.
fn extract_lang_cookie(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix("co_lang=") {
            let v = val.trim();
            if v == "pt" || v == "en" {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Detect preferred language from Accept-Language header. Defaults to "pt".
fn detect_lang_from_accept(headers: &HeaderMap) -> &'static str {
    if let Some(accept) = headers.get(header::ACCEPT_LANGUAGE)
        && let Ok(s) = accept.to_str()
    {
        // Accept-Language: pt-BR,pt;q=0.9,en;q=0.8
        // Take the first tag and check if it starts with "pt"
        if let Some(first) = s.split(',').next() {
            let tag = first.split(';').next().unwrap_or("").trim().to_lowercase();
            if tag.starts_with("pt") {
                return "pt";
            }
            if tag.starts_with("en") {
                return "en";
            }
        }
    }
    "pt"
}

fn extract_participant(headers: &HeaderMap) -> Option<String> {
    if let Some(cookie) = headers.get(header::COOKIE)
        && let Ok(cookie_str) = cookie.to_str()
    {
        for part in cookie_str.split(';') {
            let part = part.trim();
            if let Some(val) = part.strip_prefix("co_participant=") {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

/// 301-redirect legacy `/co/{slug}` to `/{slug}` after the v1.43 prefix drop.
async fn redirect_legacy_co_slug(Path(slug): Path<String>) -> Response {
    let target = format!("/{}", slug);
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, target)],
        (),
    )
        .into_response()
}

/// 301-redirect legacy `/co/{slug}/{subpath}` to `/{slug}/{subpath}`.
async fn redirect_legacy_co_subpath(Path((slug, subpath)): Path<(String, String)>) -> Response {
    let target = format!("/{}/{}", slug, subpath);
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, target)],
        (),
    )
        .into_response()
}

pub(crate) mod static_files;
use static_files::*;

/// CO-172: query parameters accepted by `/recover`. Only `return_to` is
/// validated server-side — `identifier` and `action` are pre-fill hints
/// the SPA reads from the URL after this handler serves it.
#[derive(serde::Deserialize)]
struct RecoverQuery {
    return_to: Option<String>,
    #[allow(dead_code)]
    identifier: Option<String>,
    #[allow(dead_code)]
    action: Option<String>,
}

/// CO-172 hardening: `/recover?return_to=<url>` is checked against the
/// safelist (`*.artelonga.com.br` + `quilomboaraucaria.com.br`) before the
/// SPA is served. Without this, a phishing email could send a victim to
/// `co.artelonga.com.br/recover?return_to=https://evil.com` — the URL bar
/// shows a trusted hostname, the user completes the reset, and the SPA
/// would (sans the JS check) bounce them to the attacker's domain.
///
/// Rejecting at the server cuts the URL off before it ever loads. The JS
/// check stays as defense-in-depth.
async fn serve_recover(
    Query(params): Query<RecoverQuery>,
    headers: HeaderMap,
    uri: Uri,
    State(state): State<AppState>,
) -> Response {
    if let Some(rt) = params.return_to.as_deref()
        && !crate::recovery_routes::is_allowed_return_to(rt)
    {
        return (
            StatusCode::BAD_REQUEST,
            "return_to host is not in the safelist; \
             only *.artelonga.com.br and quilomboaraucaria.com.br are allowed",
        )
            .into_response();
    }
    serve_co_index(headers, uri, State(state)).await
}

pub(crate) mod legacy;
use legacy::*;

pub(crate) mod auth_handlers;
use auth_handlers::*;

#[cfg(test)]
mod tests;
