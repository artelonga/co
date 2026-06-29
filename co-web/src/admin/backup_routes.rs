//! CO-365: Admin backup API.
//!
//! POST /api/v1/admin/backup/snapshot — trigger a snapshot now
//! GET  /api/v1/admin/backup/snapshots — list stored snapshots

use std::time::SystemTime;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::Deserialize;

use crate::admin::admin_routes::{check_admin_email, extract_claims};
use crate::platform::universe_pool::UniversePool;
use crate::server::AppState;
use crate::storage::backup::{SnapshotMeta, backend_from_env, snapshot_dir};
use crate::storage::sweep::{self, SweepOptions};

// ---------------------------------------------------------------------------
// POST /api/v1/admin/backup/snapshot
// ---------------------------------------------------------------------------

pub async fn trigger_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Response> {
    let claims = extract_claims(&headers)
        .map_err(|s| (s, Json(serde_json::json!({"error": "Unauthorized"}))).into_response())?;

    if !check_admin_email(&claims.email) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Forbidden"})),
        )
            .into_response());
    }

    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::storage::backup::worker::run_backup_tick(&state_clone).await {
            tracing::error!("manual backup snapshot failed: {e:#}");
        }
    });

    Ok(Json(serde_json::json!({
        "status": "accepted",
        "message": "Snapshot started in background",
    })))
}

// ---------------------------------------------------------------------------
// GET /api/v1/admin/backup/snapshots
// ---------------------------------------------------------------------------

pub async fn list_snapshots(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SnapshotMeta>>, Response> {
    let claims = extract_claims(&headers)
        .map_err(|s| (s, Json(serde_json::json!({"error": "Unauthorized"}))).into_response())?;

    if !check_admin_email(&claims.email) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Forbidden"})),
        )
            .into_response());
    }

    let data_dir = {
        let storage = state.core.storage.lock();
        storage.data_dir.clone()
    };

    let backend = match backend_from_env(&data_dir) {
        Some(b) => b,
        None => return Ok(Json(vec![])),
    };

    match backend.list().await {
        Ok(metas) => Ok(Json(metas)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response()),
    }
}

// ---------------------------------------------------------------------------
// CO-459: POST /api/v1/admin/backup
// Reconstructable local snapshot (VACUUM INTO + per-file sha256 manifest),
// built + verified synchronously so the manifest is returned to the caller.
// ---------------------------------------------------------------------------

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"error": "Forbidden"})),
    )
        .into_response()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "Unauthorized"})),
    )
        .into_response()
}

pub async fn trigger_local_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Response> {
    let claims = extract_claims(&headers).map_err(|_| unauthorized())?;
    if !check_admin_email(&claims.email) {
        return Err(forbidden());
    }

    let data_dir = {
        let storage = state.core.storage.lock();
        storage.data_dir.clone()
    };

    let result = tokio::task::spawn_blocking(move || {
        let root = snapshot_dir::backups_root(&data_dir);
        std::fs::create_dir_all(&root)?;
        let (dir, manifest) = snapshot_dir::build_local_snapshot(&data_dir, &root, Utc::now())?;
        let report = snapshot_dir::verify_snapshot(&dir)?;
        anyhow::Ok((dir, manifest, report))
    })
    .await;

    match result {
        Ok(Ok((dir, manifest, report))) => Ok(Json(serde_json::json!({
            "status": "ok",
            "snapshot_dir": dir.to_string_lossy(),
            "verified": report.verified,
            "manifest": manifest,
        }))),
        Ok(Err(e)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response()),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("snapshot task panicked: {e}")})),
        )
            .into_response()),
    }
}

// ---------------------------------------------------------------------------
// CO-459: POST /api/v1/admin/sweep
// Junk sweep. Dry-run by default; `?apply=true` performs the deletions.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct SweepQuery {
    #[serde(default)]
    pub apply: bool,
}

pub async fn run_sweep(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SweepQuery>,
) -> Result<Json<serde_json::Value>, Response> {
    let claims = extract_claims(&headers).map_err(|_| unauthorized())?;
    if !check_admin_email(&claims.email) {
        return Err(forbidden());
    }

    let data_dir = {
        let storage = state.core.storage.lock();
        storage.data_dir.clone()
    };
    let apply = q.apply;

    let result = tokio::task::spawn_blocking(move || {
        let opts = SweepOptions::from_env();
        let root = snapshot_dir::backups_root(&data_dir);
        let pool = UniversePool::new(&data_dir, 16);
        let conn = rusqlite::Connection::open(data_dir.join("meta.db"))?;
        let mut report = sweep::plan_sweep(
            &conn,
            &data_dir,
            &pool,
            &root,
            &opts,
            Utc::now(),
            SystemTime::now(),
        );
        if apply {
            sweep::apply_sweep(&conn, &mut report);
        }
        anyhow::Ok(report)
    })
    .await;

    match result {
        Ok(Ok(report)) => Ok(Json(serde_json::to_value(&report).unwrap_or_default())),
        Ok(Err(e)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response()),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("sweep task panicked: {e}")})),
        )
            .into_response()),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/backup", post(trigger_local_backup))
        .route("/backup/snapshot", post(trigger_snapshot))
        .route("/backup/snapshots", get(list_snapshots))
        .route("/sweep", post(run_sweep))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxumStatus};
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::server::{CoreState, IndexState, IntegrationsState, RealtimeState};

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        let config = crate::config::WebConfig {
            port: 3000,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: "universo".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_api_key: None,
            wae_endpoint: None,
            cookie_domain: None,
            bypass_rate_limit: false,
        };
        let storage = crate::storage::Storage::new(&config.data_dir);
        let experiment = crate::experiment::ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&game_db_path)
                .expect("Failed to open test game storage"),
        );
        let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
        let state = crate::server::AppState::new(crate::server::AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: StdMutex::new(std::collections::HashMap::new()),
                chat_presence: StdMutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: StdMutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: StdMutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        crate::server::build_router(state, None)
    }

    #[tokio::test]
    async fn trigger_snapshot_no_auth_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/backup/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn trigger_local_backup_no_auth_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/backup")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn run_sweep_no_auth_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/sweep")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_snapshots_no_auth_returns_401() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/admin/backup/snapshots")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), AxumStatus::UNAUTHORIZED);
    }
}
