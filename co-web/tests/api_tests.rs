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
use co_web::storage::{Storage, seed_data};
extern crate co;

// --- Helpers ---

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
    }
}

fn build_test_router(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    seed_data(&mut storage);
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
    });

    build_router(state, None)
}

// --- Helper to read body ---
async fn body_to_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// --- Tests ---

#[tokio::test]
async fn test_list_projects_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let projects: Vec<Project> = serde_json::from_str(&body).unwrap();
    assert_eq!(projects.len(), 3);
    assert!(projects.iter().any(|p| p.key == "DS"));
    assert!(projects.iter().any(|p| p.key == "API"));
    assert!(projects.iter().any(|p| p.key == "PLT"));
}

#[tokio::test]
async fn test_get_project_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let project: Project = serde_json::from_str(&body).unwrap();
    assert_eq!(project.key, "DS");
    assert_eq!(project.name, "Design System");
}

#[tokio::test]
async fn test_get_project_not_found() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/NOPE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Should return JSON error
    let body = body_to_string(response.into_body()).await;
    let error: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(error["error"], "not_found");
}

#[tokio::test]
async fn test_list_tasks_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let tasks: Vec<Task> = serde_json::from_str(&body).unwrap();
    assert_eq!(tasks.len(), 7);
    assert_eq!(tasks[0].project_key, "DS");
}

#[tokio::test]
async fn test_get_task_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS/tasks/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let task: Task = serde_json::from_str(&body).unwrap();
    assert_eq!(task.id, 1);
    assert_eq!(task.key, "DS-1");
}

#[tokio::test]
async fn test_create_task_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": "New API task",
                        "description": "Created via test",
                        "priority": "high"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = body_to_string(response.into_body()).await;
    let task: Task = serde_json::from_str(&body).unwrap();
    assert_eq!(task.title, "New API task");
    assert_eq!(task.id, 8); // After 7 seed tasks
    assert_eq!(task.status, TaskStatus::Todo); // default
}

#[tokio::test]
async fn test_update_task_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/projects/DS/tasks/1")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "status": "done",
                        "title": "Updated via API"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let task: Task = serde_json::from_str(&body).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.title, "Updated via API");
}

#[tokio::test]
async fn test_delete_task_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/projects/DS/tasks/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_delete_task_not_found() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/projects/DS/tasks/999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_project_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "New Project",
                        "key": "NP",
                        "description": "Created via API test"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = body_to_string(response.into_body()).await;
    let project: Project = serde_json::from_str(&body).unwrap();
    assert_eq!(project.key, "NP");
}

#[tokio::test]
async fn test_create_duplicate_project_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "Duplicate",
                        "key": "DS",
                        "description": ""
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

// --- Delete project tests ---

#[tokio::test]
async fn test_delete_project_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/projects/DS")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_delete_project_not_found() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/projects/NOPE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_project_cascades() {
    let dir = tempdir().unwrap();
    let config = test_config(dir.path());
    let mut storage = Storage::new(&config.data_dir);
    seed_data(&mut storage);

    // Verify tasks exist before delete
    let tasks_before = storage.list_tasks("DS");
    assert!(!tasks_before.is_empty());

    storage.delete_project("DS").unwrap();

    // Project should be gone
    assert!(storage.get_project("DS").is_none());

    // Tasks should be gone
    let tasks_after = storage.list_tasks("DS");
    assert!(tasks_after.is_empty());
}

// --- New endpoint tests ---

#[tokio::test]
async fn test_health_check() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let health: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(health["status"], "ok");
    assert!(health["version"].is_string());
}

#[tokio::test]
async fn test_comments_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // Create a comment
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks/1/comments")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "author": "Test User",
                        "body": "Great progress!"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = body_to_string(response.into_body()).await;
    let comment: Comment = serde_json::from_str(&body).unwrap();
    assert_eq!(comment.author, "Test User");
    assert_eq!(comment.body, "Great progress!");

    // List comments
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS/tasks/1/comments")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let comments: Vec<Comment> = serde_json::from_str(&body).unwrap();
    assert_eq!(comments.len(), 1);
}

#[tokio::test]
async fn test_activity_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS/activity?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let activity: Vec<ActivityEntry> = serde_json::from_str(&body).unwrap();
    // Should have entries from seeding (project_created, task_created)
    assert!(!activity.is_empty());
}

#[tokio::test]
async fn test_dashboard_api() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let dashboard: DashboardData = serde_json::from_str(&body).unwrap();
    assert!(dashboard.status_counts.total > 0);
}

#[tokio::test]
async fn test_tasks_with_archived_filter() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // Default: non-archived only
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS/tasks?archived=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_string(response.into_body()).await;
    let tasks: Vec<Task> = serde_json::from_str(&body).unwrap();
    assert_eq!(tasks.len(), 7); // All seed tasks are non-archived
}

// --- Input Validation Tests ---

#[tokio::test]
async fn test_create_task_empty_title() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": "",
                        "description": "No title"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_to_string(response.into_body()).await;
    let error: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(error["error"], "bad_request");
}

#[tokio::test]
async fn test_create_task_whitespace_only_title() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": "   ",
                        "description": "Whitespace title"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_task_oversized_title() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let long_title = "x".repeat(501);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": long_title
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_comment_empty_body() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks/1/comments")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "author": "Test",
                        "body": ""
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_project_invalid_key() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "Bad Key Project",
                        "key": "A-B!",
                        "description": ""
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_project_key_too_long() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "Long Key Project",
                        "key": "ABCDEFGHIJK",
                        "description": ""
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// --- Malicious Payload Tests ---

#[tokio::test]
async fn test_xss_payload_in_task_title() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let xss_title = "<script>alert('xss')</script>";
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": xss_title
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Backend stores raw — XSS prevention is frontend's job
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_to_string(response.into_body()).await;
    let task: Task = serde_json::from_str(&body).unwrap();
    assert_eq!(task.title, xss_title);
}

#[tokio::test]
async fn test_sql_chars_in_fields() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": "Robert'; DROP TABLE tasks;--",
                        "description": "Test SQL injection"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Parameterized queries prevent SQL injection
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_to_string(response.into_body()).await;
    let task: Task = serde_json::from_str(&body).unwrap();
    assert_eq!(task.title, "Robert'; DROP TABLE tasks;--");
}

// --- Error Handling Tests ---

#[tokio::test]
async fn test_update_nonexistent_task() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/projects/DS/tasks/9999")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": "Ghost task"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // update_task returns Internal error when task not found (via anyhow)
    // After error.rs fix, internal errors get generic message
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_malformed_json_body() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not valid json"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Axum returns 400 or 422 for malformed JSON
    let status = response.status().as_u16();
    assert!(
        status == 400 || status == 422,
        "Expected 400 or 422, got {status}"
    );
}

// --- Boundary Tests ---

#[tokio::test]
async fn test_bulk_update_empty_ids() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks/bulk-update")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "task_ids": [],
                        "status": "done"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_bulk_delete_nonexistent_ids() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks/bulk-delete")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "task_ids": [9998, 9999]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Idempotent: deleting nonexistent IDs succeeds
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_create_task_with_too_many_labels() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let labels: Vec<String> = (0..21).map(|i| format!("label-{i}")).collect();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/DS/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "title": "Too many labels",
                        "labels": labels
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// --- Security Headers Test ---

#[tokio::test]
async fn test_response_has_security_headers() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let headers = response.headers();
    assert_eq!(
        headers.get("x-frame-options").map(|v| v.to_str().unwrap()),
        Some("DENY")
    );
    assert_eq!(
        headers
            .get("x-content-type-options")
            .map(|v| v.to_str().unwrap()),
        Some("nosniff")
    );
    assert_eq!(
        headers.get("referrer-policy").map(|v| v.to_str().unwrap()),
        Some("strict-origin-when-cross-origin")
    );
}

// --- Pagination Test ---

#[tokio::test]
async fn test_list_tasks_with_pagination() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // Request only 3 tasks
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS/tasks?limit=3&offset=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_to_string(response.into_body()).await;
    let tasks: Vec<Task> = serde_json::from_str(&body).unwrap();
    assert_eq!(tasks.len(), 3);

    // Request with offset
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/projects/DS/tasks?limit=3&offset=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_to_string(response.into_body()).await;
    let tasks: Vec<Task> = serde_json::from_str(&body).unwrap();
    assert_eq!(tasks.len(), 2); // 7 total, offset 5 → 2 remaining
}
