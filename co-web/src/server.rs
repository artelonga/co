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
        .route("/v1/auth/logout", post(logout_handler))
        // CO-44: password-based login for UAT (returns 404 in prod)
        .route("/v1/auth/uat-login", post(uat_login_handler));

    // --- Board public routes (GET — no auth required) ---
    let board_public = Router::new()
        .route("/projects/{key}", get(get_project))
        .route("/projects/{key}/tasks", get(list_tasks))
        .route("/projects/{key}/tasks/{id}", get(get_task))
        .route("/projects/{key}/tasks/{id}/comments", get(list_comments))
        .route("/projects/{key}/activity", get(list_activity))
        .route("/projects/{key}/dashboard", get(get_dashboard))
        .route("/health", get(health_check));

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

    // --- CO-43: Hidden dev board (admin only) ---
    let dev_board_api = crate::dev_board::router();

    // --- CRDT WebSocket route (no body limit, no auth middleware — auth done inside) ---
    let ws_route = Router::new().route("/ws/doc/{slug}/{doc_id}", get(crate::ws::ws_handler));

    // --- /co landing + universe routes (serve index.html for SPA routing) ---
    let co_routes = Router::new()
        .route("/co", get(serve_co_index))
        // CO-46: admin telemetry dashboard (specific route takes priority over /{slug})
        .route(
            "/co/co-dev/telemetria",
            get(crate::telemetry::serve_admin_dashboard),
        )
        .route("/co/{slug}", get(serve_co_index))
        // CO-38: Yggdrasil game view — /co/yggdrasil/{game} served by the SPA
        .route("/co/yggdrasil/{game}", get(serve_co_index));

    let mut router = Router::new()
        .merge(ws_route)
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
        // CO-43: dev board routes use literal /co-dev prefix — take priority over /{slug} routes
        .nest("/api/v1/universes", dev_board_api)
        .nest("/api/v1/universes", universe_api)
        .nest("/api/v1/universes", vault_api)
        .nest("/api/v1/universes", entry_api)
        .nest("/api/v1/auth", token_api)
        .nest("/api/v1/themes", themes_api)
        // CO-46: public event ingestion + admin summary/export
        .nest("/api/v1/telemetry", telemetry_public)
        .nest("/api/v1/admin", telemetry_admin);

    // Mount plugin routes if any plugins were loaded
    if let Some(plugin_router) = plugin_routes {
        router = router.nest("/api/v1/plugins", plugin_router);
    }

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
fn uat_startup(config: &WebConfig) {
    let data_dir = std::path::Path::new(&config.data_dir);
    let reset_flag = data_dir.join("uat-reset.flag");

    // --- Reset flag handling ---
    if reset_flag.exists() {
        tracing::info!("UAT: reset flag detected — resetting database...");

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

        // 6. Re-seed template universe.
        if !storage.template_exists() {
            storage.seed_template_universe();
        }
        // Re-seed Yggdrasil.
        if !storage.yggdrasil_universe_exists() {
            storage.seed_yggdrasil_universe();
        }

        drop(storage);

        // 7. Delete flag.
        let _ = std::fs::remove_file(&reset_flag);
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

        // --- Clean up anonymous universes from previous session ---
        let cleaned = storage.cleanup_anon_universes();
        if cleaned > 0 {
            tracing::info!("UAT: removed {cleaned} anonymous universe(s) from previous session");
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
    }

    // CO-44: UAT-specific startup — runs only when CO_ENV=uat.
    if config.is_uat() {
        tracing::info!("UAT mode enabled (CO_ENV=uat)");
        uat_startup(&config);
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

    let state: AppState = Arc::new(AppStateInner {
        storage: Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config: config.clone(),
        auth_store: Mutex::new(auth_store),
        mail: mail_provider,
        game_storage,
        plugin_registry,
        doc_rooms: crate::ws::new_room_manager(),
    });

    let plugin_routes: Option<Router<AppState>> = None; // TODO: integrate plugin routes with AppState

    let app = build_router(state, plugin_routes);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("\n  Project Board\n  http://localhost:{}\n", config.port);

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

/// Serve `index.html` for `/co` and `/co/{slug}` — SPA landing routes.
async fn serve_co_index(headers: HeaderMap, State(state): State<AppState>) -> Response {
    let variant = extract_variant(&headers, &state.config);
    let embed_path = format!("variants/{}/index.html", variant);
    let fs_path = std::path::Path::new(&state.config.static_dir).join(&embed_path);

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

async fn serve_variant_file(
    headers: HeaderMap,
    uri: Uri,
    State(state): State<AppState>,
) -> Response {
    let variant = extract_variant(&headers, &state.config);
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

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
        Some("js") => "application/javascript; charset=utf-8",
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

    let cookie =
        format!("session={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=604800");

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

async fn logout_handler() -> Response {
    let clear_cookie = "session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";
    (
        StatusCode::OK,
        [(header::SET_COOKIE, clear_cookie)],
        Json(serde_json::json!({ "message": "Logged out" })),
    )
        .into_response()
}

// --- UAT: password-based login (CO-44) ---

/// Request body for the UAT password login endpoint.
#[derive(serde::Deserialize)]
struct UatLoginRequest {
    email: String,
    password: String,
}

/// POST /api/v1/auth/uat-login — email + password login for UAT testing.
///
/// Only available when `CO_ENV=uat`. Returns 404 in production so the endpoint
/// existence is not revealed to non-UAT deployments.
async fn uat_login_handler(
    State(state): State<AppState>,
    Json(req): Json<UatLoginRequest>,
) -> Result<Response, AppError> {
    if !state.config.is_uat() {
        return Err(AppError::NotFound("Not found".into()));
    }

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

    let cookie =
        format!("session={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=604800");

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
