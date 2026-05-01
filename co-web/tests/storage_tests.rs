use co_web::models::*;
use co_web::storage::{Storage, seed_data};
use tempfile::tempdir;

#[test]
fn test_create_and_list_projects() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    let project = storage
        .create_project(CreateProject {
            name: "Test Project".into(),
            key: "TP".into(),
            description: "A test project".into(),
            ..Default::default()
        })
        .unwrap();

    assert_eq!(project.key, "TP");
    assert_eq!(project.name, "Test Project");
    assert_eq!(project.next_id, 1);
    assert!(!project.archived);

    let projects = storage.list_projects();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].key, "TP");
}

#[test]
fn test_duplicate_project_rejected() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "First".into(),
            key: "DUP".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    let result = storage.create_project(CreateProject {
        name: "Second".into(),
        key: "DUP".into(),
        description: "".into(),
        ..Default::default()
    });

    assert!(result.is_err());
}

#[test]
fn test_get_project() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Findable".into(),
            key: "FND".into(),
            description: "desc".into(),
            ..Default::default()
        })
        .unwrap();

    let found = storage.get_project("FND");
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Findable");

    let missing = storage.get_project("NOPE");
    assert!(missing.is_none());
}

#[test]
fn test_project_key_case_insensitive() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Case Test".into(),
            key: "ct".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    // Key should be uppercased
    let projects = storage.list_projects();
    assert_eq!(projects[0].key, "CT");

    // Lookup should work case-insensitively
    assert!(storage.get_project("CT").is_some());
}

#[test]
fn test_create_and_list_tasks() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Task Test".into(),
            key: "TT".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    let task = storage
        .create_task(
            "TT",
            CreateTask {
                title: "First task".into(),
                description: "Description here".into(),
                status: TaskStatus::Todo,
                priority: Priority::High,
                due_date: None,
                parent: None,
                labels: vec!["test".into()],
                assignee: None,
            },
        )
        .unwrap();

    assert_eq!(task.id, 1);
    assert_eq!(task.key, "TT-1");
    assert_eq!(task.project_key, "TT");
    assert_eq!(task.title, "First task");
    assert_eq!(task.status, TaskStatus::Todo);
    assert_eq!(task.priority, Priority::High);
    assert_eq!(task.labels, vec!["test".to_string()]);
    assert!(!task.archived);

    let tasks = storage.list_tasks("TT");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, 1);
}

#[test]
fn test_task_ids_monotonically_increase() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "ID Test".into(),
            key: "ID".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    let t1 = storage
        .create_task(
            "ID",
            CreateTask {
                title: "First".into(),
                description: "".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    let t2 = storage
        .create_task(
            "ID",
            CreateTask {
                title: "Second".into(),
                description: "".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    assert_eq!(t1.id, 1);
    assert_eq!(t2.id, 2);
    assert!(t2.id > t1.id);
}

#[test]
fn test_get_task() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Get Test".into(),
            key: "GT".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    storage
        .create_task(
            "GT",
            CreateTask {
                title: "Findable task".into(),
                description: "Body text".into(),
                status: TaskStatus::InProgress,
                priority: Priority::Critical,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    let found = storage.get_task("GT", 1);
    assert!(found.is_some());
    let task = found.unwrap();
    assert_eq!(task.title, "Findable task");
    assert_eq!(task.description, "Body text");
    assert_eq!(task.status, TaskStatus::InProgress);

    let missing = storage.get_task("GT", 999);
    assert!(missing.is_none());
}

#[test]
fn test_update_task() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Update Test".into(),
            key: "UT".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    storage
        .create_task(
            "UT",
            CreateTask {
                title: "Original".into(),
                description: "Original desc".into(),
                status: TaskStatus::Todo,
                priority: Priority::Low,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    let updated = storage
        .update_task(
            "UT",
            1,
            UpdateTask {
                title: Some("Updated".into()),
                status: Some(TaskStatus::Done),
                priority: Some(Priority::High),
                description: None,
                due_date: None,
                parent: None,
                labels: Some(vec!["done".into()]),
                archived: None,
                assignee: None,
            },
        )
        .unwrap();

    assert_eq!(updated.title, "Updated");
    assert_eq!(updated.status, TaskStatus::Done);
    assert_eq!(updated.priority, Priority::High);
    assert_eq!(updated.description, "Original desc"); // unchanged
    assert_eq!(updated.labels, vec!["done".to_string()]);

    // Verify persisted
    let reloaded = storage.get_task("UT", 1).unwrap();
    assert_eq!(reloaded.title, "Updated");
    assert_eq!(reloaded.status, TaskStatus::Done);
}

#[test]
fn test_delete_task() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Delete Test".into(),
            key: "DT".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    storage
        .create_task(
            "DT",
            CreateTask {
                title: "To delete".into(),
                description: "".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    assert!(storage.get_task("DT", 1).is_some());

    storage.delete_task("DT", 1).unwrap();

    assert!(storage.get_task("DT", 1).is_none());
    assert_eq!(storage.list_tasks("DT").len(), 0);
}

#[test]
fn test_delete_nonexistent_task_errors() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Test".into(),
            key: "DNE".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    let result = storage.delete_task("DNE", 999);
    assert!(result.is_err());
}

#[test]
fn test_seed_data() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    seed_data(&mut storage);

    let projects = storage.list_projects();
    assert_eq!(projects.len(), 3);

    let ds = projects.iter().find(|p| p.key == "DS").unwrap();
    assert_eq!(ds.name, "Design System");

    let api = projects.iter().find(|p| p.key == "API").unwrap();
    assert_eq!(api.name, "Backend API");

    let plt = projects.iter().find(|p| p.key == "PLT").unwrap();
    assert_eq!(plt.name, "Platform");

    let ds_tasks = storage.list_tasks("DS");
    assert_eq!(ds_tasks.len(), 7);

    let api_tasks = storage.list_tasks("API");
    assert_eq!(api_tasks.len(), 8);

    let plt_tasks = storage.list_tasks("PLT");
    assert_eq!(plt_tasks.len(), 3);

    // Verify parent relationships
    let subtask = ds_tasks
        .iter()
        .find(|t| t.title == "Select color palette")
        .unwrap();
    assert_eq!(subtask.parent, Some(1));
}

#[test]
fn test_task_with_parent() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Parent Test".into(),
            key: "PT".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    let parent = storage
        .create_task(
            "PT",
            CreateTask {
                title: "Parent".into(),
                description: "".into(),
                status: TaskStatus::Todo,
                priority: Priority::High,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    let child = storage
        .create_task(
            "PT",
            CreateTask {
                title: "Child".into(),
                description: "".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: Some(parent.id),
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    assert_eq!(child.parent, Some(1));

    let reloaded = storage.get_task("PT", 2).unwrap();
    assert_eq!(reloaded.parent, Some(1));
}

// --- New tests for production features ---

#[test]
fn test_has_data() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());

    assert!(!storage.has_data());

    drop(storage);

    let mut storage = Storage::new(dir.path());
    seed_data(&mut storage);
    assert!(storage.has_data());
}

#[test]
fn test_comments() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Comment Test".into(),
            key: "CM".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    storage
        .create_task(
            "CM",
            CreateTask {
                title: "Commentable".into(),
                description: "".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    // No comments initially
    let comments = storage.list_comments("CM", 1);
    assert_eq!(comments.len(), 0);

    // Add comment
    let c1 = storage
        .create_comment(
            "CM",
            1,
            CreateComment {
                author: "Alice".into(),
                body: "First comment".into(),
            },
        )
        .unwrap();

    assert_eq!(c1.author, "Alice");
    assert_eq!(c1.body, "First comment");
    assert_eq!(c1.task_id, 1);

    // Add second comment
    storage
        .create_comment(
            "CM",
            1,
            CreateComment {
                author: "Bob".into(),
                body: "Second comment".into(),
            },
        )
        .unwrap();

    let comments = storage.list_comments("CM", 1);
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].author, "Alice");
    assert_eq!(comments[1].author, "Bob");
}

#[test]
fn test_activity_log() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Activity Test".into(),
            key: "AC".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    storage
        .create_task(
            "AC",
            CreateTask {
                title: "Tracked task".into(),
                description: "".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    // activity_log was dropped in migration v13 — list_activity returns empty
    let activity = storage.list_activity("AC", 50);
    assert!(activity.is_empty());
}

#[test]
fn test_archive_task() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Archive Test".into(),
            key: "AR".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    storage
        .create_task(
            "AR",
            CreateTask {
                title: "To archive".into(),
                description: "".into(),
                status: TaskStatus::Done,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    // Archive it
    storage
        .update_task(
            "AR",
            1,
            UpdateTask {
                archived: Some(true),
                title: None,
                description: None,
                status: None,
                priority: None,
                due_date: None,
                parent: None,
                labels: None,
                assignee: None,
            },
        )
        .unwrap();

    // Default list excludes archived
    let tasks = storage.list_tasks("AR");
    assert_eq!(tasks.len(), 0);

    // Include archived
    let all_tasks = storage.list_tasks_filtered("AR", None);
    assert_eq!(all_tasks.len(), 1);
    assert!(all_tasks[0].archived);
}

#[test]
fn test_bulk_operations() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Bulk Test".into(),
            key: "BK".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    for i in 0..3 {
        storage
            .create_task(
                "BK",
                CreateTask {
                    title: format!("Task {}", i + 1),
                    description: "".into(),
                    status: TaskStatus::Todo,
                    priority: Priority::Medium,
                    due_date: None,
                    parent: None,
                    labels: vec![],
                    assignee: None,
                },
            )
            .unwrap();
    }

    // Bulk update status
    let updated = storage
        .bulk_update_tasks(
            "BK",
            BulkUpdateTasks {
                task_ids: vec![1, 2],
                status: Some(TaskStatus::Done),
                archived: None,
            },
        )
        .unwrap();

    assert_eq!(updated.len(), 2);
    assert!(updated.iter().all(|t| t.status == TaskStatus::Done));

    // Task 3 unchanged
    let t3 = storage.get_task("BK", 3).unwrap();
    assert_eq!(t3.status, TaskStatus::Todo);

    // Bulk delete
    storage
        .bulk_delete_tasks(
            "BK",
            BulkDeleteTasks {
                task_ids: vec![1, 3],
            },
        )
        .unwrap();

    let remaining = storage.list_tasks("BK");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, 2);
}

#[test]
fn test_dashboard() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    seed_data(&mut storage);

    let dashboard = storage.get_dashboard("DS");
    assert!(dashboard.status_counts.total > 0);
    assert_eq!(
        dashboard.status_counts.total,
        dashboard.status_counts.todo
            + dashboard.status_counts.in_progress
            + dashboard.status_counts.in_review
            + dashboard.status_counts.done
    );
}

// --- Phase 9: New Edge Case Tests ---

#[test]
fn test_create_task_unicode_title() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Unicode Test".into(),
            key: "UC".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    let task = storage
        .create_task(
            "UC",
            CreateTask {
                title: "日本語タスク 🎉 Ñoño".into(),
                description: "Descrição com acentos: é, ã, ü, ç".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec!["日本語".into()],
                assignee: None,
            },
        )
        .unwrap();

    assert_eq!(task.title, "日本語タスク 🎉 Ñoño");
    assert_eq!(task.description, "Descrição com acentos: é, ã, ü, ç");

    let reloaded = storage.get_task("UC", 1).unwrap();
    assert_eq!(reloaded.title, "日本語タスク 🎉 Ñoño");
    assert_eq!(reloaded.labels, vec!["日本語".to_string()]);
}

#[test]
fn test_create_task_special_chars() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Special".into(),
            key: "SP".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    let task = storage
        .create_task(
            "SP",
            CreateTask {
                title: "Task with \"quotes\" & <angle> brackets".into(),
                description: "Line1\nLine2\tTabbed".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec!["a&b".into(), "c<d".into()],
                assignee: None,
            },
        )
        .unwrap();

    assert_eq!(task.title, "Task with \"quotes\" & <angle> brackets");

    let reloaded = storage.get_task("SP", 1).unwrap();
    assert_eq!(reloaded.title, task.title);
    assert_eq!(reloaded.labels, vec!["a&b".to_string(), "c<d".to_string()]);
}

#[test]
fn test_activity_log_limit_cap() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Limit Test".into(),
            key: "LT".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    // Create many tasks to generate activity entries
    for i in 0..15 {
        storage
            .create_task(
                "LT",
                CreateTask {
                    title: format!("Task {i}"),
                    description: "".into(),
                    status: TaskStatus::Todo,
                    priority: Priority::Medium,
                    due_date: None,
                    parent: None,
                    labels: vec![],
                    assignee: None,
                },
            )
            .unwrap();
    }

    // activity_log was dropped in migration v13 — list_activity returns empty
    let activity = storage.list_activity("LT", 5);
    assert!(activity.is_empty());
}

#[test]
fn test_list_tasks_with_pagination() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "Pagination Test".into(),
            key: "PG".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    for i in 0..10 {
        storage
            .create_task(
                "PG",
                CreateTask {
                    title: format!("Task {}", i + 1),
                    description: "".into(),
                    status: TaskStatus::Todo,
                    priority: Priority::Medium,
                    due_date: None,
                    parent: None,
                    labels: vec![],
                    assignee: None,
                },
            )
            .unwrap();
    }

    // First page
    let page1 = storage.list_tasks_paginated("PG", Some(false), 3, 0);
    assert_eq!(page1.len(), 3);
    assert_eq!(page1[0].id, 1);
    assert_eq!(page1[2].id, 3);

    // Second page
    let page2 = storage.list_tasks_paginated("PG", Some(false), 3, 3);
    assert_eq!(page2.len(), 3);
    assert_eq!(page2[0].id, 4);

    // Last page (partial)
    let page4 = storage.list_tasks_paginated("PG", Some(false), 3, 9);
    assert_eq!(page4.len(), 1);
    assert_eq!(page4[0].id, 10);

    // Beyond data
    let empty = storage.list_tasks_paginated("PG", Some(false), 3, 100);
    assert_eq!(empty.len(), 0);

    // Limit capped at 500
    let all = storage.list_tasks_paginated("PG", Some(false), 1000, 0);
    assert_eq!(all.len(), 10);
}

#[test]
fn test_schema_version_tracking() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path());

    assert_eq!(storage.schema_version(), 28); // CO-124 added v28 (last migration)
    assert_eq!(storage.schema_version(), 28);

    // Creating a second Storage instance on same dir should not re-run migrations
    drop(storage);
    let storage2 = Storage::new(dir.path());
    assert_eq!(storage2.schema_version(), 28); // CO-124 added v28 (last migration)
    assert_eq!(storage2.schema_version(), 28);
}

#[test]
fn test_delete_project_cascade() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    storage
        .create_project(CreateProject {
            name: "To Delete".into(),
            key: "DEL".into(),
            description: "".into(),
            ..Default::default()
        })
        .unwrap();

    storage
        .create_task(
            "DEL",
            CreateTask {
                title: "Task to cascade".into(),
                description: "".into(),
                status: TaskStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                parent: None,
                labels: vec![],
                assignee: None,
            },
        )
        .unwrap();

    storage
        .create_comment(
            "DEL",
            1,
            CreateComment {
                body: "Comment to cascade".into(),
                author: "tester".into(),
            },
        )
        .unwrap();

    assert!(storage.get_project("DEL").is_some());
    assert_eq!(storage.list_tasks("DEL").len(), 1);
    assert_eq!(storage.list_comments("DEL", 1).len(), 1);

    storage.delete_project("DEL").unwrap();

    assert!(storage.get_project("DEL").is_none());
    assert_eq!(storage.list_tasks("DEL").len(), 0);
    assert_eq!(storage.list_comments("DEL", 1).len(), 0);
}

#[test]
fn test_delete_project_not_found() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    let result = storage.delete_project("NOPE");
    assert!(result.is_err());
}
