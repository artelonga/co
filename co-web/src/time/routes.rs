//! CO-387: `GET /api/v1/universes/{slug}/calendar` — per-universe calendar
//! lens configuration.
//!
//! Returns the parsed `_calendar.yaml` (see [`super::calendar_loader`]) or
//! the built-in Gregorian default when the universe has none. Lens metadata
//! is universe-scoped and contains no entry data — safe for public read,
//! same posture as the CO-355 workspace-template registry.

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    response::IntoResponse,
    routing::get,
};

use super::calendar_loader::{CalendarConfig, load_calendar};
use crate::server::AppState;

async fn get_calendar_handler(
    State(state): State<AppState>,
    AxumPath(universe_slug): AxumPath<String>,
) -> impl IntoResponse {
    let root =
        crate::workspace_template_routes::resolve_universe_content_root(&state, &universe_slug);
    let cfg: CalendarConfig = load_calendar(&root);
    Json(cfg)
}

pub fn router() -> Router<AppState> {
    Router::new().route("/{slug}/calendar", get(get_calendar_handler))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let config = crate::config::WebConfig {
            data_dir: dir.to_str().unwrap().to_string(),
            port: 0,
            static_dir: String::new(),
            default_variant: "a".into(),
            experiments: false,
            plugins_dir: String::new(),
            game_db_path: None,
            universo_dir: String::new(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "test".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: true,
        };
        let storage = crate::storage::Storage::new(dir);
        let experiment = crate::experiment::ExperimentStore::new(dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let state = crate::server::AppState::new(crate::server::AppStateInner {
            core: Arc::new(crate::server::CoreState::from_storage(
                storage, config, auth_store,
            )),
            realtime: Arc::new(crate::server::RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
                chat_presence: Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(crate::server::IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(crate::server::IntegrationsState {
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

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// AC: universe without `_calendar.yaml` defaults to Gregorian.
    #[tokio::test]
    async fn calendar_defaults_to_gregorian_when_absent() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/universes/some-universe/calendar")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["default_lens"], "gregorian");
        assert_eq!(json["lenses"][0]["id"], "gregorian");
        assert_eq!(json["lenses"][0]["week_length_days"], 7);
    }
}
