//! CO-333: Feedback system — Yggdrasil-compatible + per-entry locus.
//!
//! POST /api/v1/feedback                              — universe-wide (Yggdrasil compat)
//! POST /api/v1/feedback/{universe}/{*entry_path}     — per-entry locus
//! GET  /api/v1/feedback/{universe}                   — list; owner: all, anon: open sugestao
//! GET  /api/v1/feedback/{universe}/entry/{*path}     — per-entry list (anon-safe)
//! PATCH /api/v1/feedback/{id}                        — status update (owner-only)

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/feedback", post(submit_universe_wide))
        // GET = list universe; PATCH = update status (key is universe_key or feedback_id resp.)
        .route(
            "/feedback/{key}",
            get(list_universe_feedback).patch(update_status),
        )
        .route(
            "/feedback/{universe_key}/{*entry_path}",
            post(submit_per_entry),
        )
        .route(
            "/feedback/{universe_key}/entry/{*entry_path}",
            get(list_entry_feedback),
        )
}

// ---------------------------------------------------------------------------
// Rate limiter: 10 submissions / hour per IP
// ---------------------------------------------------------------------------

static FEEDBACK_RATE: OnceLock<parking_lot::Mutex<HashMap<String, VecDeque<Instant>>>> =
    OnceLock::new();

fn check_rate(ip: &str) -> bool {
    let store = FEEDBACK_RATE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()));
    let mut store = store.lock();
    let window = store.entry(ip.to_string()).or_default();
    let cutoff = Instant::now() - Duration::from_secs(3600);
    while window.front().is_some_and(|t| *t < cutoff) {
        window.pop_front();
    }
    if window.len() >= 10 {
        return false;
    }
    window.push_back(Instant::now());
    true
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SubmitFeedbackBody {
    /// Yggdrasil compat: universe key in body (for POST /feedback without URL params).
    pub universe: Option<String>,
    pub kind: String,
    pub message: String,
    pub name: Option<String>,
    pub email: Option<String>,
    /// Optional entry_path when submitting from chat (CO-332 integration).
    pub entry_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FeedbackCreated {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackItem {
    pub id: String,
    pub universe_key: String,
    pub entry_path: Option<String>,
    pub kind: String,
    pub message: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub user_sub: Option<String>,
    pub anonymous: bool,
    pub created_at: i64,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStatusBody {
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct FeedbackList {
    pub items: Vec<FeedbackItem>,
    pub total: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Input for a new feedback row (avoids too-many-arguments clippy lint).
pub struct FeedbackCreate<'a> {
    pub universe_key: &'a str,
    pub entry_path: Option<&'a str>,
    pub kind: &'a str,
    pub message: &'a str,
    pub name: Option<&'a str>,
    pub email: Option<&'a str>,
    pub user_sub: Option<&'a str>,
}

fn validate_kind(kind: &str) -> Result<(), AppError> {
    match kind {
        "feedback" | "duvida" | "sugestao" => Ok(()),
        _ => Err(AppError::BadRequest(format!(
            "kind must be 'feedback', 'duvida', or 'sugestao'; got '{kind}'"
        ))),
    }
}

fn universe_owner(state: &AppState, universe_key: &str) -> Result<String, AppError> {
    state
        .core
        .storage
        .lock()
        .get_universe(universe_key)
        .map(|u| u.owner_id)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{universe_key}' not found")))
}

pub fn insert_feedback(state: &AppState, create: FeedbackCreate<'_>) -> Result<String, AppError> {
    let id = nanoid::nanoid!(21);
    let now = chrono::Utc::now().timestamp();
    let anonymous = i64::from(create.user_sub.is_none());
    state
        .core
        .storage
        .lock()
        .conn()
        .execute(
            "INSERT INTO feedback \
             (id, universe_key, entry_path, kind, message, name, email, user_sub, anonymous, created_at, status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'open')",
            rusqlite::params![
                id, create.universe_key, create.entry_path, create.kind, create.message,
                create.name, create.email, create.user_sub, anonymous, now
            ],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(id)
}

fn notify_owner(state: &AppState, universe_key: &str, entry_path: Option<&str>, id: &str) {
    let Ok(owner_id) = universe_owner(state, universe_key) else {
        return;
    };
    let storage = state.core.storage.lock();
    let _ = storage.create_notification(
        &owner_id,
        "feedback_received",
        Some(universe_key),
        None,
        "system",
        id,
        "feedback.received",
        serde_json::json!({"entry_path": entry_path, "feedback_id": id}),
    );
}

/// Fire-and-forget federation forward (CO_FEEDBACK_FORWARD_URL env var).
fn maybe_forward(
    universe_key: String,
    entry_path: Option<String>,
    kind: String,
    message: String,
    name: Option<String>,
    email: Option<String>,
) {
    let Ok(url) = std::env::var("CO_FEEDBACK_FORWARD_URL") else {
        return;
    };
    tokio::spawn(async move {
        let body = serde_json::json!({
            "universe": universe_key,
            "entry_path": entry_path,
            "kind": kind,
            "message": message,
            "name": name,
            "email": email,
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        let _ = client.post(&url).json(&body).send().await;
    });
}

fn read_items(
    state: &AppState,
    sql: &str,
    p1: &str,
    p2: Option<&str>,
) -> Result<Vec<FeedbackItem>, AppError> {
    let storage = state.core.storage.lock();
    let conn = storage.conn();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let items: Vec<FeedbackItem> = if let Some(p2v) = p2 {
        stmt.query_map(rusqlite::params![p1, p2v], row_to_item)
    } else {
        stmt.query_map(rusqlite::params![p1], row_to_item)
    }
    .map_err(|e| AppError::Internal(e.to_string()))?
    .filter_map(|r| r.ok())
    .collect();
    Ok(items)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedbackItem> {
    Ok(FeedbackItem {
        id: row.get(0)?,
        universe_key: row.get(1)?,
        entry_path: row.get(2)?,
        kind: row.get(3)?,
        message: row.get(4)?,
        name: row.get(5)?,
        email: row.get(6)?,
        user_sub: row.get(7)?,
        anonymous: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        status: row.get(10)?,
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/feedback — Yggdrasil-compatible universe-wide submission.
pub async fn submit_universe_wide(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<SubmitFeedbackBody>,
) -> Result<impl IntoResponse, AppError> {
    let ip = client_ip(&headers);
    if !check_rate(&ip) {
        return Err(AppError::TooManyRequests(
            "Rate limit: 10 feedback submissions per hour".into(),
        ));
    }

    let universe_key = body
        .universe
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("'universe' field required".into()))?;
    validate_kind(&body.kind)?;
    if body.message.trim().is_empty() {
        return Err(AppError::BadRequest("message cannot be empty".into()));
    }

    // Verify universe exists
    universe_owner(&state, universe_key)?;

    let user_sub = crate::auth::resolve_user_id(&state, &headers);
    let entry_path = body.entry_path.as_deref();

    let id = insert_feedback(
        &state,
        FeedbackCreate {
            universe_key,
            entry_path,
            kind: &body.kind,
            message: body.message.trim(),
            name: body.name.as_deref(),
            email: body.email.as_deref(),
            user_sub: user_sub.as_deref(),
        },
    )?;

    maybe_forward(
        universe_key.to_string(),
        entry_path.map(String::from),
        body.kind.clone(),
        body.message.trim().to_string(),
        body.name.clone(),
        body.email.clone(),
    );
    notify_owner(&state, universe_key, entry_path, &id);

    Ok((StatusCode::CREATED, axum::Json(FeedbackCreated { id })))
}

/// POST /api/v1/feedback/{universe}/{*entry_path} — per-entry locus.
pub async fn submit_per_entry(
    State(state): State<AppState>,
    Path((universe_key, entry_path)): Path<(String, String)>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<SubmitFeedbackBody>,
) -> Result<impl IntoResponse, AppError> {
    let ip = client_ip(&headers);
    if !check_rate(&ip) {
        return Err(AppError::TooManyRequests(
            "Rate limit: 10 feedback submissions per hour".into(),
        ));
    }

    validate_kind(&body.kind)?;
    if body.message.trim().is_empty() {
        return Err(AppError::BadRequest("message cannot be empty".into()));
    }

    // Verify universe exists
    universe_owner(&state, &universe_key)?;

    let user_sub = crate::auth::resolve_user_id(&state, &headers);

    let id = insert_feedback(
        &state,
        FeedbackCreate {
            universe_key: &universe_key,
            entry_path: Some(&entry_path),
            kind: &body.kind,
            message: body.message.trim(),
            name: body.name.as_deref(),
            email: body.email.as_deref(),
            user_sub: user_sub.as_deref(),
        },
    )?;

    maybe_forward(
        universe_key.clone(),
        Some(entry_path.clone()),
        body.kind.clone(),
        body.message.trim().to_string(),
        body.name.clone(),
        body.email.clone(),
    );
    notify_owner(&state, &universe_key, Some(&entry_path), &id);

    Ok((StatusCode::CREATED, axum::Json(FeedbackCreated { id })))
}

/// GET /api/v1/feedback/{universe} — list feedback for the universe.
/// Owner sees all; anonymous sees only open sugestao.
pub async fn list_universe_feedback(
    State(state): State<AppState>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let owner_id = universe_owner(&state, &key)?;
    let caller = crate::auth::resolve_user_id(&state, &headers);
    let is_owner = caller.as_deref() == Some(owner_id.as_str());

    let items = if is_owner {
        read_items(
            &state,
            "SELECT id, universe_key, entry_path, kind, message, name, email, user_sub, \
             anonymous, created_at, status FROM feedback \
             WHERE universe_key = ?1 ORDER BY created_at DESC",
            &key,
            None,
        )?
    } else {
        read_items(
            &state,
            "SELECT id, universe_key, entry_path, kind, message, name, email, user_sub, \
             anonymous, created_at, status FROM feedback \
             WHERE universe_key = ?1 AND kind = 'sugestao' AND status = 'open' \
             ORDER BY created_at DESC",
            &key,
            None,
        )?
    };

    let total = items.len();
    Ok(axum::Json(FeedbackList { items, total }))
}

/// GET /api/v1/feedback/{universe}/entry/{*path} — per-entry list (anon-safe).
pub async fn list_entry_feedback(
    State(state): State<AppState>,
    Path((universe_key, entry_path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let owner_id = universe_owner(&state, &universe_key)?;
    let caller = crate::auth::resolve_user_id(&state, &headers);
    let is_owner = caller.as_deref() == Some(owner_id.as_str());

    let items = if is_owner {
        read_items(
            &state,
            "SELECT id, universe_key, entry_path, kind, message, name, email, user_sub, \
             anonymous, created_at, status FROM feedback \
             WHERE universe_key = ?1 AND entry_path = ?2 ORDER BY created_at DESC",
            &universe_key,
            Some(&entry_path),
        )?
    } else {
        read_items(
            &state,
            "SELECT id, universe_key, entry_path, kind, message, name, email, user_sub, \
             anonymous, created_at, status FROM feedback \
             WHERE universe_key = ?1 AND entry_path = ?2 \
             AND kind = 'sugestao' AND status = 'open' ORDER BY created_at DESC",
            &universe_key,
            Some(&entry_path),
        )?
    };

    let total = items.len();
    Ok(axum::Json(FeedbackList { items, total }))
}

/// PATCH /api/v1/feedback/{id} — update status (owner-only).
pub async fn update_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UpdateStatusBody>,
) -> Result<impl IntoResponse, AppError> {
    match body.status.as_str() {
        "open" | "reviewed" | "addressed" => {}
        _ => {
            return Err(AppError::BadRequest(format!(
                "status must be 'open', 'reviewed', or 'addressed'; got '{}'",
                body.status
            )));
        }
    }

    let universe_key: String = state
        .core
        .storage
        .lock()
        .conn()
        .query_row(
            "SELECT universe_key FROM feedback WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .map_err(|_| AppError::NotFound(format!("Feedback '{id}' not found")))?;

    let owner_id = universe_owner(&state, &universe_key)?;
    let caller = crate::auth::resolve_user_id(&state, &headers)
        .ok_or_else(|| AppError::Unauthorized("Authentication required".into()))?;

    if caller != owner_id {
        return Err(AppError::Forbidden(
            "Only the universe owner can update feedback status".into(),
        ));
    }

    state
        .core
        .storage
        .lock()
        .conn()
        .execute(
            "UPDATE feedback SET status = ?1 WHERE id = ?2",
            rusqlite::params![body.status, id],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(StatusCode::OK)
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
    use crate::server::{
        AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
    };
    use crate::storage::Storage;

    fn isolate_env() {
        unsafe { std::env::set_var("JWT_SECRET", "test-feedback-secret") };
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
            co_env: "test".into(),
            wae_api_key: None,
            wae_endpoint: None,
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
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("open game storage"));
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let state = AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
                chat_presence: Mutex::new(std::collections::HashMap::new()),
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
                rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        crate::server::build_router(state, None)
    }

    fn insert_user(dir: &std::path::Path, email: &str) -> String {
        let storage = Storage::new(dir.to_str().unwrap());
        let id = format!("usr_{}", nanoid::nanoid!(8));
        let usuario = email.split('@').next().unwrap_or("u").to_lowercase();
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
                 VALUES (?1, ?2, ?3, 'player', ?4, ?5)",
                params![id, email, email, now, usuario],
            )
            .unwrap();
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
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'owner', ?3)",
                params![key, owner_id, now],
            )
            .unwrap();
        storage.ensure_default_room(key).unwrap();
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    // -----------------------------------------------------------------------
    // 1. POST /api/v1/feedback — universe-wide (Yggdrasil compat)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_submit_feedback_universe_wide_201() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb1@example.com");
        insert_universe(dir.path(), "fb_uni1", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.0.1")
                    .body(Body::from(
                        r#"{"universe":"fb_uni1","kind":"feedback","message":"Great content!"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let json = body_json(resp.into_body()).await;
        assert!(json["id"].is_string());
    }

    // -----------------------------------------------------------------------
    // 2. POST /api/v1/feedback/{universe}/{*path} — per-entry locus
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_submit_feedback_per_entry_201() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb2@example.com");
        insert_universe(dir.path(), "fb_uni2", &owner_id);

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback/fb_uni2/content/about.md")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.0.2")
                    .body(Body::from(
                        r#"{"kind":"duvida","message":"What does this mean?"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let json = body_json(resp.into_body()).await;
        assert!(json["id"].is_string());
    }

    // -----------------------------------------------------------------------
    // 3. GET /api/v1/feedback/{universe} — list (no auth = only sugestao/open)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_universe_feedback_anon() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb3@example.com");
        insert_universe(dir.path(), "fb_uni3", &owner_id);

        // Insert one feedback, one sugestao
        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('id1', 'fb_uni3', 'feedback', 'private comment', 1, 1000, 'open')",
                    [],
                )
                .unwrap();
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('id2', 'fb_uni3', 'sugestao', 'public suggestion', 1, 2000, 'open')",
                    [],
                )
                .unwrap();
        }

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/feedback/fb_uni3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        // Anon should only see sugestao/open
        assert_eq!(json["total"], 1);
        assert_eq!(json["items"][0]["kind"], "sugestao");
    }

    // Helper: create HS256 JWT signed with the test JWT_SECRET.
    fn test_jwt(user_id: &str) -> String {
        use jsonwebtoken::{Algorithm, EncodingKey, Header};
        #[derive(serde::Serialize)]
        struct C {
            sub: String,
            exp: i64,
            iat: i64,
            email: String,
            tier: String,
            usuario: String,
        }
        jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &C {
                sub: user_id.to_string(),
                exp: chrono::Utc::now().timestamp() + 3600,
                iat: chrono::Utc::now().timestamp(),
                email: String::new(),
                tier: "player".into(),
                usuario: user_id.to_string(),
            },
            &EncodingKey::from_secret(b"test-feedback-secret"),
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // 4. PATCH /api/v1/feedback/{id} — owner updates status
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_status_owner_200() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb4@example.com");
        insert_universe(dir.path(), "fb_uni4", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('fb_id4', 'fb_uni4', 'feedback', 'hello', 1, 1000, 'open')",
                    [],
                )
                .unwrap();
        }

        let token = test_jwt(&owner_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/feedback/fb_id4")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"status":"reviewed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let storage2 = Storage::new(dir.path().to_str().unwrap());
        let status: String = storage2
            .conn()
            .query_row(
                "SELECT status FROM feedback WHERE id = 'fb_id4'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "reviewed");
    }

    // -----------------------------------------------------------------------
    // 5. PATCH /api/v1/feedback/{id} — non-owner gets 403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_status_non_owner_403() {
        isolate_env();
        let dir = tempdir().unwrap();
        let owner_id = insert_user(dir.path(), "owner_fb5@example.com");
        let other_id = insert_user(dir.path(), "other_fb5@example.com");
        insert_universe(dir.path(), "fb_uni5", &owner_id);

        {
            let storage = Storage::new(dir.path().to_str().unwrap());
            storage
                .conn()
                .execute(
                    "INSERT INTO feedback (id, universe_key, kind, message, anonymous, created_at, status) \
                     VALUES ('fb_id5', 'fb_uni5', 'feedback', 'hello', 1, 1000, 'open')",
                    [],
                )
                .unwrap();
        }

        let token = test_jwt(&other_id);
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/feedback/fb_id5")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"status":"reviewed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // 6. POST — unknown universe returns 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_submit_unknown_universe_404() {
        isolate_env();
        let dir = tempdir().unwrap();

        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/feedback")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "10.0.0.6")
                    .body(Body::from(
                        r#"{"universe":"nonexistent","kind":"feedback","message":"hello"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
