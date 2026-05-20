use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::models::*;
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use co_web::storage::Storage;
extern crate co;

fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 3000,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: true,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: "quilomboaraucaria".to_string(),
        gestao_github_admins: vec!["artelonga".to_string()],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
    }
}

/// Generate a valid JWT for test write requests using the default dev secret.
fn test_bearer() -> String {
    let (token, _) = co_web::auth::sign_jwt(
        "test-user",
        "test@example.com",
        "player",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

fn build_test_app(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);

    // Create a project for tests
    storage
        .create_project(CreateProject {
            name: "Concurrency Test".into(),
            key: "CC".into(),
            description: "".into(),
            universe_key: None,
        })
        .unwrap();

    let experiment = ExperimentStore::new(&config.data_dir);

    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
    );
    let state: AppState = AppState::new(AppStateInner {
        core: Arc::new(CoreState {
            storage: parking_lot::Mutex::new(storage),
            config,
            auth_store: Mutex::new(auth_store),
            event_bus: co_web::events::Bus::new(),
        }),
        realtime: Arc::new(RealtimeState {
            doc_rooms: co_web::ws::new_room_manager(),
            sync_rooms: co_web::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: co_web::cache::CacheLayer::new(),
            embeddings: std::sync::Arc::new(co_web::embedding::EmbeddingService::disabled()),
            embedding_tx: {
                let (tx, _) = co_web::embedding_worker::channel();
                tx
            },
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: std::sync::Arc::new(co_web::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: co_web::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(co_web::auth::JwtKey::load_or_generate()),
            rate_limiter: std::sync::Mutex::new(co_web::rate_limit::RateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: co_web::worker_supervisor::WorkerSupervisor::new(),
        }),
    });

    build_router(state, None)
}

#[tokio::test]
async fn test_concurrent_task_creation() {
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    let mut handles = Vec::new();

    for i in 0..10 {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/projects/CC/tasks")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::AUTHORIZATION, test_bearer())
                        .body(Body::from(
                            serde_json::to_string(&serde_json::json!({
                                "title": format!("Concurrent task {i}"),
                                "description": "Created concurrently"
                            }))
                            .unwrap(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::CREATED);

            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let task: Task = serde_json::from_slice(&bytes).unwrap();
            task.id
        }));
    }

    let mut ids = Vec::new();
    for handle in handles {
        ids.push(handle.await.unwrap());
    }

    // All IDs should be unique
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 10, "Expected 10 unique task IDs, got {:?}", ids);

    // Verify total count
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/CC/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let tasks: Vec<Task> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(tasks.len(), 10);
}

#[tokio::test]
async fn test_concurrent_read_write() {
    let dir = tempdir().unwrap();
    let app = build_test_app(dir.path());

    // Seed some tasks first
    for i in 0..5 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects/CC/tasks")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, test_bearer())
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "title": format!("Seed task {i}"),
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    let mut handles = Vec::new();

    // Spawn readers
    for _ in 0..5 {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/api/projects/CC/tasks")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let tasks: Vec<Task> = serde_json::from_slice(&bytes).unwrap();
            assert!(tasks.len() >= 5, "Expected at least 5 tasks");
        }));
    }

    // Spawn writers simultaneously
    for i in 0..5 {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/projects/CC/tasks")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::AUTHORIZATION, test_bearer())
                        .body(Body::from(
                            serde_json::to_string(&serde_json::json!({
                                "title": format!("Concurrent write {i}"),
                            }))
                            .unwrap(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::CREATED);
        }));
    }

    // All should complete without panics
    for handle in handles {
        handle.await.unwrap();
    }

    // Final state: 5 seed + 5 concurrent = 10
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/CC/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let tasks: Vec<Task> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(tasks.len(), 10);
}
