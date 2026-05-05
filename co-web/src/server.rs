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
    pub storage: Mutex<Storage>,
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
}

pub type AppState = Arc<AppStateInner>;

fn lock_storage(state: &AppState) -> Result<std::sync::MutexGuard<'_, Storage>, AppError> {
    state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("Storage lock failed".into()))
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
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
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

    // --- A/B flag admin routes (CO-121) ---
    let ab_admin = crate::ab_routes::admin_router()
        .layer(axum::Extension(github_token_cache))
        .layer(axum::Extension(allowed_admins));

    // --- Telemetry public route (CO-46) ---
    let telemetry_public = crate::telemetry::router();

    // --- Universe multi-tenancy routes ---
    let universe_api = crate::universe_routes::router();

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
        // Telemetry admin dashboard for the `co` universe (dogfooding).
        .route(
            "/co/telemetria",
            get(crate::telemetry::serve_admin_dashboard),
        )
        // Sync settings page — creates/shows API token for co-sync.
        .route("/settings/sync", get(serve_sync_settings))
        // CO-38: Yggdrasil game view — served by the SPA.
        .route("/yggdrasil/{game}", get(serve_co_index))
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

    // --- CO-105: Admin dashboard API + static page ---
    let admin_dashboard_api = crate::admin_routes::api_router();

    let mut router = Router::new()
        .merge(ws_route)
        .merge(sync_ws_route)
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
        // CO-142 (Phase A): dev board moved to /api/v1/admin to un-shadow the public universe_api.
        .nest("/api/v1/admin", dev_board_api)
        .nest("/api/v1/universes", universe_api)
        // CO-161: all universe-content routes under a single visibility + writer gate.
        .nest("/api/v1/universes", universe_content_api)
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
        // CO-144 Phase C: process model — first process is alterar-pagina-na-web
        .nest("/api/v1/processos", crate::processos::router());

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

    // Seed template universe once on first boot.
    {
        let mut storage = Storage::new(&config.data_dir);
        if !storage.template_exists() {
            tracing::info!("No template universe found — seeding template...");
            storage.seed_template_universe();
            tracing::info!("Template universe seeded (universe: template, project: MP)");
        } else {
            // Template already exists — refresh the content pages (intro + legal)
            // so the binary's bundled markdown is the source of truth on every boot.
            // Tasks/projects are NOT touched (user may have edited those).
            storage.reseed_template_content_pages();
            // Force theme preset back to 'modern' so the public landing page
            // always renders with the intended default — old migrations could
            // have left it on 'scholarly-light'.
            storage.ensure_template_theme_preset("modern");
            tracing::info!(
                "Template content pages refreshed from bundled seed files; theme preset pinned to 'modern'"
            );
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

    let state: AppState = Arc::new(AppStateInner {
        storage: Mutex::new(storage),
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
    });

    let plugin_routes: Option<Router<AppState>> = None; // TODO: integrate plugin routes with AppState

    let app = build_router(state.clone(), plugin_routes);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("\n  Project Board\n  http://localhost:{}\n", config.port);

    // CO-72: spawn doc-gen worker loop.
    crate::job_queue::spawn_worker(Arc::clone(&state));

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
    let (db_status, disk_status) = match lock_storage(&state) {
        Err(_) => ("lock failed".to_string(), "lock failed".to_string()),
        Ok(storage) => {
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
        }
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

/// Returns `true` when the URL path looks like a static asset request
/// (has a file extension somewhere, or starts with a known asset prefix).
/// Used to keep `/{slug}` from swallowing `/style.css`, `/shared/foo.css`,
/// `/variants/a/app.js`, etc. after the v1.43 URL refactor.
fn looks_like_static_asset(path: &str) -> bool {
    const ASSET_PREFIXES: &[&str] = &["shared/", "variants/", "pdfjs/", "games/", "icons/"];
    let stripped = path.trim_start_matches('/');
    if ASSET_PREFIXES.iter().any(|p| stripped.starts_with(p)) {
        return true;
    }
    // Last path segment contains a `.` → treat as filename.
    stripped
        .rsplit('/')
        .next()
        .map(|seg| seg.contains('.'))
        .unwrap_or(false)
}

/// Serve `index.html` for `/`, `/{slug}`, and deep SPA paths. The hub
/// (`/`) and any universe view (`/{slug}/...`) all return the same SPA
/// shell; the client-side router resolves the path.
///
/// After the v1.43 URL refactor `/{slug}` matches every top-level path,
/// including filename-like ones (`/style.css`, `/app.js`, `/manifest.json`)
/// and asset-prefix paths (`/shared/production.css`). [`looks_like_static_asset`]
/// detects these and delegates to the static-file handler.
async fn serve_co_index(headers: HeaderMap, uri: Uri, State(state): State<AppState>) -> Response {
    if looks_like_static_asset(uri.path()) {
        return serve_variant_file(headers, uri, State(state)).await;
    }

    let variant = extract_variant(&headers, &state.config);
    let embed_path = format!("variants/{}/index.html", variant);
    let fs_path = std::path::Path::new(&state.config.static_dir).join(&embed_path);

    // CO-121: assign + expose home_v2_layout for each visitor (fire-and-forget).
    // CO-97: prefer al_vid (apex marketing cookie) for unified visitor identity.
    let visitor_token = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            let mut visitante = None;
            for part in cookies.split(';') {
                let part = part.trim();
                if let Some(v) = part.strip_prefix("al_vid=") {
                    return Some(v.to_string());
                }
                if let Some(v) = part.strip_prefix("visitante_id=") {
                    visitante = Some(v.to_string());
                }
            }
            visitante
        })
        .unwrap_or_else(|| nanoid::nanoid!(24));
    {
        let state_clone = Arc::clone(&state);
        let uid = visitor_token.clone();
        tokio::spawn(async move {
            if let Ok(storage) = state_clone.storage.lock()
                && let Ok(Some(ab_variant)) =
                    crate::ab::assign(storage.conn(), &uid, "home_v2_layout")
            {
                let _ =
                    crate::ab::expose(storage.conn(), &uid, "home_v2_layout", &ab_variant, None);
            }
        });
    }

    if let Some(contents) = resolve_asset(&embed_path, Some(&fs_path)) {
        let mut response = (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
            contents,
        )
            .into_response();

        if extract_lang_cookie(&headers).is_none() {
            let lang = detect_lang_from_accept(&headers);
            if let Ok(v) =
                format!("co_lang={}; Path=/; SameSite=Lax; Max-Age=31536000", lang).parse()
            {
                response.headers_mut().append(header::SET_COOKIE, v);
            }
        }

        return response;
    }

    (StatusCode::NOT_FOUND, "Not found").into_response()
}

/// CO-150: Serve the asset browser page at `/{slug}/assets`.
async fn serve_assets_page(State(state): State<AppState>) -> Response {
    let embed_path = "shared/assets.html";
    let fs_path = std::path::Path::new(&state.config.static_dir).join(embed_path);
    if let Some(contents) = resolve_asset(embed_path, Some(&fs_path)) {
        return (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
            contents,
        )
            .into_response();
    }
    (StatusCode::NOT_FOUND, "Asset browser page not found").into_response()
}

/// GET /settings/sync — shows API token. Paths live in co-universes.yaml locally.
/// Auth required.
async fn serve_sync_settings(headers: HeaderMap, State(state): State<AppState>) -> Response {
    let user_id = match crate::auth::resolve_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::FOUND,
                [(header::LOCATION, HeaderValue::from_static("/"))],
                (),
            )
                .into_response();
        }
    };

    let token = {
        let storage = match state.storage.lock() {
            Ok(s) => s,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Storage error").into_response(),
        };
        match storage.create_api_token(&user_id, "co-sync") {
            Ok(t) => t.token,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Token error: {e}"),
                )
                    .into_response();
            }
        }
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>CO — Sync</title>
<style>
  body{{font-family:system-ui,sans-serif;max-width:560px;margin:60px auto;padding:0 24px;color:#111;}}
  h1{{font-size:1.4rem;margin-bottom:6px;}}
  p{{color:#555;line-height:1.6;margin:0 0 16px;}}
  .cmd{{background:#f4f4f5;border:1px solid #e5e7eb;border-radius:8px;padding:14px 16px;
        font-family:monospace;font-size:.95rem;position:relative;user-select:all;}}
  .copy{{position:absolute;top:8px;right:8px;background:#fff;border:1px solid #ddd;
         border-radius:4px;padding:4px 12px;cursor:pointer;font-size:.8rem;}}
  .copy:active{{background:#eee;}}
  .step{{font-size:.8rem;color:#888;margin:4px 0 12px;}}
  hr{{border:none;border-top:1px solid #eee;margin:28px 0;}}
  code{{background:#f4f4f5;padding:2px 6px;border-radius:4px;font-size:.85rem;}}
  a{{color:#2563eb;}}
</style>
</head>
<body>
<h1>Sync</h1>
<p>Universe paths are declared in <code>co-universes.yaml</code> — no configuration needed here.
Copy your token and run once from the CO repo directory.</p>

<div class="cmd" id="cmd">co-sync {token}<button class="copy" onclick="copy()">Copy</button></div>
<p class="step">Reads <code>co-universes.yaml</code>, syncs all universes, watches for changes.</p>

<hr>
<p style="font-size:.85rem;">
  <strong>Install</strong> (from the CO repo):<br>
  <code>cargo install --path co-agent --bin co-sync</code>
</p>
<p style="font-size:.85rem;">
  <strong>Auto-start at login</strong><br>
  Add <code>co-sync</code> to System Settings → General → Login Items.
</p>

<script>
function copy() {{
  navigator.clipboard.writeText(document.getElementById('cmd').textContent.replace('Copy','').trim()).then(() => {{
    document.querySelector('.copy').textContent = 'Copied!';
    setTimeout(() => document.querySelector('.copy').textContent = 'Copy', 2000);
  }});
}}
</script>
</body>
</html>"#,
        token = token,
    );
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        html,
    )
        .into_response()
}

async fn serve_variant_file(
    headers: HeaderMap,
    uri: Uri,
    State(state): State<AppState>,
) -> Response {
    let variant = extract_variant(&headers, &state.config);
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // CO-160: Serve PDF.js viewer bundle at /pdfjs/...
    if path.starts_with("pdfjs/") {
        let fs_path = std::path::Path::new(&state.config.static_dir).join(path);
        if let Some(contents) = resolve_asset(path, Some(&fs_path)) {
            let content_type = guess_content_type(path);
            let cache_header = cache_control_for(path);
            return (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                    (header::CACHE_CONTROL, cache_header),
                ],
                contents,
            )
                .into_response();
        }
    }

    // Try shared/ first (for experiment.js, experiment.css)
    if path.starts_with("shared/") || path == "manifest.json" || path == "sw.js" {
        let embed_path = if path.starts_with("shared/") {
            path.to_string()
        } else {
            format!("shared/{}", path)
        };
        let fs_path = std::path::Path::new(&state.config.static_dir).join(&embed_path);
        if let Some(contents) = resolve_asset(&embed_path, Some(&fs_path)) {
            let content_type = guess_content_type(path);
            let cache_header = cache_control_for(path);
            return (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                    (header::CACHE_CONTROL, cache_header),
                ],
                contents,
            )
                .into_response();
        }
    }

    // Try variant-specific file
    let embed_path = format!("variants/{}/{}", variant, path);
    let fs_path = std::path::Path::new(&state.config.static_dir).join(&embed_path);

    if let Some(contents) = resolve_asset(&embed_path, Some(&fs_path)) {
        let content_type = guess_content_type(path);
        let cache_header = cache_control_for(path);
        let mut response = (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                (header::CACHE_CONTROL, cache_header),
            ],
            contents,
        )
            .into_response();

        // Set variant cookie if not present
        if extract_variant(&headers, &state.config) == state.config.default_variant {
            response.headers_mut().insert(
                header::SET_COOKIE,
                format!(
                    "co_variant={}; Path=/; SameSite=Lax; HttpOnly; Max-Age=31536000",
                    variant
                )
                .parse()
                .unwrap(),
            );
        }

        // Set participant cookie if not present
        if extract_participant(&headers).is_none() {
            let participant_id = uuid::Uuid::new_v4().to_string();
            response.headers_mut().append(
                header::SET_COOKIE,
                format!(
                    "co_participant={}; Path=/; SameSite=Lax; HttpOnly; Max-Age=31536000",
                    participant_id
                )
                .parse()
                .unwrap(),
            );
        }

        // Set co_lang cookie for HTML responses when not already set.
        // co_lang cookie overrides Accept-Language on subsequent loads.
        if (path.ends_with(".html") || path == "index.html")
            && extract_lang_cookie(&headers).is_none()
        {
            let lang = detect_lang_from_accept(&headers);
            response.headers_mut().append(
                header::SET_COOKIE,
                format!("co_lang={}; Path=/; SameSite=Lax; Max-Age=31536000", lang)
                    .parse()
                    .unwrap(),
            );
        }

        return response;
    }

    (StatusCode::NOT_FOUND, "Not found").into_response()
}

fn guess_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

fn cache_control_for(path: &str) -> HeaderValue {
    match path.rsplit('.').next() {
        Some("html") => HeaderValue::from_static("no-cache"),
        Some("css") | Some("js") => HeaderValue::from_static("public, max-age=60, must-revalidate"),
        Some("png") | Some("svg") | Some("ico") | Some("woff2") => {
            HeaderValue::from_static("public, max-age=31536000, immutable")
        }
        _ => HeaderValue::from_static("no-cache"),
    }
}

// --- Template guard ---

/// Returns Forbidden if the given project belongs to a template (read-only) universe.
fn guard_template(state: &AppState, project_key: &str) -> Result<(), AppError> {
    if lock_storage(state)?.is_project_in_template(project_key) {
        return Err(AppError::Forbidden("Template universe is read-only".into()));
    }
    Ok(())
}

/// Check whether a universe has hit the anonymous usage limit (100 entries).
/// Returns Ok(()) if allowed, Err(AppError::UsageLimitExceeded) if blocked.
fn check_usage_gate(storage: &crate::storage::Storage, universe_key: &str) -> Result<(), AppError> {
    let Some(universe) = storage.get_universe(universe_key) else {
        return Ok(()); // Unknown universe — let it through (other validation will catch it)
    };
    if universe.owner_id.starts_with("anon-") && universe.content_count >= 100 {
        return Err(AppError::UsageLimitExceeded {
            current: universe.content_count,
        });
    }
    Ok(())
}

// --- Project Handlers ---

async fn list_projects(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
) -> Result<Json<Vec<Project>>, AppError> {
    let storage = lock_storage(&state)?;
    let projects = storage.list_projects_for_user(&user_id.0);
    Ok(Json(projects))
}

async fn get_project(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Project>, AppError> {
    let storage = lock_storage(&state)?;
    storage
        .get_project(&key)
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("Project '{}' not found", key)))
}

async fn create_project(
    State(state): State<AppState>,
    Json(mut body): Json<CreateProject>,
) -> Result<impl IntoResponse, AppError> {
    validate_project_name(&body.name)?;
    validate_project_key(&body.key)?;

    // Prevent creating projects inside the template universe.
    if body.universe_key.as_deref() == Some("template") {
        return Err(AppError::Forbidden("Template universe is read-only".into()));
    }

    // Server-side universe scope takes precedence over client-supplied value.
    if state.config.universe_key.is_some() {
        body.universe_key = state.config.universe_key.clone();
    }

    let mut storage = lock_storage(&state)?;

    // Check usage gate for universe-scoped projects.
    if let Some(ref ukey) = body.universe_key {
        check_usage_gate(&storage, ukey)?;
    }

    // Capture the universe_key before consuming body.
    let universe_key = body.universe_key.clone();

    let project = storage
        .create_project(body)
        .map_err(|e| AppError::Conflict(e.to_string()))?;

    // Increment content_count for the universe.
    if let Some(ref ukey) = universe_key {
        storage.increment_universe_content_count(ukey);
    }

    Ok((StatusCode::CREATED, Json(project)))
}

async fn delete_project(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<StatusCode, AppError> {
    guard_template(&state, &key)?;
    let mut storage = lock_storage(&state)?;

    let universe_key = storage.get_project_universe_key(&key);
    let project_content = universe_key
        .as_deref()
        .map(|_| storage.count_project_content(&key))
        .unwrap_or(0);

    storage
        .delete_project(&key)
        .map_err(|_| AppError::NotFound(format!("Project '{}' not found", key)))?;

    // Decrement: 1 for the project itself + tasks + their comments
    if let Some(ref ukey) = universe_key {
        storage.decrement_universe_content_count(ukey, 1 + project_content);
    }

    Ok(StatusCode::NO_CONTENT)
}

// --- Task Handlers ---

async fn list_tasks(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<Vec<Task>>, AppError> {
    let limit = query.limit.min(500);
    let storage = lock_storage(&state)?;
    Ok(Json(storage.list_tasks_paginated(
        &key,
        query.archived,
        limit,
        query.offset,
    )))
}

async fn get_task(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
) -> Result<Json<Task>, AppError> {
    let storage = lock_storage(&state)?;
    storage
        .get_task(&key, id)
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("Task {}-{} not found", key, id)))
}

async fn create_task(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<CreateTask>,
) -> Result<impl IntoResponse, AppError> {
    guard_template(&state, &key)?;
    validate_task_title(&body.title)?;
    validate_task_description(&body.description)?;
    validate_labels(&body.labels)?;

    let mut storage = lock_storage(&state)?;

    // Check usage gate.
    if let Some(ukey) = storage.get_project_universe_key(&key) {
        check_usage_gate(&storage, &ukey)?;
        let task = storage
            .create_task(&key, body)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        storage.increment_universe_content_count(&ukey);
        return Ok((StatusCode::CREATED, Json(task)));
    }

    storage
        .create_task(&key, body)
        .map(|t| (StatusCode::CREATED, Json(t)))
        .map_err(|e| AppError::Internal(e.to_string()))
}

async fn update_task(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
    Json(body): Json<UpdateTask>,
) -> Result<Json<Task>, AppError> {
    guard_template(&state, &key)?;
    if let Some(ref title) = body.title {
        validate_task_title(title)?;
    }
    if let Some(ref description) = body.description {
        validate_task_description(description)?;
    }
    if let Some(ref labels) = body.labels {
        validate_labels(labels)?;
    }

    let mut storage = lock_storage(&state)?;
    storage
        .update_task(&key, id, body)
        .map(Json)
        .map_err(|e| AppError::Internal(e.to_string()))
}

async fn delete_task(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
) -> Result<StatusCode, AppError> {
    guard_template(&state, &key)?;
    let mut storage = lock_storage(&state)?;

    let universe_key = storage.get_project_universe_key(&key);
    let comment_count = universe_key
        .as_deref()
        .map(|_| storage.count_task_comments(&key, id))
        .unwrap_or(0);

    storage
        .delete_task(&key, id)
        .map_err(|e| AppError::NotFound(e.to_string()))?;

    if let Some(ref ukey) = universe_key {
        storage.decrement_universe_content_count(ukey, 1 + comment_count);
    }

    Ok(StatusCode::NO_CONTENT)
}

// --- Comment Handlers ---

async fn list_comments(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
) -> Result<Json<Vec<Comment>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(storage.list_comments(&key, id)))
}

async fn create_comment(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
    Json(body): Json<CreateComment>,
) -> Result<impl IntoResponse, AppError> {
    guard_template(&state, &key)?;
    validate_comment_body(&body.body)?;
    validate_comment_author(&body.author)?;

    let mut storage = lock_storage(&state)?;

    // Check usage gate.
    if let Some(ukey) = storage.get_project_universe_key(&key) {
        check_usage_gate(&storage, &ukey)?;
        let comment = storage
            .create_comment(&key, id, body)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        storage.increment_universe_content_count(&ukey);
        return Ok((StatusCode::CREATED, Json(comment)));
    }

    storage
        .create_comment(&key, id, body)
        .map(|c| (StatusCode::CREATED, Json(c)))
        .map_err(|e| AppError::Internal(e.to_string()))
}

// --- Activity Handler ---

async fn list_activity(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Vec<ActivityEntry>>, AppError> {
    let limit = query.limit.min(200);
    let storage = lock_storage(&state)?;
    Ok(Json(storage.list_activity(&key, limit)))
}

// --- Dashboard Handler ---

async fn get_dashboard(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<DashboardData>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(storage.get_dashboard(&key)))
}

// --- Bulk Operations ---

async fn bulk_update_tasks(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<BulkUpdateTasks>,
) -> Result<Json<Vec<Task>>, AppError> {
    guard_template(&state, &key)?;
    if body.task_ids.is_empty() {
        return Err(AppError::BadRequest("task_ids cannot be empty".into()));
    }

    let mut storage = lock_storage(&state)?;
    storage
        .bulk_update_tasks(&key, body)
        .map(Json)
        .map_err(|e| AppError::Internal(e.to_string()))
}

async fn bulk_delete_tasks(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<BulkDeleteTasks>,
) -> Result<StatusCode, AppError> {
    guard_template(&state, &key)?;
    let mut storage = lock_storage(&state)?;
    storage
        .bulk_delete_tasks(&key, body)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| AppError::Internal(e.to_string()))
}

// --- Experiment Handlers ---

async fn get_variant(headers: HeaderMap, State(state): State<AppState>) -> Json<VariantResponse> {
    let variant = extract_variant(&headers, &state.config);
    Json(VariantResponse { variant })
}

async fn switch_variant(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<SwitchVariant>,
) -> Result<Response, AppError> {
    let participant_id =
        extract_participant(&headers).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let variant = body.variant.clone();
    if !matches!(
        variant.as_str(),
        "a" | "b" | "c" | "d" | "e" | "f" | "g" | "h"
    ) {
        return Err(AppError::BadRequest("Invalid variant".into()));
    }

    {
        let mut experiment = lock_experiment(&state)?;
        experiment.switch_variant(&participant_id, &variant);
    }

    let cookie = format!(
        "co_variant={}; Path=/; SameSite=Lax; HttpOnly; Max-Age=31536000",
        variant
    );

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(VariantResponse { variant }),
    )
        .into_response())
}

async fn submit_feedback(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<SubmitFeedback>,
) -> Result<impl IntoResponse, AppError> {
    let participant_id =
        extract_participant(&headers).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let variant = extract_variant(&headers, &state.config);

    let mut experiment = lock_experiment(&state)?;
    let entry = experiment.submit_feedback(&participant_id, &variant, body);

    Ok((StatusCode::CREATED, Json(entry)))
}

async fn get_summary(State(state): State<AppState>) -> Result<Json<ExperimentSummary>, AppError> {
    let experiment = lock_experiment(&state)?;
    Ok(Json(experiment.get_summary()))
}

// --- Auth Handlers ---

async fn login_handler(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let email = body.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::BadRequest("Email is required".into()));
    }

    // Rate limit check.
    {
        let auth = lock_auth(&state)?;
        if !auth.check_rate_limit(&email)? {
            return Err(AppError::TooManyRequests(
                "Too many code requests. Please wait before requesting another.".into(),
            ));
        }
        auth.record_request(&email)?;
    }

    // Look up user — new emails auto-register on verify, so always send code.
    let user_id = {
        let storage = lock_storage(&state)?;
        storage.get_user_by_email(&email).map(|u| u.id)
    };

    let code = generate_code();
    let entry = new_code_entry(user_id, code.clone());

    {
        let auth = lock_auth(&state)?;
        auth.store_code(&email, &entry)?;
    }

    let subject = "Seu código de acesso";
    let body_text =
        format!("Seu código de verificação é: {code}\n\nEste código expira em 5 minutos.");
    if let Err(e) = state.mail.send(&email, subject, &body_text) {
        tracing::warn!("Failed to send verification email to {email}: {e}");
    }

    Ok(Json(LoginResponse {
        message: "If registered, a code has been sent to your email".into(),
    }))
}

async fn verify_handler(
    State(state): State<AppState>,
    Json(body): Json<VerifyRequest>,
) -> Result<Response, AppError> {
    let email = body.email.trim().to_lowercase();
    let code = body.code.trim().to_string();

    let entry = {
        let auth = lock_auth(&state)?;
        auth.get_code(&email)?
    };

    let entry = match entry {
        None => return Err(AppError::Gone("Code not found or already used".into())),
        Some(e) => e,
    };

    // Check expiry.
    if Utc::now() > entry.expires_at {
        let auth = lock_auth(&state)?;
        auth.delete_code(&email)?;
        return Err(AppError::Gone("Code has expired".into()));
    }

    if entry.code != code {
        let new_attempts = entry.attempts.saturating_sub(1);

        if new_attempts == 0 {
            let auth = lock_auth(&state)?;
            auth.delete_code(&email)?;
            let body = serde_json::json!({ "error": "Code expired, request a new one" });
            return Ok((StatusCode::UNAUTHORIZED, Json(body)).into_response());
        }

        // Update attempts.
        let updated = crate::auth::VerifyCodeEntry {
            attempts: new_attempts,
            ..entry
        };
        {
            let auth = lock_auth(&state)?;
            auth.store_code(&email, &updated)?;
        }

        let body = serde_json::json!({ "remaining_attempts": new_attempts });
        return Ok((StatusCode::UNAUTHORIZED, Json(body)).into_response());
    }

    // Code matches — resolve or create user.
    let (user_id, display_name, tier) = match entry.user_id {
        Some(ref id) => {
            let storage = lock_storage(&state)?;
            let u = storage
                .get_user_by_id(id)
                .unwrap_or_else(|| crate::models::User {
                    id: id.clone(),
                    email: email.clone(),
                    display_name: String::new(),
                    tier: "player".to_string(),
                    created_at: Utc::now(),
                });
            (id.clone(), u.display_name, u.tier)
        }
        None => {
            // First-time user — auto-register.
            let display_name = email.split('@').next().unwrap_or("user").to_string();
            let user = {
                let mut storage = lock_storage(&state)?;
                storage
                    .create_user(&email, &display_name)
                    .map_err(|e| AppError::Internal(e.to_string()))?
            };
            tracing::info!("Auto-registered new user: {} <{}>", user.id, email);
            (user.id, user.display_name, user.tier)
        }
    };

    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret".to_string());
    let (token, expires_at) = sign_jwt(&user_id, &email, &tier, &jwt_secret)?;

    // Delete used code.
    {
        let auth = lock_auth(&state)?;
        auth.delete_code(&email)?;
    }

    let cookie = format!("session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800");

    // CO-156: emit auth.login telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.login",
            universe: String::new(),
            list: Some("magic-link".to_string()),
            key: Some(user_id.clone()),
            actor: Some(user_id.clone()),
            session_id: None,
            extra: None,
        },
    );

    let response_body = VerifyResponse {
        user_id,
        email,
        display_name,
        expires_at,
    };

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response_body),
    )
        .into_response())
}

// --- Auth: Me & Logout ---

async fn me_handler(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
) -> Result<Json<MeResponse>, AppError> {
    let storage = lock_storage(&state)?;

    // Check board users table first, then fall back to quilombo users.
    if let Some(user) = storage.get_user_by_id(&user_id.0) {
        return Ok(Json(MeResponse {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            tier: user.tier,
        }));
    }

    if let Some(u) = crate::quilombo_storage::obter_usuario_por_id(storage.conn(), &user_id.0) {
        return Ok(Json(MeResponse {
            user_id: u.id,
            email: String::new(),
            display_name: if u.nome.is_empty() {
                u.usuario.clone()
            } else {
                u.nome
            },
            tier: u.papel.to_string(),
        }));
    }

    Err(AppError::NotFound("User not found".into()))
}

/// GET /api/v1/auth/stats — user statistics (universes count, sizes, articles, etc.)
async fn user_stats_handler(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
) -> Result<Json<serde_json::Value>, AppError> {
    let storage = lock_storage(&state)?;
    let universes = storage.list_universes_for_user(&user_id.0);

    let mut stats = Vec::new();
    for u in &universes {
        let entry_count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                rusqlite::params![u.key],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let page_count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1 AND entry_type = 'page'",
                rusqlite::params![u.key],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let task_count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1 AND entry_type = 'task'",
                rusqlite::params![u.key],
                |row| row.get(0),
            )
            .unwrap_or(0);

        stats.push(serde_json::json!({
            "key": u.key,
            "name": u.name,
            "entries": entry_count,
            "pages": page_count,
            "tasks": task_count,
            "content_count": u.content_count,
            "is_public": u.is_public,
        }));
    }

    Ok(Json(serde_json::json!({
        "user_id": user_id.0,
        "universes": stats,
        "total_universes": universes.len(),
        "total_entries": stats.iter().map(|s| s["entries"].as_i64().unwrap_or(0)).sum::<i64>(),
    })))
}

async fn logout_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // CO-156: emit auth.logout telemetry (best-effort, before cookie is cleared)
    let session_id = crate::telemetry::extract_session_id(&headers);
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.logout",
            universe: String::new(),
            list: None,
            key: None,
            actor: crate::auth::resolve_user_id(&state, &headers),
            session_id,
            extra: None,
        },
    );

    let clear_cookie = "session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";
    (
        StatusCode::OK,
        [(header::SET_COOKIE, clear_cookie)],
        Json(serde_json::json!({ "message": "Logged out" })),
    )
        .into_response()
}

// --- Password-based login (CO-85) ---

/// Request body for password login.
#[derive(serde::Deserialize)]
struct PasswordLoginRequest {
    email: String,
    password: String,
}

/// POST /api/v1/auth/password-login — Argon2id password login, any environment.
///
/// Succeeds when the user record has a non-NULL `password_hash` set.
/// Returns 401 for unknown email, wrong password, or missing hash — all
/// indistinguishable to callers to prevent user enumeration.
async fn password_login_handler(
    State(state): State<AppState>,
    Json(req): Json<PasswordLoginRequest>,
) -> Result<Response, AppError> {
    let email = req.email.trim().to_lowercase();

    let (user, hash_opt) = {
        let storage = lock_storage(&state)?;
        storage
            .get_user_by_email_with_hash(&email)
            .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?
    };

    let hash = hash_opt.ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;

    // Verify password with Argon2id (blocking — CPU-intensive).
    let password = req.password.clone();
    let hash_clone = hash.clone();
    tokio::task::spawn_blocking(move || {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        let parsed =
            PasswordHash::new(&hash_clone).map_err(|_| AppError::Internal("Bad hash".into()))?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| AppError::Unauthorized("Invalid credentials".into()))
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    let jwt_secret = crate::auth::jwt_secret();
    let (token, expires_at) = sign_jwt(&user.id, &user.email, &user.tier, &jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let cookie = format!("session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800");

    // CO-156: emit auth.login telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.login",
            universe: String::new(),
            list: Some("password".to_string()),
            key: Some(user.id.clone()),
            actor: Some(user.id.clone()),
            session_id: None,
            extra: None,
        },
    );

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(VerifyResponse {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            expires_at,
        }),
    )
        .into_response())
}

/// POST /api/v1/auth/uat-login — compat alias for UAT scripts and CLAUDE.md docs.
///
/// Delegates to `password_login_handler` when `CO_ENV=uat`; returns 404 in
/// production so the endpoint existence is not revealed to non-UAT deployments.
async fn uat_login_handler(
    State(state): State<AppState>,
    Json(req): Json<PasswordLoginRequest>,
) -> Result<Response, AppError> {
    if !state.config.is_uat() {
        return Err(AppError::NotFound("Not found".into()));
    }
    password_login_handler(State(state), Json(req)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::storage::Storage;

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
            game_core::storage::Storage::open(&game_db_path)
                .expect("Failed to open test game storage"),
        );
        let state: AppState = Arc::new(AppStateInner {
            storage: Mutex::new(storage),
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
        });
        build_router(state, None)
    }

    fn argon2_hash(password: &str) -> String {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .expect("hash failed")
            .to_string()
    }

    fn insert_test_user(dir: &std::path::Path, email: &str, password_hash: Option<&str>) -> String {
        let storage = Storage::new(dir.to_str().unwrap());
        let id = format!(
            "usr_test_{}",
            &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
        );
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, password_hash) \
             VALUES (?1, ?2, 'Test', 'player', ?3, ?4)",
                rusqlite::params![id, email, now, password_hash],
            )
            .expect("insert test user");
        id
    }

    async fn body_str(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    // --- password-login endpoint tests ---

    #[tokio::test]
    async fn test_password_login_valid_creds() {
        let dir = tempdir().unwrap();
        let hash = argon2_hash("correctpassword");
        insert_test_user(dir.path(), "alice@example.com", Some(&hash));
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/password-login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"email":"alice@example.com","password":"correctpassword"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_str(resp.into_body()).await;
        assert!(body.contains("alice@example.com"), "body: {body}");
    }

    #[tokio::test]
    async fn test_password_login_wrong_password() {
        let dir = tempdir().unwrap();
        let hash = argon2_hash("correctpassword");
        insert_test_user(dir.path(), "bob@example.com", Some(&hash));
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/password-login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"email":"bob@example.com","password":"wrongpassword"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_password_login_no_password_hash() {
        let dir = tempdir().unwrap();
        insert_test_user(dir.path(), "carol@example.com", None);
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/password-login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"email":"carol@example.com","password":"anypassword"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// `/{slug}` must NOT swallow static asset requests. Regression for the
    /// 1.43.0 URL refactor where `/style.css` returned the SPA HTML instead
    /// of CSS, breaking layout entirely.
    #[test]
    fn test_looks_like_static_asset_recognizes_filenames_and_prefixes() {
        // Top-level filenames
        assert!(looks_like_static_asset("/style.css"));
        assert!(looks_like_static_asset("/app.js"));
        assert!(looks_like_static_asset("/sw.js"));
        assert!(looks_like_static_asset("/manifest.json"));
        assert!(looks_like_static_asset("/icon.png"));
        // Asset prefixes
        assert!(looks_like_static_asset("/shared/production.css"));
        assert!(looks_like_static_asset("/variants/a/style.css"));
        assert!(looks_like_static_asset("/pdfjs/web/viewer.html"));
        // Universe slugs (no extension) — must be SPA
        assert!(!looks_like_static_asset("/"));
        assert!(!looks_like_static_asset("/co"));
        assert!(!looks_like_static_asset("/mbya"));
        assert!(!looks_like_static_asset("/mbya/refs/foo"));
        assert!(!looks_like_static_asset("/co/telemetria"));
    }

    // --- seed_admin_user_from_env drift detection tests ---

    #[test]
    fn test_seed_admin_user_from_env_inserts_new_user() {
        let dir = tempdir().unwrap();
        let mut storage = Storage::new(dir.path().to_str().unwrap());
        let hash = argon2_hash("adminpass");

        storage
            .seed_admin_user_from_env("admin@example.com", &hash)
            .unwrap();

        let (_, stored_hash) = storage
            .get_user_by_email_with_hash("admin@example.com")
            .expect("user should exist");
        assert_eq!(stored_hash.as_deref(), Some(hash.as_str()));
    }

    #[test]
    fn test_seed_admin_user_from_env_same_hash_no_op() {
        let dir = tempdir().unwrap();
        let mut storage = Storage::new(dir.path().to_str().unwrap());
        let hash = argon2_hash("adminpass");

        storage
            .seed_admin_user_from_env("admin@example.com", &hash)
            .unwrap();
        // Second call with same hash — should be a no-op (no error, no change)
        storage
            .seed_admin_user_from_env("admin@example.com", &hash)
            .unwrap();

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM users WHERE email = 'admin@example.com'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "should not duplicate user on no-op");
    }

    #[test]
    fn test_seed_admin_user_from_env_hash_drift_updates() {
        let dir = tempdir().unwrap();
        let mut storage = Storage::new(dir.path().to_str().unwrap());
        let hash1 = argon2_hash("oldpass");
        let hash2 = argon2_hash("newpass");

        storage
            .seed_admin_user_from_env("admin@example.com", &hash1)
            .unwrap();
        storage
            .seed_admin_user_from_env("admin@example.com", &hash2)
            .unwrap();

        let (_, stored_hash) = storage
            .get_user_by_email_with_hash("admin@example.com")
            .expect("user should exist");
        assert_eq!(
            stored_hash.as_deref(),
            Some(hash2.as_str()),
            "hash should be updated when drift detected"
        );
    }

    // --- CO-80: rate limiting + quota integration tests ---

    /// Anonymous user gets HTTP 429 after exhausting the 20-reads/min bucket.
    #[tokio::test]
    async fn test_rate_limit_anonymous_reads_returns_429() {
        // SAFETY: single-threaded test setup, no concurrent set_var calls.
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let make_req = || {
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/template")
                .body(Body::empty())
                .unwrap()
        };

        // First 20 requests: rate limiter allows them (bucket starts full at 20).
        for i in 0..20 {
            let status = app.clone().oneshot(make_req()).await.unwrap().status();
            assert_ne!(
                status,
                StatusCode::TOO_MANY_REQUESTS,
                "request {i} should not be rate limited"
            );
        }

        // 21st request: bucket is empty → 429.
        let status = app.clone().oneshot(make_req()).await.unwrap().status();
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "21st request should be rate limited"
        );
    }

    /// HTTP 429 response includes a Retry-After header.
    #[tokio::test]
    async fn test_rate_limit_429_has_retry_after_header() {
        // SAFETY: single-threaded test setup, no concurrent set_var calls.
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let make_req = || {
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/template")
                .body(Body::empty())
                .unwrap()
        };

        // Exhaust the anonymous read bucket (20 slots).
        for _ in 0..20 {
            let _ = app.clone().oneshot(make_req()).await.unwrap();
        }

        let resp = app.clone().oneshot(make_req()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            resp.headers().contains_key("retry-after"),
            "429 response must include Retry-After header"
        );
    }

    /// Admin user authenticating via long-lived API token (CO-35) gets the
    /// admin tier (unlimited), not the Anonymous-by-IP fallback. Pre-fix the
    /// rate-limit middleware decoded only JWTs, so admin API tokens hit the
    /// 20-reads/min anonymous bucket — breaking multi-watcher background sync.
    #[tokio::test]
    async fn test_rate_limit_admin_api_token_resolves_to_admin_tier() {
        // SAFETY: single-threaded test setup, no concurrent set_var calls.
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let api_token = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let user_id = format!(
                "usr_admin_{}",
                &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
            );
            let now = chrono::Utc::now().to_rfc3339();
            storage
                .conn()
                .execute(
                    "INSERT INTO users (id, email, display_name, tier, created_at) \
                     VALUES (?1, 'admin@test.local', 'Admin Test', 'admin', ?2)",
                    rusqlite::params![user_id, now],
                )
                .unwrap();
            storage
                .create_api_token(&user_id, "test-rate-limit")
                .unwrap()
                .token
        };

        // 25 reads is past the 20/min anonymous read budget — none should 429.
        for i in 0..25 {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/api/v1/universes/template")
                        .header("Authorization", format!("Bearer {}", api_token))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                resp.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "request {i}: admin API token must not be rate-limited"
            );
        }
    }

    /// 1.45.0: tier collapse removes the storage quota for authenticated
    /// users — every authed user is admin, with unlimited storage. The test
    /// now verifies the inverse: an authenticated user is NOT 402'd even
    /// past the legacy 10k-entry cap. Anonymous quotas (100-entry cap) live
    /// on a different code path and remain enforced.
    #[tokio::test]
    async fn test_authed_user_storage_unlimited_post_tier_collapse() {
        // SAFETY: single-threaded test setup, no concurrent set_var calls.
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let user_id = {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let id = format!(
                "usr_quota_{}",
                &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
            );
            let now = chrono::Utc::now().to_rfc3339();
            // tier='user' is a legacy stored value; Tier::parse maps it to
            // Admin under the 1.45.0 model.
            storage
                .conn()
                .execute(
                    "INSERT INTO users (id, email, display_name, tier, created_at) \
                     VALUES (?1, 'quota@test.local', 'Quota Test', 'user', ?2)",
                    rusqlite::params![id, now],
                )
                .unwrap();
            storage
                .conn()
                .execute(
                    "INSERT OR IGNORE INTO universes \
                     (key, name, description, owner_id, created_at, is_template, is_public, \
                      content_count, visibility) \
                     VALUES ('quota-u', 'Quota Universe', '', ?1, ?2, 0, 0, 10001, 'private')",
                    rusqlite::params![id, now],
                )
                .unwrap();
            id
        };

        let jwt = crate::auth::sign_jwt(&user_id, "quota@test.local", "user", "test-secret")
            .unwrap()
            .0;

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes/quota-u/entries")
                    .header("Authorization", format!("Bearer {}", jwt))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"test.md","frontmatter":{"type":"page","title":"T"},"body":""}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(
            resp.status(),
            StatusCode::PAYMENT_REQUIRED,
            "authenticated user must NOT hit storage quota under 1.45.0 collapse"
        );
    }

    /// Admin user with X-Admin-Override-Quota bypasses quota check (audit logged).
    #[tokio::test]
    async fn test_admin_override_quota_bypasses_universe_quota() {
        // SAFETY: single-threaded test setup, no concurrent set_var calls.
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        // Create an admin user who already has 1 universe (the "user" tier limit is 10,
        // but admin has no limit — this tests the override header is accepted for admin).
        let user_id = "usr_admin_override";
        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            let now = chrono::Utc::now().to_rfc3339();
            storage
                .conn()
                .execute(
                    "INSERT INTO users (id, email, display_name, tier, created_at) \
                     VALUES (?1, 'admin@test.local', 'Admin', 'admin', ?2)",
                    rusqlite::params![user_id, now],
                )
                .unwrap();
        }

        let jwt = crate::auth::sign_jwt(user_id, "admin@test.local", "admin", "test-secret")
            .unwrap()
            .0;

        // Admin creating a universe with X-Admin-Override-Quota: true must not get 402.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/universes")
                    .header("Authorization", format!("Bearer {}", jwt))
                    .header("X-Admin-Override-Quota", "true")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"key":"admin-override-test","name":"Admin Override Test"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(
            resp.status(),
            StatusCode::PAYMENT_REQUIRED,
            "admin with override header must not get 402, got: {}",
            resp.status()
        );
    }
}
