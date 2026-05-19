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

// --- Submodules ---

pub(crate) mod auth_handlers;
pub(crate) mod legacy;
pub mod router;
pub mod seed_orchestrator;
pub mod state;
pub(crate) mod static_files;
#[cfg(test)]
mod tests;
pub mod uat_boot;
pub mod validation;

// --- Re-exports (public API at `crate::server::*`) ---

pub use router::build_router;
pub use state::{AppState, AppStateInner, lock_auth, lock_experiment, lock_storage};
pub use validation::*;

use auth_handlers::*;
use legacy::*;
use static_files::*;

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
async fn cache_stats_handler(State(state): State<AppState>) -> Json<crate::cache::CacheStats> {
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

    seed_orchestrator::run_startup_seeds(&config);
    seed_orchestrator::run_co142_refresh(&config);

    // CO-44: UAT-specific startup — runs only when CO_ENV=uat.
    let uat_reset_just_happened = if config.is_uat() {
        tracing::info!("UAT mode enabled (CO_ENV=uat)");
        uat_boot::uat_startup(&config)
    } else {
        false
    };

    seed_orchestrator::run_admin_seeding(&config);
    seed_orchestrator::backfill_chat_push(&config);

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

    let geo = {
        let path = std::env::var("GEOIP_DB_PATH")
            .unwrap_or_else(|_| "/data/GeoLite2-City.mmdb".to_string());
        std::sync::Arc::new(crate::geo::GeoDb::open(&path))
    };

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
        geo,
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
