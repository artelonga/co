//! CO-365: Admin backup API.
//!
//! POST /api/v1/admin/backup/snapshot — trigger a snapshot now
//! GET  /api/v1/admin/backup/snapshots — list stored snapshots

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};

use crate::admin::admin_routes::{check_admin_email, extract_claims};
use crate::server::AppState;
use crate::storage::backup::{SnapshotMeta, backend_from_env};

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
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/backup/snapshot", post(trigger_snapshot))
        .route("/backup/snapshots", get(list_snapshots))
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
            universo_dir: "quilomboaraucaria".to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
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
                rate_limiter: StdMutex::new(crate::rate_limit::RateLimiter::new()),
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
