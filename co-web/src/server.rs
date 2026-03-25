use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{DefaultBodyLimit, Json, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
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
    // --- co-web auth (email codes) ---
    let auth_api = Router::new()
        .route("/v1/auth/login", post(login_handler))
        .route("/v1/auth/verify", post(verify_handler));

    // --- Task/project CRUD (co-web) ---
    let api = Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{key}", get(get_project))
        .route("/projects/{key}/tasks", get(list_tasks).post(create_task))
        .route(
            "/projects/{key}/tasks/{id}",
            get(get_task).put(update_task).delete(delete_task),
        )
        .route(
            "/projects/{key}/tasks/{id}/comments",
            get(list_comments).post(create_comment),
        )
        .route("/projects/{key}/activity", get(list_activity))
        .route("/projects/{key}/dashboard", get(get_dashboard))
        .route("/projects/{key}/tasks/bulk-update", post(bulk_update_tasks))
        .route("/projects/{key}/tasks/bulk-delete", post(bulk_delete_tasks))
        .route("/health", get(health_check));

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
        .route("/v1/games/{game_name}/leaderboard", get(game_routes::get_leaderboard))
        .route("/v1/players/{username}", get(game_routes::get_player_profile));

    let game_protected = Router::new()
        .route("/v1/profile", get(game_routes::get_profile))
        .route("/v1/wallet", get(game_routes::get_wallet))
        .route("/v1/games/{game_name}/result", post(game_routes::record_game_result))
        .route("/v1/games/{game_name}/stats", get(game_routes::get_game_stats))
        .layer(axum::middleware::from_fn(crate::auth::require_auth));

    // Middleware stack
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE]);

    let mut router = Router::new()
        .nest("/api", api)
        .nest("/api", auth_api)
        .nest("/api", experiment_api)
        .nest("/api", game_public)
        .nest("/api", game_protected);

    // Mount plugin routes if any plugins were loaded
    if let Some(plugin_router) = plugin_routes {
        router = router.nest("/api/v1/universes", plugin_router);
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

    let storage = Storage::new(&config.data_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = AuthStore::new(std::path::Path::new(&config.data_dir))
        .expect("Failed to create auth store");

    let mail_provider: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);

    // Initialize game-core encrypted storage
    let game_db_path = config.game_db_path.clone().unwrap_or_else(|| {
        let data_dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        data_dir.join("game").join("game.db").to_string_lossy().to_string()
    });
    let game_db_dir = std::path::Path::new(&game_db_path).parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(game_db_dir).ok();
    let game_storage = Arc::new(
        game_core::storage::Storage::open(std::path::Path::new(&game_db_path))
            .expect("Failed to open game storage")
    );
    tracing::info!("Game storage opened at {}", game_db_path);

    // Load plugins
    let plugins_dir = std::path::Path::new(&config.plugins_dir);
    let (plugin_registry, plugin_router) =
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
        Some("css") | Some("js") => HeaderValue::from_static("public, max-age=31536000, immutable"),
        Some("png") | Some("svg") | Some("ico") => {
            HeaderValue::from_static("public, max-age=31536000, immutable")
        }
        _ => HeaderValue::from_static("no-cache"),
    }
}

// --- Project Handlers ---

async fn list_projects(State(state): State<AppState>) -> Result<Json<Vec<Project>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(storage.list_projects()))
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
    Json(body): Json<CreateProject>,
) -> Result<impl IntoResponse, AppError> {
    validate_project_name(&body.name)?;
    validate_project_key(&body.key)?;

    let mut storage = lock_storage(&state)?;
    storage
        .create_project(body)
        .map(|p| (StatusCode::CREATED, Json(p)))
        .map_err(|e| AppError::Conflict(e.to_string()))
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
    validate_task_title(&body.title)?;
    validate_task_description(&body.description)?;
    validate_labels(&body.labels)?;

    let mut storage = lock_storage(&state)?;
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
    let mut storage = lock_storage(&state)?;
    storage
        .delete_task(&key, id)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| AppError::NotFound(e.to_string()))
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
    validate_comment_body(&body.body)?;
    validate_comment_author(&body.author)?;

    let mut storage = lock_storage(&state)?;
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

    // Look up user (we don't reveal whether the user exists).
    let user = {
        let storage = lock_storage(&state)?;
        storage.get_user_by_email(&email)
    };

    let user_id = user.as_ref().map(|u| u.id.clone());
    let code = generate_code();
    let entry = new_code_entry(user_id, code.clone());

    {
        let auth = lock_auth(&state)?;
        auth.store_code(&email, &entry)?;
    }

    // Send email only if user exists (but we always return 200).
    if user.is_some() {
        let subject = "Your login code";
        let body_text =
            format!("Your verification code is: {code}\n\nThis code expires in 5 minutes.");
        if let Err(e) = state.mail.send(&email, subject, &body_text) {
            tracing::warn!("Failed to send verification email to {email}: {e}");
        }
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

    // Code matches — sign JWT.
    let user_id = match entry.user_id {
        Some(id) => id,
        None => {
            // No user found for this email; treat as wrong code.
            let auth = lock_auth(&state)?;
            auth.delete_code(&email)?;
            let body = serde_json::json!({ "error": "Code expired, request a new one" });
            return Ok((StatusCode::UNAUTHORIZED, Json(body)).into_response());
        }
    };

    let user = {
        let storage = lock_storage(&state)?;
        storage.get_user_by_email(&email)
    };

    let (display_name, tier) = user
        .as_ref()
        .map(|u| (u.display_name.clone(), u.tier.clone()))
        .unwrap_or_else(|| ("".to_string(), "player".to_string()));

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
