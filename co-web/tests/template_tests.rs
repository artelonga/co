// CO-22: Template universe — seed data, read-only, clone flow
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::models::*;
use co_web::server::{AppState, AppStateInner, build_router};
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
    }
}

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

fn build_template_app(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    // Seed the template universe (what start_server does on first boot)
    storage.seed_template_universe();
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
    );
    let state: AppState = Arc::new(AppStateInner {
        storage: Mutex::new(storage),
        experiment: Mutex::new(experiment),
        config,
        auth_store: Mutex::new(auth_store),
        mail,
        game_storage,
        plugin_registry: game_core::plugin::PluginRegistry::new(),
        doc_rooms: co_web::ws::new_room_manager(),
    });
    build_router(state, None)
}

// --- Test 1: template_exists after seed ---

#[tokio::test]
async fn test_template_exists_after_seed() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    assert!(!storage.template_exists(), "should not exist before seed");
    storage.seed_template_universe();
    assert!(storage.template_exists(), "should exist after seed");
}

// --- Test 2: seed idempotent (no duplicates) ---

#[tokio::test]
async fn test_seed_idempotent() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_template_universe();
    storage.seed_template_universe(); // second call must not fail or duplicate
    let projects = storage.list_projects_for_universe("template");
    assert_eq!(projects.len(), 1, "exactly one project expected");
}

// --- Test 3: template universe has 8 sample tasks ---

#[tokio::test]
async fn test_template_has_sample_tasks() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_template_universe();
    let tasks = storage.list_tasks("MP");
    assert_eq!(
        tasks.len(),
        8,
        "expected 8 sample tasks, got {}",
        tasks.len()
    );

    // All statuses present
    let statuses: std::collections::HashSet<_> =
        tasks.iter().map(|t| t.status.to_string()).collect();
    assert!(statuses.contains("todo"), "missing todo");
    assert!(statuses.contains("in_progress"), "missing in_progress");
    assert!(statuses.contains("in_review"), "missing in_review");
    assert!(statuses.contains("done"), "missing done");

    // Subtask relationships present (tasks with parent)
    let with_parent = tasks.iter().filter(|t| t.parent.is_some()).count();
    assert!(with_parent >= 2, "expected at least 2 subtasks");

    // Labels present
    let with_labels = tasks.iter().filter(|t| !t.labels.is_empty()).count();
    assert_eq!(with_labels, 8, "all tasks should have labels");
}

// --- Test 4: template projects visible without auth ---

#[tokio::test]
async fn test_template_projects_public() {
    let dir = tempdir().unwrap();
    let app = build_template_app(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/template/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let projects: Vec<Project> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].key, "MP");
}

// --- Test 5: write to template project returns 403 ---

#[tokio::test]
async fn test_write_to_template_forbidden() {
    let dir = tempdir().unwrap();
    let app = build_template_app(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/MP/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": "Should be blocked"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "expected 403 Forbidden for write to template project"
    );
}

// --- Test 6: update task in template returns 403 ---

#[tokio::test]
async fn test_update_template_task_forbidden() {
    let dir = tempdir().unwrap();
    let app = build_template_app(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/projects/MP/tasks/1")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": "Hacked"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// --- Test 7: clone template creates new universe ---

#[tokio::test]
async fn test_clone_template() {
    let dir = tempdir().unwrap();
    let app = build_template_app(dir.path());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/template/clone")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "Alice Universe",
                        "key": "alice",
                        "description": ""
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let universe: Universe = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(universe.key, "alice");
    assert!(
        !universe.is_template,
        "cloned universe must not be template"
    );

    // Verify cloned projects exist and are readable
    let proj_response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/alice/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // alice is private — should return 403 (not public)
    assert_eq!(proj_response.status(), StatusCode::FORBIDDEN);
}

// --- Test 8: after clone, cloned project is writable ---

#[tokio::test]
async fn test_cloned_project_writable() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_template_universe();
    // Clone manually to get the new project key
    let universe = storage
        .clone_universe("template", "bob", "Bob Universe", "", "bob-owner")
        .unwrap();
    assert_eq!(universe.key, "bob");

    // Find the cloned project key
    let projects = storage.list_projects_for_universe("bob");
    assert_eq!(projects.len(), 1);
    let proj_key = &projects[0].key;

    // Verify it is NOT in a template universe
    assert!(
        !storage.is_project_in_template(proj_key),
        "cloned project should not be in template universe"
    );

    // Verify tasks were copied
    let tasks = storage.list_tasks(proj_key);
    assert_eq!(tasks.len(), 8, "cloned project should have 8 tasks");
}

// --- Test 9: seed_template does not run twice (template_exists check) ---

#[tokio::test]
async fn test_template_exists_prevents_double_seed() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    assert!(!storage.template_exists());
    storage.seed_template_universe();
    assert!(storage.template_exists());
    // Simulate what start_server does: only seed if not exists
    if !storage.template_exists() {
        storage.seed_template_universe();
    }
    let projects = storage.list_projects_for_universe("template");
    assert_eq!(projects.len(), 1);
}
