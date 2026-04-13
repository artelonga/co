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
        co_env: "prod".into(),
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

// --- Test 3: template universe has 7 tutorial tasks ---

#[tokio::test]
async fn test_template_has_sample_tasks() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_template_universe();
    let tasks = storage.list_tasks("CO");
    assert_eq!(
        tasks.len(),
        9,
        "expected 9 onboarding tasks, got {}",
        tasks.len()
    );

    // All tasks should be todo (interactive tutorial)
    let statuses: std::collections::HashSet<_> =
        tasks.iter().map(|t| t.status.to_string()).collect();
    assert!(statuses.contains("todo"), "missing todo");

    // Subtask relationships present (at least one task with parent)
    let with_parent = tasks.iter().filter(|t| t.parent.is_some()).count();
    assert!(with_parent >= 1, "expected at least 1 subtask");

    // Labels present — all should have tutorial label
    let with_labels = tasks.iter().filter(|t| !t.labels.is_empty()).count();
    assert_eq!(with_labels, 9, "all tasks should have labels");
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
    assert_eq!(projects[0].key, "CO");
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
                .uri("/api/projects/CO/tasks")
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
                .uri("/api/projects/CO/tasks/1")
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
    let universe = storage
        .clone_universe("template", "bob", "Bob Universe", "", "bob-owner")
        .unwrap();
    assert_eq!(universe.key, "bob");

    let projects = storage.list_projects_for_universe("bob");
    assert_eq!(projects.len(), 1);
    let proj_key = &projects[0].key;

    assert!(
        !storage.is_project_in_template(proj_key),
        "cloned project should not be in template universe"
    );

    let tasks = storage.list_tasks(proj_key);
    assert_eq!(tasks.len(), 9, "cloned project should have 9 tasks");
}

// --- Test 9: seed_template does not run twice (template_exists check) ---

#[tokio::test]
async fn test_template_exists_prevents_double_seed() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    assert!(!storage.template_exists());
    storage.seed_template_universe();
    assert!(storage.template_exists());
    if !storage.template_exists() {
        storage.seed_template_universe();
    }
    let projects = storage.list_projects_for_universe("template");
    assert_eq!(projects.len(), 1);
}

// --- CO-38: Yggdrasil universe ---

#[tokio::test]
async fn test_yggdrasil_seed_and_requires_login() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    assert!(
        !storage.yggdrasil_universe_exists(),
        "yggdrasil should not exist before seed"
    );
    storage.seed_yggdrasil_universe();
    assert!(
        storage.yggdrasil_universe_exists(),
        "yggdrasil should exist after seed"
    );

    let u = storage
        .get_universe("yggdrasil")
        .expect("yggdrasil not found");
    assert!(u.requires_login, "yggdrasil must require login");
    assert!(u.is_public, "yggdrasil must be public");
    assert!(!u.is_template, "yggdrasil must not be a template");
    assert_eq!(u.owner_id, "system");

    // Seed is idempotent
    storage.seed_yggdrasil_universe();
    assert!(storage.yggdrasil_universe_exists());
}

/// CO-38: anonymous access to a requires_login universe returns 401.
#[tokio::test]
async fn test_yggdrasil_requires_login_returns_401() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_yggdrasil_universe();
    let app = build_template_app(dir.path());

    // Anonymous request (no auth header) — should get 401
    let req = Request::builder()
        .uri("/api/v1/universes/yggdrasil")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "anonymous access to yggdrasil must return 401"
    );
}

/// CO-38: authenticated access to yggdrasil succeeds.
#[tokio::test]
async fn test_yggdrasil_authenticated_access_ok() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_yggdrasil_universe();
    let app = build_template_app(dir.path());

    let req = Request::builder()
        .uri("/api/v1/universes/yggdrasil")
        .header(header::AUTHORIZATION, test_bearer())
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "authenticated access to yggdrasil should succeed"
    );

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info["key"], "yggdrasil");
    assert_eq!(info["requires_login"], true);
}

/// CO-38: non-yggdrasil public universes (template) remain accessible without login.
#[tokio::test]
async fn test_template_still_accessible_without_login() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path().to_str().unwrap());
    storage.seed_template_universe();
    storage.seed_yggdrasil_universe();
    let app = build_template_app(dir.path());

    let req = Request::builder()
        .uri("/api/v1/universes/template")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "template universe must still be accessible anonymously"
    );
}
